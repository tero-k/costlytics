/**
 * Top services / top accounts horizontal bar charts for the Overview page
 * (plan §21-22).
 *
 * Renders two independent ECharts horizontal bar charts, one for
 * `dimension: "service"` into `#top-services` and one for
 * `dimension: "account"` into `#top-accounts`. Both fetch
 * `POST /api/v1/cost/breakdown` with `limit: 10` using the page's shared
 * date range / metric controls (see `kpiCards.ts` / `trendChart.ts` for the
 * same `readControls()` convention), and both add a synthetic "Other" bar
 * computed from an overall `summary()` total minus the sum of the returned
 * top-10 rows, when that remainder is non-trivial.
 *
 * `key: null` rows (untagged/uncategorized data) are labeled "(none)"
 * rather than left blank.
 *
 * No click/drill-down behavior this session — bars render, nothing else.
 */

import * as echarts from 'echarts';
import {
  ApiError,
  getBreakdown,
  getSummary,
  type BreakdownRow,
  type CostMetric,
  type Dimension,
} from './api.ts';

// ---------------------------------------------------------------------------
// Config: one entry per chart
// ---------------------------------------------------------------------------

interface ChartDef {
  containerId: string;
  dimension: Dimension;
}

const CHART_DEFS: ChartDef[] = [
  { containerId: 'top-services', dimension: 'service' },
  { containerId: 'top-accounts', dimension: 'account' },
];

/** Below this fraction of the overall total, the "Other" bucket is omitted as negligible. */
const OTHER_EPSILON_FRACTION = 0.001;

// ---------------------------------------------------------------------------
// Date helpers (UTC-based, mirroring kpiCards.ts / trendChart.ts conventions)
// ---------------------------------------------------------------------------

function addDaysIso(dateIso: string, days: number): string {
  const d = new Date(`${dateIso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
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

function formatKeyLabel(key: string | null): string {
  return key === null || key === '' ? '(none)' : key;
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

// ---------------------------------------------------------------------------
// Chart state / DOM helpers (one ECharts instance per chart def)
// ---------------------------------------------------------------------------

const chartInstances = new Map<string, echarts.ECharts>();

function getContainer(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}`);
}

function clearOverlays(container: HTMLElement): void {
  container.querySelectorAll('.chart-empty, .chart-error').forEach((el) => el.remove());
}

function showOverlay(
  containerId: string,
  container: HTMLElement,
  className: 'chart-empty' | 'chart-error',
  message: string,
): void {
  clearOverlays(container);
  chartInstances.get(containerId)?.clear();
  const overlay = document.createElement('div');
  overlay.className = className;
  overlay.textContent = message;
  container.appendChild(overlay);
}

function ensureChart(containerId: string, container: HTMLElement): echarts.ECharts {
  let instance = chartInstances.get(containerId);
  if (!instance || instance.isDisposed()) {
    instance = echarts.init(container);
    chartInstances.set(containerId, instance);
    window.addEventListener('resize', () => instance?.resize());
  }
  return instance;
}

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadChart(def: ChartDef, controls: Controls): Promise<void> {
  const container = getContainer(def.containerId);
  if (!container) return;

  clearOverlays(container);

  const start = controls.startIso;
  const end = addDaysIso(controls.endIsoInclusive, 1);

  try {
    const [breakdown, summary] = await Promise.all([
      getBreakdown({
        start,
        end,
        metric: controls.metric,
        dimension: def.dimension,
        limit: 10,
      }),
      getSummary({
        start,
        end,
        metric: controls.metric,
      }),
    ]);

    if (breakdown.rows.length === 0) {
      showOverlay(def.containerId, container, 'chart-empty', 'No data for this period.');
      return;
    }

    const rows: Array<{ label: string; total: number }> = breakdown.rows.map((row: BreakdownRow) => ({
      label: formatKeyLabel(row.key),
      total: row.total,
    }));

    const sumOfRows = breakdown.rows.reduce((acc, row) => acc + row.total, 0);
    const remainder = summary.total - sumOfRows;
    if (remainder > summary.total * OTHER_EPSILON_FRACTION) {
      rows.push({ label: 'Other', total: remainder });
    }

    renderChart(def.containerId, container, rows, breakdown.currency);
  } catch (err) {
    showOverlay(def.containerId, container, 'chart-error', errorMessage(err));
  }
}

function renderChart(
  containerId: string,
  container: HTMLElement,
  rows: Array<{ label: string; total: number }>,
  currency: string,
): void {
  const instance = ensureChart(containerId, container);

  // ECharts renders horizontal bar category axes bottom-to-top, so reverse
  // to keep the highest-cost row at the top of the chart.
  const reversed = [...rows].reverse();
  const categories = reversed.map((row) => row.label);
  const totals = reversed.map((row) => row.total);

  instance.setOption(
    {
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        formatter: (params: unknown) => {
          const items = Array.isArray(params) ? params : [params];
          const first = items[0] as { dataIndex: number } | undefined;
          if (!first) return '';
          const row = reversed[first.dataIndex];
          if (!row) return '';
          return [`<strong>${row.label}</strong>`, formatCurrency(row.total, currency)].join('<br/>');
        },
      },
      grid: {
        left: 8,
        right: 24,
        top: 16,
        bottom: 8,
        containLabel: true,
      },
      xAxis: {
        type: 'value',
        axisLabel: {
          formatter: (value: number) => formatCurrencyCompact(value, currency),
        },
      },
      yAxis: {
        type: 'category',
        data: categories,
      },
      series: [
        {
          type: 'bar',
          data: totals,
          itemStyle: {
            borderRadius: [0, 4, 4, 0],
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

export function initTopBreakdownCharts(): void {
  const defs = CHART_DEFS.filter((def) => getContainer(def.containerId) !== null);
  if (defs.length === 0) return;

  const refresh = (): void => {
    const controls = readControls();
    if (!controls) return;
    for (const def of defs) {
      void loadChart(def, controls);
    }
  };

  document.querySelector('#date-start')?.addEventListener('change', refresh);
  document.querySelector('#date-end')?.addEventListener('change', refresh);
  document.querySelector('#metric-select')?.addEventListener('change', refresh);

  refresh();
}
