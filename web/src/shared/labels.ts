/**
 * Dimension-key labeling helper shared by `topBreakdown.ts` (Overview page)
 * and `explorerTrend.ts` (Cost Explorer page) — both render rows/series
 * keyed by a `string | null` dimension value (`BreakdownRow.key` /
 * `TimeSeriesPoint.group`), where `null` (or an empty string) means
 * untagged/uncategorized data and should be labeled "(none)" rather than
 * left blank.
 */
export function formatKeyLabel(key: string | null): string {
  return key === null || key === '' ? '(none)' : key;
}
