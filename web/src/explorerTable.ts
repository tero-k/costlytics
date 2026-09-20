/**
 * Detailed sortable comparison table for the Cost Explorer page (plan §25).
 *
 * Thin page-specific wrapper around `shared/comparisonTable.ts`: renders
 * into `#explorer-table`, for the SAME grouping dimension as the page's
 * trend chart (`explorerTrend.ts`), using the same shared date-range /
 * metric / Top N controls (`readExplorerControls()`). The one thing this
 * file supplies that the shared module can't: Explorer auto-derives its
 * "previous" period from a single date range via `previousPeriod()`, and
 * subscribes to its own controls directly (each of `explorer.html`'s
 * components owns its own subscription, unlike Cost Changes' centrally
 * driven refresh).
 */

import { addDaysIso, previousPeriod } from './shared/dates.ts';
import { readExplorerControls, subscribeToControls, type ExplorerControls } from './shared/controls.ts';
import { createComparisonTable, type ComparisonRanges } from './shared/comparisonTable.ts';

function resolveRanges(controls: ExplorerControls): ComparisonRanges {
  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const { start: previousStart, end: previousEnd } = previousPeriod(currentStart, currentEnd);
  return { currentStart, currentEnd, previousStart, previousEnd };
}

// `onCurrency` is only known once `initExplorerTable(onCurrency)` is called
// (from `explorerMain.ts`'s `refreshers`), but the table instance itself —
// and its sort/rows/currency state — must be built exactly once at module
// scope, matching the previous non-shared implementation's module-level
// `sortColumn`/`lastRows`/etc. This indirection lets the callback be wired
// up after construction without rebuilding (and thereby resetting) the table.
let onCurrencyCallback: ((currency: string) => void) | undefined;

const table = createComparisonTable<ExplorerControls>({
  containerId: 'explorer-table',
  readControls: readExplorerControls,
  resolveRanges,
  onCurrency: (currency) => onCurrencyCallback?.(currency),
});

export const getExportTableData = table.getExportTableData;

export function initExplorerTable(onCurrency?: (currency: string) => void): Promise<void> {
  onCurrencyCallback = onCurrency;
  subscribeToControls(() => void table.refresh(), { explorer: true });
  return table.refresh();
}
