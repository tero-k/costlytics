import './style.css';
import { getSources, type SourceStatus } from './api.ts';
import { setLoadingIndicatorVisible } from './shared/statusBar.ts';
import { escapeHtml } from './shared/html.ts';
import { errorMessage } from './shared/format.ts';
import { subscribeToControls } from './shared/controls.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { RequestGuard } from './shared/requestGuard.ts';

/**
 * App shell / orchestrator for the Data Sources diagnostics page (Session 14
 * Task 4).
 *
 * Unlike every other page, this page's own content (`GET /api/v1/sources`)
 * is NOT scoped by `source_id` — it always returns every configured
 * source's status regardless of which one is "active". The shared
 * `#source-picker` control is still present and wired up here (via
 * `initSourcePicker()`) purely for shell consistency with the other five
 * pages (so a viewer navigating here doesn't lose their selection, and the
 * picker's own "which sources are registered/selectable" behavior stays
 * correct), not because it changes what this page displays.
 *
 * This page intentionally shows ALL configured sources, including SKIPPED
 * ones the picker itself excludes (`sourcePicker.ts` only offers registered
 * sources) — that's the whole point of a diagnostics view.
 */

const CONTAINER_ID = 'data-sources-table';

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

const refreshGuard = new RequestGuard();

function updateStatusSourceCount(sources: SourceStatus[]): void {
  const el = document.querySelector<HTMLElement>('#status-source-count');
  if (!el) return;
  const registeredCount = sources.filter((s) => s.state === 'registered').length;
  const skippedCount = sources.length - registeredCount;
  el.textContent = `${sources.length} configured source${sources.length === 1 ? '' : 's'} (${registeredCount} registered, ${skippedCount} skipped)`;
}

function renderLoading(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-loading">Loading&hellip;</div>`;
}

function renderEmpty(container: HTMLElement): void {
  container.innerHTML = `<div class="table-status table-empty">No sources configured.</div>`;
}

function renderError(container: HTMLElement, message: string): void {
  container.innerHTML = `<div class="table-status table-error">${escapeHtml(message)}</div>`;
}

/**
 * `configured_type` is an enum straight off the wire (`'auto' | 'cur2' |
 * 'focus10' | 'focus12'`), not user-controlled text — used directly, no
 * escaping strictly required, but routed through `escapeHtml` anyway for
 * defense-in-depth consistency with every other dynamic cell in this row.
 */
function renderTable(container: HTMLElement, sources: SourceStatus[]): void {
  const bodyHtml = sources
    .map((s) => {
      const statusCls = s.state === 'registered' ? 'change-good' : 'change-bad';
      const statusLabel = s.state === 'registered' ? 'Registered' : 'Skipped';
      const detectedFormat = s.detected_format ?? '—';
      const fileCount = s.file_count !== undefined ? String(s.file_count) : '—';
      const reason = s.reason ?? '—';

      return `
        <tr>
          <td class="col-left">${escapeHtml(s.id)}</td>
          <td class="col-left">${escapeHtml(s.name)}</td>
          <td class="col-left">${escapeHtml(s.configured_type)}</td>
          <td class="col-left ${statusCls}">${escapeHtml(statusLabel)}</td>
          <td class="col-left">${escapeHtml(detectedFormat)}</td>
          <td class="col-right">${escapeHtml(fileCount)}</td>
          <td class="col-left">${escapeHtml(reason)}</td>
        </tr>`;
    })
    .join('');

  container.innerHTML = `
    <table class="explorer-table static-table">
      <thead>
        <tr>
          <th class="col-left">ID</th>
          <th class="col-left">Name</th>
          <th class="col-left">Configured Type</th>
          <th class="col-left">Status</th>
          <th class="col-left">Detected Format</th>
          <th class="col-right">File Count</th>
          <th class="col-left">Skip Reason</th>
        </tr>
      </thead>
      <tbody>${bodyHtml}</tbody>
    </table>`;
}

async function loadSources(): Promise<void> {
  const container = getContainer();
  if (!container) return;

  renderLoading(container);
  const token = refreshGuard.next();

  try {
    const { sources } = await getSources();
    if (!refreshGuard.isCurrent(token)) return;

    updateStatusSourceCount(sources);

    if (sources.length === 0) {
      renderEmpty(container);
      return;
    }
    renderTable(container, sources);
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    renderError(container, errorMessage(err));
  }
}

async function bootstrap(): Promise<void> {
  await initSourcePicker();
  // No date/metric controls exist on this page; `subscribeToControls`
  // no-ops on ids that aren't present (see `controls.ts`), so this only
  // ever reacts to `#source-picker` changes here. This page's own content
  // doesn't depend on the active source, but re-running the (idempotent,
  // cheap) fetch keeps this page's refresh wiring identical to every other
  // page's rather than being a special case.
  subscribeToControls(() => void loadSources());

  setLoadingIndicatorVisible(true);
  try {
    await loadSources();
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
