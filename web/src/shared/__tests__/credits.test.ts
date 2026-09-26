import { describe, expect, it } from 'vitest';
import type { TimeSeriesPoint } from '../../api.ts';
import { isCreditCategory, splitCredits } from '../credits.ts';

const p = (period: string, group: string | null, total: number, row_count = 1): TimeSeriesPoint => ({
  period: `${period}T00:00:00Z`,
  group,
  total,
  row_count,
});

describe('isCreditCategory', () => {
  it('treats credits, refunds, discounts and adjustments as credits', () => {
    for (const c of ['Credit', 'Refund', 'Discount', 'Adjustment']) expect(isCreditCategory(c)).toBe(true);
    for (const c of ['Usage', 'Purchase', 'Tax', 'Other', null]) expect(isCreditCategory(c)).toBe(false);
  });
});

describe('splitCredits', () => {
  it('splits charges from credits per period and nets them', () => {
    const split = splitCredits([
      p('2026-08-02', 'Usage', 100, 3),
      p('2026-08-01', 'Usage', 50),
      p('2026-08-01', 'Tax', 5),
      p('2026-08-01', 'Credit', -40),
      p('2026-08-02', 'Discount', -10),
      p('2026-08-02', null, 2),
    ]);
    expect(split.hasCredits).toBe(true);
    expect(split.periods.map((x) => x.slice(0, 10))).toEqual(['2026-08-01', '2026-08-02']);
    expect(split.charges).toEqual([55, 102]);
    expect(split.credits).toEqual([-40, -10]);
    expect(split.net).toEqual([15, 92]);
    expect(split.creditByCategory.get('Credit')).toEqual([-40, 0]);
    expect(split.totals).toEqual({
      charges: 157,
      credits: -50,
      net: 107,
      creditCategories: [
        { category: 'Credit', total: -40 },
        { category: 'Discount', total: -10 },
      ],
    });
    expect(split.netPoints.map((x) => [x.total, x.row_count])).toEqual([
      [15, 3],
      [92, 5],
    ]);
  });

  it('reports no credits for a charges-only series', () => {
    const split = splitCredits([p('2026-08-01', 'Usage', 10)]);
    expect(split.hasCredits).toBe(false);
    expect(split.totals.credits).toBe(0);
  });
});
