/**
 * Detailed sortable comparison table for the Cost Changes page (plan §30).
 *
 * Thin page-specific wrapper around `shared/comparisonTable.ts`: renders
 * into `#changes-table`, for the page's selected dimension
 * (`#dimension-select`), using this page's own INDEPENDENT current/previous
 * period controls (`readChangesControls()`) rather than Explorer's
 * auto-derived previous period — this table's raison d'être is comparing two
 * ARBITRARY periods, not just a period against its immediate predecessor.
 *
 * Unlike `explorerTable.ts` (which subscribes to controls itself, since
 * `explorer.html`'s components each own their own subscription), this page
 * follows `costChangesSummary.ts`/`costChangesMovers.ts`'s convention: a
 * single `subscribeToControls(refresh, { changes: true })` call lives in
 * `costChangesMain.ts` and re-invokes every registered `init*` function
 * (including this one, via `refreshers`) on any control change — so
 * `initChangesTable` runs repeatedly, and the table instance (with its
 * sort/rows/currency state) must be built exactly once at module scope
 * rather than rebuilt on every call.
 */

import { addDaysIso } from './shared/dates.ts';
import { readChangesControls, type ChangesControls } from './shared/controls.ts';
import { createComparisonTable, type ComparisonRanges } from './shared/comparisonTable.ts';

function resolveRanges(controls: ChangesControls): ComparisonRanges {
  return {
    currentStart: controls.startIso,
    currentEnd: addDaysIso(controls.endIsoInclusive, 1),
    previousStart: controls.previousStartIso,
    previousEnd: addDaysIso(controls.previousEndIsoInclusive, 1),
  };
}

// See `explorerTable.ts` for why the currency callback is wired up through
// this indirection rather than passed directly to `createComparisonTable`.
let onCurrencyCallback: ((currency: string) => void) | undefined;

const table = createComparisonTable<ChangesControls>({
  containerId: 'changes-table',
  readControls: readChangesControls,
  resolveRanges,
  onCurrency: (currency) => onCurrencyCallback?.(currency),
});

export const getExportTableData = table.getExportTableData;

export function initChangesTable(onCurrency?: (currency: string) => void): Promise<void> {
  onCurrencyCallback = onCurrency;
  return table.refresh();
}
