import './style.css';
import {
  setLoadingIndicatorVisible,
  updateStatusCurrency,
} from './shared/statusBar.ts';
import { initAppShell } from './shared/appShell.ts';
import { initPageFilters } from './shared/pageFilters.ts';
import { initExplorerTrendChart } from './explorerTrend.ts';
import { initExplorerTable, getExportTableData } from './explorerTable.ts';
import { toCsv, downloadTextFile } from './shared/csv.ts';
import { readExplorerControls } from './shared/controls.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { checkInitialLoad, installCostGuard, type PageQueries } from './shared/costGuard.ts';

/**
 * App shell / orchestrator for the Costlytics Cost Explorer page.
 *
 * Mirrors `main.ts`'s bootstrap pattern (shared status bar / date-range
 * defaults, an `allSettled`-based initial-load indicator). The grouped
 * trend chart (`#explorer-trend`, Task 2) and the detailed comparison table
 * (`#explorer-table`, Task 3) each register their `init*` function into
 * `refreshers` below.
 */

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure).
 */
const refreshers: Array<() => Promise<void>> = [
  () => initExplorerTrendChart(updateStatusCurrency),
  () => initExplorerTable(updateStatusCurrency),
];

/**
 * CSV export (Task 4): serializes the comparison table's CURRENTLY RENDERED
 * rows — i.e. whatever sort order is active, same cell text as on screen —
 * to CSV text and triggers a browser download. Does not re-fetch; if the
 * table has no data yet (still loading, errored, or empty for the current
 * filters), the button does nothing rather than exporting a bogus file.
 */
function initCsvExport(): void {
  const button = document.querySelector<HTMLButtonElement>('#csv-export-btn');
  if (!button) return;

  button.addEventListener('click', () => {
    const data = getExportTableData();
    if (!data) return;

    const csv = toCsv([data.headers, ...data.rows]);

    const controls = readExplorerControls();
    const dimension = controls?.dimension ?? 'export';
    const dateSuffix = controls ? `_${controls.startIso}_${controls.endIsoInclusive}` : '';
    const filename = `cost-explorer_${dimension}${dateSuffix}.csv`;

    downloadTextFile(filename, csv);
  });
}

/** One refresh: the trend chart plus the current-vs-previous table. */
const EXPLORER_QUERIES: PageQueries = { current: 1, compare: 1 };

async function bootstrap(): Promise<void> {
  initAppShell();
  initCsvExport();
  await initSourcePicker();
  initPageFilters(['services', 'accounts']);
  installCostGuard(EXPLORER_QUERIES);
  void checkInitialLoad(EXPLORER_QUERIES);

  setLoadingIndicatorVisible(true);
  try {
    await Promise.allSettled(refreshers.map((refresh) => refresh()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
