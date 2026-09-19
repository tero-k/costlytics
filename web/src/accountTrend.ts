/**
 * Cost-over-time trend chart for the Account Detail page's selected account
 * (plan §26). Renders an ECharts line/area chart into a `chart-area` element
 * nested under `#account-trend`, showing total cost over time for the
 * currently selected account (`accounts: [selectedAccount]`), ungrouped
 * (`group_by` omitted — the account is already known from the filter, no
 * need to re-group by it) — mirrors `trendChart.ts`'s (Overview page) /
 * `serviceTrend.ts`'s (Service Detail page) pattern, including
 * auto-granularity derived from the selected range's length
 * (`shared/granularity.ts`).
 *
 * Unlike the Overview page, this page's HTML (`account-detail.html`) has no
 * static chart-header/chart-area markup to fill — `#account-trend` is an
 * empty placeholder `div` — so this module builds that shell itself on
 * first render, the same way `accountKpi.ts` builds its own `.kpi-grid`.
 *
 * Re-fetches whenever the account picker or the shared date/metric controls
 * change. When no account is selected, the component clears itself and
 * skips fetching, per this page's "nothing to fetch yet" convention.
 */

import { getTimeseries, type TimeGranularity, type TimeSeriesPoint } from './api.ts';
import { getSelectedAccount } from './shared/accountPicker.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { autoGranularity, formatPeriodLabel } from './shared/granularity.ts';

const CONTAINER_ID = 'account-trend';
const CHART_ID = 'account-trend-chart';

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

function getChartArea(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CHART_ID}`);
}

function renderShell(container: HTMLElement): void {
  container.innerHTML = `
    <section class="chart-section">
      <div class="chart-header">
        <h2>Cost trend</h2>
      </div>
      <div class="chart-area" id="${CHART_ID}"></div>
    </section>`;
}

const refreshGuard = new RequestGuard();

async function loadTrendChart(
  account: string,
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const chartArea = getChartArea();
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
      accounts: [account],
      // group_by omitted -> a single ungrouped series for this account
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.series.length === 0) {
      showOverlay(CHART_ID, chartArea, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(result.currency);
    renderChart(chartArea, result.series, result.currency, granularity);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    showOverlay(CHART_ID, chartArea, 'chart-error', errorMessage(err));
  }
}

function renderChart(
  chartArea: HTMLElement,
  series: TimeSeriesPoint[],
  currency: string,
  granularity: TimeGranularity,
): void {
  const instance = ensureChart(CHART_ID, chartArea);

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

export function initAccountTrend(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  renderShell(container);

  const refresh = async (): Promise<void> => {
    const account = getSelectedAccount();
    if (!account) {
      // Clear any previously rendered chart in place, rather than tearing
      // down and rebuilding the `.chart-area` DOM node on every deselect —
      // `ensureChart`'s instance cache is keyed by container id and would
      // otherwise end up pointing at a detached element.
      const chartArea = getChartArea();
      if (chartArea) {
        showOverlay(CHART_ID, chartArea, 'chart-empty', 'Select an account to view its cost trend.');
      }
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadTrendChart(account, controls, token, onCurrency);
  };

  subscribeToControls(refresh, { extraIds: ['#account-picker'] });

  return refresh();
}
