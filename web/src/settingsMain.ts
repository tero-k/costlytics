import './style.css';
import {
  deleteSource,
  getSettings,
  getSources,
  reloadSource,
  saveCostGuard,
  saveSource,
  testSource,
  type CostGuardSettings,
  type SourceSettings,
  type SourceStatus,
} from './api.ts';
import { escapeHtml } from './shared/html.ts';
import { errorMessage } from './shared/format.ts';
import { setLoadingIndicatorVisible } from './shared/statusBar.ts';
import { formToSource, sourceToForm, type SourceFormValues } from './shared/sourceForm.ts';
import { isTauri } from './transport.ts';
import { initAppShell } from './shared/appShell.ts';

initAppShell();

/**
 * Settings page: lists configured data sources with their live registration
 * status (the old Data Sources diagnostics table, now with actions) and an
 * add/edit form. Every mutation goes through `crates/service`'s
 * `CostlyticsService`, which persists `settings.toml`, keeps access-key
 * secrets in the OS keychain and re-registers the source immediately.
 */

let settings: SourceSettings[] = [];
let editing: SourceSettings | null = null;
/** Id whose Delete button is armed (second click confirms). */
let armedDelete: string | null = null;

const REFRESH_POLL_MS = 1000;
/** ~2 minutes: mirrors `sourcePicker.ts`'s `PENDING_POLL_LIMIT`, so a stuck
 * pending source stops this page's re-poll instead of running forever. */
const REFRESH_POLL_LIMIT = 120;
/** Consecutive pending-triggered `refresh()` calls so far; reset to 0 as soon as nothing is pending. */
let pendingPollAttempts = 0;

const $ = <T extends HTMLElement>(sel: string): T => document.querySelector<T>(sel)!;

/** Fills the Cost guard form once; later refreshes (polling) keep the user's unsaved edits. */
let costGuardFilled = false;

function fillCostGuardForm(guard: CostGuardSettings): void {
  if (costGuardFilled) return;
  costGuardFilled = true;
  $<HTMLSelectElement>('#cg-enabled').value = String(guard.enabled);
  $<HTMLInputElement>('#cg-soft').value = String(guard.soft_limit_usd);
  $<HTMLInputElement>('#cg-hard').value = String(guard.hard_limit_usd);
  $<HTMLInputElement>('#cg-egress').value = String(guard.egress_usd_per_gb);
  $<HTMLInputElement>('#cg-get').value = String(guard.get_usd_per_1000);
}

async function submitCostGuard(): Promise<void> {
  const result = $('#cost-guard-result');
  const num = (id: string): number => Number.parseFloat($<HTMLInputElement>(id).value);
  const guard: CostGuardSettings = {
    enabled: $<HTMLSelectElement>('#cg-enabled').value === 'true',
    soft_limit_usd: num('#cg-soft'),
    hard_limit_usd: num('#cg-hard'),
    egress_usd_per_gb: num('#cg-egress'),
    get_usd_per_1000: num('#cg-get'),
  };
  if (Object.values(guard).some((v) => typeof v === 'number' && !Number.isFinite(v))) {
    result.className = 'form-result change-bad';
    result.textContent = 'Every field needs a number.';
    return;
  }
  try {
    await saveCostGuard(guard);
    result.className = 'form-result change-good';
    result.textContent = 'Saved.';
  } catch (err) {
    result.className = 'form-result change-bad';
    result.textContent = errorMessage(err);
  }
}

function statusCell(s: SourceStatus | undefined): string {
  if (!s || s.state === 'pending') return `<td class="col-left">Registering&hellip;</td>`;
  if (s.state === 'registered') {
    return `<td class="col-left change-good">Registered · ${escapeHtml(s.detected_format ?? '')} · ${escapeHtml(String(s.file_count ?? 0))} files</td>`;
  }
  return `<td class="col-left change-bad" title="${escapeHtml(s.reason ?? '')}">Skipped: ${escapeHtml(s.reason ?? '')}</td>`;
}

function renderTable(statuses: SourceStatus[]): void {
  const container = $('#sources-table');
  const count = $('#status-source-count');
  count.textContent = `${settings.length} source${settings.length === 1 ? '' : 's'}`;
  if (settings.length === 0) {
    container.innerHTML = `<div class="table-status table-empty">No sources yet — click “Add source”.</div>`;
    return;
  }
  const byId = new Map(statuses.map((s) => [s.id, s]));
  const rows = settings
    .map((s) => {
      const id = escapeHtml(s.id);
      const deleteLabel = armedDelete === s.id ? 'Confirm delete' : 'Delete';
      return `
        <tr data-id="${id}">
          <td class="col-left"><strong>${escapeHtml(s.name)}</strong><br /><small>${id}</small></td>
          <td class="col-left">${escapeHtml(s.s3_uri)}</td>
          ${statusCell(byId.get(s.id))}
          <td class="col-left row-actions">
            <button type="button" data-action="edit">Edit</button>
            <button type="button" data-action="reload">Reload</button>
            <button type="button" data-action="delete" class="${armedDelete === s.id ? 'danger' : ''}">${deleteLabel}</button>
          </td>
        </tr>`;
    })
    .join('');
  container.innerHTML = `
    <table class="explorer-table static-table">
      <thead><tr>
        <th class="col-left">Source</th><th class="col-left">Location</th>
        <th class="col-left">Status</th><th class="col-left">Actions</th>
      </tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

async function refresh(): Promise<void> {
  try {
    const [s, statuses] = await Promise.all([getSettings(), getSources()]);
    fillCostGuardForm(s.cost_guard);
    settings = s.sources;
    renderTable(statuses.sources);
    if (statuses.sources.some((x) => x.state === 'pending')) {
      pendingPollAttempts += 1;
      if (pendingPollAttempts < REFRESH_POLL_LIMIT) setTimeout(() => void refresh(), REFRESH_POLL_MS);
    } else {
      pendingPollAttempts = 0;
    }
  } catch (err) {
    $('#sources-table').innerHTML = `<div class="table-status table-error">${escapeHtml(errorMessage(err))}</div>`;
  }
}

// ── form ────────────────────────────────────────────────────────────────────

function readForm(): SourceFormValues {
  return {
    id: $<HTMLInputElement>('#f-id').value,
    name: $<HTMLInputElement>('#f-name').value,
    kind: $<HTMLSelectElement>('#f-kind').value as SourceFormValues['kind'],
    location: $<HTMLInputElement>('#f-location').value,
    region: $<HTMLInputElement>('#f-region').value,
    format: $<HTMLSelectElement>('#f-format').value as SourceFormValues['format'],
    authType: $<HTMLSelectElement>('#f-auth').value as SourceFormValues['authType'],
    profile: $<HTMLInputElement>('#f-profile').value,
    keyId: $<HTMLInputElement>('#f-key-id').value,
  };
}

function writeForm(v: SourceFormValues): void {
  $<HTMLInputElement>('#f-id').value = v.id;
  $<HTMLInputElement>('#f-name').value = v.name;
  $<HTMLSelectElement>('#f-kind').value = v.kind;
  $<HTMLInputElement>('#f-location').value = v.location;
  $<HTMLInputElement>('#f-region').value = v.region;
  $<HTMLSelectElement>('#f-format').value = v.format;
  $<HTMLSelectElement>('#f-auth').value = v.authType;
  $<HTMLInputElement>('#f-profile').value = v.profile;
  $<HTMLInputElement>('#f-key-id').value = v.keyId;
  $<HTMLInputElement>('#f-secret').value = '';
}

function syncVisibility(): void {
  const s3 = $<HTMLSelectElement>('#f-kind').value === 's3';
  const accessKey = $<HTMLSelectElement>('#f-auth').value === 'access_key';
  $('#f-location-label').textContent = s3 ? 'S3 URI' : 'Folder path';
  $<HTMLInputElement>('#f-location').placeholder = s3 ? 's3://my-bucket/exports/my-export/data' : 'C:\\exports\\focus';
  $('#f-browse').hidden = s3 || !isTauri();
  document.querySelectorAll<HTMLElement>('.s3-only').forEach((el) => (el.hidden = !s3));
  document.querySelectorAll<HTMLElement>('.key-only').forEach((el) => (el.hidden = !accessKey));
  document.querySelectorAll<HTMLElement>('.chain-only').forEach((el) => (el.hidden = accessKey));
  $<HTMLInputElement>('#f-secret').placeholder = editing?.has_secret ? '••• stored — leave blank to keep' : '';
}

function setResult(message: string, ok: boolean): void {
  const el = $('#source-test-result');
  el.textContent = message;
  el.className = `form-result ${ok ? 'change-good' : 'change-bad'}`;
}

/**
 * Unlike `setResult` (which writes into `#source-test-result`, inside the
 * add/edit form section and thus invisible whenever that section is
 * `hidden`), this writes into the always-visible `#sources-message` right
 * under the table — used for row-action (reload/delete) errors and the
 * "saved, but skipped" outcome, none of which can rely on the form being
 * open to be seen.
 */
function setPageMessage(message: string, ok: boolean): void {
  const el = $('#sources-message');
  el.textContent = message;
  el.className = `form-result ${ok ? 'change-good' : 'change-bad'}`;
}

function openForm(source: SourceSettings | null): void {
  editing = source;
  $('#source-form-title').textContent = source ? `Edit ${source.name}` : 'Add source';
  writeForm(
    source
      ? sourceToForm(source)
      : { id: '', name: '', kind: 's3', location: '', region: '', format: 'auto', authType: 'credential_chain', profile: '', keyId: '' },
  );
  $<HTMLInputElement>('#f-id').readOnly = source !== null;
  setResult('', true);
  $('#source-form-section').hidden = false;
  syncVisibility();
  $<HTMLInputElement>('#f-name').focus();
}

function closeForm(): void {
  editing = null;
  $('#source-form-section').hidden = true;
}

function secretValue(): string | null {
  const v = $<HTMLInputElement>('#f-secret').value;
  return v === '' ? null : v;
}

async function withBusy(button: HTMLButtonElement, fn: () => Promise<void>): Promise<void> {
  button.disabled = true;
  setLoadingIndicatorVisible(true);
  try {
    await fn();
  } finally {
    button.disabled = false;
    setLoadingIndicatorVisible(false);
  }
}

async function onTest(): Promise<void> {
  setResult('Testing…', true);
  try {
    const r = await testSource(formToSource(readForm()), secretValue());
    if (r.file_count === 0) {
      setResult(
        'No billing data found at this location — point it at the folder containing BILLING_PERIOD=YYYY-MM directories.',
        false,
      );
      return;
    }
    const periods = r.billing_periods.length ? `${r.billing_periods[0]} – ${r.billing_periods[r.billing_periods.length - 1]}` : 'none';
    setResult(`OK: ${r.detected_format}, ${r.file_count} files, billing periods ${periods}`, true);
  } catch (err) {
    setResult(errorMessage(err), false);
  }
}

async function onSave(): Promise<void> {
  try {
    const status = await saveSource(formToSource(readForm()), secretValue(), editing === null);
    closeForm();
    setPageMessage('', true);
    await refresh();
    if (status.state === 'skipped') setPageMessage(`Saved, but the source was skipped: ${status.reason ?? ''}`, false);
  } catch (err) {
    setResult(errorMessage(err), false);
  }
}

async function onRowAction(id: string, action: string): Promise<void> {
  const source = settings.find((s) => s.id === id) ?? null;
  if (action === 'edit') return openForm(source);
  if (action === 'reload') {
    await reloadSource(id);
    setPageMessage('', true);
    return refresh();
  }
  if (action === 'delete') {
    if (armedDelete !== id) {
      armedDelete = id;
      return refresh();
    }
    armedDelete = null;
    try {
      await deleteSource(id);
      if (editing?.id === id) closeForm();
      setPageMessage('', true);
    } finally {
      await refresh();
    }
  }
}

function bind(): void {
  $('#add-source').addEventListener('click', () => openForm(null));
  $('#f-cancel').addEventListener('click', closeForm);
  $('#f-kind').addEventListener('change', syncVisibility);
  $('#f-auth').addEventListener('change', syncVisibility);
  $<HTMLButtonElement>('#f-test').addEventListener('click', (e) => void withBusy(e.currentTarget as HTMLButtonElement, onTest));
  $('#cost-guard-form').addEventListener('submit', (e) => {
    e.preventDefault();
    void submitCostGuard();
  });
  $('#source-form').addEventListener('submit', (e) => {
    e.preventDefault();
    void withBusy($<HTMLButtonElement>('#f-save'), onSave);
  });
  $('#f-browse').addEventListener('click', async () => {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === 'string') $<HTMLInputElement>('#f-location').value = dir;
  });
  $('#sources-table').addEventListener('click', (e) => {
    const button = (e.target as HTMLElement).closest<HTMLButtonElement>('button[data-action]');
    const row = button?.closest<HTMLElement>('tr[data-id]');
    if (!button || !row) return;
    void withBusy(button, () =>
      onRowAction(row.dataset.id!, button.dataset.action!).catch((err) => setPageMessage(errorMessage(err), false)),
    );
  });
}

bind();
setLoadingIndicatorVisible(true);
void refresh().finally(() => setLoadingIndicatorVisible(false));
