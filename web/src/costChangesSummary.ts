/**
 * Summary KPI cards for the Cost Changes page (plan §30), rendered into
 * `#changes-summary`.
 *
 * Calls `getCompare()` with NO `dimension`, so the API returns a single
 * aggregate row (`key: null`) covering the whole current-vs-previous period
 * pair — unlike `costChangesMovers.ts`, which requests the same endpoint
 * WITH the page's selected dimension. Reuses `kpiCards.ts`'s CSS
 * classes/card pattern (`.kpi-grid` / `.kpi-card`) rather than inventing a
 * new card style, per the task's mandatory-reuse constraint.
 *
 * Renders 4 cards: Current total, Previous total, Change (absolute), and
 * Change % ("N/A" when `percentage_change` is `null`, i.e. previous === 0).
 */

import { getCompare } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, formatPercent, formatSignedCurrency, changeClass, errorMessage } from './shared/format.ts';
import { readChangesControls, type ChangesControls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'changes-summary';

const CARD_DEFS: Array<{ id: string; label: string }> = [
  { id: 'changes-kpi-current', label: 'Current period total' },
  { id: 'changes-kpi-previous', label: 'Previous period total' },
  { id: 'changes-kpi-change', label: 'Change' },
  { id: 'changes-kpi-change-pct', label: 'Change %' },
];

const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// DOM helpers (mirrors `kpiCards.ts`'s shell/loading/error/value helpers)
// ---------------------------------------------------------------------------

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

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

function setValue(id: string, value: string, subClass?: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', 'error');
  const valueEl = card.querySelector<HTMLElement>('.value');
  if (valueEl) valueEl.textContent = value;
  if (subClass) card.classList.add(subClass);
}

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadSummary(
  controls: ChangesControls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const allIds = CARD_DEFS.map((c) => c.id);
  allIds.forEach(setLoading);

  // The canonical `end` is exclusive; both date pickers' "To" values are
  // inclusive, so both requests' `*_end` are the day after them.
  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const previousStart = controls.previousStartIso;
  const previousEnd = addDaysIso(controls.previousEndIsoInclusive, 1);

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
    setValue('changes-kpi-current', formatCurrency(row.current, result.currency));
    setValue('changes-kpi-previous', formatCurrency(row.previous, result.currency));

    const cls = changeClass(row.absolute_change);
    setValue('changes-kpi-change', formatSignedCurrency(row.absolute_change, result.currency), cls);
    const changePctText = row.percentage_change === null ? 'N/A' : formatPercent(row.percentage_change);
    setValue('changes-kpi-change-pct', changePctText, cls);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    allIds.forEach((id) => setError(id, message));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initChangesSummary(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  renderShell(container);

  const refresh = async (): Promise<void> => {
    const controls = readChangesControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadSummary(controls, token, onCurrency);
  };

  return refresh();
}
