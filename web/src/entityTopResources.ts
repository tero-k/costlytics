/**
 * Generic top resources table for a "detail page" entity (plan §26), e.g.
 * the Service Detail or Account Detail page's currently selected
 * service/account.
 *
 * Renders a simple read-only HTML `<table>` into `#{idPrefix}-top-resources`
 * from `POST /cost/breakdown` with `dimension: "resource"`, filtered to the
 * currently selected entity (`config.buildFilter(selected)`), limited to the top
 * 15 rows by cost. Resource IDs are often long opaque strings (ARNs,
 * instance IDs, etc.), so a table reads better here than a bar chart — see
 * `serviceBreakdowns.ts`'s module doc for that chart-vs-table split.
 *
 * Generalized from `serviceTopResources.ts`/`accountTopResources.ts`
 * (Session 11) once Session 10's whole-branch review found those two modules
 * had zero code differences beyond entity-noun substitution — see
 * `shared/entityConfig.ts`'s doc comment for the substitution points this
 * module parameterizes on.
 *
 * Deliberately simpler than `explorerTable.ts`: no client-side sorting, no
 * CSV export, no "Other" bucket — just the top-15 rows the API already
 * returns in cost-descending order, rendered as-is. Keeps its own
 * `RequestGuard` and try/catch so a failure here can't blank the other
 * components on the page (same `Promise.allSettled` discipline as the rest
 * of the app).
 *
 * When no entity is selected (`config.getSelected()` returns `null`), the
 * table clears itself and skips fetching entirely, per this page family's
 * "nothing to fetch yet" convention.
 */

import { getBreakdown, type BreakdownRow } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, errorMessage } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { formatKeyLabel } from './shared/labels.ts';
import { readControls, subscribeToControls, type Controls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import type { EntityConfig } from './shared/entityConfig.ts';

const RESOURCE_LIMIT = 15;

function getContainer(idPrefix: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${idPrefix}-top-resources`);
}

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

/** A resource ID links to its Resources-tab detail (`resourceDetailMain.ts` intercepts it in-page). */
function resourceCell(key: string | null): string {
  if (key === null) return escapeHtml(formatKeyLabel(key));
  const href = `/resource-detail.html?resource=${encodeURIComponent(key)}`;
  return `<a class="resource-link" href="${escapeHtml(href)}" data-resource="${escapeHtml(key)}">${escapeHtml(key)}</a>`;
}

function renderTable(container: HTMLElement, rows: BreakdownRow[], currency: string): void {
  const bodyHtml = rows
    .map(
      (row) => `
        <tr>
          <td class="col-left">${resourceCell(row.key)}</td>
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
// Public entry point
// ---------------------------------------------------------------------------

/** Handle returned by `initEntityTopResources`: a stable, reusable `refresh()` that reuses the same `RequestGuard` and does NOT re-register control listeners on repeat calls. */
export interface EntityTopResourcesHandle {
  refresh: () => Promise<void>;
}

export function initEntityTopResources(
  config: EntityConfig,
  onCurrency?: (currency: string) => void,
): EntityTopResourcesHandle {
  const containerOrNull = getContainer(config.idPrefix);
  if (!containerOrNull) return { refresh: () => Promise.resolve() };
  const container = containerOrNull;

  const refreshGuard = new RequestGuard();

  async function loadTopResources(selected: string, controls: Controls, token: number): Promise<void> {
    renderMessage(container, 'table-loading', 'Loading…');

    const start = controls.startIso;
    const end = addDaysIso(controls.endIsoInclusive, 1);

    try {
      const result = await getBreakdown({
        start,
        end,
        metric: controls.metric,
        dimension: 'resource',
        ...config.buildFilter(selected),
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

  const refresh = async (): Promise<void> => {
    const selected = config.getSelected();
    if (!selected) {
      refreshGuard.next();
      renderMessage(container, 'table-empty', `Select a ${config.entityNoun} to view top resources.`);
      return;
    }

    const controls = readControls();
    if (!controls) return;

    const token = refreshGuard.next();
    await loadTopResources(selected, controls, token);
  };

  subscribeToControls(refresh, { extraIds: [config.pickerSelector] });

  return { refresh };
}
