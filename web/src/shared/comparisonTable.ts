/**
 * Shared sortable comparison table logic for `explorerTable.ts` (Cost
 * Explorer) and `costChangesTable.ts` (Cost Changes).
 *
 * Both pages render an HTML `<table>` with columns Dimension | Cost |
 * Previous Cost | Difference | Difference % | % Total, for a selected
 * grouping dimension, comparing a `current` period against a `previous`
 * period fetched via `POST /api/v1/cost/compare`. `compare()` has no
 * `limit` parameter — it always returns every row for the dimension —  so
 * Top-N truncation and "Other" aggregation happen client-side here, ranking
 * by `current` descending. "% Total" is each row's `current` divided by the
 * OVERALL total across ALL rows (including the ones folded into "Other"),
 * not just the top-N subset.
 *
 * Sorting is entirely client-side: clicking a column header re-sorts the
 * currently-rendered rows (no re-fetch), toggling ascending/descending on
 * repeated clicks of the same column.
 *
 * The two pages genuinely differ in ONE respect: how the current/previous
 * date range pair is resolved. Cost Explorer auto-derives "previous" from a
 * single date range (`previousPeriod()`); Cost Changes uses two independent
 * user-controlled ranges. That resolution is left entirely to the caller via
 * `resolveRanges` — this module never assumes either convention.
 */

import { getCompare, type CompareRow, type CostMetric, type Dimension } from '../api.ts';
import { formatCurrency, formatPercent, formatSignedCurrency, changeClass, errorMessage } from './format.ts';
import { escapeHtml } from './html.ts';
import { formatKeyLabel } from './labels.ts';
import { RequestGuard } from './requestGuard.ts';

const OTHER_LABEL = 'Other';

// ---------------------------------------------------------------------------
// Row model
// ---------------------------------------------------------------------------

interface TableRow {
  key: string | null;
  isOther: boolean;
  current: number;
  previous: number;
  absoluteChange: number;
  percentageChange: number | null;
  percentageTotal: number;
}

type SortColumn = 'key' | 'current' | 'previous' | 'absoluteChange' | 'percentageChange' | 'percentageTotal';
type SortDirection = 'asc' | 'desc';

interface ColumnDef {
  id: SortColumn;
  label: string;
  align: 'left' | 'right';
}

const COLUMNS: ColumnDef[] = [
  { id: 'key', label: 'Dimension', align: 'left' },
  { id: 'current', label: 'Cost', align: 'right' },
  { id: 'previous', label: 'Previous Cost', align: 'right' },
  { id: 'absoluteChange', label: 'Difference', align: 'right' },
  { id: 'percentageChange', label: 'Difference %', align: 'right' },
  { id: 'percentageTotal', label: '% Total', align: 'right' },
];

// ---------------------------------------------------------------------------
// Public parameterization surface
// ---------------------------------------------------------------------------

/** The current/previous date range pair a page's controls resolve to, in `compare()`'s exclusive-end ISO format. */
export interface ComparisonRanges {
  currentStart: string;
  currentEnd: string;
  previousStart: string;
  previousEnd: string;
}

/** The minimal control surface every comparison table needs, regardless of how a page resolves its date ranges. */
export interface ComparisonControls {
  metric: CostMetric;
  dimension: Dimension;
  topN: number;
}

export interface ComparisonTableOptions<TControls extends ComparisonControls> {
  /** DOM id (no `#`) of the container the table renders into. */
  containerId: string;
  /** Reads this page's controls, or `null` when they aren't ready yet (mirrors `readExplorerControls`/`readChangesControls`). */
  readControls: () => TControls | null;
  /** Resolves this page's current/previous date range pair from its controls — the one place Explorer and Cost Changes genuinely differ. */
  resolveRanges: (controls: TControls) => ComparisonRanges;
  /** Registers `refresh` to re-run on this page's own control-change events. Optional: Cost Changes' `costChangesMain.ts` drives refresh centrally instead. */
  subscribe?: (refresh: () => void) => void;
  /** Called with the fetched result's currency code on each successful load, so the page's status bar can display it (same convention as the KPI-card/chart modules). */
  onCurrency?: (currency: string) => void;
}

export interface ComparisonTableHandle {
  /** Fetches + renders (or re-renders the loading/error/empty state), also usable as the page's own `refresh` callback. */
  refresh: () => Promise<void>;
  /**
   * Returns the table's current column headers plus its CURRENTLY RENDERED
   * rows (respecting whatever sort column/direction is active), each cell
   * pre-formatted exactly as it's displayed on screen (formatted currency,
   * "N/A" for a null Difference %, etc.). Returns `null` when there is
   * nothing rendered to export (no data loaded yet, or an error/empty state).
   */
  getExportTableData: () => { headers: string[]; rows: string[][] } | null;
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

function buildRows(rows: CompareRow[], topN: number): TableRow[] {
  const overallTotal = rows.reduce((acc, r) => acc + r.current, 0);

  // Rank by `current` descending to pick the top N.
  const ranked = [...rows].sort((a, b) => b.current - a.current);
  const top = ranked.slice(0, topN);
  const rest = ranked.slice(topN);

  const toRow = (r: CompareRow): TableRow => ({
    key: r.key,
    isOther: false,
    current: r.current,
    previous: r.previous,
    absoluteChange: r.absolute_change,
    percentageChange: r.percentage_change,
    percentageTotal: overallTotal !== 0 ? (r.current / overallTotal) * 100 : 0,
  });

  const result = top.map(toRow);

  if (rest.length > 0) {
    const otherCurrent = rest.reduce((acc, r) => acc + r.current, 0);
    const otherPrevious = rest.reduce((acc, r) => acc + r.previous, 0);
    const otherAbsoluteChange = otherCurrent - otherPrevious;
    // Recomputed from the aggregated current/previous, not averaged from
    // the individual rows' percentages.
    const otherPercentageChange = otherPrevious !== 0 ? (otherAbsoluteChange / otherPrevious) * 100 : null;

    result.push({
      key: OTHER_LABEL,
      isOther: true,
      current: otherCurrent,
      previous: otherPrevious,
      absoluteChange: otherAbsoluteChange,
      percentageChange: otherPercentageChange,
      percentageTotal: overallTotal !== 0 ? (otherCurrent / overallTotal) * 100 : 0,
    });
  }

  return result;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function renderLoading(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-loading">Loading&hellip;</div>`;
}

function renderEmpty(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-empty">No data for this period.</div>`;
}

function renderError(container: HTMLElement, message: string): void {
  container.innerHTML = `<div class="table-status table-error">${escapeHtml(message)}</div>`;
}

function sortValue(row: TableRow, column: SortColumn): number | string {
  switch (column) {
    case 'key':
      return formatKeyLabel(row.key).toLowerCase();
    case 'current':
      return row.current;
    case 'previous':
      return row.previous;
    case 'absoluteChange':
      return row.absoluteChange;
    case 'percentageChange':
      // `null` (undefined % change) sorts after every real value regardless
      // of direction, rather than being treated as 0.
      return row.percentageChange ?? Number.NEGATIVE_INFINITY;
    case 'percentageTotal':
      return row.percentageTotal;
  }
}

/**
 * One independent instance of comparison-table state (sort/rows/currency)
 * plus the fetch/render/sort/CSV-export logic over it. `createComparisonTable`
 * constructs one of these per page, so Explorer's and Cost Changes' module-
 * level state never bleed into each other.
 */
export function createComparisonTable<TControls extends ComparisonControls>(
  options: ComparisonTableOptions<TControls>,
): ComparisonTableHandle {
  const { containerId, readControls, resolveRanges, subscribe, onCurrency } = options;

  const refreshGuard = new RequestGuard();

  // Sort state persists across re-fetches (e.g. re-sorting by Cost survives a
  // date-range change), but resets to nothing special — default is
  // current-desc, matching the Top-N ranking order rows arrive in.
  let sortColumn: SortColumn = 'current';
  let sortDirection: SortDirection = 'desc';

  // Last-rendered rows + currency, kept so header clicks can re-sort and
  // re-render without a re-fetch.
  let lastRows: TableRow[] | null = null;
  let lastCurrency = '';

  function getContainer(): HTMLElement | null {
    return document.querySelector<HTMLElement>(`#${containerId}`);
  }

  function sortedRows(rows: TableRow[]): TableRow[] {
    const withNulls = rows.filter((r) => r.percentageChange === null && sortColumn === 'percentageChange');
    const withoutNulls = rows.filter((r) => !(r.percentageChange === null && sortColumn === 'percentageChange'));

    const compare = (a: TableRow, b: TableRow): number => {
      const av = sortValue(a, sortColumn);
      const bv = sortValue(b, sortColumn);
      let cmp: number;
      if (typeof av === 'string' && typeof bv === 'string') {
        cmp = av.localeCompare(bv);
      } else {
        cmp = (av as number) - (bv as number);
      }
      return sortDirection === 'asc' ? cmp : -cmp;
    };

    if (sortColumn === 'percentageChange') {
      // Null ("N/A") rows always sort to the end, independent of direction.
      return [...withoutNulls.sort(compare), ...withNulls];
    }
    return [...rows].sort(compare);
  }

  function renderTable(container: HTMLElement): void {
    if (!lastRows) return;
    const rows = sortedRows(lastRows);

    const headerHtml = COLUMNS.map((col) => {
      const isActive = col.id === sortColumn;
      const arrow = isActive ? (sortDirection === 'asc' ? ' ▲' : ' ▼') : '';
      return `<th class="col-${col.align}${isActive ? ' sorted' : ''}" data-column="${col.id}" role="button" tabindex="0">${escapeHtml(col.label)}${arrow}</th>`;
    }).join('');

    const bodyHtml = rows
      .map((row) => {
        const label = row.isOther ? OTHER_LABEL : formatKeyLabel(row.key);
        const changeCls = changeClass(row.absoluteChange);
        const diffText = formatSignedCurrency(row.absoluteChange, lastCurrency);
        const diffPctText = row.percentageChange === null ? 'N/A' : formatPercent(row.percentageChange);
        const pctTotalText = `${row.percentageTotal.toFixed(1)}%`;

        return `
          <tr${row.isOther ? ' class="row-other"' : ''}>
            <td class="col-left">${escapeHtml(label)}</td>
            <td class="col-right">${formatCurrency(row.current, lastCurrency)}</td>
            <td class="col-right">${formatCurrency(row.previous, lastCurrency)}</td>
            <td class="col-right ${changeCls}">${diffText}</td>
            <td class="col-right ${changeCls}">${diffPctText}</td>
            <td class="col-right">${pctTotalText}</td>
          </tr>`;
      })
      .join('');

    container.innerHTML = `
      <table class="explorer-table">
        <thead><tr>${headerHtml}</tr></thead>
        <tbody>${bodyHtml}</tbody>
      </table>`;

    container.querySelectorAll<HTMLElement>('th[data-column]').forEach((th) => {
      const onActivate = (): void => {
        const column = th.dataset['column'] as SortColumn;
        if (sortColumn === column) {
          sortDirection = sortDirection === 'asc' ? 'desc' : 'asc';
        } else {
          sortColumn = column;
          // Numeric columns default to descending (largest first); the
          // dimension name column defaults to ascending (A-Z).
          sortDirection = column === 'key' ? 'asc' : 'desc';
        }
        renderTable(container);
      };
      th.addEventListener('click', onActivate);
      th.addEventListener('keydown', (e) => {
        if (e instanceof KeyboardEvent && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault();
          onActivate();
        }
      });
    });
  }

  async function loadTable(controls: TControls, token: number): Promise<void> {
    const container = getContainer();
    if (!container) return;

    renderLoading(container);

    const ranges = resolveRanges(controls);

    try {
      const result = await getCompare({
        current_start: ranges.currentStart,
        current_end: ranges.currentEnd,
        previous_start: ranges.previousStart,
        previous_end: ranges.previousEnd,
        metric: controls.metric,
        dimension: controls.dimension,
      });

      if (!refreshGuard.isCurrent(token)) return;

      if (result.rows.length === 0) {
        lastRows = null;
        renderEmpty(container);
        return;
      }

      onCurrency?.(result.currency);

      lastRows = buildRows(result.rows, controls.topN);
      lastCurrency = result.currency;
      renderTable(container);
    } catch (err) {
      if (!refreshGuard.isCurrent(token)) return;
      lastRows = null;
      renderError(container, errorMessage(err));
    }
  }

  const refresh = async (): Promise<void> => {
    const controls = readControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadTable(controls, token);
  };

  subscribe?.(refresh);

  function getExportTableData(): { headers: string[]; rows: string[][] } | null {
    if (!lastRows) return null;

    const rows = sortedRows(lastRows).map((row) => {
      const label = row.isOther ? OTHER_LABEL : formatKeyLabel(row.key);
      const diffPctText = row.percentageChange === null ? 'N/A' : formatPercent(row.percentageChange);
      return [
        label,
        formatCurrency(row.current, lastCurrency),
        formatCurrency(row.previous, lastCurrency),
        formatSignedCurrency(row.absoluteChange, lastCurrency),
        diffPctText,
        `${row.percentageTotal.toFixed(1)}%`,
      ];
    });

    return { headers: COLUMNS.map((col) => col.label), rows };
  }

  return { refresh, getExportTableData };
}
