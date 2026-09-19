/**
 * KPI summary for the Account Detail page's selected account (plan §26).
 *
 * Renders three cards into `#account-kpi` — total cost, row count, and
 * currency — from `POST /cost/summary` filtered to the currently selected
 * account (`accounts: [selectedAccount]`) for the page's shared date range
 * and metric. Reuses the same `.kpi-grid`/`.kpi-card` visual pattern as
 * `kpiCards.ts` (Overview page) / `serviceKpi.ts` (Service Detail page)
 * rather than inventing a new card style.
 *
 * Re-fetches whenever the account picker or the shared date/metric controls
 * change (`subscribeToControls(refresh, { extraIds: ['#account-picker'] })`).
 * When no account is selected (`getSelectedAccount()` returns `null`), the
 * component clears itself and skips fetching entirely, per this page's
 * "nothing to fetch yet" convention (see `accountDetailMain.ts`).
 */

import { getSummary } from './api.ts';
import { getSelectedAccount } from './shared/accountPicker.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, errorMessage } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'account-kpi';

const CARD_DEFS: Array<{ id: string; label: string }> = [
  { id: 'account-kpi-total', label: 'Total cost' },
  { id: 'account-kpi-rows', label: 'Row count' },
  { id: 'account-kpi-currency', label: 'Currency' },
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
  account: string,
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  setLoading('account-kpi-total');
  setLoading('account-kpi-rows');
  setLoading('account-kpi-currency');

  const start = controls.startIso;
  // `end` is exclusive; the date picker's "To" value is inclusive.
  const end = addDaysIso(controls.endIsoInclusive, 1);

  try {
    const summary = await getSummary({
      start,
      end,
      metric: controls.metric,
      accounts: [account],
    });

    if (!refreshGuard.isCurrent(token)) return;

    onCurrency?.(summary.currency);
    setValue('account-kpi-total', formatCurrency(summary.total, summary.currency));
    setValue('account-kpi-rows', summary.row_count.toLocaleString());
    setValue('account-kpi-currency', summary.currency || 'N/A');
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    setError('account-kpi-total', message);
    setError('account-kpi-rows', message);
    setError('account-kpi-currency', message);
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initAccountKpi(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  renderShell(container);

  const refresh = async (): Promise<void> => {
    const account = getSelectedAccount();
    if (!account) {
      refreshGuard.next();
      renderShell(container);
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadKpiCards(account, controls, token, onCurrency);
  };

  subscribeToControls(refresh, { extraIds: ['#account-picker'] });

  return refresh();
}
