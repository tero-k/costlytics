/**
 * Horizontal bar chart renderer shared by `topBreakdown.ts` (Overview page)
 * and `serviceBreakdowns.ts` (Service Detail page) — both render the exact
 * same ECharts horizontal bar chart (row-reversal so the highest-cost row
 * ends up at the top despite ECharts' bottom-to-top category axis, same
 * grid/tooltip/bar styling) against a `{ label, total }[]` row list, differing
 * only in which container id each caller uses as the chart's instance-cache
 * key (see `shared/chart.ts`'s `ensureChart`).
 */

import { formatCurrency, formatCurrencyCompact } from './format.ts';
import { escapeHtml } from './html.ts';
import { ensureChart } from './chart.ts';

/**
 * Renders `rows` as a horizontal bar chart into `element`, using `chartId` as
 * the `ensureChart` instance-cache key (callers may key this differently from
 * `element`'s own DOM id — see `serviceBreakdowns.ts`, which nests its chart
 * area under a separate outer container).
 */
export function renderHorizontalBarChart(
  chartId: string,
  element: HTMLElement,
  rows: Array<{ label: string; total: number }>,
  currency: string,
): void {
  const instance = ensureChart(chartId, element);

  // ECharts renders horizontal bar category axes bottom-to-top, so reverse
  // to keep the highest-cost row at the top of the chart.
  const reversed = [...rows].reverse();
  const categories = reversed.map((row) => row.label);
  const totals = reversed.map((row) => row.total);

  instance.setOption(
    {
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
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
          return [`<strong>${escapeHtml(row.label)}</strong>`, formatCurrency(row.total, currency)].join('<br/>');
        },
      },
      grid: {
        left: 8,
        right: 24,
        top: 16,
        bottom: 8,
        containLabel: true,
      },
      xAxis: {
        type: 'value',
        axisLabel: {
          formatter: (value: number) => formatCurrencyCompact(value, currency),
        },
      },
      yAxis: {
        type: 'category',
        data: categories,
      },
      series: [
        {
          type: 'bar',
          data: totals,
          itemStyle: {
            borderRadius: [0, 4, 4, 0],
          },
        },
      ],
    },
    true,
  );
}
