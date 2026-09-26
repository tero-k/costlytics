import './style.css';
import { setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initAppShell } from './shared/appShell.ts';
import { initPageFilters } from './shared/pageFilters.ts';
import { addDaysIso, previousPeriod } from './shared/dates.ts';
import { readChangesControls, subscribeToControls } from './shared/controls.ts';
import { toCsv, downloadTextFile } from './shared/csv.ts';
import { initChangesSummary } from './costChangesSummary.ts';
import { initChangesMovers } from './costChangesMovers.ts';
import { initChangesTable, getExportTableData } from './costChangesTable.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { checkInitialLoad, installCostGuard, type PageQueries } from './shared/costGuard.ts';

/**
 * App shell / orchestrator for the Costlytics Cost Changes page.
 *
 * Mirrors `explorerMain.ts`'s bootstrap pattern (shared status bar /
 * date-range defaults, an `allSettled`-based initial-load indicator), but
 * this page's defining feature is its INDEPENDENT current/previous period
 * controls (`#date-start`/`#date-end` for the current period,
 * `#prev-date-start`/`#prev-date-end` for the previous period — unlike Cost
 * Explorer, which auto-derives "previous" from a single date range).
 *
 * On first load there is no persisted state to restore (no URL/localStorage
 * layer for this page — kept simple per the task plan), so the current
 * period defaults the same way every other page does
 * (`appState.ts`: last used range, else "current month to date") and the previous
 * period defaults to the immediately-preceding period of the same length
 * (`previousPeriod()`), giving a sensible starting comparison that the user
 * can then override on either side independently.
 *
 * The summary cards (`#changes-summary`, Task 2), biggest-movers lists
 * (`#changes-movers`, Task 2), and full comparison table (`#changes-table`,
 * Task 3) each register their `init*` function into `refreshers` below.
 */

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure).
 */
const refreshers: Array<() => Promise<void>> = [
  () => initChangesSummary(updateStatusCurrency),
  () => initChangesMovers(),
  () => initChangesTable(updateStatusCurrency),
];

function refresh(): void {
  void Promise.allSettled(refreshers.map((r) => r()));
}

/**
 * CSV export (Task 3): serializes the comparison table's CURRENTLY RENDERED
 * rows — i.e. whatever sort order is active, same cell text as on screen —
 * to CSV text and triggers a browser download. Does not re-fetch; if the
 * table has no data yet (still loading, errored, or empty for the current
 * filters), the button does nothing rather than exporting a bogus file.
 * Mirrors `explorerMain.ts`'s `initCsvExport()`.
 */
function initCsvExport(): void {
  const button = document.querySelector<HTMLButtonElement>('#csv-export-btn');
  if (!button) return;

  button.addEventListener('click', () => {
    const data = getExportTableData();
    if (!data) return;

    const csv = toCsv([data.headers, ...data.rows]);

    const controls = readChangesControls();
    const dimension = controls?.dimension ?? 'export';
    const dateSuffix = controls ? `_${controls.startIso}_${controls.endIsoInclusive}_vs_${controls.previousStartIso}_${controls.previousEndIsoInclusive}` : '';
    const filename = `cost-changes_${dimension}${dateSuffix}.csv`;

    downloadTextFile(filename, csv);
  });
}

/** Defaults `#prev-date-start`/`#prev-date-end` to the period immediately preceding the current one. */
function initPreviousPeriodDefaults(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const prevStartInput = document.querySelector<HTMLInputElement>('#prev-date-start');
  const prevEndInput = document.querySelector<HTMLInputElement>('#prev-date-end');
  if (!startInput?.value || !endInput?.value || !prevStartInput || !prevEndInput) return;

  // `previousPeriod()` takes/returns an EXCLUSIVE end, but `#date-end`/
  // `#prev-date-end` are INCLUSIVE (same convention as every other date
  // input in the app — see `Controls.endIsoInclusive`), so both ends need
  // the +1/-1 day conversion at this boundary, exactly like `kpiCards.ts`
  // and `explorerTable.ts`'s calls do.
  const previous = previousPeriod(startInput.value, addDaysIso(endInput.value, 1));
  prevStartInput.value = previous.start;
  prevEndInput.value = addDaysIso(previous.end, -1);
}

/** One refresh: summary cards, movers and table, each a compare. */
const CHANGES_QUERIES: PageQueries = { current: 0, compare: 3 };

async function bootstrap(): Promise<void> {
  initAppShell();
  initPreviousPeriodDefaults();
  initCsvExport();
  await initSourcePicker();
  initPageFilters(['services', 'accounts']);
  installCostGuard(CHANGES_QUERIES);
  void checkInitialLoad(CHANGES_QUERIES);
  subscribeToControls(refresh, { changes: true });

  setLoadingIndicatorVisible(true);
  try {
    await Promise.allSettled(refreshers.map((r) => r()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
