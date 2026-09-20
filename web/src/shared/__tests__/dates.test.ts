import { describe, expect, it } from 'vitest';
import { addDaysIso, daysBetweenIso, previousPeriod } from '../dates.ts';

describe('addDaysIso', () => {
  it('adds whole days within a month', () => {
    expect(addDaysIso('2025-06-10', 5)).toBe('2025-06-15');
  });

  it('subtracts whole days for a negative offset', () => {
    expect(addDaysIso('2025-06-10', -5)).toBe('2025-06-05');
  });

  it('rolls over a month boundary', () => {
    expect(addDaysIso('2025-01-31', 1)).toBe('2025-02-01');
  });

  it('rolls back over a month boundary', () => {
    expect(addDaysIso('2025-03-01', -1)).toBe('2025-02-28');
  });

  it('rolls over a year boundary', () => {
    expect(addDaysIso('2025-12-31', 1)).toBe('2026-01-01');
  });

  it('handles a leap-year Feb 29 correctly (2024 is a leap year)', () => {
    expect(addDaysIso('2024-02-28', 1)).toBe('2024-02-29');
    expect(addDaysIso('2024-02-29', 1)).toBe('2024-03-01');
  });

  it('is a no-op for days=0', () => {
    expect(addDaysIso('2025-06-10', 0)).toBe('2025-06-10');
  });
});

describe('daysBetweenIso', () => {
  it('computes the whole-day span between two dates in the same month', () => {
    expect(daysBetweenIso('2025-06-01', '2025-06-08')).toBe(7);
  });

  it('returns 0 for identical start/end', () => {
    expect(daysBetweenIso('2025-06-01', '2025-06-01')).toBe(0);
  });

  it('returns a negative span when end precedes start', () => {
    expect(daysBetweenIso('2025-06-08', '2025-06-01')).toBe(-7);
  });

  it('spans a month boundary correctly', () => {
    expect(daysBetweenIso('2025-01-25', '2025-02-05')).toBe(11);
  });

  it('spans a year boundary correctly', () => {
    expect(daysBetweenIso('2025-12-25', '2026-01-05')).toBe(11);
  });

  it('is the inverse of addDaysIso', () => {
    const start = '2025-06-10';
    const end = addDaysIso(start, 42);
    expect(daysBetweenIso(start, end)).toBe(42);
  });
});

describe('previousPeriod', () => {
  // Hand-traced in Session 6's KPI-cards review: previous period is the
  // same-length period immediately preceding the current one, with
  // previous_end === current_start (end exclusive on both sides).
  it('returns the immediately-preceding period of the same length (30-day period)', () => {
    // Current: [2025-06-01, 2025-07-01) -> 30 days.
    const result = previousPeriod('2025-06-01', '2025-07-01');
    expect(result).toEqual({ start: '2025-05-02', end: '2025-06-01' });
    // Same length as the current period.
    expect(daysBetweenIso(result.start, result.end)).toBe(daysBetweenIso('2025-06-01', '2025-07-01'));
  });

  it('returns the immediately-preceding period for a single-day range', () => {
    const result = previousPeriod('2025-06-10', '2025-06-11');
    expect(result).toEqual({ start: '2025-06-09', end: '2025-06-10' });
  });

  it('returns the immediately-preceding period for a 7-day range', () => {
    const result = previousPeriod('2025-06-08', '2025-06-15');
    expect(result).toEqual({ start: '2025-06-01', end: '2025-06-08' });
  });

  it('previous_end always equals current_start', () => {
    const result = previousPeriod('2025-03-15', '2025-04-15');
    expect(result.end).toBe('2025-03-15');
  });

  it('handles a period straddling a year boundary', () => {
    // Current: [2026-01-01, 2026-01-15) -> 14 days.
    const result = previousPeriod('2026-01-01', '2026-01-15');
    expect(result).toEqual({ start: '2025-12-18', end: '2026-01-01' });
  });
});
