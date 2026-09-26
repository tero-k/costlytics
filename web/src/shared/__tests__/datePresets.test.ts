import { describe, expect, it } from 'vitest';
import { DATE_PRESETS, matchingPresets, presetRange } from '../datePresets.ts';

const TODAY = new Date(2026, 2, 15); // Mar 15 2026, local

describe('presetRange', () => {
  it.each([
    ['mtd', '2026-03-01', '2026-03-15'],
    ['last-month', '2026-02-01', '2026-02-28'],
    ['30d', '2026-02-14', '2026-03-15'],
    ['3m', '2026-01-01', '2026-03-15'],
    ['6m', '2025-10-01', '2026-03-15'],
    ['12m', '2025-04-01', '2026-03-15'],
    ['ytd', '2026-01-01', '2026-03-15'],
    ['last-year', '2025-01-01', '2025-12-31'],
  ] as const)('%s', (id, start, end) => {
    expect(presetRange(id, TODAY)).toEqual({ start, end });
  });

  it('crosses the year boundary for last month in January', () => {
    expect(presetRange('last-month', new Date(2026, 0, 10))).toEqual({ start: '2025-12-01', end: '2025-12-31' });
  });
});

describe('matchingPresets', () => {
  it('recognizes every preset range and nothing else', () => {
    for (const { id } of DATE_PRESETS) {
      const { start, end } = presetRange(id, TODAY);
      expect(matchingPresets(start, end, TODAY)).toContain(id);
    }
    expect(matchingPresets('2026-03-02', '2026-03-15', TODAY)).toEqual([]);
  });

  it('reports every preset that denotes the same range', () => {
    expect(matchingPresets('2026-01-01', '2026-03-15', TODAY)).toEqual(['3m', 'ytd']);
  });
});
