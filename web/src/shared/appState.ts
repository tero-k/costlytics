/**
 * Shared, persisted page state: the date range and cost metric every page
 * shows, plus the selected source. Each page is a separate HTML document, so
 * this state lives in localStorage (survives navigation and app restarts)
 * and is mirrored into the URL (`?from=&to=&metric=`, plus the source
 * picker's own `?source=`) so a copied link reproduces the view.
 *
 * Resolution on load, per field: URL param → stored value → default
 * (month to date, amortized). Invalid values fall through to the next tier.
 *
 * The DOM inputs (`#date-start` / `#date-end` / `#metric-select`) remain the
 * live state every fetching module reads (see `controls.ts`); this module
 * only seeds them on load and records their committed changes. Its change
 * listeners sit on the inputs themselves, so they run after
 * `costGuard.ts`'s capture-phase interception — a change the user cancels
 * in the cost dialog is never persisted.
 */

import type { CostMetric } from '../api.ts';
import { toLocalIso } from './datePresets.ts';

export interface SharedState {
  /** Inclusive `YYYY-MM-DD`. */
  startIso: string;
  /** Inclusive `YYYY-MM-DD`. */
  endIso: string;
  metric: CostMetric;
}

interface StoredState extends Partial<SharedState> {
  sourceId?: string;
}

const STORAGE_KEY = 'costlytics.shared.v1';
const METRICS: readonly CostMetric[] = ['amortized', 'billed', 'list', 'contracted'];
const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

export const URL_PARAMS = { start: 'from', end: 'to', metric: 'metric' } as const;

function isIsoDate(value: unknown): value is string {
  return typeof value === 'string' && ISO_DATE.test(value) && !Number.isNaN(Date.parse(`${value}T00:00:00Z`));
}

function isMetric(value: unknown): value is CostMetric {
  return typeof value === 'string' && (METRICS as readonly string[]).includes(value);
}

export function defaultSharedState(today: Date): SharedState {
  return {
    startIso: toLocalIso(new Date(today.getFullYear(), today.getMonth(), 1)),
    endIso: toLocalIso(today),
    metric: 'amortized',
  };
}

/**
 * Pure resolution of the shared state from URL params and stored state.
 * The date range resolves as a pair: if the tier-by-tier pick yields an
 * inverted range (e.g. a URL `from` later than the stored `to`), both dates
 * fall back to the default range rather than silently swapping.
 */
export function resolveSharedState(params: URLSearchParams, stored: StoredState, today: Date): SharedState {
  const defaults = defaultSharedState(today);
  const pick = <T>(candidates: unknown[], valid: (v: unknown) => v is T, fallback: T): T =>
    (candidates.find(valid) as T | undefined) ?? fallback;

  let startIso = pick([params.get(URL_PARAMS.start), stored.startIso], isIsoDate, defaults.startIso);
  let endIso = pick([params.get(URL_PARAMS.end), stored.endIso], isIsoDate, defaults.endIso);
  if (startIso > endIso) {
    startIso = defaults.startIso;
    endIso = defaults.endIso;
  }
  const metric = pick([params.get(URL_PARAMS.metric), stored.metric], isMetric, defaults.metric);
  return { startIso, endIso, metric };
}

function readStored(): StoredState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    return parsed && typeof parsed === 'object' ? (parsed as StoredState) : {};
  } catch {
    return {};
  }
}

function writeStored(patch: StoredState): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...readStored(), ...patch }));
  } catch {
    // Storage unavailable (private mode, blocked): state just won't persist.
  }
}

function syncUrl(state: SharedState): void {
  const url = new URL(window.location.href);
  url.searchParams.set(URL_PARAMS.start, state.startIso);
  url.searchParams.set(URL_PARAMS.end, state.endIso);
  url.searchParams.set(URL_PARAMS.metric, state.metric);
  window.history.replaceState(null, '', url);
}

/** Last source the user picked on any page, or `null`. Validated by `sourcePicker.ts` against registered sources. */
export function getStoredSourceId(): string | null {
  const id = readStored().sourceId;
  return typeof id === 'string' && id ? id : null;
}

export function persistSourceId(id: string | null): void {
  writeStored({ sourceId: id ?? undefined });
}

function readInputs(): SharedState | null {
  const start = document.querySelector<HTMLInputElement>('#date-start')?.value;
  const end = document.querySelector<HTMLInputElement>('#date-end')?.value;
  const metric = document.querySelector<HTMLSelectElement>('#metric-select')?.value;
  if (!isIsoDate(start) || !isIsoDate(end) || !isMetric(metric)) return null;
  return { startIso: start, endIso: end, metric };
}

/**
 * Seeds `#date-start` / `#date-end` / `#metric-select` from the resolved
 * shared state and persists every later committed change. A no-op on pages
 * without those inputs (Settings). Returns the applied state, or `null`.
 */
export function initSharedControls(today: Date = new Date()): SharedState | null {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const metricSelect = document.querySelector<HTMLSelectElement>('#metric-select');
  if (!startInput || !endInput) return null;

  const state = resolveSharedState(new URL(window.location.href).searchParams, readStored(), today);
  startInput.value = state.startIso;
  endInput.value = state.endIso;
  if (metricSelect) metricSelect.value = state.metric;
  writeStored(state);
  syncUrl(state);

  const onCommit = (): void => {
    const current = readInputs();
    if (!current || current.startIso > current.endIso) return;
    writeStored(current);
    syncUrl(current);
  };
  for (const el of [startInput, endInput, metricSelect]) el?.addEventListener('change', onCommit);

  return state;
}
