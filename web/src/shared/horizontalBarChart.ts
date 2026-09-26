/**
 * Horizontal bar chart renderer shared by `topBreakdown.ts` (Overview page)
 * and `entityBreakdowns.ts` (drilldown pages) — both render the same ranked
 * bar list (row-reversal so the highest-cost row ends up at the top despite
 * ECharts' bottom-to-top category axis) against a `{ label, total }[]` row
 * list, differing only in which container id each caller uses as the
 * chart's instance-cache key (see `shared/chart.ts`'s `ensureChart`).
 *
 * One series, one hue (slot 1): the bars encode magnitude, not identity, so
 * they aren't colored per row. A folded "Other" row gets the neutral fill.
 * Each bar carries its value and share as a direct label.
 */

import { formatCurrency, formatCurrencyCompact } from './format.ts';
import { escapeHtml } from './html.ts';
import { setChartOption } from './chart.ts';
import { baseChartOption, chartColors, tooltipRow, tooltipStyle, valueAxisStyle } from './chartTheme.ts';

const OTHER_LABEL = 'Other';
const MAX_LABEL_CHARS = 28;

function truncate(label: string): string {
  return label.length > MAX_LABEL_CHARS ? `${label.slice(0, MAX_LABEL_CHARS - 1)}…` : label;
}

/**
 * Renders `rows` as a horizontal bar chart into `element`, using `chartId` as
 * the `ensureChart` instance-cache key (callers may key this differently from
 * `element`'s own DOM id — see `entityBreakdowns.ts`, which nests its chart
 * area under a separate outer container).
 */
export function renderHorizontalBarChart(
  chartId: string,
  element: HTMLElement,
  rows: Array<{ label: string; total: number }>,
  currency: string,
): void {
  // ECharts renders horizontal bar category axes bottom-to-top, so reverse
  // to keep the highest-cost row at the top of the chart.
  const reversed = [...rows].reverse();
  const grandTotal = rows.reduce((sum, row) => sum + row.total, 0);
  const share = (value: number): string => (grandTotal > 0 ? `${((value / grandTotal) * 100).toFixed(0)}%` : '');

  // Height follows the row count, so a 2-row breakdown isn't a tall, empty card.
  element.style.minHeight = `${Math.max(150, rows.length * 32 + 48)}px`;

  setChartOption(chartId, element, () => {
    const c = chartColors();
    return {
      ...baseChartOption(c),
      tooltip: {
        trigger: 'axis',
        ...tooltipStyle(c),
        axisPointer: { type: 'shadow', shadowStyle: { color: c.dark ? 'rgba(255,255,255,0.04)' : 'rgba(15,23,42,0.04)' } },
        formatter: (params: unknown) => {
          const items = Array.isArray(params) ? params : [params];
          const first = items[0] as { dataIndex: number } | undefined;
          if (!first) return '';
          const row = reversed[first.dataIndex];
          if (!row) return '';
          // `row.label` traces back to `breakdown()`'s `key` field, i.e.
          // real cost-data values (service/account/region/etc. names) —
          // escape before interpolating into the HTML `tooltip` formatter
          // returns (ECharts' default `renderMode: 'html'` does not escape
          // it for us).
          const color = row.label === OTHER_LABEL ? c.other : c.series[0];
          return (
            `<div class="tt-title">${escapeHtml(row.label)}</div>` +
            tooltipRow(color, 'Cost', formatCurrency(row.total, currency), `<span class="tt-share">${share(row.total)}</span>`)
          );
        },
      },
      grid: { left: 8, right: 88, top: 8, bottom: 8, containLabel: true },
      xAxis: {
        type: 'value',
        ...valueAxisStyle(c),
        axisLabel: {
          ...(valueAxisStyle(c).axisLabel as object),
          formatter: (value: number) => formatCurrencyCompact(value, currency),
        },
      },
      yAxis: {
        type: 'category',
        data: reversed.map((row) => row.label),
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { color: c.textSecondary, fontSize: 12, formatter: truncate },
      },
      series: [
        {
          type: 'bar',
          barMaxWidth: 22,
          barCategoryGap: '35%',
          data: reversed.map((row) => ({
            value: row.total,
            itemStyle: { color: row.label === OTHER_LABEL ? c.other : c.series[0], borderRadius: [0, 4, 4, 0] },
          })),
          label: {
            show: true,
            position: 'right',
            distance: 8,
            color: c.textSecondary,
            fontSize: 11,
            formatter: (p: { value: number }) => `${formatCurrencyCompact(p.value, currency)}  ${share(p.value)}`,
          },
          emphasis: { itemStyle: { opacity: 0.85 } },
        },
      ],
    };
  });
}
