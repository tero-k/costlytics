/**
 * Secondary breakdowns for the Account Detail page's selected account (plan
 * §26): cost by service and by region, each filtered to
 * `accounts: [selectedAccount]`.
 *
 * Structurally identical to `serviceBreakdowns.ts`'s (Service Detail page)
 * horizontal-bar-chart / "Other" bucket pattern (reusing
 * `shared/horizontalBarChart.ts`'s `renderHorizontalBarChart` and
 * `shared/otherBucket.ts`'s `buildRowsWithOther`, plus `shared/chart.ts`'s
 * overlay helpers and `shared/labels.ts`'s `formatKeyLabel` via
 * `buildRowsWithOther`), just applied against `Dimension::Service` and
 * `Dimension::Region` instead of account/region/charge-category — only TWO
 * breakdowns for accounts (no charge-category breakdown per the plan), kept
 * in one module rather than two near-duplicate files, same as
 * `serviceBreakdowns.ts`.
 *
 * Each of the two charts owns its own `RequestGuard` (rather than one shared
 * per refresh cycle) and its own try/catch, so one dimension's request
 * failing (or resolving out of order) cannot blank out or block the other —
 * same `Promise.allSettled` discipline as the rest of the app.
 *
 * When no account is selected (`getSelectedAccount()` returns `null`), both
 * charts clear themselves and skip fetching, per this page's "nothing to
 * fetch yet" convention (see `accountDetailMain.ts`).
 */

import { getBreakdown, getSummary, type Dimension } from './api.ts';
import { getSelectedAccount } from './shared/accountPicker.ts';
import { addDaysIso } from './shared/dates.ts';
import { errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { buildRowsWithOther } from './shared/otherBucket.ts';
import { renderHorizontalBarChart } from './shared/horizontalBarChart.ts';

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
  { containerId: 'account-by-service', dimension: 'service', title: 'Cost by service', guard: new RequestGuard() },
  { containerId: 'account-by-region', dimension: 'region', title: 'Cost by region', guard: new RequestGuard() },
];

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

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadChart(
  def: ChartDef,
  account: string,
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
        accounts: [account],
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
    renderHorizontalBarChart(`${def.containerId}-chart`, chartArea, rows, breakdown.currency);
  } catch (err) {
    if (!def.guard.isCurrent(token)) return;
    showOverlay(def.containerId, chartArea, 'chart-error', errorMessage(err));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initAccountBreakdowns(onCurrency?: (currency: string) => void): Promise<void> {
  const defs = CHART_DEFS.filter((def) => getContainer(def.containerId) !== null);
  if (defs.length === 0) return Promise.resolve();

  for (const def of defs) {
    const container = getContainer(def.containerId);
    if (container) renderShell(def, container);
  }

  const refresh = async (): Promise<void> => {
    const account = getSelectedAccount();
    if (!account) {
      for (const def of defs) {
        def.guard.next();
        const chartArea = getChartArea(def.containerId);
        if (chartArea) {
          showOverlay(def.containerId, chartArea, 'chart-empty', 'Select an account to view this breakdown.');
        }
      }
      return;
    }

    const controls = readControls();
    if (!controls) return;

    // Both dimensions need the same account-scoped overall total for their
    // "Other" bucket; fetch it once per refresh cycle and share the
    // in-flight promise instead of issuing two identical requests.
    const summaryPromise = getSummary({
      start: controls.startIso,
      end: addDaysIso(controls.endIsoInclusive, 1),
      metric: controls.metric,
      accounts: [account],
    });

    await Promise.allSettled(
      defs.map((def) => {
        const token = def.guard.next();
        return loadChart(def, account, controls, summaryPromise, token, onCurrency);
      }),
    );
  };

  subscribeToControls(refresh, { extraIds: ['#account-picker'] });

  return refresh();
}
