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

import * as echarts from 'echarts';
import { ApiError, getTimeseries, type CostMetric, type TimeGranularity, type TimeSeriesPoint } from './api.ts';

// ---------------------------------------------------------------------------
// Date helpers (UTC-based, mirroring kpiCards.ts conventions)
// ---------------------------------------------------------------------------

function addDaysIso(dateIso: string, days: number): string {
  const d = new Date(`${dateIso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function daysBetweenIso(startIso: string, endIsoExclusive: string): number {
  const start = new Date(`${startIso}T00:00:00Z`).getTime();
  const end = new Date(`${endIsoExclusive}T00:00:00Z`).getTime();
  return Math.round((end - start) / 86_400_000);
}

/** Auto-select granularity per plan §20's rule, from the range length in days. */
function autoGranularity(durationDays: number): TimeGranularity {
  if (durationDays <= 90) return 'day';
  if (durationDays <= 366 * 3) return 'month';
  return 'year';
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function formatCurrency(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(value);
  } catch {
    return `${value.toFixed(2)} ${currency}`;
  }
}

function formatCurrencyCompact(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      notation: 'compact',
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return `${value.toFixed(0)} ${currency}`;
  }
}

function formatPeriodLabel(periodIso: string, granularity: TimeGranularity): string {
  const date = new Date(periodIso);
  if (Number.isNaN(date.getTime())) return periodIso;

  switch (granularity) {
    case 'day':
      return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' }).format(date);
    case 'month':
      return new Intl.DateTimeFormat(undefined, { month: 'short', year: 'numeric', timeZone: 'UTC' }).format(date);
    case 'year':
      return new Intl.DateTimeFormat(undefined, { year: 'numeric', timeZone: 'UTC' }).format(date);
  }
}

function errorMessage(err: unknown): string {
  if (err instanceof ApiError) return err.message;
  if (err instanceof Error) return err.message;
  return 'Unknown error';
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

interface Controls {
  metric: CostMetric;
  startIso: string;
  endIsoInclusive: string;
}

function readControls(): Controls | null {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const metricSelect = document.querySelector<HTMLSelectElement>('#metric-select');
  if (!startInput?.value || !endInput?.value) return null;

  return {
    metric: (metricSelect?.value as CostMetric | undefined) ?? 'amortized',
    startIso: startInput.value,
    endIsoInclusive: endInput.value,
  };
}

function readGranularitySelection(): TimeGranularity | 'auto' {
  const select = document.querySelector<HTMLSelectElement>('#granularity-select');
  const value = select?.value;
  if (value === 'day' || value === 'month' || value === 'year') return value;
  return 'auto';
}

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

let chart: echarts.ECharts | null = null;

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>('#trend-chart');
}

function clearOverlays(container: HTMLElement): void {
  container.querySelectorAll('.chart-empty, .chart-error').forEach((el) => el.remove());
}

function showOverlay(container: HTMLElement, className: 'chart-empty' | 'chart-error', message: string): void {
  clearOverlays(container);
  chart?.clear();
  const overlay = document.createElement('div');
  overlay.className = className;
  overlay.textContent = message;
  container.appendChild(overlay);
}

function ensureChart(container: HTMLElement): echarts.ECharts {
  if (!chart || chart.isDisposed()) {
    chart = echarts.init(container);
    window.addEventListener('resize', () => chart?.resize());
  }
  return chart;
}

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadTrendChart(controls: Controls): Promise<void> {
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

    if (result.series.length === 0) {
      showOverlay(container, 'chart-empty', 'No data for this period.');
      return;
    }

    renderChart(container, result.series, result.currency, granularity);
  } catch (err) {
    showOverlay(container, 'chart-error', errorMessage(err));
  }
}

function renderChart(
  container: HTMLElement,
  series: TimeSeriesPoint[],
  currency: string,
  granularity: TimeGranularity,
): void {
  const instance = ensureChart(container);

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
          return [
            `<strong>${point.period}</strong>`,
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

export function initTrendChart(): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    await loadTrendChart(controls);
  };

  document.querySelector('#date-start')?.addEventListener('change', () => void refresh());
  document.querySelector('#date-end')?.addEventListener('change', () => void refresh());
  document.querySelector('#metric-select')?.addEventListener('change', () => void refresh());
  document.querySelector('#granularity-select')?.addEventListener('change', () => void refresh());

  return refresh();
}
