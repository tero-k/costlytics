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
