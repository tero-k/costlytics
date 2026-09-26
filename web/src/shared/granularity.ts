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

/**
 * Indices into a chronologically sorted `periods` list where the UTC year
 * changes — where a trend chart draws its year marker. Never index 0 (the
 * first point has nothing to separate), and none at `year` granularity
 * (every point is its own year already).
 */
export function yearBoundaryIndices(periods: string[], granularity: TimeGranularity): number[] {
  if (granularity === 'year') return [];
  const indices: number[] = [];
  let previousYear: number | null = null;
  periods.forEach((period, i) => {
    const date = new Date(period);
    if (Number.isNaN(date.getTime())) return;
    const year = date.getUTCFullYear();
    if (previousYear !== null && year !== previousYear) indices.push(i);
    previousYear = year;
  });
  return indices;
}

/**
 * Compact x-axis label: the year appears only where it's news — on the
 * first point and on each year boundary ("Jan 2025", "Jan 1, 2025") —
 * and is dropped elsewhere ("Feb", "Feb 3"), since the year markers
 * already carry it. Tooltips keep the full `formatPeriodLabel`.
 */
export function formatAxisLabel(periodIso: string, granularity: TimeGranularity, withYear: boolean): string {
  const date = new Date(periodIso);
  if (Number.isNaN(date.getTime()) || granularity === 'year') return formatPeriodLabel(periodIso, granularity);
  const opts: Intl.DateTimeFormatOptions =
    granularity === 'day' ? { month: 'short', day: 'numeric', timeZone: 'UTC' } : { month: 'short', timeZone: 'UTC' };
  if (withYear) opts.year = 'numeric';
  return new Intl.DateTimeFormat(undefined, opts).format(date);
}
