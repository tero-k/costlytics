/**
 * S3 cost guard: before a date-range (or source) change re-runs a page's
 * queries, asks the backend what those queries would read from S3
 * (`estimateCost`) and, per the configured limits (Settings → Cost guard):
 *
 * - `soft` tier: lets the change through and shows a dismissible banner.
 * - `hard` tier: holds the change behind a confirm dialog. Continue lets it
 *   through (and remembers the choice for this session); Cancel restores
 *   the previous input values so nothing is fetched.
 *
 * Every fetching module subscribes directly to its inputs' `change` events
 * (see `controls.ts`), so the guard intercepts in the CAPTURE phase on
 * `document` — ahead of all of them — stops the original event, and
 * re-dispatches a fresh `change` once the change is allowed. It fails open:
 * if the estimate call fails or the source can't be estimated, the change
 * goes through untouched.
 */

import { estimateCost, type EstimateRange, type EstimateResponse } from '../api.ts';
import { addDaysIso, previousPeriod } from './dates.ts';

/** The page's current range selection, with inclusive end dates as in the pickers. */
export interface GuardDates {
  startIso: string;
  endIsoInclusive: string;
  previousStartIso?: string;
  previousEndIsoInclusive?: string;
}

/** The queries a page fires on refresh, as `{ current, compare }` counts. */
export interface PageQueries {
  /** Queries that read only the current range. */
  current: number;
  /** Queries that read the current AND previous range (`/cost/compare`). */
  compare: number;
}

export type GuardDecision = { action: 'proceed' } | { action: 'banner' } | { action: 'confirm' };

/** Pure: builds the estimate request's `scans` for one page refresh. */
export function buildScans(dates: GuardDates, queries: PageQueries): EstimateRange[][] {
  const current: EstimateRange = {
    start: dates.startIso,
    end: addDaysIso(dates.endIsoInclusive, 1),
  };
  const previous: EstimateRange =
    dates.previousStartIso && dates.previousEndIsoInclusive
      ? {
          start: dates.previousStartIso,
          end: addDaysIso(dates.previousEndIsoInclusive, 1),
        }
      : previousPeriod(current.start, current.end);
  return [
    ...Array.from({ length: queries.current }, () => [current]),
    ...Array.from({ length: queries.compare }, () => [current, previous]),
  ];
}

/** Pure: what to do with an estimate. `alreadyConfirmed` skips the dialog for a selection the user OK'd this session. */
export function decide(estimate: EstimateResponse, alreadyConfirmed: boolean): GuardDecision {
  if (!estimate.remote || !estimate.known) return { action: 'proceed' };
  if (estimate.tier === 'hard') return alreadyConfirmed ? { action: 'banner' } : { action: 'confirm' };
  if (estimate.tier === 'soft') return { action: 'banner' };
  return { action: 'proceed' };
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatUsd(usd: number): string {
  return usd < 0.01 ? '<$0.01' : `$${usd.toFixed(2)}`;
}

/** Pure: the one-line summary used by both the banner and the dialog. */
export function describeEstimate(estimate: EstimateResponse): string {
  const prefix = estimate.stats_missing ? 'up to ' : '~';
  return (
    `This view reads ${prefix}${formatBytes(estimate.bytes)} from S3 ` +
    `(${prefix}${formatUsd(estimate.usd)} per page load, ${estimate.requests.toLocaleString()} requests).`
  );
}

// ---------------------------------------------------------------------------
// DOM integration
// ---------------------------------------------------------------------------

const CONFIRMED_KEY = 'costlytics.costGuard.confirmed';
const BANNER_ID = 'cost-guard-banner';

function confirmedSet(): Set<string> {
  try {
    const raw = sessionStorage.getItem(CONFIRMED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

function rememberConfirmed(key: string): void {
  try {
    const set = confirmedSet();
    set.add(key);
    sessionStorage.setItem(CONFIRMED_KEY, JSON.stringify([...set]));
  } catch {
    // Storage unavailable: the dialog just shows again next time.
  }
}

function selectionKey(sourceId: string | undefined, scans: EstimateRange[][]): string {
  return JSON.stringify([sourceId ?? '', scans]);
}

function inputValue(id: string): string | undefined {
  return document.querySelector<HTMLInputElement | HTMLSelectElement>(id)?.value || undefined;
}

function readDates(): GuardDates | null {
  const startIso = inputValue('#date-start');
  const endIsoInclusive = inputValue('#date-end');
  if (!startIso || !endIsoInclusive) return null;
  return {
    startIso,
    endIsoInclusive,
    previousStartIso: inputValue('#prev-date-start'),
    previousEndIsoInclusive: inputValue('#prev-date-end'),
  };
}

export function hideCostBanner(): void {
  document.getElementById(BANNER_ID)?.remove();
}

function showCostBanner(estimate: EstimateResponse): void {
  hideCostBanner();
  const banner = document.createElement('div');
  banner.id = BANNER_ID;
  banner.className = 'cost-guard-banner';
  banner.setAttribute('role', 'status');
  const text = document.createElement('span');
  text.textContent = `${describeEstimate(estimate)} Narrow the date range to reduce it.`;
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'cost-guard-banner-close';
  close.setAttribute('aria-label', 'Dismiss');
  close.textContent = '×';
  close.addEventListener('click', hideCostBanner);
  banner.append(text, close);
  const statusBar = document.getElementById('status-bar');
  if (statusBar) statusBar.after(banner);
  else document.body.prepend(banner);
}

/** Opens the confirm dialog; resolves true for Continue, false for Cancel/Escape. */
function confirmLargeScan(estimate: EstimateResponse): Promise<boolean> {
  const dialog = document.createElement('dialog');
  dialog.className = 'cost-guard-dialog';
  dialog.innerHTML = `
    <h2>Large S3 read</h2>
    <p class="cost-guard-dialog-summary"></p>
    <p class="cost-guard-dialog-note"></p>
    <div class="cost-guard-dialog-actions">
      <button type="button" value="cancel">Cancel</button>
      <button type="button" value="continue" class="button-primary">Load anyway</button>
    </div>`;
  dialog.querySelector('.cost-guard-dialog-summary')!.textContent = describeEstimate(estimate);
  dialog.querySelector('.cost-guard-dialog-note')!.textContent =
    `That is above your ${formatUsd(estimate.hard_limit_usd)} confirmation limit (Settings → Cost guard). ` +
    'The estimate uses S3 internet egress and GET request rates and ignores any free allowance.';
  document.body.append(dialog);

  return new Promise((resolve) => {
    let result = false;
    for (const button of dialog.querySelectorAll('button')) {
      button.addEventListener('click', () => {
        result = button.value === 'continue';
        dialog.close();
      });
    }
    dialog.addEventListener('close', () => {
      dialog.remove();
      resolve(result);
    });
    dialog.showModal();
    dialog.querySelector<HTMLButtonElement>('button[value="cancel"]')?.focus();
  });
}

/** Estimates the given selection; `null` (fail open) if the call fails. */
async function estimateSelection(
  queries: PageQueries,
  sourceId: string | undefined,
): Promise<{ estimate: EstimateResponse; key: string } | null> {
  const dates = readDates();
  if (!dates) return null;
  const scans = buildScans(dates, queries);
  try {
    const estimate = await estimateCost(scans, sourceId);
    return {
      estimate,
      key: selectionKey(sourceId ?? inputValue('#source-picker'), scans),
    };
  } catch {
    return null;
  }
}

/** Replayed `change` events; shared so no guard ever re-intercepts one. */
const replayed = new WeakSet<Event>();

const GUARDED_IDS = ['date-start', 'date-end', 'prev-date-start', 'prev-date-end', 'source-picker'];

/**
 * Installs the change interceptor for this page. Call once, after the
 * source picker is initialized and before the first fetch. `queries` is
 * what one refresh of this page fires (see each page's entry module).
 * Returns a function that removes the interceptor (used by tests).
 */
export function installCostGuard(queries: PageQueries): () => void {
  const committed = new Map<string, string>();
  const snapshot = (): void => {
    for (const id of GUARDED_IDS) {
      const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
      if (el) committed.set(id, el.value);
    }
  };
  snapshot();
  // Serializes checks so a second change waits for the first dialog.
  let queue: Promise<void> = Promise.resolve();

  const onChange = (event: Event): void => {
    const target = event.target as HTMLInputElement | HTMLSelectElement | null;
    if (!target || !GUARDED_IDS.includes(target.id) || replayed.has(event)) return;
    event.stopImmediatePropagation();

    queue = queue.then(async () => {
      // Nothing to do if the value is back to what downstream already has
      // (e.g. a second `change` for an edit that was just cancelled).
      if (target.value === committed.get(target.id)) return;
      const sourceId = target.id === 'source-picker' ? target.value || undefined : undefined;
      const result = await estimateSelection(queries, sourceId);
      const decision: GuardDecision = result
        ? decide(result.estimate, confirmedSet().has(result.key))
        : { action: 'proceed' };

      if (decision.action === 'confirm' && result) {
        if (!(await confirmLargeScan(result.estimate))) {
          // Restore every guarded input: nothing downstream saw the change.
          for (const [id, value] of committed) {
            const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement | null;
            if (el) el.value = value;
          }
          return;
        }
        rememberConfirmed(result.key);
      }
      if (decision.action === 'proceed') hideCostBanner();
      else if (result) showCostBanner(result.estimate);

      snapshot();
      const replay = new Event('change', { bubbles: true });
      replayed.add(replay);
      target.dispatchEvent(replay);
    });
  };
  document.addEventListener('change', onChange, { capture: true });
  return () => document.removeEventListener('change', onChange, { capture: true });
}

/**
 * Initial-load check: the page loads regardless (there is no previous
 * selection to fall back to), but an expensive default selection gets the
 * banner so the user knows before the next refresh.
 */
export async function checkInitialLoad(queries: PageQueries): Promise<void> {
  const result = await estimateSelection(queries, undefined);
  if (result && decide(result.estimate, true).action !== 'proceed') showCostBanner(result.estimate);
}
