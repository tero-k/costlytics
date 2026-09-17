/**
 * KPI card row for the Overview page (plan §19).
 *
 * Renders five cards into `#overview`:
 *   1. Selected-period cost
 *   2. Previous-period cost (same-length period immediately preceding the
 *      selected range)
 *   3. Change (absolute + percentage, both server-computed by `compare()`)
 *   4. Month-to-date cost (always relative to "now", independent of the
 *      page's date-range selector)
 *   5. Projected current-month cost — a simple client-side run-rate
 *      estimate computed from the MTD total, explicitly labeled as an
 *      estimate (never presented as a precise forecast)
 *
 * The two data sources (`compare()` for cards 1-3, `summary()` for cards
 * 4-5) are fetched independently: a failure or slowness in one group must
 * not prevent the other group from rendering.
 */

import { ApiError, getCompare, getSummary, type CostMetric } from './api.ts';

// ---------------------------------------------------------------------------
// DOM shell
// ---------------------------------------------------------------------------

const CARD_DEFS: Array<{ id: string; label: string }> = [
  { id: 'kpi-current', label: 'Selected period' },
  { id: 'kpi-previous', label: 'Previous period' },
  { id: 'kpi-change', label: 'Change' },
  { id: 'kpi-mtd', label: 'Month to date' },
  { id: 'kpi-projected', label: 'Projected (run-rate estimate)' },
];

function renderShell(container: HTMLElement): void {
  const cardsHtml = CARD_DEFS.map(
    ({ id, label }) => `
      <div class="kpi-card loading" id="${id}">
        <div class="label">${label}</div>
        <div class="value">&hellip;</div>
        <div class="sub"></div>
      </div>`,
  ).join('');

  container.innerHTML = `<div class="kpi-grid">${cardsHtml}</div>`;
}

function setLoading(id: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('error', 'change-bad', 'change-good', 'change-neutral');
  card.classList.add('loading');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = '…';
  if (sub) sub.textContent = '';
}

function setError(id: string, message: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', 'change-bad', 'change-good', 'change-neutral');
  card.classList.add('error');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = 'Error';
  if (sub) sub.textContent = message;
}

function setValue(id: string, value: string, sub?: string, subClass?: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', 'error');
  const valueEl = card.querySelector<HTMLElement>('.value');
  const subEl = card.querySelector<HTMLElement>('.sub');
  if (valueEl) valueEl.textContent = value;
  if (subEl) {
    subEl.textContent = sub ?? '';
    subEl.className = 'sub' + (subClass ? ` ${subClass}` : '');
  }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function formatCurrency(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(value);
  } catch {
    // Fall back gracefully if the API ever returns a currency code that
    // Intl doesn't recognize.
    return `${value.toFixed(2)} ${currency}`;
  }
}

/** `pct` is already a percentage value (e.g. `37.2` means 37.2%), per the API's `percentage_change`. */
function formatPercent(pct: number): string {
  const sign = pct > 0 ? '+' : '';
  return `${sign}${pct.toFixed(1)}%`;
}

function formatSignedCurrency(value: number, currency: string): string {
  const formatted = formatCurrency(Math.abs(value), currency);
  return value > 0 ? `+${formatted}` : value < 0 ? `-${formatted}` : formatted;
}

function errorMessage(err: unknown): string {
  if (err instanceof ApiError) return err.message;
  if (err instanceof Error) return err.message;
  return 'Unknown error';
}

// ---------------------------------------------------------------------------
// Date helpers (all UTC-based to avoid local-timezone off-by-one errors)
// ---------------------------------------------------------------------------

function addDaysIso(dateIso: string, days: number): string {
  const d = new Date(`${dateIso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function daysBetweenIso(startIso: string, endIsoExclusive: string): number {
  const start = new Date(`${startIso}T00:00:00Z`).getTime();
  const end = new Date(`${endIsoExclusive}T00:00:00Z`).getTime();
  return Math.round((end - start) / 86_400_000);
}

function toDateIso(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function daysInMonth(year: number, monthIndex0: number): number {
  return new Date(year, monthIndex0 + 1, 0).getDate();
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

interface Controls {
  metric: CostMetric;
  /** Inclusive start date (`YYYY-MM-DD`), as selected in the date picker. */
  startIso: string;
  /** Inclusive end date (`YYYY-MM-DD`), as selected in the date picker. */
  endIsoInclusive: string;
}

function readControls(): Controls | null {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const metricSelect = document.querySelector<HTMLSelectElement>('#metric-select');
  if (!startInput?.value || !endInput?.value) return null;

  return {
    metric: (metricSelect?.value as CostMetric | undefined) ?? 'amortized',
    startIso: startInput.value,
    endIsoInclusive: endInput.value,
  };
}

// ---------------------------------------------------------------------------
// Fetch + render: current / previous / change (from `compare()`)
// ---------------------------------------------------------------------------

async function loadCompareCards(controls: Controls): Promise<void> {
  setLoading('kpi-current');
  setLoading('kpi-previous');
  setLoading('kpi-change');

  // The canonical `end` is exclusive; the date picker's "To" value is
  // inclusive, so the request's current_end is the day after it.
  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const durationDays = daysBetweenIso(currentStart, currentEnd);

  // Previous period: the same-length window immediately preceding the
  // current one (previous_end === current_start).
  const previousEnd = currentStart;
  const previousStart = addDaysIso(currentStart, -durationDays);

  try {
    const result = await getCompare({
      current_start: currentStart,
      current_end: currentEnd,
      previous_start: previousStart,
      previous_end: previousEnd,
      metric: controls.metric,
      // dimension omitted -> a single aggregate row with key: null
    });

    const row = result.rows.find((r) => r.key === null) ?? result.rows[0];
    if (!row) {
      throw new Error('compare() returned no rows');
    }

    setValue('kpi-current', formatCurrency(row.current, result.currency));
    setValue('kpi-previous', formatCurrency(row.previous, result.currency));

    const changeValue = formatSignedCurrency(row.absolute_change, result.currency);
    const changePct = row.percentage_change === null ? 'N/A' : formatPercent(row.percentage_change);
    // Cost metric: an increase is bad (red), a decrease is good (green).
    const changeClass =
      row.absolute_change > 0 ? 'change-bad' : row.absolute_change < 0 ? 'change-good' : 'change-neutral';
    setValue('kpi-change', `${changeValue} (${changePct})`);
    document.getElementById('kpi-change')?.classList.add(changeClass);
  } catch (err) {
    const message = errorMessage(err);
    setError('kpi-current', message);
    setError('kpi-previous', message);
    setError('kpi-change', message);
  }
}

// ---------------------------------------------------------------------------
// Fetch + render: month-to-date + projected run-rate (from `summary()`)
// ---------------------------------------------------------------------------

async function loadMtdCards(controls: Controls): Promise<void> {
  setLoading('kpi-mtd');
  setLoading('kpi-projected');

  const now = new Date();
  const startOfMonth = toDateIso(new Date(now.getFullYear(), now.getMonth(), 1));
  // `end` is exclusive; use tomorrow so today's data is included.
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const endExclusive = toDateIso(tomorrow);

  const dayOfMonth = now.getDate();
  const totalDaysInMonth = daysInMonth(now.getFullYear(), now.getMonth());

  try {
    const summary = await getSummary({
      start: startOfMonth,
      end: endExclusive,
      metric: controls.metric,
    });

    setValue('kpi-mtd', formatCurrency(summary.total, summary.currency));

    const projected = dayOfMonth > 0 ? (summary.total / dayOfMonth) * totalDaysInMonth : summary.total;
    setValue(
      'kpi-projected',
      formatCurrency(projected, summary.currency),
      `Based on ${dayOfMonth} of ${totalDaysInMonth} days elapsed`,
    );
  } catch (err) {
    const message = errorMessage(err);
    setError('kpi-mtd', message);
    setError('kpi-projected', message);
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initKpiCards(): void {
  const container = document.querySelector<HTMLElement>('#overview');
  if (!container) return;

  renderShell(container);

  const refresh = (): void => {
    const controls = readControls();
    if (!controls) return;
    void loadCompareCards(controls);
    void loadMtdCards(controls);
  };

  document.querySelector('#date-start')?.addEventListener('change', refresh);
  document.querySelector('#date-end')?.addEventListener('change', refresh);
  document.querySelector('#metric-select')?.addEventListener('change', refresh);

  refresh();
}
