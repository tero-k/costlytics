/**
 * Cost trend chart for the Overview page (plan §20).
 *
 * Renders an ECharts line/area chart into `#trend-chart`, showing total cost
 * over time for the page's selected date range and metric, ungrouped
 * (`group_by` omitted — a single aggregate series). A granularity control
 * (`#granularity-select`) lets the user pick Day/Month/Year explicitly, or
 * leave it on "Auto", in which case the granularity is derived from the
 * selected range's length per the plan's rule:
 *   - <= 90 days  -> day
 *   - <= 3 years  -> month
 *   - > 3 years   -> year
 *
 * Shares the same `#date-start` / `#date-end` / `#metric-select` controls
 * that `kpiCards.ts` reads from — this component does not maintain its own
 * date-range/metric state, only its own granularity override.
 */

import { getTimeseries, type TimeGranularity, type TimeSeriesPoint } from './api.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { autoGranularity, formatPeriodLabel } from './shared/granularity.ts';

const CONTAINER_ID = 'trend-chart';

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function readGranularitySelection(): TimeGranularity | 'auto' {
  const select = document.querySelector<HTMLSelectElement>('#granularity-select');
  const value = select?.value;
  if (value === 'day' || value === 'month' || value === 'year') return value;
  return 'auto';
}

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadTrendChart(
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const container = getContainer();
  if (!container) return;

  clearOverlays(container);

  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const durationDays = daysBetweenIso(currentStart, currentEnd);

  const selection = readGranularitySelection();
  const granularity: TimeGranularity = selection === 'auto' ? autoGranularity(durationDays) : selection;

  try {
    const result = await getTimeseries({
      start: currentStart,
      end: currentEnd,
      metric: controls.metric,
      granularity,
      // group_by omitted -> a single ungrouped total-over-time series
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.series.length === 0) {
      showOverlay(CONTAINER_ID, container, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(result.currency);
    renderChart(container, result.series, result.currency, granularity);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    showOverlay(CONTAINER_ID, container, 'chart-error', errorMessage(err));
  }
}

function renderChart(
  container: HTMLElement,
  series: TimeSeriesPoint[],
  currency: string,
  granularity: TimeGranularity,
): void {
  const instance = ensureChart(CONTAINER_ID, container);

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

export function initTrendChart(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadTrendChart(controls, token, onCurrency);
  };

  document.querySelector('#date-start')?.addEventListener('change', () => void refresh());
  document.querySelector('#date-end')?.addEventListener('change', () => void refresh());
  document.querySelector('#metric-select')?.addEventListener('change', () => void refresh());
  document.querySelector('#granularity-select')?.addEventListener('change', () => void refresh());

  return refresh();
}
