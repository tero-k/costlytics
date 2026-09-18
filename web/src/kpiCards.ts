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

import { getCompare, getSummary } from './api.ts';
import { addDaysIso, previousPeriod } from './shared/dates.ts';
import { formatCurrency, errorMessage, formatPercent, formatSignedCurrency, changeClass } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

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
// Date helpers not shared with other modules (MTD-specific)
// ---------------------------------------------------------------------------

function toDateIso(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function daysInMonth(year: number, monthIndex0: number): number {
  return new Date(year, monthIndex0 + 1, 0).getDate();
}

// Guards both fetch groups within a single `refresh()` cycle together.
const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// Fetch + render: current / previous / change (from `compare()`)
// ---------------------------------------------------------------------------

async function loadCompareCards(
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  setLoading('kpi-current');
  setLoading('kpi-previous');
  setLoading('kpi-change');

  // The canonical `end` is exclusive; the date picker's "To" value is
  // inclusive, so the request's current_end is the day after it.
  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);

  // Previous period: the same-length window immediately preceding the
  // current one (previous_end === current_start).
  const { start: previousStart, end: previousEnd } = previousPeriod(currentStart, currentEnd);

  try {
    const result = await getCompare({
      current_start: currentStart,
      current_end: currentEnd,
      previous_start: previousStart,
      previous_end: previousEnd,
      metric: controls.metric,
      // dimension omitted -> a single aggregate row with key: null
    });

    if (!refreshGuard.isCurrent(token)) return;

    const row = result.rows.find((r) => r.key === null) ?? result.rows[0];
    if (!row) {
      throw new Error('compare() returned no rows');
    }

    onCurrency?.(result.currency);
    setValue('kpi-current', formatCurrency(row.current, result.currency));
    setValue('kpi-previous', formatCurrency(row.previous, result.currency));

    const changeValue = formatSignedCurrency(row.absolute_change, result.currency);
    const changePct = row.percentage_change === null ? 'N/A' : formatPercent(row.percentage_change);
    setValue('kpi-change', `${changeValue} (${changePct})`);
    document.getElementById('kpi-change')?.classList.add(changeClass(row.absolute_change));
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    setError('kpi-current', message);
    setError('kpi-previous', message);
    setError('kpi-change', message);
  }
}

// ---------------------------------------------------------------------------
// Fetch + render: month-to-date + projected run-rate (from `summary()`)
// ---------------------------------------------------------------------------

async function loadMtdCards(
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
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

    if (!refreshGuard.isCurrent(token)) return;

    onCurrency?.(summary.currency);
    setValue('kpi-mtd', formatCurrency(summary.total, summary.currency));

    const projected = dayOfMonth > 0 ? (summary.total / dayOfMonth) * totalDaysInMonth : summary.total;
    setValue(
      'kpi-projected',
      formatCurrency(projected, summary.currency),
      `Based on ${dayOfMonth} of ${totalDaysInMonth} days elapsed`,
    );
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    setError('kpi-mtd', message);
    setError('kpi-projected', message);
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initKpiCards(onCurrency?: (currency: string) => void): Promise<void> {
  const container = document.querySelector<HTMLElement>('#overview');
  if (!container) return Promise.resolve();

  renderShell(container);

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await Promise.allSettled([
      loadCompareCards(controls, token, onCurrency),
      loadMtdCards(controls, token, onCurrency),
    ]);
  };

  subscribeToControls(refresh);

  return refresh();
}
