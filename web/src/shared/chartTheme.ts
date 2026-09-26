/**
 * Chart colors and chrome for both color schemes.
 *
 * The categorical palette is the dataviz reference palette's fixed 8-slot
 * order, validated (lightness band, chroma, adjacent CVD separation,
 * normal-vision floor) against this app's own chart surfaces — `#ffffff`
 * light, `#1c1f26` dark (`--color-surface` in `style.css`). Light mode's
 * aqua/yellow/magenta sit below 3:1 on white, so every chart ships tooltips,
 * legends and (on Explorer / Cost Changes) a table view as relief.
 *
 * Slots are assigned in fixed order and never cycled: charts show at most
 * `MAX_SERIES` named series and fold the rest into a neutral "Other".
 */

export interface ChartColors {
  dark: boolean;
  series: readonly string[];
  /** Neutral fill for an "Other" bucket — deliberately not a palette slot. */
  other: string;
  surface: string;
  text: string;
  textSecondary: string;
  textMuted: string;
  grid: string;
  axis: string;
  tooltipBg: string;
  tooltipBorder: string;
}

const LIGHT: ChartColors = {
  dark: false,
  series: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#4a3aa7', '#e34948'],
  other: '#a8a7a1',
  surface: '#ffffff',
  text: '#1a1d23',
  textSecondary: '#52514e',
  textMuted: '#6b7280',
  grid: '#eceae4',
  axis: '#c3c2b7',
  tooltipBg: '#ffffff',
  tooltipBorder: 'rgba(11, 11, 11, 0.10)',
};

const DARK: ChartColors = {
  dark: true,
  series: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9', '#e66767'],
  other: '#6b6a66',
  surface: '#1c1f26',
  text: '#eaecef',
  textSecondary: '#c3c2b7',
  textMuted: '#9aa1ac',
  grid: '#2a2e36',
  axis: '#3a3f49',
  tooltipBg: '#23272f',
  tooltipBorder: 'rgba(255, 255, 255, 0.10)',
};

/** Named series a chart shows before folding the rest into "Other". */
export const MAX_SERIES = LIGHT.series.length;

const DARK_QUERY = '(prefers-color-scheme: dark)';

export function isDarkScheme(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia(DARK_QUERY).matches;
}

export function chartColors(): ChartColors {
  return isDarkScheme() ? DARK : LIGHT;
}

/** Calls `callback` whenever the OS color scheme flips. */
export function onSchemeChange(callback: () => void): void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
  window.matchMedia(DARK_QUERY).addEventListener('change', callback);
}

/** `#rrggbb` → `rgba(r, g, b, alpha)`. */
export function withAlpha(hex: string, alpha: number): string {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

/**
 * Color follows the entity, not its rank: keeps each key on the slot it was
 * first given (in `memory`, which the caller holds across refreshes), so a
 * filter or date change that reorders or drops series never repaints the
 * survivors. New keys take the lowest slot not used by another key in this
 * render; a slot is only reclaimed from a key that isn't on screen.
 * `keys.length` must not exceed the palette size. @internal exported for tests
 */
export function assignStableSlots(keys: string[], memory: Map<string, number>, slotCount = MAX_SERIES): number[] {
  const taken = new Set<number>();
  const result = new Array<number>(keys.length).fill(-1);
  keys.forEach((key, i) => {
    const slot = memory.get(key);
    if (slot !== undefined && !taken.has(slot)) {
      result[i] = slot;
      taken.add(slot);
    }
  });
  keys.forEach((key, i) => {
    if (result[i] !== -1) return;
    let slot = 0;
    while (taken.has(slot) && slot < slotCount - 1) slot++;
    result[i] = slot;
    taken.add(slot);
    memory.set(key, slot);
  });
  return result;
}

/** Shared option fragments: text, tooltip and axis styling for the current scheme. */
export function baseChartOption(c: ChartColors): Record<string, unknown> {
  return {
    backgroundColor: 'transparent',
    color: [...c.series],
    textStyle: {
      fontFamily: getComputedStyle(document.body).fontFamily,
      color: c.textSecondary,
    },
    animationDuration: 400,
    animationDurationUpdate: 300,
  };
}

export function tooltipStyle(c: ChartColors): Record<string, unknown> {
  return {
    backgroundColor: c.tooltipBg,
    borderColor: c.tooltipBorder,
    borderWidth: 1,
    padding: [8, 12],
    textStyle: { color: c.text, fontSize: 12 },
    extraCssText: `border-radius: 8px; box-shadow: 0 6px 20px ${c.dark ? 'rgba(0,0,0,0.45)' : 'rgba(15,23,42,0.12)'};`,
  };
}

export function valueAxisStyle(c: ChartColors): Record<string, unknown> {
  return {
    axisLine: { show: false },
    axisTick: { show: false },
    axisLabel: { color: c.textMuted, fontSize: 11 },
    splitLine: { lineStyle: { color: c.grid } },
  };
}

export function categoryAxisStyle(c: ChartColors): Record<string, unknown> {
  return {
    axisLine: { lineStyle: { color: c.axis } },
    axisTick: { show: false },
    axisLabel: { color: c.textMuted, fontSize: 11, hideOverlap: true },
  };
}

/** A tooltip row: colored dot (identity) + text in ink colors, never the series color. */
export function tooltipRow(color: string, label: string, value: string, extra = ''): string {
  return (
    `<div class="tt-row"><span class="tt-dot" style="background:${color}"></span>` +
    `<span class="tt-label">${label}</span><span class="tt-value">${value}</span>${extra}</div>`
  );
}
