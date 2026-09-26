import { describe, expect, it } from 'vitest';
import { assignStableSlots, withAlpha } from '../chartTheme.ts';
import { sparklinePaths } from '../kpiCard.ts';

describe('assignStableSlots', () => {
  it('keeps a key on its slot when ranking changes or other keys drop out', () => {
    const memory = new Map<string, number>();
    expect(assignStableSlots(['a', 'b', 'c'], memory)).toEqual([0, 1, 2]);
    // Re-ranked, 'b' gone, 'd' new: 'd' takes the lowest slot not on screen.
    expect(assignStableSlots(['c', 'd', 'a'], memory)).toEqual([2, 1, 0]);
  });

  it('never gives two on-screen keys the same slot', () => {
    const memory = new Map([['x', 0]]);
    const slots = assignStableSlots(['y', 'x', 'z'], memory, 8);
    expect(new Set(slots).size).toBe(3);
    expect(slots[1]).toBe(0);
  });
});

describe('withAlpha', () => {
  it('converts hex to rgba', () => {
    expect(withAlpha('#2a78d6', 0.5)).toBe('rgba(42, 120, 214, 0.5)');
  });
});

describe('sparklinePaths', () => {
  it('needs two points', () => {
    expect(sparklinePaths([1], 100, 28)).toBeNull();
  });

  it('spans the box with the max at the top', () => {
    const paths = sparklinePaths([0, 10], 100, 28)!;
    expect(paths.line).toBe('M0.0,26.0 L100.0,2.0');
    expect(paths.area.endsWith('L100,28 L0,28 Z')).toBe(true);
  });

  it('draws a flat series mid-height', () => {
    expect(sparklinePaths([5, 5, 5], 100, 28)!.line).toBe('M0.0,14.0 L50.0,14.0 L100.0,14.0');
  });
});
