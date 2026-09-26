/**
 * Charges vs. credits split for the cost trends and KPI cards.
 *
 * Every normalized cost row carries a `charge_category`: FOCUS exports'
 * own `ChargeCategory`, or CUR 2.0's `line_item_type` mapped in
 * `crates/data/src/adapters/cur2.rs`. The trend modules fetch their series
 * grouped by that category (still one query) and split it here:
 *   - credits = Credit, Refund, Discount (CUR EDP/bundled/private-rate
 *     discounts) and Adjustment (e.g. Savings Plan negation) rows;
 *   - charges = everything else (Usage, Purchase, Tax, Other, unknown);
 *   - net     = charges + credits (credits are normally negative), which is
 *     what every total in the app already shows.
 * Classification is by category, not by sign, so a positive adjustment is
 * still reported under credits — the tooltip lists categories so it's clear.
 */

import type { TimeSeriesPoint } from '../api.ts';

export const CREDIT_CATEGORIES: readonly string[] = ['Credit', 'Refund', 'Discount', 'Adjustment'];

export function isCreditCategory(category: string | null): boolean {
  return category !== null && CREDIT_CATEGORIES.includes(category);
}

export interface CreditTotals {
  charges: number;
  credits: number;
  net: number;
  /** Credit-side totals per category, largest magnitude first. */
  creditCategories: Array<{ category: string; total: number }>;
}

export interface CreditSplit {
  /** Sorted chronologically. */
  periods: string[];
  charges: number[];
  credits: number[];
  net: number[];
  /** Credit-side totals per category, per period (same order as `periods`). */
  creditByCategory: Map<string, number[]>;
  /** One ungrouped point per period (net total, summed row counts) — the pre-split series shape. */
  netPoints: TimeSeriesPoint[];
  totals: CreditTotals;
  /** Whether any credit-category row is present; `false` → draw the plain single-series trend. */
  hasCredits: boolean;
}

/** Splits a series grouped by `charge_category` into charges / credits / net per period. */
export function splitCredits(points: TimeSeriesPoint[]): CreditSplit {
  const periods = Array.from(new Set(points.map((p) => p.period))).sort((a, b) => a.localeCompare(b));
  const index = new Map(periods.map((p, i) => [p, i]));
  const zeros = (): number[] => new Array<number>(periods.length).fill(0);
  const charges = zeros();
  const credits = zeros();
  const rows = zeros();
  const creditByCategory = new Map<string, number[]>();
  let hasCredits = false;

  for (const point of points) {
    const i = index.get(point.period)!;
    rows[i] += point.row_count;
    if (isCreditCategory(point.group)) {
      hasCredits = true;
      credits[i] += point.total;
      const key = point.group!;
      if (!creditByCategory.has(key)) creditByCategory.set(key, zeros());
      creditByCategory.get(key)![i] += point.total;
    } else {
      charges[i] += point.total;
    }
  }

  const net = periods.map((_, i) => charges[i] + credits[i]);
  const sum = (xs: number[]): number => xs.reduce((a, b) => a + b, 0);
  const creditCategories = Array.from(creditByCategory, ([category, values]) => ({ category, total: sum(values) })).sort(
    (a, b) => Math.abs(b.total) - Math.abs(a.total),
  );

  return {
    periods,
    charges,
    credits,
    net,
    creditByCategory,
    netPoints: periods.map((period, i) => ({ period, group: null, total: net[i], row_count: rows[i] })),
    totals: { charges: sum(charges), credits: sum(credits), net: sum(net), creditCategories },
    hasCredits,
  };
}
