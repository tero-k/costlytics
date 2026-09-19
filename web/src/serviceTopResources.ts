/**
 * Top resources table for the Service Detail page's selected service (plan
 * §26).
 *
 * Renders a simple read-only HTML `<table>` into `#service-top-resources`
 * from `POST /cost/breakdown` with `dimension: "resource"`, filtered to the
 * currently selected service (`services: [selectedService]`), limited to the
 * top 15 rows by cost. Resource IDs are often long opaque strings (ARNs,
 * instance IDs, etc.), so a table reads better here than a bar chart — see
 * `serviceBreakdowns.ts`'s module doc for that chart-vs-table split.
 *
 * Deliberately simpler than `explorerTable.ts`: no client-side sorting, no
 * CSV export, no "Other" bucket — just the top-15 rows the API already
 * returns in cost-descending order, rendered as-is. Keeps its own
 * `RequestGuard` and try/catch so a failure here can't blank the other four
 * Service Detail components (same `Promise.allSettled` discipline as the
 * rest of the app).
 *
 * When no service is selected (`getSelectedService()` returns `null`), the
 * table clears itself and skips fetching entirely, per this page's "nothing
 * to fetch yet" convention (see `serviceDetailMain.ts`).
 */

import { getBreakdown, type BreakdownRow } from './api.ts';
import { getSelectedService } from './shared/servicePicker.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { formatKeyLabel } from './shared/labels.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';

const CONTAINER_ID = 'service-top-resources';
const RESOURCE_LIMIT = 15;

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

const refreshGuard = new RequestGuard();

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function renderMessage(container: HTMLElement, className: string, message: string): void {
  container.innerHTML = `
    <section class="chart-section">
      <div class="chart-header">
        <h2>Top resources</h2>
      </div>
      <div class="table-status ${className}">${escapeHtml(message)}</div>
    </section>`;
}

function renderTable(container: HTMLElement, rows: BreakdownRow[], currency: string): void {
  const bodyHtml = rows
    .map(
      (row) => `
        <tr>
          <td class="col-left">${escapeHtml(formatKeyLabel(row.key))}</td>
          <td class="col-right">${formatCurrency(row.total, currency)}</td>
          <td class="col-right">${row.row_count.toLocaleString()}</td>
        </tr>`,
    )
    .join('');

  container.innerHTML = `
    <section class="chart-section">
      <div class="chart-header">
        <h2>Top resources</h2>
      </div>
      <table class="explorer-table">
        <thead>
          <tr>
            <th class="col-left">Resource ID</th>
            <th class="col-right">Cost</th>
            <th class="col-right">Row Count</th>
          </tr>
        </thead>
        <tbody>${bodyHtml}</tbody>
      </table>
    </section>`;
}

// ---------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------

async function loadTopResources(
  service: string,
  controls: Controls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const container = getContainer();
  if (!container) return;

  renderMessage(container, 'table-loading', 'Loading…');

  const start = controls.startIso;
  const end = addDaysIso(controls.endIsoInclusive, 1);

  try {
    const result = await getBreakdown({
      start,
      end,
      metric: controls.metric,
      dimension: 'resource',
      services: [service],
      limit: RESOURCE_LIMIT,
    });

    if (!refreshGuard.isCurrent(token)) return;

    if (result.rows.length === 0) {
      renderMessage(container, 'table-empty', 'No data for this period.');
      return;
    }

    onCurrency?.(result.currency);
    renderTable(container, result.rows, result.currency);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    renderMessage(container, 'table-error', errorMessage(err));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initServiceTopResources(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  const refresh = async (): Promise<void> => {
    const service = getSelectedService();
    if (!service) {
      refreshGuard.next();
      renderMessage(container, 'table-empty', 'Select a service to view top resources.');
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadTopResources(service, controls, token, onCurrency);
  };

  subscribeToControls(refresh, { extraIds: ['#service-picker'] });

  return refresh();
}
