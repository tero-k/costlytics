import './style.css';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible } from './shared/statusBar.ts';

/**
 * App shell / orchestrator for the Costlytics Cost Explorer page.
 *
 * Mirrors `main.ts`'s bootstrap pattern (shared status bar / date-range
 * defaults, an `allSettled`-based initial-load indicator), but this page
 * has no per-component modules yet — the trend chart (`#explorer-trend`)
 * and breakdown table (`#explorer-table`) are built in later tasks. Those
 * tasks register their `init*` functions into `refreshers` below rather
 * than editing this bootstrap function directly.
 */

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure). Populated by later tasks (trend chart, breakdown table); left
 * empty here since Task 1 only builds the page shell and controls.
 */
const refreshers: Array<() => Promise<void>> = [];

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();

  setLoadingIndicatorVisible(true);
  try {
    await Promise.allSettled(refreshers.map((refresh) => refresh()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
