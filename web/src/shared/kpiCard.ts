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
  /** Start hidden (e.g. the credits card, shown only when a period has credits). */
  hidden?: boolean;
}

export interface KpiValueOptions {
  /** Text for the card's `.sub` line (e.g. a run-rate caveat). */
  sub?: string;
  /** Extra class added to the card element itself (e.g. a change-good/bad/neutral class). */
  cardClass?: string;
  /** A small delta badge next to the value, e.g. `+12.3%` toned by `changeClass`. */
  pill?: { text: string; tone: 'change-bad' | 'change-good' | 'change-neutral' };
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
      ({ id, label, hidden }) => `
      <div class="kpi-card loading" id="${id}"${hidden ? ' hidden' : ''}>
        <div class="label">${label}</div>
        <div class="value-row"><div class="value">&hellip;</div><span class="pill" hidden></span></div>
        <div class="sub"></div>
        <div class="spark" aria-hidden="true"></div>
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
  hidePill(card);
}

function hidePill(card: HTMLElement): void {
  const pill = card.querySelector<HTMLElement>('.pill');
  if (pill) pill.hidden = true;
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
  hidePill(card);
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
  const pill = card.querySelector<HTMLElement>('.pill');
  if (pill) {
    pill.hidden = !options?.pill;
    pill.className = `pill ${options?.pill?.tone ?? ''}`;
    pill.textContent = options?.pill?.text ?? '';
  }
}

/**
 * SVG path data for a sparkline through `values`, scaled into a `width` x
 * `height` box (y inverted; a flat series sits mid-height). Returns the
 * line and the closed area under it. @internal exported for tests
 */
export function sparklinePaths(values: number[], width: number, height: number): { line: string; area: string } | null {
  if (values.length < 2) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min;
  const pad = 2;
  const points = values.map((v, i) => {
    const x = (i / (values.length - 1)) * width;
    const y = span === 0 ? height / 2 : pad + (1 - (v - min) / span) * (height - pad * 2);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = `M${points.join(' L')}`;
  return { line, area: `${line} L${width},${height} L0,${height} Z` };
}

/**
 * Draws a sparkline of `values` along the bottom of a card (e.g. the total
 * card, fed by the trend chart's already-fetched series). Fewer than two
 * points clears it.
 */
export function setKpiSparkline(id: string, values: number[]): void {
  const spark = document.getElementById(id)?.querySelector<HTMLElement>('.spark');
  if (!spark) return;
  const paths = sparklinePaths(values, 100, 28);
  spark.innerHTML = paths
    ? `<svg viewBox="0 0 100 28" preserveAspectRatio="none"><path class="spark-area" d="${paths.area}"/><path class="spark-line" d="${paths.line}" vector-effect="non-scaling-stroke"/></svg>`
    : '';
}

/**
 * Shows the credits card for a period — credits as the value, with gross
 * charges and the share credits cancel out underneath — or hides it when
 * the period has none, so accounts without credits see no extra card.
 * Fed by the trend's already-fetched, category-grouped series.
 */
export function setKpiCredits(
  id: string,
  totals: { charges: number; credits: number } | null,
  format: (value: number) => string,
): void {
  const card = document.getElementById(id);
  if (!card) return;
  if (!totals || totals.credits === 0) {
    card.hidden = true;
    return;
  }
  card.hidden = false;
  const share = totals.charges > 0 ? ` · ${Math.round((Math.abs(totals.credits) / totals.charges) * 100)}% of charges` : '';
  setKpiValue(id, format(totals.credits), { sub: `Before credits: ${format(totals.charges)}${share}`, cardClass: 'credits' });
}
