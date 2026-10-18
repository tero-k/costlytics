/**
 * Pure helpers for the Anomalies page, the sidebar badge and the Settings
 * rules form. No DOM, no fetching.
 */

import type { Anomaly, AnomalyKind, AnomalyPoint, ScopeKind, Severity, SourceRun } from '../api.ts';
import { addDaysIso } from './dates.ts';
import { formatCurrency } from './format.ts';

export const MUTE_OPTIONS = [7, 30, 90] as const;
/** Days of context shown before an anomaly on the detail page. */
const DETAIL_LEAD_DAYS = 14;
const NONE_KEY = '(none)';
const SEVERITY_RANK: Record<Severity, number> = { low: 0, medium: 1, high: 2 };

export function sortAnomalies(list: Anomaly[]): Anomaly[] {
  return [...list].sort(
    (a, b) => SEVERITY_RANK[b.severity] - SEVERITY_RANK[a.severity] || b.last_day.localeCompare(a.last_day),
  );
}

export function visibleAnomalies(list: Anomaly[], showAll: boolean): Anomaly[] {
  return showAll ? list : list.filter((a) => a.status === 'open' && !a.muted);
}

export function badgeText(count: number): string {
  if (count <= 0) return '';
  return count > 99 ? '99+' : String(count);
}

export function kindLabel(kind: AnomalyKind): string {
  return kind === 'new_spend' ? 'New spend' : kind === 'drop' ? 'Drop' : 'Spike';
}

export function scopeLabel(kind: ScopeKind): string {
  return kind === 'service' ? 'Service' : 'Account';
}

export function dateRangeLabel(first: string, last: string): string {
  return first === last ? first : `${first} – ${last}`;
}

/** The Service/Account Detail page for the anomaly's scope and dates, or `null` when it can't filter (NULL key). */
export function detailHref(a: Anomaly): string | null {
  if (a.scope_key === NONE_KEY) return null;
  const page = a.scope_kind === 'service' ? '/service-detail.html' : '/account-detail.html';
  const params = new URLSearchParams({
    [a.scope_kind]: a.scope_key,
    from: addDaysIso(a.first_day, -DETAIL_LEAD_DAYS),
    to: a.last_day,
    source: a.source_id,
  });
  return `${page}?${params.toString()}`;
}

/** SVG path data for actual and expected cost across the points, scaled into `width × height`. */
export function sparklinePaths(points: AnomalyPoint[], width: number, height: number): { actual: string; expected: string } {
  if (points.length === 0) return { actual: '', expected: '' };
  const max = Math.max(...points.flatMap((p) => [p.actual, p.expected]), 0) || 1;
  const step = points.length > 1 ? width / (points.length - 1) : 0;
  const path = (value: (p: AnomalyPoint) => number): string =>
    points
      .map((p, i) => `${i === 0 ? 'M' : 'L'}${(i * step).toFixed(1)} ${(height - (value(p) / max) * height).toFixed(1)}`)
      .join(' ');
  return { actual: path((p) => p.actual), expected: path((p) => p.expected) };
}

export function runStatusText(r: SourceRun): string {
  if (!r.run) return `${r.source_name}: not checked yet`;
  const o = r.run.outcome;
  switch (o.kind) {
    case 'ok':
      return r.run.last_checked_day
        ? `${r.source_name}: checked through ${r.run.last_checked_day}`
        : `${r.source_name}: checked, no settled data yet`;
    case 'skipped_cost_guard':
      return `${r.source_name}: automatic check skipped — scanning would cost about ${formatCurrency(o.estimate_usd, 'USD')}`;
    case 'error':
      return `${r.source_name}: last check failed — ${o.message}`;
  }
}

export function pctToFraction(pct: number): number {
  return Math.round(pct * 1e6) / 1e8;
}

export function fractionToPct(fraction: number): number {
  return Math.round(fraction * 1e8) / 1e6;
}
