/**
 * ECharts option builders for the cost-over-time charts: the single-series
 * trend (Overview `trendChart.ts`, drilldowns' `entityTrend.ts`) and the
 * stacked, grouped trend (Cost Explorer `explorerTrend.ts`), and the charges
 * vs. credits split (`credits.ts`). One place for
 * the shared anatomy — themed axes, year markers, zoom for long ranges,
 * line/bar forms, and escaped HTML tooltips.
 */

import type { EChartsCoreOption } from 'echarts';
import * as echarts from 'echarts';
import type { TimeGranularity, TimeSeriesPoint } from '../api.ts';
import { splitCredits, type CreditSplit } from './credits.ts';
import { formatCurrency, formatCurrencyCompact } from './format.ts';
import { escapeHtml } from './html.ts';
import { formatAxisLabel, formatPeriodLabel, yearBoundaryIndices } from './granularity.ts';
import { setChartOption, timeZoom, yearMarkLine, type ChartType } from './chart.ts';
import {
  baseChartOption,
  categoryAxisStyle,
  chartColors,
  tooltipRow,
  tooltipStyle,
  valueAxisStyle,
  withAlpha,
  type ChartColors,
} from './chartTheme.ts';

function axisLabels(periods: string[], granularity: TimeGranularity): string[] {
  const boundaries = new Set(yearBoundaryIndices(periods, granularity));
  return periods.map((p, i) => formatAxisLabel(p, granularity, i === 0 || boundaries.has(i)));
}

function formatPercent(value: number): string {
  return `${value >= 0 ? '+' : ''}${value.toFixed(1)}%`;
}

function commonFrame(c: ChartColors, categories: string[], currency: string, chartType: ChartType, bottom: number) {
  return {
    ...baseChartOption(c),
    grid: { left: 8, right: 16, top: 32, bottom, containLabel: true },
    xAxis: {
      type: 'category',
      data: categories,
      boundaryGap: chartType === 'bar',
      ...categoryAxisStyle(c),
    },
    yAxis: {
      type: 'value',
      ...valueAxisStyle(c),
      axisLabel: {
        ...(valueAxisStyle(c).axisLabel as object),
        formatter: (value: number) => formatCurrencyCompact(value, currency),
      },
    },
  };
}

export interface SingleTrendInput {
  /** Sorted chronologically. */
  points: TimeSeriesPoint[];
  currency: string;
  granularity: TimeGranularity;
  chartType: ChartType;
}

/** Total cost over time as one series: gradient area line, or rounded bars. */
export function buildSingleTrendOption({ points, currency, granularity, chartType }: SingleTrendInput): EChartsCoreOption {
  const c = chartColors();
  const color = c.series[0];
  const periods = points.map((p) => p.period);
  const zoom = timeZoom(points.length, c);
  const markLine = yearMarkLine(periods, granularity, c);

  const series =
    chartType === 'bar'
      ? {
          type: 'bar',
          name: 'Cost',
          data: points.map((p) => p.total),
          barMaxWidth: 28,
          itemStyle: { color, borderRadius: [4, 4, 0, 0] },
          emphasis: { itemStyle: { color: withAlpha(color, 0.85) } },
          markLine,
        }
      : {
          type: 'line',
          name: 'Cost',
          data: points.map((p) => p.total),
          smooth: 0.25,
          symbol: 'circle',
          symbolSize: 8,
          showSymbol: points.length <= 31,
          lineStyle: { width: 2, color },
          itemStyle: { color, borderColor: c.surface, borderWidth: 2 },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: withAlpha(color, c.dark ? 0.35 : 0.25) },
              { offset: 1, color: withAlpha(color, 0.02) },
            ]),
          },
          markLine,
        };

  return {
    ...commonFrame(c, axisLabels(periods, granularity), currency, chartType, 8 + zoom.bottomExtra),
    tooltip: {
      trigger: 'axis',
      ...tooltipStyle(c),
      axisPointer: chartType === 'bar' ? { type: 'shadow' } : { type: 'line', lineStyle: { color: c.axis } },
      formatter: (params: unknown) => {
        const first = (Array.isArray(params) ? params[0] : params) as { dataIndex: number } | undefined;
        const idx = first?.dataIndex ?? -1;
        const point = points[idx];
        if (!point) return '';
        const prev = points[idx - 1];
        const change =
          prev && prev.total !== 0
            ? `<div class="tt-sub">${formatPercent(((point.total - prev.total) / Math.abs(prev.total)) * 100)} vs previous</div>`
            : '';
        return (
          `<div class="tt-title">${escapeHtml(formatPeriodLabel(point.period, granularity))}</div>` +
          tooltipRow(color, 'Cost', formatCurrency(point.total, currency)) +
          change +
          `<div class="tt-sub">${point.row_count.toLocaleString()} rows</div>`
        );
      },
    },
    dataZoom: zoom.dataZoom,
    series: [series],
  };
}

export interface StackedSeries {
  name: string;
  data: number[];
  color: string;
}

export interface StackedTrendInput {
  /** Sorted chronologically; the shared x-axis every series is pivoted onto. */
  periods: string[];
  series: StackedSeries[];
  currency: string;
  granularity: TimeGranularity;
  chartType: ChartType;
}

/** Cost over time by group: stacked area or stacked bars, legend below. */
export function buildStackedTrendOption({ periods, series, currency, granularity, chartType }: StackedTrendInput): EChartsCoreOption {
  const c = chartColors();
  const zoom = timeZoom(periods.length, c);
  const markLine = yearMarkLine(periods, granularity, c);
  const legendHeight = 36;

  const echartsSeries = series.map((s, i) =>
    chartType === 'bar'
      ? {
          name: s.name,
          type: 'bar',
          stack: 'total',
          data: s.data,
          barMaxWidth: 32,
          // Surface-colored border = the 2px gap between stacked segments.
          itemStyle: {
            color: s.color,
            borderColor: c.surface,
            borderWidth: 1,
            borderRadius: i === series.length - 1 ? [4, 4, 0, 0] : 0,
          },
          markLine: i === 0 ? markLine : undefined,
        }
      : {
          name: s.name,
          type: 'line',
          stack: 'total',
          data: s.data,
          smooth: 0.2,
          showSymbol: false,
          symbolSize: 8,
          lineStyle: { width: 1.5, color: s.color },
          itemStyle: { color: s.color },
          areaStyle: { color: s.color, opacity: c.dark ? 0.55 : 0.45 },
          emphasis: { disabled: true },
          markLine: i === 0 ? markLine : undefined,
        },
  );

  return {
    ...commonFrame(c, axisLabels(periods, granularity), currency, chartType, 8 + legendHeight + zoom.bottomExtra),
    tooltip: {
      trigger: 'axis',
      ...tooltipStyle(c),
      axisPointer: chartType === 'bar' ? { type: 'shadow' } : { type: 'line', lineStyle: { color: c.axis } },
      formatter: (params: unknown) => {
        const items = (Array.isArray(params) ? params : [params]) as Array<{
          dataIndex: number;
          seriesIndex: number;
          value?: unknown;
        }>;
        const idx = items[0]?.dataIndex;
        if (idx === undefined || !periods[idx]) return '';
        const rows = items
          .map((item) => ({ s: series[item.seriesIndex], value: typeof item.value === 'number' ? item.value : 0 }))
          .filter((r): r is { s: StackedSeries; value: number } => r.s !== undefined)
          .sort((a, b) => b.value - a.value);
        const total = rows.reduce((sum, r) => sum + r.value, 0);
        // Series names trace back to cost-data values (service/account/
        // resource/tag names) — escape before interpolating into the HTML
        // the formatter returns (ECharts does not escape it).
        const body = rows
          .map((r) =>
            tooltipRow(
              r.s.color,
              escapeHtml(r.s.name),
              formatCurrency(r.value, currency),
              total > 0 ? `<span class="tt-share">${((r.value / total) * 100).toFixed(0)}%</span>` : '',
            ),
          )
          .join('');
        return (
          `<div class="tt-title">${escapeHtml(formatPeriodLabel(periods[idx], granularity))}</div>` +
          body +
          `<div class="tt-total"><span>Total</span><span>${formatCurrency(total, currency)}</span></div>`
        );
      },
    },
    legend: {
      type: 'scroll',
      bottom: 4 + zoom.bottomExtra,
      icon: 'roundRect',
      itemWidth: 10,
      itemHeight: 10,
      itemGap: 14,
      textStyle: { color: c.textSecondary, fontSize: 12 },
      pageTextStyle: { color: c.textMuted },
      data: series.map((s) => s.name),
    },
    dataZoom: zoom.dataZoom,
    series: echartsSeries,
  };
}

export interface CreditSplitTrendInput {
  split: CreditSplit;
  currency: string;
  granularity: TimeGranularity;
  chartType: ChartType;
}

const CREDIT_LABEL = 'Credits & discounts';

/**
 * Charges above zero, credits below zero, and the net result as a line on
 * top — so a period where credits cancel most of the bill reads as exactly
 * that, instead of as a dip in spend. Bars stack same-sign, so a period's
 * charges and credits share one slot, pointing opposite ways.
 */
export function buildCreditSplitTrendOption({ split, currency, granularity, chartType }: CreditSplitTrendInput): EChartsCoreOption {
  const c = chartColors();
  const chargeColor = c.series[0];
  const creditColor = c.series[2];
  const { periods } = split;
  const zoom = timeZoom(periods.length, c);
  const markLine = yearMarkLine(periods, granularity, c);
  const legendHeight = 36;

  const area = (color: string) =>
    new echarts.graphic.LinearGradient(0, 0, 0, 1, [
      { offset: 0, color: withAlpha(color, c.dark ? 0.35 : 0.25) },
      { offset: 1, color: withAlpha(color, 0.04) },
    ]);
  const creditArea = new echarts.graphic.LinearGradient(0, 0, 0, 1, [
    { offset: 0, color: withAlpha(creditColor, 0.04) },
    { offset: 1, color: withAlpha(creditColor, c.dark ? 0.4 : 0.3) },
  ]);

  const bars = chartType === 'bar';
  const series = [
    bars
      ? {
          name: 'Charges',
          type: 'bar',
          stack: 'split',
          data: split.charges,
          barMaxWidth: 28,
          itemStyle: { color: chargeColor, borderRadius: [4, 4, 0, 0] },
          markLine,
        }
      : {
          name: 'Charges',
          type: 'line',
          data: split.charges,
          smooth: 0.25,
          showSymbol: false,
          lineStyle: { width: 1.5, color: chargeColor },
          itemStyle: { color: chargeColor },
          areaStyle: { color: area(chargeColor) },
          markLine,
        },
    bars
      ? {
          name: CREDIT_LABEL,
          type: 'bar',
          stack: 'split',
          data: split.credits,
          barMaxWidth: 28,
          itemStyle: { color: creditColor, borderRadius: [0, 0, 4, 4] },
        }
      : {
          name: CREDIT_LABEL,
          type: 'line',
          data: split.credits,
          smooth: 0.25,
          showSymbol: false,
          lineStyle: { width: 1.5, color: creditColor },
          itemStyle: { color: creditColor },
          areaStyle: { color: creditArea },
        },
    {
      name: 'Net',
      type: 'line',
      data: split.net,
      smooth: 0.25,
      symbol: 'circle',
      symbolSize: 7,
      showSymbol: periods.length <= 31,
      z: 5,
      lineStyle: { width: 2, color: c.text },
      itemStyle: { color: c.text, borderColor: c.surface, borderWidth: 2 },
    },
  ];

  return {
    ...commonFrame(c, axisLabels(periods, granularity), currency, chartType, 8 + legendHeight + zoom.bottomExtra),
    tooltip: {
      trigger: 'axis',
      ...tooltipStyle(c),
      axisPointer: bars ? { type: 'shadow' } : { type: 'line', lineStyle: { color: c.axis } },
      formatter: (params: unknown) => {
        const first = (Array.isArray(params) ? params[0] : params) as { dataIndex: number } | undefined;
        const i = first?.dataIndex ?? -1;
        if (!periods[i]) return '';
        const categoryRows = Array.from(split.creditByCategory)
          .filter(([, values]) => values[i] !== 0)
          .map(
            ([category, values]) =>
              `<div class="tt-row tt-indent"><span class="tt-label">${escapeHtml(category)}</span><span class="tt-value">${formatCurrency(values[i], currency)}</span></div>`,
          )
          .join('');
        return (
          `<div class="tt-title">${escapeHtml(formatPeriodLabel(periods[i], granularity))}</div>` +
          tooltipRow(chargeColor, 'Charges', formatCurrency(split.charges[i], currency)) +
          tooltipRow(creditColor, CREDIT_LABEL, formatCurrency(split.credits[i], currency)) +
          categoryRows +
          `<div class="tt-total"><span>Net</span><span>${formatCurrency(split.net[i], currency)}</span></div>`
        );
      },
    },
    legend: {
      bottom: 4 + zoom.bottomExtra,
      icon: 'roundRect',
      itemWidth: 10,
      itemHeight: 10,
      itemGap: 16,
      textStyle: { color: c.textSecondary, fontSize: 12 },
      data: ['Charges', CREDIT_LABEL, 'Net'],
    },
    dataZoom: zoom.dataZoom,
    series,
  };
}

/**
 * Draws a total-cost trend from a series grouped by `charge_category`: the
 * charges/credits/net split when the period has credit rows, otherwise the
 * plain single-series trend (unchanged for accounts without credits).
 * Returns the split so callers can feed KPI cards from the same data.
 */
export function drawCostTrend(
  containerId: string,
  container: HTMLElement,
  points: TimeSeriesPoint[],
  currency: string,
  granularity: TimeGranularity,
  getChartType: () => ChartType,
): CreditSplit {
  const split = splitCredits(points);
  setChartOption(containerId, container, () =>
    split.hasCredits
      ? buildCreditSplitTrendOption({ split, currency, granularity, chartType: getChartType() })
      : buildSingleTrendOption({ points: split.netPoints, currency, granularity, chartType: getChartType() }),
  );
  return split;
}
