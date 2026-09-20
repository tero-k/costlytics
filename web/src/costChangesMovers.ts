/**
 * "Biggest movers" view for the Cost Changes page (plan §30) — described in
 * the plan as "one of the most useful FinOps views": the largest individual
 * cost increases and decreases between the current and previous period, by
 * the page's selected dimension. Rendered into `#changes-movers`.
 *
 * Calls `getCompare()` WITH the page's selected `dimension`, using the
 * page's own INDEPENDENT current/previous ranges from `readChangesControls()`
 * (not an auto-derived previous period, unlike `explorerTable.ts`). Ranks
 * the returned rows client-side by `absolute_change` and shows the top 5 (or
 * fewer, if there aren't that many rows) increases and top 5 decreases as
 * two side-by-side lists — a simple scannable list, not a full table (the
 * full sortable table is `costChangesTable.ts`, Task 3).
 */

import { getCompare, type CompareRow } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, formatSignedCurrency, changeClass, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { formatKeyLabel } from './shared/labels.ts';
import { readChangesControls, type ChangesControls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'changes-movers';
const TOP_COUNT = 5;

const refreshGuard = new RequestGuard();

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

function renderLoading(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-loading">Loading&hellip;</div>`;
}

function renderEmpty(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-empty">No data for this period.</div>`;
}

function renderError(container: HTMLElement, message: string): void {
  container.innerHTML = `<div class="table-status table-error">${escapeHtml(message)}</div>`;
}

function renderMoverList(title: string, rows: CompareRow[], currency: string): string {
  if (rows.length === 0) {
    return `
      <div class="movers-column">
        <h3>${escapeHtml(title)}</h3>
        <div class="table-status table-empty">None</div>
      </div>`;
  }

  const itemsHtml = rows
    .map((row) => {
      const label = formatKeyLabel(row.key);
      const cls = changeClass(row.absolute_change);
      return `
        <li class="movers-item">
          <span class="movers-key">${escapeHtml(label)}</span>
          <span class="movers-values">
            <span class="movers-change ${cls}">${formatSignedCurrency(row.absolute_change, currency)}</span>
            <span class="movers-sub">${formatCurrency(row.previous, currency)} &rarr; ${formatCurrency(row.current, currency)}</span>
          </span>
        </li>`;
    })
    .join('');

  return `
    <div class="movers-column">
      <h3>${escapeHtml(title)}</h3>
      <ul class="movers-list">${itemsHtml}</ul>
    </div>`;
}

function renderMovers(container: HTMLElement, rows: CompareRow[], currency: string): void {
  // Rank by `absolute_change`: largest positive values first for increases,
  // largest-magnitude negative values first for decreases.
  const increases = [...rows]
    .filter((r) => r.absolute_change > 0)
    .sort((a, b) => b.absolute_change - a.absolute_change)
    .slice(0, TOP_COUNT);

  const decreases = [...rows]
    .filter((r) => r.absolute_change < 0)
    .sort((a, b) => a.absolute_change - b.absolute_change)
    .slice(0, TOP_COUNT);

  container.innerHTML = `
    <h2>Biggest movers</h2>
    <div class="movers-grid">
      ${renderMoverList('Biggest increases', increases, currency)}
      ${renderMoverList('Biggest decreases', decreases, currency)}
    </div>`;
}

async function loadMovers(controls: ChangesControls, token: number): Promise<void> {
  const container = getContainer();
  if (!container) return;

  renderLoading(container);

  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const previousStart = controls.previousStartIso;
  const previousEnd = addDaysIso(controls.previousEndIsoInclusive, 1);

  try {
    const result = await getCompare({
      current_start: currentStart,
      current_end: currentEnd,
      previous_start: previousStart,
      previous_end: previousEnd,
      metric: controls.metric,
      dimension: controls.dimension,
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.rows.length === 0) {
      renderEmpty(container);
      return;
    }

    renderMovers(container, result.rows, result.currency);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    renderError(container, errorMessage(err));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initChangesMovers(): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const controls = readChangesControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadMovers(controls, token);
  };

  return refresh();
}
