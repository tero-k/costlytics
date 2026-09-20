/**
 * Shared DOM-shell helpers for the `.kpi-grid` / `.kpi-card` visual pattern
 * used across the Overview page (`kpiCards.ts`), the entity detail pages'
 * generic KPI summary (`entityKpi.ts`), and the Cost Changes summary
 * (`costChangesSummary.ts`).
 *
 * This module owns ONLY the DOM plumbing: render the card grid once, then
 * toggle each card between loading/error/value states. Each consumer keeps
 * its own fetch logic, card definitions (which cards, what data), and
 * currency threading exactly as before — only this shell moved.
 */

export interface KpiCardDef {
  id: string;
  label: string;
}

export interface KpiValueOptions {
  /** Text for the card's `.sub` line (e.g. a run-rate caveat). */
  sub?: string;
  /** Extra class added to the card element itself (e.g. a change-good/bad/neutral class). */
  cardClass?: string;
}

// Classes a card may have picked up from a previous `change` styling pass
// (Overview's "Change" card, Cost Changes' "Change"/"Change %" cards). Always
// cleared on loading/error so a stale change-* class never lingers from a
// prior render; a no-op for cards that never receive them.
const CHANGE_CLASSES = ['change-bad', 'change-good', 'change-neutral'];

/** Renders the initial (loading) card grid into `container`. */
export function renderKpiShell(container: HTMLElement, cardDefs: readonly KpiCardDef[]): void {
  const cardsHtml = cardDefs
    .map(
      ({ id, label }) => `
      <div class="kpi-card loading" id="${id}">
        <div class="label">${label}</div>
        <div class="value">&hellip;</div>
        <div class="sub"></div>
      </div>`,
    )
    .join('');

  container.innerHTML = `<div class="kpi-grid">${cardsHtml}</div>`;
}

export function setKpiLoading(id: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('error', ...CHANGE_CLASSES);
  card.classList.add('loading');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = '…';
  if (sub) sub.textContent = '';
}

export function setKpiError(id: string, message: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', ...CHANGE_CLASSES);
  card.classList.add('error');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = 'Error';
  if (sub) sub.textContent = message;
}

export function setKpiValue(id: string, value: string, options?: KpiValueOptions): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', 'error');
  const valueEl = card.querySelector<HTMLElement>('.value');
  const subEl = card.querySelector<HTMLElement>('.sub');
  if (valueEl) valueEl.textContent = value;
  if (subEl) subEl.textContent = options?.sub ?? '';
  if (options?.cardClass) card.classList.add(options.cardClass);
}
