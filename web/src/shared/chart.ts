/**
 * ECharts instance/overlay boilerplate shared between `trendChart.ts` and
 * `topBreakdown.ts`. Each chart is keyed by its container's DOM id, so a
 * single registry works for both single-chart modules (one key) and
 * multi-chart modules (one key per chart def).
 */

import * as echarts from 'echarts';

const chartInstances = new Map<string, echarts.ECharts>();

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
  const overlay = document.createElement('div');
  overlay.className = className;
  overlay.textContent = message;
  container.appendChild(overlay);
}

/** Returns the (lazily created) ECharts instance for a container, re-initializing if disposed. */
export function ensureChart(containerId: string, container: HTMLElement): echarts.ECharts {
  let instance = chartInstances.get(containerId);
  if (!instance || instance.isDisposed()) {
    instance = echarts.init(container);
    chartInstances.set(containerId, instance);
    window.addEventListener('resize', () => instance?.resize());
  }
  return instance;
}
