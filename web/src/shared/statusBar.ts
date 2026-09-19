/**
 * Page-level status bar / date-range-default bootstrapping shared by every
 * Costlytics page (Overview's `main.ts`, Cost Explorer's `explorerMain.ts`).
 *
 * Every page shares the same `#date-start` / `#date-end` / `#metric-select`
 * controls and the same `#status-range` / `#status-metric` /
 * `#status-currency` / `#status-loading` status-bar elements (see
 * `index.html` / `explorer.html`), so the wiring between them lives here
 * once rather than being re-implemented per page.
 */

import type { CostMetric } from '../api.ts';

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

/** Defaults `#date-start`/`#date-end` to "current month to date" if present. */
export function initDateRangeDefaults(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  if (!startInput || !endInput) return;

  const today = new Date();
  const startOfMonth = new Date(today.getFullYear(), today.getMonth(), 1);

  startInput.value = toDateInputValue(startOfMonth);
  endInput.value = toDateInputValue(today);
}

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
