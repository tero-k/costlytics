/**
 * Page-level status bar wiring shared by every Costlytics page. The status
 * bar itself is rendered by `appShell.ts`; every page shares the same
 * `#date-start` / `#date-end` / `#metric-select` controls and the same
 * `#status-range` / `#status-metric` / `#status-currency` /
 * `#status-filters` / `#status-loading` elements, so the wiring between
 * them lives here once. Initial date/metric values come from
 * `appState.ts`.
 */

import type { CostMetric } from '../api.ts';

const METRIC_LABELS: Record<CostMetric, string> = {
  amortized: 'Amortized',
  billed: 'Billed',
  list: 'List',
  contracted: 'Contracted',
};

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

/**
 * Set once the first successful API response reports a currency; stays
 * displayed across later refreshes. A falsy/empty `currency` (e.g. a
 * zero-row query result, whose `currency` field comes back as `""`) is
 * ignored rather than blanking out whatever was last shown — otherwise
 * switching to a service/range with no data would wipe a previously known
 * currency from the status bar.
 */
export function updateStatusCurrency(currency: string): void {
  if (!currency) return;
  const currencyEl = document.querySelector<HTMLElement>('#status-currency');
  if (currencyEl) currencyEl.textContent = `Currency: ${currency}`;
}

export function setLoadingIndicatorVisible(visible: boolean): void {
  const loadingEl = document.querySelector<HTMLElement>('#status-loading');
  if (loadingEl) loadingEl.hidden = !visible;
}

/** Wires the status bar's range/metric display to live-update on control changes. */
export function initStatusBar(): void {
  updateStatusRangeAndMetric();
  document.querySelector('#date-start')?.addEventListener('change', updateStatusRangeAndMetric);
  document.querySelector('#date-end')?.addEventListener('change', updateStatusRangeAndMetric);
  document.querySelector('#metric-select')?.addEventListener('change', updateStatusRangeAndMetric);
}

/**
 * Shows (or hides, for an empty string) the active per-page filter summary,
 * e.g. "Filtered: 2 services, 1 account" — plan §57: a dashboard must always
 * show what it is scoped to, not just in the filter widgets.
 */
export function updateStatusFilters(summary: string): void {
  const el = document.querySelector<HTMLElement>('#status-filters');
  const sep = document.querySelector<HTMLElement>('#status-filters-sep');
  if (!el) return;
  el.textContent = summary;
  el.hidden = summary === '';
  if (sep) sep.hidden = summary === '';
}
