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
