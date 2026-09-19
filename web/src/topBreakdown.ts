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

import { getBreakdown, getSummary, type Dimension } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { errorMessage } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { buildRowsWithOther } from './shared/otherBucket.ts';
import { renderHorizontalBarChart } from './shared/horizontalBarChart.ts';

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

    const rows = buildRowsWithOther(breakdown.rows, summary.total);

    renderHorizontalBarChart(def.containerId, container, rows, breakdown.currency);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    showOverlay(def.containerId, container, 'chart-error', errorMessage(err));
  }
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

  subscribeToControls(refresh);

  return refresh();
}
