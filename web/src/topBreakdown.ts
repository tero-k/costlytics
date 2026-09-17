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

import { getBreakdown, getSummary, type BreakdownRow, type Dimension } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { formatKeyLabel } from './shared/labels.ts';

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
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}`);
}

// Guards the whole refresh cycle: both charts in a given `refresh()` call
// share one token, since they're issued together and should be discarded
// together if a newer refresh has since started.
const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadChart(
  def: ChartDef,
  controls: Controls,
  summaryPromise: ReturnType<typeof getSummary>,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
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
      summaryPromise,
    ]);

    if (!refreshGuard.isCurrent(token)) return;

    if (breakdown.rows.length === 0) {
      showOverlay(def.containerId, container, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(breakdown.currency);

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
    if (!refreshGuard.isCurrent(token)) return;
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
          // `row.label` traces back to `breakdown()`'s `key` field, i.e.
          // real cost-data values (service/account/resource/tag names) —
          // escape before interpolating into the HTML `tooltip` formatter
          // returns (ECharts' default `renderMode: 'html'` does not escape
          // it for us).
          return [`<strong>${escapeHtml(row.label)}</strong>`, formatCurrency(row.total, currency)].join('<br/>');
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

export function initTopBreakdownCharts(onCurrency?: (currency: string) => void): Promise<void> {
  const defs = CHART_DEFS.filter((def) => getContainer(def.containerId) !== null);
  if (defs.length === 0) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    const token = refreshGuard.next();
    // Both charts need the same overall total for their "Other" bucket; fetch
    // it once per refresh cycle and share the in-flight promise instead of
    // issuing two identical requests.
    const summaryPromise = getSummary({
      start: controls.startIso,
      end: addDaysIso(controls.endIsoInclusive, 1),
      metric: controls.metric,
    });
    await Promise.allSettled(defs.map((def) => loadChart(def, controls, summaryPromise, token, onCurrency)));
  };

  document.querySelector('#date-start')?.addEventListener('change', () => void refresh());
  document.querySelector('#date-end')?.addEventListener('change', () => void refresh());
  document.querySelector('#metric-select')?.addEventListener('change', () => void refresh());

  return refresh();
}
