/**
 * Currency/error formatting helpers shared by the Overview page's fetching
 * modules (`kpiCards.ts`, `trendChart.ts`, `topBreakdown.ts`).
 */

import { ApiError } from '../api.ts';

/**
 * `currency` is NOT app configuration — it comes straight from the source
 * cost data (`BillingCurrency` in FOCUS 1.2, cast to `VARCHAR` by the data
 * layer) and reaches the API response unvalidated. `Intl.NumberFormat`
 * throws a `RangeError` for anything that isn't a well-formed currency
 * code, and both formatters below fall back to interpolating `currency`
 * directly into a plain string that callers embed as raw HTML (table
 * cells, chart tooltips). Sanitize it before it ever reaches that fallback
 * string: if it doesn't look like a plausible currency code/label, drop it
 * entirely rather than trying to escape it — a degraded "just show the
 * number" fallback is safe, echoing an attacker-controlled string into HTML
 * is not.
 */
function sanitizeCurrencyForFallback(currency: string): string {
  return /^[A-Za-z0-9 ]{0,12}$/.test(currency) ? currency : '';
}

export function formatCurrency(value: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(value);
  } catch {
    // Fall back gracefully if the API ever returns a currency code that
    // Intl doesn't recognize.
    return `${value.toFixed(2)} ${sanitizeCurrencyForFallback(currency)}`.trimEnd();
  }
}

/**
 * Short currency label for chart axes and bar labels ("$1.2K"). Sub-unit
 * amounts keep two significant digits ("$0.004") rather than rounding to
 * "$0" — otherwise a trend of fractions of a cent labels every tick "$0" /
 * "-$0". Negative zero is normalized so a zero tick never reads "-$0".
 */
export function formatCurrencyCompact(value: number, currency: string): string {
  const v = value === 0 ? 0 : value;
  const small = Math.abs(v) < 1;
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      ...(small ? { maximumSignificantDigits: 2 } : { notation: 'compact', maximumFractionDigits: 1 }),
    }).format(v);
  } catch {
    return `${small ? v.toPrecision(2) : v.toFixed(0)} ${sanitizeCurrencyForFallback(currency)}`.trimEnd();
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
