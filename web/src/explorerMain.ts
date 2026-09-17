import './style.css';
import {
  initDateRangeDefaults,
  initStatusBar,
  setLoadingIndicatorVisible,
  updateStatusCurrency,
} from './shared/statusBar.ts';
import { initExplorerTrendChart } from './explorerTrend.ts';
import { initExplorerTable } from './explorerTable.ts';

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
