/**
 * Generic KPI summary for a "detail page" entity (plan §26), e.g. the
 * Service Detail or Account Detail page's currently selected service/account.
 *
 * Renders three cards into `#{idPrefix}-kpi` — total cost, row count, and
 * currency — from `POST /cost/summary` filtered to the currently selected
 * entity (`config.buildFilter(selected)`) for the page's shared date range
 * and metric. Reuses the same `.kpi-grid`/`.kpi-card` visual pattern as
 * `kpiCards.ts` (Overview page).
 *
 * Generalized from `serviceKpi.ts`/`accountKpi.ts` (Session 11) once Session
 * 10's whole-branch review found those two modules had zero code differences
 * beyond entity-noun substitution — see `shared/entityConfig.ts`'s doc
 * comment for the substitution points this module parameterizes on.
 *
 * Re-fetches whenever the entity picker or the shared date/metric controls
 * change (`subscribeToControls(refresh, { extraIds: [config.pickerSelector] })`).
 * When no entity is selected (`config.getSelected()` returns `null`), the
 * component clears itself and skips fetching entirely, per this page family's
 * "nothing to fetch yet" convention.
 */

import { getSummary } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, errorMessage } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import type { EntityConfig } from './shared/entityConfig.ts';

interface CardIds {
  total: string;
  rows: string;
  currency: string;
}

function cardIds(idPrefix: string): CardIds {
  return {
    total: `${idPrefix}-kpi-total`,
    rows: `${idPrefix}-kpi-rows`,
    currency: `${idPrefix}-kpi-currency`,
  };
}

function getContainer(idPrefix: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${idPrefix}-kpi`);
}

function renderShell(container: HTMLElement, ids: CardIds): void {
  const cardsHtml = [
    { id: ids.total, label: 'Total cost' },
    { id: ids.rows, label: 'Row count' },
    { id: ids.currency, label: 'Currency' },
  ]
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

function setLoading(id: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('error');
  card.classList.add('loading');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = '…';
  if (sub) sub.textContent = '';
}

function setError(id: string, message: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading');
  card.classList.add('error');
  const value = card.querySelector<HTMLElement>('.value');
  const sub = card.querySelector<HTMLElement>('.sub');
  if (value) value.textContent = 'Error';
  if (sub) sub.textContent = message;
}

function setValue(id: string, value: string, sub?: string): void {
  const card = document.getElementById(id);
  if (!card) return;
  card.classList.remove('loading', 'error');
  const valueEl = card.querySelector<HTMLElement>('.value');
  const subEl = card.querySelector<HTMLElement>('.sub');
  if (valueEl) valueEl.textContent = value;
  if (subEl) subEl.textContent = sub ?? '';
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/** Handle returned by `initEntityKpi`: a stable, reusable `refresh()` that reuses the same `RequestGuard` and does NOT re-register control listeners on repeat calls. */
export interface EntityKpiHandle {
  refresh: () => Promise<void>;
}

export function initEntityKpi(
  config: EntityConfig,
  onCurrency?: (currency: string) => void,
): EntityKpiHandle {
  const container = getContainer(config.idPrefix);
  if (!container) return { refresh: () => Promise.resolve() };

  const ids = cardIds(config.idPrefix);
  const refreshGuard = new RequestGuard();

  renderShell(container, ids);

  async function loadKpiCards(selected: string, controls: Controls, token: number): Promise<void> {
    setLoading(ids.total);
    setLoading(ids.rows);
    setLoading(ids.currency);

    const start = controls.startIso;
    // `end` is exclusive; the date picker's "To" value is inclusive.
    const end = addDaysIso(controls.endIsoInclusive, 1);

    try {
      const summary = await getSummary({
        start,
        end,
        metric: controls.metric,
        ...config.buildFilter(selected),
      });

      if (!refreshGuard.isCurrent(token)) return;

      onCurrency?.(summary.currency);
      setValue(ids.total, formatCurrency(summary.total, summary.currency));
      setValue(ids.rows, summary.row_count.toLocaleString());
      setValue(ids.currency, summary.currency || 'N/A');
    } catch (err) {
      if (!refreshGuard.isCurrent(token)) return;
      const message = errorMessage(err);
      setError(ids.total, message);
      setError(ids.rows, message);
      setError(ids.currency, message);
    }
  }

  const refresh = async (): Promise<void> => {
    const selected = config.getSelected();
    if (!selected) {
      refreshGuard.next();
      renderShell(container, ids);
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadKpiCards(selected, controls, token);
  };

  subscribeToControls(refresh, { extraIds: [config.pickerSelector] });

  return { refresh };
}
