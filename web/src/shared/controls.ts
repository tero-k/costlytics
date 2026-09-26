/**
 * Shared date-range / metric control reading for the Overview page. All
 * fetching modules read the same `#date-start` / `#date-end` /
 * `#metric-select` DOM inputs, which act as the page's shared state (see
 * `main.ts`'s module doc comment).
 */

import type { CostMetric, Dimension } from '../api.ts';

export interface Controls {
  metric: CostMetric;
  /** Inclusive start date (`YYYY-MM-DD`), as selected in the date picker. */
  startIso: string;
  /** Inclusive end date (`YYYY-MM-DD`), as selected in the date picker. */
  endIsoInclusive: string;
}

export function readControls(): Controls | null {
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

/**
 * Cost Explorer page-local extension of {@link Controls}, adding the
 * "Group by" dimension and "Top N" controls that only exist on that page
 * (`#dimension-select` / `#top-n-input` in `explorer.html`). Composes the
 * shared type rather than growing it, so the Overview page's usage of
 * `Controls`/`readControls()` is unaffected.
 */
export interface ExplorerControls extends Controls {
  dimension: Dimension;
  topN: number;
}

const DEFAULT_DIMENSION: Dimension = 'service';
const DEFAULT_TOP_N = 10;

export function readExplorerControls(): ExplorerControls | null {
  const base = readControls();
  if (!base) return null;

  const dimensionSelect = document.querySelector<HTMLSelectElement>('#dimension-select');
  const topNInput = document.querySelector<HTMLInputElement>('#top-n-input');

  const dimension = (dimensionSelect?.value as Dimension | undefined) ?? DEFAULT_DIMENSION;
  const parsedTopN = topNInput?.value ? Number.parseInt(topNInput.value, 10) : NaN;
  const topN = Number.isFinite(parsedTopN) && parsedTopN > 0 ? parsedTopN : DEFAULT_TOP_N;

  return { ...base, dimension, topN };
}

/**
 * Cost Changes page-local extension of {@link Controls}, adding an
 * INDEPENDENT previous-period date range (`#prev-date-start` /
 * `#prev-date-end` in `cost-changes.html`) plus the "Group by" dimension
 * (reusing the same `#dimension-select` id/options list as Explorer's).
 *
 * Unlike Explorer's auto-derived previous period (`previousPeriod()`,
 * recomputed at read-time from the current range), this page's previous
 * period is read directly from its own DOM inputs so the two ranges can be
 * set to arbitrary, unrelated spans (e.g. "this month vs. the same month
 * last year").
 */
export interface ChangesControls extends Controls {
  previousStartIso: string;
  previousEndIsoInclusive: string;
  dimension: Dimension;
  /** "Top N" control (`#top-n-input`, same id/pattern as Explorer's), used by `costChangesTable.ts`'s Top-N + "Other" folding. */
  topN: number;
}

export function readChangesControls(): ChangesControls | null {
  const base = readControls();
  if (!base) return null;

  const prevStartInput = document.querySelector<HTMLInputElement>('#prev-date-start');
  const prevEndInput = document.querySelector<HTMLInputElement>('#prev-date-end');
  if (!prevStartInput?.value || !prevEndInput?.value) return null;

  const dimensionSelect = document.querySelector<HTMLSelectElement>('#dimension-select');
  const dimension = (dimensionSelect?.value as Dimension | undefined) ?? DEFAULT_DIMENSION;

  const topNInput = document.querySelector<HTMLInputElement>('#top-n-input');
  const parsedTopN = topNInput?.value ? Number.parseInt(topNInput.value, 10) : NaN;
  const topN = Number.isFinite(parsedTopN) && parsedTopN > 0 ? parsedTopN : DEFAULT_TOP_N;

  return {
    ...base,
    previousStartIso: prevStartInput.value,
    previousEndIsoInclusive: prevEndInput.value,
    dimension,
    topN,
  };
}

/**
 * Subscribes `refresh` to `change` events on the shared date/metric
 * controls (and, for the Cost Explorer page, the dimension/top-N controls;
 * or for the Cost Changes page, the independent previous-period range and
 * dimension controls), so every fetching module doesn't have to repeat its
 * own `document.querySelector(...)?.addEventListener('change', ...)` block.
 *
 * `#source-picker` (Session 14 Task 3) is included UNCONDITIONALLY,
 * alongside `#date-start`/`#date-end`/`#metric-select`, on every page — like
 * those three, it's a control every page has (see `shared/sourcePicker.ts`)
 * and every fetching module must re-fetch on. Including it here, rather
 * than editing every module's own `subscribeToControls(...)` call site to
 * add it as an `extraIds` entry, is what lets switching sources re-fetch
 * every component WITHOUT touching `kpiCards.ts`/`entityKpi.ts`/etc.
 * (mirrors `api.ts`'s `postJson`/`getJson` choke point for the same reason).
 *
 * `#page-filters` (the shell's per-page Service/Account filter slot,
 * `shared/pageFilters.ts`) is included unconditionally for the same reason:
 * its widgets bubble one `change` per committed selection, and the
 * selection reaches requests via `api.ts`'s page-filter provider.
 *
 * `opts.extraIds` covers page-local controls beyond the shared set (e.g.
 * the Overview page's `#granularity-select`, used only by `trendChart.ts`).
 */
export function subscribeToControls(
  refresh: () => void,
  opts?: { explorer?: boolean; changes?: boolean; extraIds?: string[] },
): void {
  const ids = ['#date-start', '#date-end', '#metric-select', '#source-picker', '#page-filters'];
  if (opts?.explorer) ids.push('#dimension-select', '#top-n-input');
  if (opts?.changes) ids.push('#prev-date-start', '#prev-date-end', '#dimension-select', '#top-n-input');
  if (opts?.extraIds) ids.push(...opts.extraIds);

  for (const id of ids) {
    document.querySelector(id)?.addEventListener('change', () => void refresh());
  }
}
