import './style.css';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible } from './shared/statusBar.ts';
import { previousPeriod } from './shared/dates.ts';
import { subscribeToControls } from './shared/controls.ts';

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
 * (`initDateRangeDefaults()`, "current month to date") and the previous
 * period defaults to the immediately-preceding period of the same length
 * (`previousPeriod()`), giving a sensible starting comparison that the user
 * can then override on either side independently.
 *
 * The summary cards (`#changes-summary`, Task 2), biggest-movers lists
 * (`#changes-movers`, Task 2), and full comparison table (`#changes-table`,
 * Task 3) each register their `init*` function into `refreshers` below —
 * empty for now, filled in by later tasks.
 */

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure). Empty in Task 1; Tasks 2-3 push their `init*` functions here.
 */
const refreshers: Array<() => Promise<void>> = [];

function refresh(): void {
  void Promise.allSettled(refreshers.map((r) => r()));
}

/** Defaults `#prev-date-start`/`#prev-date-end` to the period immediately preceding the current one. */
function initPreviousPeriodDefaults(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const prevStartInput = document.querySelector<HTMLInputElement>('#prev-date-start');
  const prevEndInput = document.querySelector<HTMLInputElement>('#prev-date-end');
  if (!startInput?.value || !endInput?.value || !prevStartInput || !prevEndInput) return;

  const previous = previousPeriod(startInput.value, endInput.value);
  prevStartInput.value = previous.start;
  prevEndInput.value = previous.end;
}

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initPreviousPeriodDefaults();
  initStatusBar();
  subscribeToControls(refresh, { changes: true });

  setLoadingIndicatorVisible(true);
  try {
    await Promise.allSettled(refreshers.map((r) => r()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
