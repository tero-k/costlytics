/**
 * Pure ISO-date-string arithmetic helpers, shared by the Overview page's
 * fetching modules (`kpiCards.ts`, `trendChart.ts`, `topBreakdown.ts`).
 * All UTC-based to avoid local-timezone off-by-one errors.
 */

/** Adds (or subtracts, for negative `days`) whole days to an ISO date string (`YYYY-MM-DD`). */
export function addDaysIso(dateIso: string, days: number): string {
  const d = new Date(`${dateIso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/** Whole-day span between two ISO date strings (`endIsoExclusive - startIso`). */
export function daysBetweenIso(startIso: string, endIsoExclusive: string): number {
  const start = new Date(`${startIso}T00:00:00Z`).getTime();
  const end = new Date(`${endIsoExclusive}T00:00:00Z`).getTime();
  return Math.round((end - start) / 86_400_000);
}
