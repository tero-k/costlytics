import { describe, expect, it } from 'vitest';
import { buildRowsWithOther, OTHER_EPSILON_FRACTION } from '../otherBucket.ts';
import type { BreakdownRow } from '../../api.ts';

function row(key: string | null, total: number): BreakdownRow {
  return { key, total, row_count: 1 };
}

describe('buildRowsWithOther', () => {
  it('maps each row to its label/total, formatting null/empty keys via formatKeyLabel', () => {
    const rows = [row('Amazon EC2', 100), row(null, 20), row('', 5)];
    const result = buildRowsWithOther(rows, 125);
    expect(result).toEqual([
      { label: 'Amazon EC2', total: 100 },
      { label: '(none)', total: 20 },
      { label: '(none)', total: 5 },
      // sumOfRows === overallTotal here, so no "Other" bucket.
    ]);
  });

  it('adds an "Other" bucket for the remainder when rows sum to less than the overall total by more than the epsilon threshold', () => {
    const rows = [row('Amazon EC2', 700), row('Amazon S3', 200)];
    const overallTotal = 1000; // sum = 900, remainder = 100, well above 1000*0.001=1
    const result = buildRowsWithOther(rows, overallTotal);
    expect(result).toEqual([
      { label: 'Amazon EC2', total: 700 },
      { label: 'Amazon S3', total: 200 },
      { label: 'Other', total: 100 },
    ]);
  });

  it('omits the "Other" bucket when rows sum to within epsilon of the overall total', () => {
    const overallTotal = 1000;
    // remainder = 0.5, which is below overallTotal * OTHER_EPSILON_FRACTION (=1).
    const rows = [row('Amazon EC2', 999.5)];
    expect(rows[0].total).toBeLessThan(overallTotal);
    const result = buildRowsWithOther(rows, overallTotal);
    expect(result).toEqual([{ label: 'Amazon EC2', total: 999.5 }]);
  });

  it('omits the "Other" bucket when rows sum to exactly the overall total', () => {
    const rows = [row('Amazon EC2', 600), row('Amazon S3', 400)];
    const result = buildRowsWithOther(rows, 1000);
    expect(result).toEqual([
      { label: 'Amazon EC2', total: 600 },
      { label: 'Amazon S3', total: 400 },
    ]);
  });

  it('adds an "Other" bucket at exactly the epsilon fraction boundary is exclusive (remainder must be strictly greater than the threshold)', () => {
    const overallTotal = 1000;
    const threshold = overallTotal * OTHER_EPSILON_FRACTION; // 1
    // Rows sum leaves remainder exactly equal to the threshold -> "remainder > threshold" is false.
    const rows = [row('Amazon EC2', overallTotal - threshold)];
    const result = buildRowsWithOther(rows, overallTotal);
    expect(result).toEqual([{ label: 'Amazon EC2', total: overallTotal - threshold }]);
  });

  // --- Negative overall total (credits/refunds) -----------------------------
  //
  // `buildRowsWithOther`'s epsilon check is:
  //   remainder > overallTotal * OTHER_EPSILON_FRACTION
  // For a POSITIVE overallTotal, the right-hand side is a small positive
  // number, so this correctly requires the remainder to be a non-trivial
  // positive gap before adding "Other".
  //
  // For a NEGATIVE overallTotal (e.g. a period dominated by credits/refunds),
  // the right-hand side becomes a small NEGATIVE number close to zero
  // (e.g. -1000 * 0.001 = -1), while `remainder` itself is deeply negative
  // (e.g. -900) when the rows only account for a small slice of the total.
  // `-900 > -1` is false, so the epsilon check silently fails to add an
  // "Other" bucket even though the unaccounted remainder is 90% of the
  // total. This reproduces CURRENT (buggy) behavior — see the report for
  // this session; this is NOT the behavior a correct epsilon comparison
  // would produce (which would need to compare against `Math.abs(overallTotal)`).
  it('CURRENT BEHAVIOR (possible bug): with a negative overall total, a large unaccounted remainder does NOT produce an "Other" bucket', () => {
    const overallTotal = -1000; // e.g. large refund/credit period
    const rows = [row('Refund adjustment', -100)]; // only accounts for 10% of the credit
    const remainder = overallTotal - -100; // -900
    expect(remainder).toBe(-900);
    // Sanity-check the epsilon threshold used internally: -1000 * 0.001 = -1.
    expect(overallTotal * OTHER_EPSILON_FRACTION).toBe(-1);

    const result = buildRowsWithOther(rows, overallTotal);
    // A -900 remainder against a -1000 total is a huge (90%) unaccounted
    // chunk, so a correct implementation would be expected to surface an
    // "Other" bucket here. It does not: the epsilon comparison's sign
    // handling inverts for negative totals, so no "Other" row is added.
    expect(result).toEqual([{ label: 'Refund adjustment', total: -100 }]);
  });

  it('CURRENT BEHAVIOR (possible bug): with a negative overall total, a small negative remainder (rows overshoot the credit) DOES produce an "Other" bucket', () => {
    const overallTotal = -1000;
    // Rows sum to MORE negative than the total (overshoot by 50), so
    // remainder = overallTotal - sumOfRows = -1000 - (-1050) = 50, which is
    // positive and comfortably greater than the (-1) threshold.
    const rows = [row('Refund adjustment', -1050)];
    const result = buildRowsWithOther(rows, overallTotal);
    expect(result).toEqual([
      { label: 'Refund adjustment', total: -1050 },
      { label: 'Other', total: 50 },
    ]);
  });

  it('handles an empty rows array against a positive overall total by adding the whole total as "Other"', () => {
    const result = buildRowsWithOther([], 500);
    expect(result).toEqual([{ label: 'Other', total: 500 }]);
  });

  it('handles an empty rows array against a zero overall total by adding no "Other" bucket (remainder=0 is not > 0)', () => {
    const result = buildRowsWithOther([], 0);
    expect(result).toEqual([]);
  });
});
