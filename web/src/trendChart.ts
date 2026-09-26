/**
 * Cost trend chart for the Overview page (plan §20).
 *
 * Renders an ECharts line/area chart into `#trend-chart`, showing total cost
 * over time for the page's selected date range and metric, ungrouped
 * (`group_by` omitted — a single aggregate series). A granularity control
 * (`#granularity-select`) lets the user pick Day/Month/Year explicitly, or
 * leave it on "Auto", in which case the granularity is derived from the
 * selected range's length per the plan's rule:
 *   - <= 90 days  -> day
 *   - <= 3 years  -> month
 *   - > 3 years   -> year
 *
 * Shares the same `#date-start` / `#date-end` / `#metric-select` controls
 * that `kpiCards.ts` reads from — this component does not maintain its own
 * date-range/metric state, only its own granularity override.
 */

import { getTimeseries, type TimeGranularity } from './api.ts';
import { addDaysIso, daysBetweenIso } from './shared/dates.ts';
import { errorMessage } from './shared/format.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { clearOverlays, showOverlay, initChartTypeToggle, showChartLoading, type ChartType } from './shared/chart.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { autoGranularity } from './shared/granularity.ts';
import { drawCostTrend } from './shared/trendOptions.ts';
import type { CreditSplit } from './shared/credits.ts';

const CONTAINER_ID = 'trend-chart';

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

function readGranularitySelection(): TimeGranularity | 'auto' {
  const select = document.querySelector<HTMLSelectElement>('#granularity-select');
  const value = select?.value;
  if (value === 'day' || value === 'month' || value === 'year') return value;
  return 'auto';
}

// ---------------------------------------------------------------------------
// Chart state / DOM helpers
// ---------------------------------------------------------------------------

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

const refreshGuard = new RequestGuard();

/** Line/Bar toggle (`#trend-chart-type`); set up in `initTrendChart`. */
let getChartType: () => ChartType = () => 'line';

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadTrendChart(
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
  onSeries?: TrendSeriesCallback,
): Promise<void> {
  const container = getContainer();
  if (!container) return;

  clearOverlays(container);
  showChartLoading(container);

  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const durationDays = daysBetweenIso(currentStart, currentEnd);

  const selection = readGranularitySelection();
  const granularity: TimeGranularity = selection === 'auto' ? autoGranularity(durationDays) : selection;

  try {
    const result = await getTimeseries({
      start: currentStart,
      end: currentEnd,
      metric: controls.metric,
      granularity,
      // Grouped by charge category (still one query) so credits can be
      // drawn apart from charges — see `shared/credits.ts`.
      group_by: 'charge_category',
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.series.length === 0) {
      onSeries?.(null, result.currency);
      showOverlay(CONTAINER_ID, container, 'chart-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(result.currency);
    const split = drawCostTrend(CONTAINER_ID, container, result.series, result.currency, granularity, getChartType);
    onSeries?.(split, result.currency);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    showOverlay(CONTAINER_ID, container, 'chart-error', errorMessage(err));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/**
 * Receives each loaded trend, split into charges/credits/net (`null` when
 * the period is empty) — feeds the KPI sparkline and credits card without
 * an extra query.
 */
export type TrendSeriesCallback = (split: CreditSplit | null, currency: string) => void;

export function initTrendChart(
  onCurrency?: (currency: string) => void,
  onSeries?: TrendSeriesCallback,
): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  getChartType = initChartTypeToggle('trend-chart-type', CONTAINER_ID);

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadTrendChart(controls, token, onCurrency, onSeries);
  };

  subscribeToControls(refresh, { extraIds: ['#granularity-select'] });

  return refresh();
}
