import './style.css';
import { initKpiCards } from './kpiCards.ts';
import { initTrendChart } from './trendChart.ts';
import { initTopBreakdownCharts } from './topBreakdown.ts';

/**
 * Basic app shell for the Costlytics Overview page.
 *
 * This bootstraps the date-range / metric controls with sensible defaults
 * (current month to date, amortized metric), then wires up the KPI card
 * row and the cost trend chart, both of which read these shared controls'
 * values and populate their own containers via `src/api.ts`.
 */

function toDateInputValue(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function initDateRangeDefaults(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  if (!startInput || !endInput) return;

  const today = new Date();
  const startOfMonth = new Date(today.getFullYear(), today.getMonth(), 1);

  startInput.value = toDateInputValue(startOfMonth);
  endInput.value = toDateInputValue(today);
}

initDateRangeDefaults();
initKpiCards();
initTrendChart();
initTopBreakdownCharts();
