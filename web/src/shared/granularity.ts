/**
 * Time-series granularity helpers shared by `trendChart.ts` (Overview page)
 * and `explorerTrend.ts` (Cost Explorer page) — both render a granularity
 * that can be auto-derived from the selected date range's length, and both
 * format a `TimeSeriesPoint.period` ISO datetime into an axis label
 * appropriate for that granularity.
 */

import type { TimeGranularity } from '../api.ts';

/** Auto-select granularity per plan §20's rule, from the range length in days. */
export function autoGranularity(durationDays: number): TimeGranularity {
  if (durationDays <= 90) return 'day';
  if (durationDays <= 366 * 3) return 'month';
  return 'year';
}

/** Formats a `period` ISO datetime as a short axis/tooltip label for the given granularity. */
export function formatPeriodLabel(periodIso: string, granularity: TimeGranularity): string {
  const date = new Date(periodIso);
  if (Number.isNaN(date.getTime())) return periodIso;

  switch (granularity) {
    case 'day':
      return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', timeZone: 'UTC' }).format(date);
    case 'month':
      return new Intl.DateTimeFormat(undefined, { month: 'short', year: 'numeric', timeZone: 'UTC' }).format(date);
    case 'year':
      return new Intl.DateTimeFormat(undefined, { year: 'numeric', timeZone: 'UTC' }).format(date);
  }
}
