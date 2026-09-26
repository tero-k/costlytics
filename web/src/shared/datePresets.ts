/**
 * Quick date-range presets for the shared filter bar (`appShell.ts`). Pure
 * date math on local calendar days — the same convention as the date
 * inputs themselves (`<input type="date">` values are local dates).
 */

export type PresetId = 'mtd' | 'last-month' | '30d' | '3m' | '6m' | '12m' | 'ytd' | 'last-year';

export interface DatePreset {
  id: PresetId;
  label: string;
  /** Tooltip spelling out exactly which days the preset covers. */
  title: string;
}

export const DATE_PRESETS: DatePreset[] = [
  { id: 'mtd', label: 'MTD', title: 'Month to date' },
  { id: 'last-month', label: 'Last month', title: 'The previous calendar month' },
  { id: '30d', label: '30D', title: 'Last 30 days, including today' },
  { id: '3m', label: '3M', title: 'This month plus the 2 full months before it' },
  { id: '6m', label: '6M', title: 'This month plus the 5 full months before it' },
  { id: '12m', label: '12M', title: 'This month plus the 11 full months before it' },
  { id: 'ytd', label: 'YTD', title: 'Year to date' },
  { id: 'last-year', label: 'Last year', title: 'The previous calendar year' },
];

/** Formats a `Date` as a local-calendar `YYYY-MM-DD` (what `<input type="date">` holds). */
export function toLocalIso(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

export interface IsoRange {
  /** Inclusive `YYYY-MM-DD`. */
  start: string;
  /** Inclusive `YYYY-MM-DD`. */
  end: string;
}

/** The inclusive date range a preset denotes, relative to `today`. */
export function presetRange(id: PresetId, today: Date): IsoRange {
  const y = today.getFullYear();
  const m = today.getMonth();
  const end = toLocalIso(today);
  switch (id) {
    case 'mtd':
      return { start: toLocalIso(new Date(y, m, 1)), end };
    case 'last-month':
      return { start: toLocalIso(new Date(y, m - 1, 1)), end: toLocalIso(new Date(y, m, 0)) };
    case '30d':
      return { start: toLocalIso(new Date(y, m, today.getDate() - 29)), end };
    case '3m':
      return { start: toLocalIso(new Date(y, m - 2, 1)), end };
    case '6m':
      return { start: toLocalIso(new Date(y, m - 5, 1)), end };
    case '12m':
      return { start: toLocalIso(new Date(y, m - 11, 1)), end };
    case 'ytd':
      return { start: toLocalIso(new Date(y, 0, 1)), end };
    case 'last-year':
      return { start: toLocalIso(new Date(y - 1, 0, 1)), end: toLocalIso(new Date(y - 1, 11, 31)) };
  }
}

/**
 * Every preset whose range is exactly `start`..`end` today — empty for a
 * custom range. Can be several (in March, 3M and YTD are the same range).
 */
export function matchingPresets(start: string, end: string, today: Date): PresetId[] {
  return DATE_PRESETS.filter((p) => {
    const range = presetRange(p.id, today);
    return range.start === start && range.end === end;
  }).map((p) => p.id);
}
