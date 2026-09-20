/**
 * Generic secondary breakdowns for a "detail page" entity (plan §26), e.g.
 * the Service Detail page's selected service (cost by account, by region,
 * by charge category) or the Account Detail page's selected account (cost
 * by service, by region), each filtered to `config.buildFilter(selected)`.
 *
 * Structurally identical to `topBreakdown.ts`'s horizontal-bar-chart /
 * "Other" bucket pattern (reusing `shared/chart.ts`'s `ensureChart`/overlay
 * helpers, `shared/otherBucket.ts`'s `buildRowsWithOther`, and
 * `shared/horizontalBarChart.ts`'s `renderHorizontalBarChart`), just applied
 * once per `{dimension, containerId}` pair instead of hardcoding two or
 * three near-identical charts per page.
 *
 * Generalized from `serviceBreakdowns.ts`/`accountBreakdowns.ts` (Session 11)
 * once Session 10's whole-branch review found those two modules had zero
 * code differences beyond entity-noun substitution AND the list of
 * dimensions to break down by — see `shared/entityConfig.ts`'s doc comment
 * for the entity-noun substitution points, and this module's `BreakdownDef`
 * for the per-page dimension list (Service Detail's config lists 3, Account
 * Detail's lists 2).
 *
 * Each chart owns its own `RequestGuard` (rather than one shared per refresh
 * cycle) and its own try/catch, so one dimension's request failing (or
 * resolving out of order) cannot blank out or block the others — same
 * `Promise.allSettled` discipline as the rest of the app. This per-dimension
 * isolation was Session 9/10's explicitly praised pattern and is preserved
 * here rather than regressed to a single shared guard.
 *
 * When no entity is selected (`config.getSelected()` returns `null`), all
 * charts clear themselves and skip fetching, per this page family's
 * "nothing to fetch yet" convention.
 */

import { getBreakdown, getSummary, type Dimension } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { buildRowsWithOther } from './shared/otherBucket.ts';
import { renderHorizontalBarChart } from './shared/horizontalBarChart.ts';
import type { EntityConfig } from './shared/entityConfig.ts';

/** One breakdown chart to render: which dimension, into which container, under what title. */
export interface BreakdownDef {
  containerId: string;
  dimension: Dimension;
  title: string;
}

interface ChartState extends BreakdownDef {
  guard: RequestGuard;
}

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}`);
}

function getChartArea(containerId: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${containerId}-chart`);
}

function renderShell(def: ChartState, container: HTMLElement): void {
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
  def: ChartState,
  config: EntityConfig,
  selected: string,
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
        ...config.buildFilter(selected),
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

/** Handle returned by `initEntityBreakdowns`: a stable, reusable `refresh()` that reuses the same per-dimension `RequestGuard`s and does NOT re-register control listeners on repeat calls. */
export interface EntityBreakdownsHandle {
  refresh: () => Promise<void>;
}

export function initEntityBreakdowns(
  config: EntityConfig,
  breakdowns: BreakdownDef[],
  onCurrency?: (currency: string) => void,
): EntityBreakdownsHandle {
  const defs: ChartState[] = breakdowns
    .filter((def) => getContainer(def.containerId) !== null)
    .map((def) => ({ ...def, guard: new RequestGuard() }));
  if (defs.length === 0) return { refresh: () => Promise.resolve() };

  for (const def of defs) {
    const container = getContainer(def.containerId);
    if (container) renderShell(def, container);
  }

  const refresh = async (): Promise<void> => {
    const selected = config.getSelected();
    if (!selected) {
      for (const def of defs) {
        def.guard.next();
        const chartArea = getChartArea(def.containerId);
        if (chartArea) {
          showOverlay(def.containerId, chartArea, 'chart-empty', `Select a ${config.entityNoun} to view this breakdown.`);
        }
      }
      return;
    }

    const controls = readControls();
    if (!controls) return;

    // All dimensions need the same entity-scoped overall total for their
    // "Other" bucket; fetch it once per refresh cycle and share the
    // in-flight promise instead of issuing one identical request per chart.
    const summaryPromise = getSummary({
      start: controls.startIso,
      end: addDaysIso(controls.endIsoInclusive, 1),
      metric: controls.metric,
      ...config.buildFilter(selected),
    });

    await Promise.allSettled(
      defs.map((def) => {
        const token = def.guard.next();
        return loadChart(def, config, selected, controls, summaryPromise, token, onCurrency);
      }),
    );
  };

  subscribeToControls(refresh, { extraIds: [config.pickerSelector] });

  return { refresh };
}
