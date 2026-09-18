/**
 * KPI summary for the Service Detail page's selected service (plan §26).
 *
 * Renders three cards into `#service-kpi` — total cost, row count, and
 * currency — from `POST /cost/summary` filtered to the currently selected
 * service (`services: [selectedService]`) for the page's shared date range
 * and metric. Reuses the same `.kpi-grid`/`.kpi-card` visual pattern as
 * `kpiCards.ts` (Overview page) rather than inventing a new card style.
 *
 * Re-fetches whenever the service picker or the shared date/metric controls
 * change (`subscribeToControls(refresh, { extraIds: ['#service-picker'] })`).
 * When no service is selected (`getSelectedService()` returns `null`), the
 * component clears itself and skips fetching entirely, per this page's
 * "nothing to fetch yet" convention (see `serviceDetailMain.ts`).
 */

import { getSummary } from './api.ts';
import { getSelectedService } from './serviceDetailMain.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, errorMessage } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'service-kpi';

const CARD_DEFS: Array<{ id: string; label: string }> = [
  { id: 'service-kpi-total', label: 'Total cost' },
  { id: 'service-kpi-rows', label: 'Row count' },
  { id: 'service-kpi-currency', label: 'Currency' },
];

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

const refreshGuard = new RequestGuard();

async function loadKpiCards(
  service: string,
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  setLoading('service-kpi-total');
  setLoading('service-kpi-rows');
  setLoading('service-kpi-currency');

  const start = controls.startIso;
  // `end` is exclusive; the date picker's "To" value is inclusive.
  const end = addDaysIso(controls.endIsoInclusive, 1);

  try {
    const summary = await getSummary({
      start,
      end,
      metric: controls.metric,
      services: [service],
    });

    if (!refreshGuard.isCurrent(token)) return;

    onCurrency?.(summary.currency);
    setValue('service-kpi-total', formatCurrency(summary.total, summary.currency));
    setValue('service-kpi-rows', summary.row_count.toLocaleString());
    setValue('service-kpi-currency', summary.currency || 'N/A');
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    setError('service-kpi-total', message);
    setError('service-kpi-rows', message);
    setError('service-kpi-currency', message);
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initServiceKpi(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  renderShell(container);

  const refresh = async (): Promise<void> => {
    const service = getSelectedService();
    if (!service) {
      renderShell(container);
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadKpiCards(service, controls, token, onCurrency);
  };

  subscribeToControls(refresh, { extraIds: ['#service-picker'] });

  return refresh();
}
