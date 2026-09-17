/**
 * Currency/error formatting helpers shared by the Overview page's fetching
 * modules (`kpiCards.ts`, `trendChart.ts`, `topBreakdown.ts`).
 */

import { ApiError } from '../api.ts';

export function formatCurrency(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(value);
  } catch {
    // Fall back gracefully if the API ever returns a currency code that
    // Intl doesn't recognize.
    return `${value.toFixed(2)} ${currency}`;
  }
}

export function formatCurrencyCompact(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      notation: 'compact',
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return `${value.toFixed(0)} ${currency}`;
  }
}

/** Extracts a human-readable message from a thrown value of unknown shape. */
export function errorMessage(err: unknown): string {
  if (err instanceof ApiError) return err.message;
  if (err instanceof Error) return err.message;
  return 'Unknown error';
}

// ---------------------------------------------------------------------------
// Change (current vs. previous period) formatting — shared by `kpiCards.ts`
// (Overview page) and `explorerTable.ts` (Cost Explorer page), both of which
// render `compare()`'s `absolute_change` / `percentage_change` fields with
// the same sign/color convention: for a cost metric, an increase is "bad"
// (red) and a decrease is "good" (green).
// ---------------------------------------------------------------------------

/** `pct` is already a percentage value (e.g. `37.2` means 37.2%), per the API's `percentage_change`. */
export function formatPercent(pct: number): string {
  const sign = pct > 0 ? '+' : '';
  return `${sign}${pct.toFixed(1)}%`;
}

export function formatSignedCurrency(value: number, currency: string): string {
  const formatted = formatCurrency(Math.abs(value), currency);
  return value > 0 ? `+${formatted}` : value < 0 ? `-${formatted}` : formatted;
}

/** CSS class for a cost-metric change value: increase = bad (red), decrease = good (green). */
export function changeClass(value: number): 'change-bad' | 'change-good' | 'change-neutral' {
  return value > 0 ? 'change-bad' : value < 0 ? 'change-good' : 'change-neutral';
}
