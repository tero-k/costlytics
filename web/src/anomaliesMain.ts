import './style.css';
import {
  acknowledgeAnomaly, getAnomalies, muteAnomalyScope, runAnomalyCheck,
  type AnomaliesResponse, type Anomaly,
} from './api.ts';
import { initAppShell } from './shared/appShell.ts';
import { refreshAnomalyBadge } from './shared/anomalyBadge.ts';
import {
  checkSummary, dateRangeLabel, detailHref, kindLabel, MUTE_OPTIONS, runStatusText, scopeLabel,
  sortAnomalies, sparklinePaths, visibleAnomalies,
} from './shared/anomalies.ts';
import { errorMessage, formatCurrency, formatSignedCurrency } from './shared/format.ts';
import { escapeHtml } from './shared/html.ts';
import { setLoadingIndicatorVisible } from './shared/statusBar.ts';

initAppShell();

/**
 * Anomalies inbox: every source's detected spend anomalies, newest and most
 * severe first, with Acknowledge / Mute / Open. Detection itself runs in the
 * backend (`crates/service/src/monitor.rs`); this page lists, acts and
 * offers "Check now".
 */

const POLL_MS = 60_000;
const $ = <T extends HTMLElement>(sel: string): T => document.querySelector<T>(sel)!;
let latest: AnomaliesResponse | null = null;

function sparkline(a: Anomaly): string {
  const { actual, expected } = sparklinePaths(a.points, 80, 20);
  return `<svg class="sparkline" viewBox="0 0 80 20" width="80" height="20" aria-hidden="true">
    <path d="${expected}" class="spark-expected"/><path d="${actual}" class="spark-actual"/></svg>`;
}

function row(a: Anomaly, sourceName: string): string {
  const last = a.points[a.points.length - 1];
  const href = detailHref(a);
  const muteOptions = MUTE_OPTIONS.map((d) => `<option value="${d}">${d} days</option>`).join('');
  return `<tr data-id="${escapeHtml(a.id)}" class="severity-${a.severity}${a.status === 'acknowledged' || a.muted ? ' row-dim' : ''}">
    <td><span class="severity-pill severity-${a.severity}">${a.severity}</span>${a.escalated ? ' <span class="escalated-tag" title="Severity rose after it was acknowledged">Escalated</span>' : ''}</td>
    <td>${escapeHtml(scopeLabel(a.scope_kind))}: <strong>${escapeHtml(a.scope_key)}</strong><div class="cell-sub">${escapeHtml(sourceName)}</div></td>
    <td>${kindLabel(a.kind)}</td>
    <td>${escapeHtml(dateRangeLabel(a.first_day, a.last_day))}</td>
    <td>${formatCurrency(last.expected, 'USD')} → ${formatCurrency(last.actual, 'USD')}</td>
    <td class="${a.total_impact >= 0 ? 'change-bad' : 'change-good'}">${formatSignedCurrency(a.total_impact, 'USD')}</td>
    <td>${sparkline(a)}</td>
    <td class="row-actions">
      ${a.status === 'open' ? '<button type="button" data-action="ack">Acknowledge</button>' : '<span class="cell-sub">Acknowledged</span>'}
      ${a.muted ? '<span class="cell-sub">Muted</span>' : `<select data-action="mute" aria-label="Mute"><option value="">Mute…</option>${muteOptions}</select>`}
      ${href ? `<a class="anomaly-open" href="${escapeHtml(href)}">Open</a>` : ''}
    </td>
  </tr>`;
}

function render(data: AnomaliesResponse): void {
  latest = data;
  const names = new Map(data.runs.map((r) => [r.source_id, r.source_name]));
  $('#run-status').innerHTML = data.enabled
    ? data.runs.map((r) => `<li>${escapeHtml(runStatusText(r))}</li>`).join('')
    : '<li>Anomaly detection is turned off in Settings.</li>';
  $<HTMLButtonElement>('#check-now').disabled = !data.enabled;

  const list = visibleAnomalies(sortAnomalies(data.anomalies), $<HTMLInputElement>('#show-all').checked);
  $('#anomaly-list').innerHTML = list.length
    ? `<table class="explorer-table"><thead><tr><th>Severity</th><th>Scope</th><th>Kind</th><th>Days</th>
         <th>Expected → actual (last day)</th><th>Impact</th><th></th><th></th></tr></thead>
       <tbody>${list.map((a) => row(a, names.get(a.source_id) ?? a.source_id)).join('')}</tbody></table>`
    : '<div class="table-status">No open anomalies. Costlytics checks again when new billing data arrives.</div>';
  void refreshAnomalyBadge(data.open_count);
}

async function refresh(): Promise<void> {
  try {
    render(await getAnomalies());
  } catch (err) {
    setMessage(errorMessage(err), false);
  }
}

function setMessage(text: string, ok: boolean): void {
  const el = $('#anomaly-message');
  el.className = `form-result ${ok ? 'change-good' : 'change-bad'}`;
  el.textContent = text;
}

/** Same dialog styling as the page cost guard; resolves true on Continue. */
function confirmScan(usd: number): Promise<boolean> {
  const dialog = document.createElement('dialog');
  dialog.className = 'cost-guard-dialog';
  dialog.innerHTML = `<p>Checking for anomalies would read about <strong>${formatCurrency(usd, 'USD')}</strong> of data from S3.</p>
    <div class="cost-guard-dialog-actions"><button type="button" class="button-primary" data-answer="yes">Continue</button>
    <button type="button" data-answer="no">Cancel</button></div>`;
  document.body.append(dialog);
  dialog.showModal();
  return new Promise((resolve) => {
    // Escape closes a modal dialog natively; treat that as Cancel.
    dialog.addEventListener('close', () => {
      dialog.remove();
      resolve(false);
    });
    dialog.addEventListener('click', (e) => {
      const answer = (e.target as HTMLElement).closest<HTMLButtonElement>('button[data-answer]')?.dataset.answer;
      if (!answer) return;
      resolve(answer === 'yes');
      dialog.close();
    });
  });
}

async function onCheckNow(): Promise<void> {
  const button = $<HTMLButtonElement>('#check-now');
  button.disabled = true;
  setLoadingIndicatorVisible(true);
  try {
    let results = await runAnomalyCheck(null, false);
    for (const r of results.filter((x) => x.kind === 'needs_confirmation')) {
      if (r.kind === 'needs_confirmation' && (await confirmScan(r.estimate_usd))) {
        results = results.filter((x) => x.source_id !== r.source_id).concat(await runAnomalyCheck(r.source_id, true));
      }
    }
    setMessage(checkSummary(results), !results.some((r) => r.kind === 'error'));
    await refresh();
  } catch (err) {
    setMessage(errorMessage(err), false);
  } finally {
    button.disabled = !(latest?.enabled ?? true);
    setLoadingIndicatorVisible(false);
  }
}

function bind(): void {
  $('#check-now').addEventListener('click', () => void onCheckNow());
  $('#show-all').addEventListener('change', () => latest && render(latest));
  const list = $('#anomaly-list');
  list.addEventListener('click', (e) => {
    const button = (e.target as HTMLElement).closest<HTMLButtonElement>('button[data-action="ack"]');
    const id = button?.closest<HTMLElement>('tr[data-id]')?.dataset.id;
    if (!button || !id) return;
    button.disabled = true;
    acknowledgeAnomaly(id).then(refresh, (err) => setMessage(errorMessage(err), false));
  });
  list.addEventListener('change', (e) => {
    const select = (e.target as HTMLElement).closest<HTMLSelectElement>('select[data-action="mute"]');
    const id = select?.closest<HTMLElement>('tr[data-id]')?.dataset.id;
    const a = latest?.anomalies.find((x) => x.id === id);
    if (!select || !a || !select.value) return;
    muteAnomalyScope(a.source_id, a.scope_kind, a.scope_key, Number(select.value)).then(render, (err) => setMessage(errorMessage(err), false));
  });
}

bind();
setLoadingIndicatorVisible(true);
void refresh().finally(() => setLoadingIndicatorVisible(false));
window.setInterval(() => void refresh(), POLL_MS);
