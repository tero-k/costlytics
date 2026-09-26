// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EstimateResponse } from '../../api.ts';

const estimateCost = vi.fn<(...args: unknown[]) => Promise<EstimateResponse>>();
vi.mock('../../api.ts', () => ({ estimateCost: (...args: unknown[]) => estimateCost(...args) }));

const { buildScans, decide, describeEstimate, formatBytes, installCostGuard } = await import('../costGuard.ts');

function estimate(overrides: Partial<EstimateResponse> = {}): EstimateResponse {
  return {
    remote: true,
    known: true,
    bytes: 4_200_000_000,
    requests: 1200,
    usd: 0.38,
    tier: 'none',
    stats_missing: false,
    soft_limit_usd: 0.1,
    hard_limit_usd: 1,
    ...overrides,
  };
}

describe('buildScans', () => {
  it('uses exclusive ends and one entry per query', () => {
    const scans = buildScans({ startIso: '2026-08-01', endIsoInclusive: '2026-08-31' }, { current: 2, compare: 1 });
    const current = { start: '2026-08-01', end: '2026-09-01' };
    expect(scans).toEqual([[current], [current], [current, { start: '2026-07-01', end: '2026-08-01' }]]);
  });

  it('prefers an explicit previous period', () => {
    const scans = buildScans(
      {
        startIso: '2026-08-01',
        endIsoInclusive: '2026-08-31',
        previousStartIso: '2025-08-01',
        previousEndIsoInclusive: '2025-08-31',
      },
      { current: 0, compare: 1 },
    );
    expect(scans[0][1]).toEqual({ start: '2025-08-01', end: '2025-09-01' });
  });
});

describe('decide', () => {
  it('never alerts for local or unknown sources', () => {
    expect(decide(estimate({ remote: false, tier: 'hard' }), false).action).toBe('proceed');
    expect(decide(estimate({ known: false, tier: 'hard' }), false).action).toBe('proceed');
  });

  it('maps tiers, and a confirmed hard selection only gets the banner', () => {
    expect(decide(estimate({ tier: 'none' }), false).action).toBe('proceed');
    expect(decide(estimate({ tier: 'soft' }), false).action).toBe('banner');
    expect(decide(estimate({ tier: 'hard' }), false).action).toBe('confirm');
    expect(decide(estimate({ tier: 'hard' }), true).action).toBe('banner');
  });
});

describe('formatting', () => {
  it('formats bytes and marks upper bounds', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(4_200_000_000)).toBe('4.2 GB');
    expect(describeEstimate(estimate())).toContain('~4.2 GB');
    expect(describeEstimate(estimate({ stats_missing: true }))).toContain('up to 4.2 GB');
  });
});

describe('installCostGuard', () => {
  let downstream: ReturnType<typeof vi.fn<(event: Event) => void>>;
  let uninstall: () => void;

  afterEach(() => uninstall());

  beforeEach(() => {
    document.body.innerHTML = `
      <div id="status-bar"></div>
      <input id="date-start" value="2026-08-01" />
      <input id="date-end" value="2026-08-31" />`;
    // jsdom lacks the modal dialog API.
    HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) {
      this.open = false;
      this.dispatchEvent(new Event('close'));
    };
    try {
      sessionStorage.clear();
    } catch {
      // ignore
    }
    estimateCost.mockReset();
    downstream = vi.fn<(event: Event) => void>();
    document.querySelector('#date-start')!.addEventListener('change', downstream);
    uninstall = installCostGuard({ current: 1, compare: 0 });
  });

  async function changeStart(value: string): Promise<void> {
    const input = document.querySelector<HTMLInputElement>('#date-start')!;
    input.value = value;
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(estimateCost).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
  }

  it('lets a cheap change through after the estimate', async () => {
    estimateCost.mockResolvedValue(estimate({ tier: 'none' }));
    await changeStart('2026-07-01');
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(1));
    expect(document.querySelector('#cost-guard-banner')).toBeNull();
  });

  it('shows the banner for a soft-tier change and still loads', async () => {
    estimateCost.mockResolvedValue(estimate({ tier: 'soft' }));
    await changeStart('2026-01-01');
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(1));
    expect(document.querySelector('#cost-guard-banner')?.textContent).toContain('4.2 GB');
  });

  it('holds a hard-tier change and Cancel restores the previous value', async () => {
    estimateCost.mockResolvedValue(estimate({ tier: 'hard', usd: 3.5 }));
    await changeStart('2022-01-01');
    const cancel = await vi.waitFor(() => {
      const button = document.querySelector<HTMLButtonElement>('.cost-guard-dialog button[value="cancel"]');
      expect(button).not.toBeNull();
      return button!;
    });
    cancel.click();
    await new Promise((r) => setTimeout(r, 0));
    expect(downstream).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLInputElement>('#date-start')!.value).toBe('2026-08-01');
  });

  it('Continue loads, and the same selection is not asked again this session', async () => {
    estimateCost.mockResolvedValue(estimate({ tier: 'hard', usd: 3.5 }));
    await changeStart('2022-01-01');
    const cont = await vi.waitFor(() => {
      const button = document.querySelector<HTMLButtonElement>('.cost-guard-dialog button[value="continue"]');
      expect(button).not.toBeNull();
      return button!;
    });
    cont.click();
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(1));

    // Move away (cheap), then back to the confirmed selection: no dialog.
    estimateCost.mockResolvedValue(estimate({ tier: 'none' }));
    estimateCost.mockClear();
    await changeStart('2026-08-01');
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(2));
    estimateCost.mockResolvedValue(estimate({ tier: 'hard', usd: 3.5 }));
    estimateCost.mockClear();
    await changeStart('2022-01-01');
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(3));
    expect(document.querySelector('.cost-guard-dialog')).toBeNull();
  });

  it('a duplicate change after Cancel does not ask again', async () => {
    estimateCost.mockResolvedValue(estimate({ tier: 'hard', usd: 3.5 }));
    const input = document.querySelector<HTMLInputElement>('#date-start')!;
    input.value = '2022-01-01';
    // Some inputs (and test drivers) fire `change` twice for one edit.
    input.dispatchEvent(new Event('change', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
    const cancel = await vi.waitFor(() => {
      const button = document.querySelector<HTMLButtonElement>('.cost-guard-dialog button[value="cancel"]');
      expect(button).not.toBeNull();
      return button!;
    });
    cancel.click();
    await new Promise((r) => setTimeout(r, 10));
    expect(document.querySelector('.cost-guard-dialog')).toBeNull();
    expect(estimateCost).toHaveBeenCalledTimes(1);
    expect(downstream).not.toHaveBeenCalled();
  });

  it('fails open when the estimate call errors', async () => {
    estimateCost.mockRejectedValue(new Error('backend down'));
    await changeStart('2026-07-01');
    await vi.waitFor(() => expect(downstream).toHaveBeenCalledTimes(1));
  });
});
