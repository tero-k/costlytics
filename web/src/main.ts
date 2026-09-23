import './style.css';
import { initKpiCards } from './kpiCards.ts';
import { initTrendChart } from './trendChart.ts';
import { initTopBreakdownCharts } from './topBreakdown.ts';
import {
  initDateRangeDefaults,
  initStatusBar,
  setLoadingIndicatorVisible,
  updateStatusCurrency,
} from './shared/statusBar.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';

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
 * bar / date-default wiring itself lives in `shared/statusBar.ts` since the
 * Cost Explorer page (`explorerMain.ts`) needs the same behavior.
 */

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();
  // Must resolve before any component's first fetch fires (see
  // `shared/sourcePicker.ts`'s doc comment) — awaited here, ahead of the
  // `Promise.allSettled` below.
  const registeredIds = await initSourcePicker();
  if (registeredIds !== null && registeredIds.length === 0) {
    // Loaded fine, but nothing is registered: guide the user to Settings
    // instead of rendering four error states.
    document.querySelector<HTMLElement>('#no-sources')?.removeAttribute('hidden');
    return;
  }

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
      initTrendChart(updateStatusCurrency),
      initTopBreakdownCharts(updateStatusCurrency),
    ]);
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
