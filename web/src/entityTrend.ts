/**
 * Generic cost-over-time trend chart for a "detail page" entity (plan §26),
 * e.g. the Service Detail or Account Detail page's currently selected
 * service/account. Renders an ECharts line/area chart into a `chart-area`
 * element nested under `#{idPrefix}-trend`, showing total cost over time for
 * the currently selected entity (`config.buildFilter(selected)`), ungrouped
 * (`group_by` omitted — the entity is already known from the filter, no need
 * to re-group by it) — mirrors `trendChart.ts`'s (Overview page) pattern,
 * including auto-granularity derived from the selected range's length
 * (`shared/granularity.ts`).
 *
 * Generalized from `serviceTrend.ts`/`accountTrend.ts` (Session 11) once
 * Session 10's whole-branch review found those two modules had zero code
 * differences beyond entity-noun substitution — see `shared/entityConfig.ts`'s
 * doc comment for the substitution points this module parameterizes on.
 *
 * These pages' HTML has no static chart-header/chart-area markup to fill —
 * `#{idPrefix}-trend` is an empty placeholder `div` — so this module builds
 * that shell itself on first render, the same way `entityKpi.ts` builds its
 * own `.kpi-grid`.
 *
 * Re-fetches whenever the entity picker or the shared date/metric controls
 * change. When no entity is selected, the component clears itself and skips
 * fetching, per this page family's "nothing to fetch yet" convention.
 */

import { getTimeseries, type TimeGranularity } from './api.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { errorMessage, formatCurrency } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, initChartTypeToggle, showChartLoading } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { autoGranularity } from './shared/granularity.ts';
import { drawCostTrend } from './shared/trendOptions.ts';
import type { EntityConfig } from './shared/entityConfig.ts';
import { setKpiCredits, setKpiSparkline } from './shared/kpiCard.ts';

function getContainer(idPrefix: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${idPrefix}-trend`);
}

function getChartArea(chartId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${chartId}`);
}

function renderShell(container: HTMLElement, chartId: string): void {
  container.innerHTML = `
    <section class="chart-section card">
      <div class="chart-header">
        <h2>Cost trend</h2>
        <div class="segmented" id="${chartId}-type" role="group" aria-label="Chart type">
          <button type="button" class="active" data-chart-type="line">Line</button>
          <button type="button" data-chart-type="bar">Bar</button>
        </div>
      </div>
      <div class="chart-area" id="${chartId}"></div>
    </section>`;
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/** Handle returned by `initEntityTrend`: a stable, reusable `refresh()` that reuses the same `RequestGuard` and does NOT re-register control listeners on repeat calls. */
export interface EntityTrendHandle {
  refresh: () => Promise<void>;
}

export function initEntityTrend(
  config: EntityConfig,
  onCurrency?: (currency: string) => void,
): EntityTrendHandle {
  // The trend's series doubles as the total KPI card's sparkline and the
  // credits card (`entityKpi.ts`'s `#{idPrefix}-kpi-total` / `-kpi-credits`)
  // — no extra query.
  const sparkId = `${config.idPrefix}-kpi-total`;
  const creditsId = `${config.idPrefix}-kpi-credits`;
  const container = getContainer(config.idPrefix);
  if (!container) return { refresh: () => Promise.resolve() };

  const chartId = `${config.idPrefix}-trend-chart`;
  const refreshGuard = new RequestGuard();

  renderShell(container, chartId);
  const getChartType = initChartTypeToggle(`${chartId}-type`, chartId);

  async function loadTrendChart(selected: string, controls: Controls, token: number): Promise<void> {
    const chartArea = getChartArea(chartId);
    if (!chartArea) return;

    clearOverlays(chartArea);
    showChartLoading(chartArea);

    const currentStart = controls.startIso;
    const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
    const durationDays = daysBetweenIso(currentStart, currentEnd);
    const granularity: TimeGranularity = autoGranularity(durationDays);

    try {
      const result = await getTimeseries({
        start: currentStart,
        end: currentEnd,
        metric: controls.metric,
        granularity,
        ...config.buildFilter(selected),
        // Grouped by charge category (still one query) so credits can be
        // drawn apart from charges — see `shared/credits.ts`.
        group_by: 'charge_category',
      });

      if (!refreshGuard.isCurrent(token)) return;

      if (result.series.length === 0) {
        setKpiSparkline(sparkId, []);
        setKpiCredits(creditsId, null, () => '');
        showOverlay(chartId, chartArea, 'chart-empty', 'No data for this period.');
        return;
      }

      onCurrency?.(result.currency);
      const split = drawCostTrend(chartId, chartArea, result.series, result.currency, granularity, getChartType);
      setKpiSparkline(sparkId, split.net);
      setKpiCredits(creditsId, split.totals, (v) => formatCurrency(v, result.currency));
    } catch (err) {
      if (!refreshGuard.isCurrent(token)) return;
      showOverlay(chartId, chartArea, 'chart-error', errorMessage(err));
    }
  }

  const refresh = async (): Promise<void> => {
    const selected = config.getSelected();
    if (!selected) {
      refreshGuard.next();
      // Clear any previously rendered chart in place, rather than tearing
      // down and rebuilding the `.chart-area` DOM node on every deselect —
      // `ensureChart`'s instance cache is keyed by container id and would
      // otherwise end up pointing at a detached element.
      const chartArea = getChartArea(chartId);
      if (chartArea) {
        showOverlay(
          chartId,
          chartArea,
          'chart-empty',
          `Select a ${config.entityNoun} to view its cost trend.`,
        );
      }
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadTrendChart(selected, controls, token);
  };

  subscribeToControls(refresh, { extraIds: [config.pickerSelector] });

  return { refresh };
}
