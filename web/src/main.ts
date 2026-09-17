import './style.css';
import { initKpiCards } from './kpiCards.ts';
import { initTrendChart } from './trendChart.ts';
import { initTopBreakdownCharts } from './topBreakdown.ts';
import type { CostMetric } from './api.ts';

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
 * and currency, not just bury them inside individual cards).
 */

const METRIC_LABELS: Record<CostMetric, string> = {
  amortized: 'Amortized',
  billed: 'Billed',
  list: 'List',
  contracted: 'Contracted',
};

function toDateInputValue(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function initDateRangeDefaults(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  if (!startInput || !endInput) return;

  const today = new Date();
  const startOfMonth = new Date(today.getFullYear(), today.getMonth(), 1);

  startInput.value = toDateInputValue(startOfMonth);
  endInput.value = toDateInputValue(today);
}

// ---------------------------------------------------------------------------
// Status bar: always-visible active date range / metric / currency
// ---------------------------------------------------------------------------

function formatDisplayDate(iso: string): string {
  const date = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }).format(
    date,
  );
}

function updateStatusRangeAndMetric(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const metricSelect = document.querySelector<HTMLSelectElement>('#metric-select');
  const rangeEl = document.querySelector<HTMLElement>('#status-range');
  const metricEl = document.querySelector<HTMLElement>('#status-metric');

  if (rangeEl && startInput?.value && endInput?.value) {
    rangeEl.textContent = `${formatDisplayDate(startInput.value)} – ${formatDisplayDate(endInput.value)}`;
  }
  if (metricEl) {
    const metric = (metricSelect?.value as CostMetric | undefined) ?? 'amortized';
    metricEl.textContent = METRIC_LABELS[metric];
  }
}

/** Set once the first successful API response reports a currency; stays displayed across later refreshes. */
function updateStatusCurrency(currency: string): void {
  const currencyEl = document.querySelector<HTMLElement>('#status-currency');
  if (currencyEl) currencyEl.textContent = `Currency: ${currency}`;
}

function setLoadingIndicatorVisible(visible: boolean): void {
  const loadingEl = document.querySelector<HTMLElement>('#status-loading');
  if (loadingEl) loadingEl.hidden = !visible;
}

function initStatusBar(): void {
  updateStatusRangeAndMetric();
  document.querySelector('#date-start')?.addEventListener('change', updateStatusRangeAndMetric);
  document.querySelector('#date-end')?.addEventListener('change', updateStatusRangeAndMetric);
  document.querySelector('#metric-select')?.addEventListener('change', updateStatusRangeAndMetric);
}

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();

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
