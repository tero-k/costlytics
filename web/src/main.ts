import './style.css';
import { initKpiCards } from './kpiCards.ts';
import { initTrendChart } from './trendChart.ts';
import { setKpiCredits, setKpiSparkline } from './shared/kpiCard.ts';
import { formatCurrency } from './shared/format.ts';
import { initTopBreakdownCharts } from './topBreakdown.ts';
import {
  setLoadingIndicatorVisible,
  updateStatusCurrency,
} from './shared/statusBar.ts';
import { initAppShell } from './shared/appShell.ts';
import { initPageFilters } from './shared/pageFilters.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { checkInitialLoad, installCostGuard, type PageQueries } from './shared/costGuard.ts';

/**
 * App shell / orchestrator for the Costlytics Overview page.
 *
 * Bootstraps the shared date-range / metric controls with sensible defaults
 * (current month to date, amortized metric), then initializes the four
 * independent components (KPI cards, trend chart, top services, top
 * accounts). Each component owns its own fetch/render logic and already
 * subscribes directly to the shared `#date-start` / `#date-end` /
 * `#metric-select` controls in the DOM, so there is no separate pub/sub
 * layer here — the DOM inputs themselves are the shared state, and each
 * component's `change` listener is the notification mechanism.
 *
 * This module's job is page-level concerns that span all four components:
 * an initial-load indicator, and a persistent status bar (plan §57's UX
 * rule — a dashboard must always show its active date range, cost metric,
 * and currency, not just bury them inside individual cards). The status
 * bar / shared-control wiring itself lives in `shared/appShell.ts` since the
 * Cost Explorer page (`explorerMain.ts`) needs the same behavior.
 */

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

/** One refresh: KPI summary + compare, top-breakdown summary + 2 charts, trend. */
const OVERVIEW_QUERIES: PageQueries = { current: 5, compare: 1 };

async function bootstrap(): Promise<void> {
  initAppShell();
  // Must resolve before any component's first fetch fires (see
  // `shared/sourcePicker.ts`'s doc comment) — awaited here, ahead of the
  // `Promise.allSettled` below.
  const sourceResult = await initSourcePicker();
  if (sourceResult !== null && sourceResult.registered.length === 0) {
    // Loaded fine, but nothing is registered: guide the user to Settings
    // instead of rendering four error states. Distinguish "nothing
    // configured yet" from "configured but none loaded" — see
    // `#no-sources`/`#no-sources-configured` in `index.html`.
    const emptyStateId = sourceResult.configured === 0 ? '#no-sources' : '#no-sources-configured';
    document.querySelector<HTMLElement>(emptyStateId)?.removeAttribute('hidden');
    return;
  }
  initPageFilters(['services', 'accounts']);
  installCostGuard(OVERVIEW_QUERIES);
  void checkInitialLoad(OVERVIEW_QUERIES);

  setLoadingIndicatorVisible(true);
  try {
    // Each component fetches and renders independently (per-component
    // try/catch inside each module already), and each also subscribes
    // itself to control changes for all later refreshes. `allSettled` here
    // only governs how long the page-level "Loading..." indicator shows for
    // the *initial* load — a slow or failing component never blocks or
    // hides the others.
    // All three fetching modules report their observed currency through the
    // same `updateStatusCurrency` callback (plan §57: the status bar's
    // currency must always be visible/current), so the status bar updates
    // from whichever component succeeds first/most-recently, regardless of
    // which other components are failing (e.g. `compare()` 409ing on
    // multi-currency data while `timeseries()`/`breakdown()` still succeed).
    await Promise.allSettled([
      initKpiCards(updateStatusCurrency),
      // The trend's series doubles as the "Selected period" card's
      // sparkline and the credits card (no extra query).
      initTrendChart(updateStatusCurrency, (split, currency) => {
        setKpiSparkline('kpi-current', split?.net ?? []);
        setKpiCredits('kpi-credits', split?.totals ?? null, (v) => formatCurrency(v, currency));
      }),
      initTopBreakdownCharts(updateStatusCurrency),
    ]);
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
