import { describe, expect, it } from 'vitest';
import type { Anomaly, SourceRun } from '../../api.ts';
import {
  badgeText, dateRangeLabel, detailHref, fractionToPct, kindLabel, MUTE_OPTIONS, pctToFraction,
  runStatusText, sortAnomalies, sparklinePaths, visibleAnomalies,
} from '../anomalies.ts';

function anomaly(over: Partial<Anomaly>): Anomaly {
  return {
    id: 'a', source_id: 'src', scope_kind: 'service', scope_key: 'Amazon EC2', kind: 'spike', direction: 'up',
    first_day: '2026-09-25', last_day: '2026-09-27',
    points: [
      { day: '2026-09-25', expected: 120, actual: 420, severity: 'high' },
      { day: '2026-09-26', expected: 80, actual: 420, severity: 'high' },
    ],
    total_impact: 640, severity: 'high', status: 'open', escalated: false, muted: false,
    detected_at: '2026-10-01T00:00:00Z', updated_at: '2026-10-01T00:00:00Z',
    ...over,
  };
}

describe('anomaly list helpers', () => {
  it('sorts by severity, then most recent', () => {
    const list = [
      anomaly({ id: 'low-new', severity: 'low', last_day: '2026-09-30' }),
      anomaly({ id: 'high-old', severity: 'high', last_day: '2026-09-01' }),
      anomaly({ id: 'high-new', severity: 'high', last_day: '2026-09-20' }),
    ];
    expect(sortAnomalies(list).map((a) => a.id)).toEqual(['high-new', 'high-old', 'low-new']);
  });

  it('hides acknowledged and muted unless showing all', () => {
    const list = [anomaly({ id: 'open' }), anomaly({ id: 'ack', status: 'acknowledged' }), anomaly({ id: 'muted', muted: true })];
    expect(visibleAnomalies(list, false).map((a) => a.id)).toEqual(['open']);
    expect(visibleAnomalies(list, true)).toHaveLength(3);
  });

  it('formats the badge', () => {
    expect(badgeText(0)).toBe('');
    expect(badgeText(7)).toBe('7');
    expect(badgeText(150)).toBe('99+');
  });

  it('labels kinds and ranges', () => {
    expect(kindLabel('new_spend')).toBe('New spend');
    expect(kindLabel('drop')).toBe('Drop');
    expect(dateRangeLabel('2026-09-25', '2026-09-25')).toBe('2026-09-25');
    expect(dateRangeLabel('2026-09-25', '2026-09-27')).toBe('2026-09-25 – 2026-09-27');
    expect(MUTE_OPTIONS).toEqual([7, 30, 90]);
  });

  it('links to the detail page with context, except for (none)', () => {
    expect(detailHref(anomaly({}))).toBe(
      '/service-detail.html?service=Amazon+EC2&from=2026-09-11&to=2026-09-27&source=src',
    );
    expect(detailHref(anomaly({ scope_kind: 'account', scope_key: '111' }))).toBe(
      '/account-detail.html?account=111&from=2026-09-11&to=2026-09-27&source=src',
    );
    expect(detailHref(anomaly({ scope_key: '(none)' }))).toBeNull();
  });

  it('draws sparkline paths inside the box', () => {
    const { actual, expected } = sparklinePaths(anomaly({}).points, 100, 20);
    expect(actual).toMatch(/^M0(\.0)? /);
    expect(expected.startsWith('M')).toBe(true);
    for (const n of `${actual} ${expected}`.match(/-?\d+(\.\d+)?/g)!.map(Number)) {
      expect(n).toBeGreaterThanOrEqual(0);
      expect(n).toBeLessThanOrEqual(100);
    }
    expect(sparklinePaths([], 100, 20)).toEqual({ actual: '', expected: '' });
  });

  it('describes run status', () => {
    const base: SourceRun = { source_id: 's', source_name: 'Prod', run: null };
    expect(runStatusText(base)).toBe('Prod: not checked yet');
    const ok = { fingerprint: 'f', last_checked_day: '2026-09-28', last_run_at: '2026-10-01T10:00:00Z', estimate_usd: null };
    expect(runStatusText({ ...base, run: { ...ok, outcome: { kind: 'ok' } } })).toBe('Prod: checked through 2026-09-28');
    expect(runStatusText({ ...base, run: { ...ok, outcome: { kind: 'skipped_cost_guard', estimate_usd: 3.2 } } })).toBe(
      'Prod: automatic check skipped — scanning would cost about $3.20',
    );
    expect(runStatusText({ ...base, run: { ...ok, outcome: { kind: 'error', message: 'boom' } } })).toBe('Prod: last check failed — boom');
  });

  it('converts percentages', () => {
    expect(pctToFraction(25)).toBe(0.25);
    expect(fractionToPct(0.25)).toBe(25);
    expect(fractionToPct(0.125)).toBe(12.5);
  });
});
