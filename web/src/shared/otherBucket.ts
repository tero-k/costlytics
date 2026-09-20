/**
 * Synthetic "Other" bucket computation shared by `topBreakdown.ts` (Overview
 * page) and `serviceBreakdowns.ts` (Service Detail page): both combine a
 * top-N breakdown's rows with an overall (unfiltered-by-dimension) total into
 * the row list a horizontal bar chart should render, adding a synthetic
 * "Other" row for the remainder when it's non-trivial.
 *
 * This is a different computation from `explorerTrend.ts`/`explorerTable.ts`'s
 * Top-N logic, which folds remaining rows per time period rather than against
 * one overall total — that logic is intentionally not shared with this one.
 */

import type { BreakdownRow } from '../api.ts';
import { formatKeyLabel } from './labels.ts';

/** Below this fraction of the overall total, the "Other" bucket is omitted as negligible. */
export const OTHER_EPSILON_FRACTION = 0.001;

/**
 * Combines a top-N breakdown's rows with an overall (unfiltered-by-dimension)
 * total into the row list a horizontal bar chart should render, adding a
 * synthetic "Other" row for the remainder when it's non-trivial.
 */
export function buildRowsWithOther(rows: BreakdownRow[], overallTotal: number): Array<{ label: string; total: number }> {
  const result: Array<{ label: string; total: number }> = rows.map((row) => ({
    label: formatKeyLabel(row.key),
    total: row.total,
  }));

  const sumOfRows = rows.reduce((acc, row) => acc + row.total, 0);
  const remainder = overallTotal - sumOfRows;
  // Compare magnitudes on both sides so this holds symmetrically for a
  // negative overallTotal (credit/refund-dominated periods) too — see
  // otherBucket.test.ts for the negative-total regression this fixes.
  if (Math.abs(remainder) > Math.abs(overallTotal) * OTHER_EPSILON_FRACTION) {
    result.push({ label: 'Other', total: remainder });
  }

  return result;
}
