/**
 * Shared date-range / metric control reading for the Overview page. All
 * fetching modules read the same `#date-start` / `#date-end` /
 * `#metric-select` DOM inputs, which act as the page's shared state (see
 * `main.ts`'s module doc comment).
 */

import type { CostMetric } from '../api.ts';

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
