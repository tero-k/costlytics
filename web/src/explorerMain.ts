import './style.css';
import {
  initDateRangeDefaults,
  initStatusBar,
  setLoadingIndicatorVisible,
  updateStatusCurrency,
} from './shared/statusBar.ts';
import { initExplorerTrendChart } from './explorerTrend.ts';

/**
 * App shell / orchestrator for the Costlytics Cost Explorer page.
 *
 * Mirrors `main.ts`'s bootstrap pattern (shared status bar / date-range
 * defaults, an `allSettled`-based initial-load indicator). The grouped
 * trend chart (`#explorer-trend`, Task 2) registers itself below; the
 * breakdown table (`#explorer-table`) is built in a later task and should
 * register its `init*` function into `refreshers` too rather than editing
 * this bootstrap function directly.
 */

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure).
 */
const refreshers: Array<() => Promise<void>> = [() => initExplorerTrendChart(updateStatusCurrency)];

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
