import { describe, expect, it } from 'vitest';
import { formatAxisLabel, yearBoundaryIndices } from '../granularity.ts';

function months(fromYear: number, fromMonth: number, count: number): string[] {
  return Array.from({ length: count }, (_, i) => new Date(Date.UTC(fromYear, fromMonth + i, 1)).toISOString());
}

describe('yearBoundaryIndices', () => {
  it('marks each January in a monthly series spanning years, never the first point', () => {
    const periods = months(2024, 6, 30); // Jul 2024 .. Dec 2026
    expect(yearBoundaryIndices(periods, 'month')).toEqual([6, 18]);
  });

  it('does not mark a series that starts in January', () => {
    expect(yearBoundaryIndices(months(2025, 0, 12), 'month')).toEqual([]);
  });

  it('marks Jan 1 in a daily series', () => {
    const periods = ['2025-12-30', '2025-12-31', '2026-01-01', '2026-01-02'].map((d) => `${d}T00:00:00Z`);
    expect(yearBoundaryIndices(periods, 'day')).toEqual([2]);
  });

  it('marks nothing at year granularity', () => {
    const periods = ['2024-01-01T00:00:00Z', '2025-01-01T00:00:00Z', '2026-01-01T00:00:00Z'];
    expect(yearBoundaryIndices(periods, 'year')).toEqual([]);
  });

  it('marks a gap that skips whole years once', () => {
    const periods = ['2023-11-01T00:00:00Z', '2025-02-01T00:00:00Z'];
    expect(yearBoundaryIndices(periods, 'month')).toEqual([1]);
  });
});

describe('formatAxisLabel', () => {
  it('includes the year only when asked', () => {
    const jan = '2026-01-01T00:00:00Z';
    expect(formatAxisLabel(jan, 'month', true)).toMatch(/2026/);
    expect(formatAxisLabel(jan, 'month', false)).not.toMatch(/2026/);
    expect(formatAxisLabel(jan, 'day', false)).not.toMatch(/2026/);
    expect(formatAxisLabel(jan, 'year', false)).toMatch(/2026/);
  });
});
