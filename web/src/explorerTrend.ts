/**
 * Grouped/stacked cost trend chart for the Cost Explorer page (plan §24-25).
 *
 * Renders an ECharts stacked-area chart into `#explorer-trend`, showing cost
 * over time broken down by the selected `#dimension-select` value (service,
 * account, region, etc.) — the primary visual for this page, as opposed to
 * the Overview page's single ungrouped `trendChart.ts` series.
 *
 * `POST /api/v1/cost/timeseries` returns one point per (period, group) pair
 * when `group_by` is set (`TimeSeriesPoint.group: string | null`). This
 * module pivots that flat point list into one ECharts series per distinct
 * group, applies the page's Top N control (`#top-n-input`) by ranking groups
 * by their total across the whole date range, and folds every group outside
 * the top N into a single "Other" series (summed per period). A `null`
 * group key is labeled "(none)" via the shared `formatKeyLabel` helper
 * (same convention as the Overview page's `topBreakdown.ts`).
 *
 * Granularity is auto-selected the same way `trendChart.ts` does (shared via
 * `shared/granularity.ts`) — this page has no granularity override control.
 *
 * Shares `#date-start` / `#date-end` / `#metric-select` with the rest of the
 * page, plus `#dimension-select` / `#top-n-input` (Task 1's controls) via
 * `readExplorerControls()`, and re-fetches on any of their changes.
 */

import { getTimeseries, type TimeSeriesPoint } from './api.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { formatCurrency, formatCurrencyCompact, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { formatKeyLabel } from './shared/labels.ts';
import { autoGranularity, formatPeriodLabel } from './shared/granularity.ts';
import { readExplorerControls, subscribeToControls, type ExplorerControls } from './shared/controls.ts';
import { clearOverlays, showOverlay, ensureChart } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'explorer-trend';
const OTHER_LABEL = 'Other';

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadTrendChart(
  controls: ExplorerControls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const container = getContainer();
  if (!container) return;

  clearOverlays(container);

  const start = controls.startIso;
  const end = addDaysIso(controls.endIsoInclusive, 1);
  const durationDays = daysBetweenIso(start, end);
  const granularity = autoGranularity(durationDays);

  try {
    const result = await getTimeseries({
      start,
      end,
      metric: controls.metric,
      granularity,
      group_by: controls.dimension,
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.series.length === 0) {
      showOverlay(CONTAINER_ID, container, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(result.currency);
    renderChart(container, result.series, result.currency, granularity, controls.topN);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    showOverlay(CONTAINER_ID, container, 'chart-error', errorMessage(err));
  }
}

interface EChartsSeries {
  name: string;
  type: 'line';
  stack: 'total';
  areaStyle: Record<string, never>;
  showSymbol: false;
  emphasis: { focus: 'series' };
  data: number[];
}

function renderChart(
  container: HTMLElement,
  points: TimeSeriesPoint[],
  currency: string,
  granularity: ReturnType<typeof autoGranularity>,
  topN: number,
): void {
  const instance = ensureChart(CONTAINER_ID, container);

  // Distinct periods across the whole response, sorted chronologically —
  // the shared x-axis every group's series is pivoted onto.
  const periods = Array.from(new Set(points.map((p) => p.period))).sort((a, b) => a.localeCompare(b));
  const periodIndex = new Map(periods.map((period, idx) => [period, idx]));

  // Rank groups by their TOTAL across the whole date range (not per-period),
  // per the plan's Top N rule.
  const groupTotals = new Map<string | null, number>();
  for (const point of points) {
    groupTotals.set(point.group, (groupTotals.get(point.group) ?? 0) + point.total);
  }
  const rankedGroups = Array.from(groupTotals.entries()).sort((a, b) => b[1] - a[1]);
  const topGroups = rankedGroups.slice(0, topN).map(([group]) => group);
  const topGroupSet = new Set(topGroups);

  // Pivot: one zero-filled array per top-N group, plus a shared "Other"
  // array aggregating every non-top-N group's total per period.
  const seriesData = new Map<string | null, number[]>();
  for (const group of topGroups) seriesData.set(group, new Array<number>(periods.length).fill(0));
  const otherData = new Array<number>(periods.length).fill(0);
  let hasOther = false;

  for (const point of points) {
    const idx = periodIndex.get(point.period);
    if (idx === undefined) continue;
    if (topGroupSet.has(point.group)) {
      seriesData.get(point.group)![idx] += point.total;
    } else {
      otherData[idx] += point.total;
      hasOther = true;
    }
  }

  const categories = periods.map((period) => formatPeriodLabel(period, granularity));

  const series: EChartsSeries[] = topGroups.map((group) => ({
    name: formatKeyLabel(group),
    type: 'line',
    stack: 'total',
    areaStyle: {},
    showSymbol: false,
    emphasis: { focus: 'series' },
    data: seriesData.get(group) ?? [],
  }));

  if (hasOther) {
    series.push({
      name: OTHER_LABEL,
      type: 'line',
      stack: 'total',
      areaStyle: {},
      showSymbol: false,
      emphasis: { focus: 'series' },
      data: otherData,
    });
  }

  instance.setOption(
    {
      tooltip: {
        trigger: 'axis',
        formatter: (params: unknown) => {
          const items = Array.isArray(params) ? params : [params];
          if (items.length === 0) return '';
          const first = items[0] as { axisValueLabel?: string; name?: string } | undefined;
          const axisLabel = first?.axisValueLabel ?? first?.name ?? '';
          // Series names trace back to `TimeSeriesPoint.group`, i.e. real
          // cost-data values (service/account/resource/tag names etc.) —
          // escape before interpolating into the HTML the tooltip formatter
          // returns (ECharts' default `renderMode: 'html'` does not escape
          // it for us).
          const lines = [`<strong>${escapeHtml(String(axisLabel))}</strong>`];
          for (const raw of items) {
            const item = raw as { seriesName?: string; value?: unknown };
            const seriesName = escapeHtml(String(item.seriesName ?? ''));
            const value = typeof item.value === 'number' ? item.value : 0;
            lines.push(`${seriesName}: ${formatCurrency(value, currency)}`);
          }
          return lines.join('<br/>');
        },
      },
      legend: {
        type: 'scroll',
        bottom: 0,
        data: series.map((s) => s.name),
      },
      grid: {
        left: 8,
        right: 16,
        top: 24,
        bottom: 40,
        containLabel: true,
      },
      xAxis: {
        type: 'category',
        data: categories,
        boundaryGap: false,
      },
      yAxis: {
        type: 'value',
        axisLabel: {
          formatter: (value: number) => formatCurrencyCompact(value, currency),
        },
      },
      series,
    },
    true,
  );
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initExplorerTrendChart(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const controls = readExplorerControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadTrendChart(controls, token, onCurrency);
  };

  subscribeToControls(refresh, { explorer: true });

  return refresh();
}
