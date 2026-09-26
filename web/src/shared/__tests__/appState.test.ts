import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultSharedState, getStoredSourceId, initSharedControls, persistSourceId, resolveSharedState } from '../appState.ts';

const TODAY = new Date(2026, 8, 26); // Sep 26 2026, local

describe('resolveSharedState', () => {
  it('defaults to month to date, amortized', () => {
    expect(resolveSharedState(new URLSearchParams(), {}, TODAY)).toEqual({
      startIso: '2026-09-01',
      endIso: '2026-09-26',
      metric: 'amortized',
    });
  });

  it('prefers URL params over stored values, per field', () => {
    const params = new URLSearchParams('from=2026-01-01&metric=billed');
    const stored = { startIso: '2025-06-01', endIso: '2026-03-31', metric: 'list' as const };
    expect(resolveSharedState(params, stored, TODAY)).toEqual({
      startIso: '2026-01-01',
      endIso: '2026-03-31',
      metric: 'billed',
    });
  });

  it('falls back past invalid values', () => {
    const params = new URLSearchParams('from=yesterday&metric=bogus');
    const stored = { startIso: '2026-02-30x', metric: 'contracted' as const };
    const state = resolveSharedState(params, stored, TODAY);
    expect(state.startIso).toBe(defaultSharedState(TODAY).startIso);
    expect(state.metric).toBe('contracted');
  });

  it('resets both dates when the mixed range would be inverted', () => {
    const params = new URLSearchParams('from=2026-12-01');
    const stored = { endIso: '2026-01-31' };
    const state = resolveSharedState(params, stored, TODAY);
    expect([state.startIso, state.endIso]).toEqual(['2026-09-01', '2026-09-26']);
  });
});

describe('initSharedControls', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    localStorage.clear();
    document.body.innerHTML = '';
    window.history.replaceState(null, '', '/');
  });

  function mountControls(): void {
    document.body.innerHTML = `
      <input type="date" id="date-start" /><input type="date" id="date-end" />
      <select id="metric-select"><option value="amortized">A</option><option value="billed">B</option></select>`;
  }

  it('seeds inputs from storage, mirrors them into the URL and persists changes', () => {
    localStorage.setItem('costlytics.shared.v1', JSON.stringify({ startIso: '2026-05-01', endIso: '2026-05-31', metric: 'billed' }));
    mountControls();
    initSharedControls(TODAY);

    const start = document.querySelector<HTMLInputElement>('#date-start')!;
    expect(start.value).toBe('2026-05-01');
    expect(document.querySelector<HTMLSelectElement>('#metric-select')!.value).toBe('billed');
    expect(new URL(window.location.href).searchParams.get('to')).toBe('2026-05-31');

    start.value = '2026-04-01';
    start.dispatchEvent(new Event('change'));
    expect(JSON.parse(localStorage.getItem('costlytics.shared.v1')!).startIso).toBe('2026-04-01');
    expect(new URL(window.location.href).searchParams.get('from')).toBe('2026-04-01');
  });

  it('still works when storage throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    mountControls();
    expect(initSharedControls(TODAY)?.startIso).toBe('2026-09-01');
    persistSourceId('aws');
    expect(getStoredSourceId()).toBeNull();
  });
});
