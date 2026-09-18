/**
 * Secondary breakdowns for the Service Detail page's selected service (plan
 * §26): cost by account, by region, and by charge category, each filtered
 * to `services: [selectedService]`.
 *
 * Structurally identical to `topBreakdown.ts`'s horizontal-bar-chart /
 * "Other" bucket pattern (reusing `shared/chart.ts`'s `ensureChart`/overlay
 * helpers and `shared/labels.ts`'s `formatKeyLabel`), just applied three
 * times against one service instead of twice against the whole dataset —
 * kept in one module (rather than three near-duplicate files) since the only
 * difference between the three is which `Dimension` and which container id
 * is used, per Session 7's copy-paste review feedback.
 *
 * Each of the three charts owns its own `RequestGuard` (rather than one
 * shared per refresh cycle, as `topBreakdown.ts` does) and its own
 * try/catch, so one dimension's request failing (or resolving out of order)
 * cannot blank out or block the other two — same `Promise.allSettled`
 * discipline as the rest of the app.
 *
 * When no service is selected (`getSelectedService()` returns `null`), all
 * three charts clear themselves and skip fetching, per this page's "nothing
 * to fetch yet" convention (see `serviceDetailMain.ts`).
 */

import { getBreakdown, getSummary, type BreakdownRow, type Dimension } from './api.ts';
import { getSelectedService } from './serviceDetailMain.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { formatKeyLabel } from './shared/labels.ts';

// ---------------------------------------------------------------------------
// Config: one entry per chart, each with its own RequestGuard
// ---------------------------------------------------------------------------

interface ChartDef {
  containerId: string;
  dimension: Dimension;
  title: string;
  guard: RequestGuard;
}

const CHART_DEFS: ChartDef[] = [
  { containerId: 'service-by-account', dimension: 'account', title: 'Cost by account', guard: new RequestGuard() },
  { containerId: 'service-by-region', dimension: 'region', title: 'Cost by region', guard: new RequestGuard() },
  {
    containerId: 'service-by-category',
    dimension: 'charge_category',
    title: 'Cost by charge category',
    guard: new RequestGuard(),
  },
];

/** Below this fraction of the service's overall total, the "Other" bucket is omitted as negligible. */
const OTHER_EPSILON_FRACTION = 0.001;

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}`);
}

function getChartArea(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}-chart`);
}

function renderShell(def: ChartDef, container: HTMLElement): void {
  container.innerHTML = `
    <section class="chart-section">
      <div class="chart-header">
        <h2>${escapeHtml(def.title)}</h2>
      </div>
      <div class="chart-area" id="${def.containerId}-chart"></div>
    </section>`;
}

/**
 * Combines a top-N breakdown's rows with an overall (unfiltered-by-dimension)
 * total into the row list a horizontal bar chart should render, adding a
 * synthetic "Other" row for the remainder when it's non-trivial. Pulled out
 * as a pure helper (rather than repeated inline in `loadChart` for each of
 * the three dimensions) since this task applies the exact same computation
 * three times within this one module.
 */
function buildRowsWithOther(rows: BreakdownRow[], overallTotal: number): Array<{ label: string; total: number }> {
  const result: Array<{ label: string; total: number }> = rows.map((row) => ({
    label: formatKeyLabel(row.key),
    total: row.total,
  }));

  const sumOfRows = rows.reduce((acc, row) => acc + row.total, 0);
  const remainder = overallTotal - sumOfRows;
  if (remainder > overallTotal * OTHER_EPSILON_FRACTION) {
    result.push({ label: 'Other', total: remainder });
  }

  return result;
}

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadChart(
  def: ChartDef,
  service: string,
  controls: Controls,
  summaryPromise: ReturnType<typeof getSummary>,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const container = getContainer(def.containerId);
  const chartArea = getChartArea(def.containerId);
  if (!container || !chartArea) return;

  clearOverlays(chartArea);

  const start = controls.startIso;
  const end = addDaysIso(controls.endIsoInclusive, 1);

  try {
    const [breakdown, summary] = await Promise.all([
      getBreakdown({
        start,
        end,
        metric: controls.metric,
        dimension: def.dimension,
        services: [service],
        limit: 10,
      }),
      summaryPromise,
    ]);

    if (!def.guard.isCurrent(token)) return;

    if (breakdown.rows.length === 0) {
      showOverlay(def.containerId, chartArea, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(breakdown.currency);

    const rows = buildRowsWithOther(breakdown.rows, summary.total);
    renderChart(def.containerId, chartArea, rows, breakdown.currency);
  } catch (err) {
    if (!def.guard.isCurrent(token)) return;
    showOverlay(def.containerId, chartArea, 'chart-error', errorMessage(err));
  }
}

function renderChart(
  containerId: string,
  chartArea: HTMLElement,
  rows: Array<{ label: string; total: number }>,
  currency: string,
): void {
  const instance = ensureChart(`${containerId}-chart`, chartArea);

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
          // real cost-data values (account/region/charge-category names) —
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

export function initServiceBreakdowns(onCurrency?: (currency: string) => void): Promise<void> {
  const defs = CHART_DEFS.filter((def) => getContainer(def.containerId) !== null);
  if (defs.length === 0) return Promise.resolve();

  for (const def of defs) {
    const container = getContainer(def.containerId);
    if (container) renderShell(def, container);
  }

  const refresh = async (): Promise<void> => {
    const service = getSelectedService();
    if (!service) {
      for (const def of defs) {
        const chartArea = getChartArea(def.containerId);
        if (chartArea) {
          showOverlay(def.containerId, chartArea, 'chart-empty', 'Select a service to view this breakdown.');
        }
      }
      return;
    }

    const controls = readControls();
    if (!controls) return;

    // All three dimensions need the same service-scoped overall total for
    // their "Other" bucket; fetch it once per refresh cycle and share the
    // in-flight promise instead of issuing three identical requests.
    const summaryPromise = getSummary({
      start: controls.startIso,
      end: addDaysIso(controls.endIsoInclusive, 1),
      metric: controls.metric,
      services: [service],
    });

    await Promise.allSettled(
      defs.map((def) => {
        const token = def.guard.next();
        return loadChart(def, service, controls, summaryPromise, token, onCurrency);
      }),
    );
  };

  subscribeToControls(refresh, { extraIds: ['#service-picker'] });

  return refresh();
}
