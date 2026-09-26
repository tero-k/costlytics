/**
 * ECharts instance/overlay boilerplate shared by every chart module. Each
 * chart is keyed by its container's DOM id, so a single registry works for
 * both single-chart modules (one key) and multi-chart modules (one key per
 * chart def).
 *
 * Charts are drawn through {@link setChartOption} with an option BUILDER
 * rather than a finished option: the builder is kept per chart and re-run
 * when the OS color scheme flips (colors come from `chartTheme.ts` at build
 * time) or when a chart-type toggle changes — no refetch in either case.
 */

import * as echarts from 'echarts';
import { onSchemeChange, type ChartColors } from './chartTheme.ts';
import { yearBoundaryIndices } from './granularity.ts';
import type { TimeGranularity } from '../api.ts';

type OptionBuilder = () => echarts.EChartsCoreOption;

const chartInstances = new Map<string, echarts.ECharts>();
const builders = new Map<string, OptionBuilder>();

/** Removes any previously rendered empty/error overlay from a chart container. */
export function clearOverlays(container: HTMLElement): void {
  container.querySelectorAll('.chart-empty, .chart-error').forEach((el) => el.remove());
}

/** Clears the chart's data and shows an empty/error message overlay instead. */
export function showOverlay(
  containerId: string,
  container: HTMLElement,
  className: 'chart-empty' | 'chart-error',
  message: string,
): void {
  clearOverlays(container);
  chartInstances.get(containerId)?.clear();
  builders.delete(containerId);
  container.classList.remove('is-loading');
  const overlay = document.createElement('div');
  overlay.className = className;
  overlay.textContent = message;
  container.appendChild(overlay);
}

/** Shows the skeleton shimmer until the next draw or overlay (`style.css` `.chart-area.is-loading`). */
export function showChartLoading(container: HTMLElement): void {
  container.classList.add('is-loading');
}

/** Returns the (lazily created) ECharts instance for a container, re-initializing if disposed. */
export function ensureChart(containerId: string, container: HTMLElement): echarts.ECharts {
  let instance = chartInstances.get(containerId);
  if (!instance || instance.isDisposed() || instance.getDom() !== container) {
    instance = echarts.init(container);
    chartInstances.set(containerId, instance);
    const created = instance;
    new ResizeObserver(() => created.resize()).observe(container);
  }
  return instance;
}

/** Draws (and remembers) a chart from its option builder. */
export function setChartOption(containerId: string, container: HTMLElement, build: OptionBuilder): void {
  container.classList.remove('is-loading');
  builders.set(containerId, build);
  ensureChart(containerId, container).setOption(build(), true);
}

/** Re-runs a chart's remembered builder (e.g. after its chart-type toggle changed). */
export function rerenderChart(containerId: string): void {
  const build = builders.get(containerId);
  const instance = chartInstances.get(containerId);
  if (build && instance && !instance.isDisposed()) instance.setOption(build(), true);
}

onSchemeChange(() => {
  for (const id of builders.keys()) rerenderChart(id);
});

// ---------------------------------------------------------------------------
// Chart-type toggle (Line/Bar segmented control)
// ---------------------------------------------------------------------------

export type ChartType = 'line' | 'bar';

/**
 * Wires a `.segmented` group of `[data-chart-type]` buttons: remembers the
 * choice in localStorage and re-renders `containerId` from its builder on
 * change. Returns a getter the builder reads at build time.
 */
export function initChartTypeToggle(groupId: string, containerId: string): () => ChartType {
  const storageKey = `costlytics.chartType.${groupId}`;
  let type: ChartType = 'line';
  try {
    if (localStorage.getItem(storageKey) === 'bar') type = 'bar';
  } catch {
    // Storage unavailable: default to line.
  }
  const group = document.getElementById(groupId);
  const buttons = Array.from(group?.querySelectorAll<HTMLButtonElement>('[data-chart-type]') ?? []);
  const sync = (): void => {
    for (const b of buttons) {
      const on = b.dataset.chartType === type;
      b.classList.toggle('active', on);
      b.setAttribute('aria-pressed', String(on));
    }
  };
  for (const button of buttons) {
    button.addEventListener('click', () => {
      const next = button.dataset.chartType === 'bar' ? 'bar' : 'line';
      if (next === type) return;
      type = next;
      try {
        localStorage.setItem(storageKey, type);
      } catch {
        // Not remembered; still applied.
      }
      sync();
      rerenderChart(containerId);
    });
  }
  sync();
  return () => type;
}

// ---------------------------------------------------------------------------
// Time-axis helpers
// ---------------------------------------------------------------------------

/** Points above which a time chart gets zoom controls. */
export const ZOOM_THRESHOLD = 60;

/** A dashed vertical marker + year label at each year boundary of a category time axis. */
export function yearMarkLine(periods: string[], granularity: TimeGranularity, c: ChartColors): Record<string, unknown> | undefined {
  const indices = yearBoundaryIndices(periods, granularity);
  if (indices.length === 0) return undefined;
  return {
    silent: true,
    symbol: 'none',
    animation: false,
    lineStyle: { color: c.textMuted, type: [4, 4], width: 1, opacity: 0.8 },
    label: {
      // `end` on a vertical line = horizontal text just above the plot.
      position: 'end',
      distance: 4,
      formatter: '{b}',
      color: c.textSecondary,
      fontSize: 11,
      fontWeight: 600,
      backgroundColor: c.surface,
      padding: [2, 4],
      borderRadius: 3,
    },
    data: indices.map((i) => ({ xAxis: i, name: String(new Date(periods[i]).getUTCFullYear()) })),
  };
}

/** Inside (wheel/drag) + slider zoom for long time series; nothing for short ones. */
export function timeZoom(pointCount: number, c: ChartColors): { dataZoom?: unknown[]; bottomExtra: number } {
  if (pointCount <= ZOOM_THRESHOLD) return { bottomExtra: 0 };
  return {
    bottomExtra: 36,
    dataZoom: [
      { type: 'inside', filterMode: 'none' },
      {
        type: 'slider',
        height: 18,
        bottom: 6,
        filterMode: 'none',
        borderColor: 'transparent',
        backgroundColor: c.grid,
        fillerColor: c.dark ? 'rgba(255,255,255,0.08)' : 'rgba(15,23,42,0.06)',
        dataBackground: { lineStyle: { color: c.axis }, areaStyle: { color: c.axis, opacity: 0.3 } },
        selectedDataBackground: { lineStyle: { color: c.series[0] }, areaStyle: { color: c.series[0], opacity: 0.2 } },
        handleStyle: { color: c.surface, borderColor: c.axis },
        moveHandleSize: 0,
        textStyle: { color: c.textMuted, fontSize: 10 },
      },
    ],
  };
}
