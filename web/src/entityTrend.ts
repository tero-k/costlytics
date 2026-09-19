/**
 * Generic cost-over-time trend chart for a "detail page" entity (plan §26),
 * e.g. the Service Detail or Account Detail page's currently selected
 * service/account. Renders an ECharts line/area chart into a `chart-area`
 * element nested under `#{idPrefix}-trend`, showing total cost over time for
 * the currently selected entity (`{filterKey}: [selected]`), ungrouped
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

import { getTimeseries, type TimeGranularity, type TimeSeriesPoint } from './api.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { autoGranularity, formatPeriodLabel } from './shared/granularity.ts';
import type { EntityConfig } from './shared/entityConfig.ts';

function getContainer(idPrefix: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${idPrefix}-trend`);
}

function getChartArea(chartId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${chartId}`);
}

function renderShell(container: HTMLElement, chartId: string): void {
  container.innerHTML = `
    <section class="chart-section">
      <div class="chart-header">
        <h2>Cost trend</h2>
      </div>
      <div class="chart-area" id="${chartId}"></div>
    </section>`;
}

function renderChart(
  chartId: string,
  chartArea: HTMLElement,
  series: TimeSeriesPoint[],
  currency: string,
  granularity: TimeGranularity,
): void {
  const instance = ensureChart(chartId, chartArea);

  const sorted = [...series].sort((a, b) => a.period.localeCompare(b.period));
  const categories = sorted.map((point) => formatPeriodLabel(point.period, granularity));
  const totals = sorted.map((point) => point.total);
  const rowCounts = sorted.map((point) => point.row_count);

  instance.setOption(
    {
      tooltip: {
        trigger: 'axis',
        formatter: (params: unknown) => {
          const items = Array.isArray(params) ? params : [params];
          const first = items[0] as { dataIndex: number } | undefined;
          if (!first) return '';
          const idx = first.dataIndex;
          const point = sorted[idx];
          if (!point) return '';
          // `point.period` is currently server-generated (not attacker
          // influenceable), but escape it anyway so this HTML-building
          // idiom stays safe by construction rather than by accident.
          return [
            `<strong>${escapeHtml(point.period)}</strong>`,
            `Total: ${formatCurrency(point.total, currency)}`,
            `Rows: ${rowCounts[idx]}`,
          ].join('<br/>');
        },
      },
      grid: {
        left: 8,
        right: 16,
        top: 24,
        bottom: 8,
        containLabel: true,
      },
      xAxis: {
        type: 'category',
        data: categories,
        boundaryGap: false,
      },
      yAxis: {
        type: 'value',
        axisLabel: {
          formatter: (value: number) => formatCurrencyCompact(value, currency),
        },
      },
      series: [
        {
          type: 'line',
          data: totals,
          smooth: false,
          showSymbol: sorted.length <= 60,
          areaStyle: {
            opacity: 0.15,
          },
          lineStyle: {
            width: 2,
          },
        },
      ],
    },
    true,
  );
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initEntityTrend(
  config: EntityConfig,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const container = getContainer(config.idPrefix);
  if (!container) return Promise.resolve();

  const chartId = `${config.idPrefix}-trend-chart`;
  const refreshGuard = new RequestGuard();

  renderShell(container, chartId);

  async function loadTrendChart(selected: string, controls: Controls, token: number): Promise<void> {
    const chartArea = getChartArea(chartId);
    if (!chartArea) return;

    clearOverlays(chartArea);

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
        [config.filterKey]: [selected],
        // group_by omitted -> a single ungrouped series for this entity
      });

      if (!refreshGuard.isCurrent(token)) return;

      if (result.series.length === 0) {
        showOverlay(chartId, chartArea, 'chart-empty', 'No data for this period.');
        return;
      }

      onCurrency?.(result.currency);
      renderChart(chartId, chartArea, result.series, result.currency, granularity);
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

  return refresh();
}
