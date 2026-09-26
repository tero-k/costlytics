import { afterEach, describe, expect, it } from 'vitest';
import { mergeFilters, setPageFilterProvider, withPageFiltersCompare } from '../../api.ts';
import { resolveSelection, summarize } from '../pageFilters.ts';
import { createMultiSelect, filterOptions } from '../multiSelect.ts';

describe('mergeFilters', () => {
  it('fills empty fields and never overrides a field the caller set', () => {
    const merged = mergeFilters(
      { start: 'a', services: ['EC2'], accounts: [] },
      { services: ['S3'], accounts: ['111'], regions: [] },
    );
    expect(merged).toEqual({ start: 'a', services: ['EC2'], accounts: ['111'] });
  });
});

describe('withPageFiltersCompare', () => {
  afterEach(() => setPageFilterProvider(null));

  it('applies page filters to both periods', () => {
    setPageFilterProvider(() => ({ accounts: ['111'] }));
    const req = withPageFiltersCompare({
      current_start: 'a',
      current_end: 'b',
      previous_start: 'c',
      previous_end: 'd',
      current: { services: ['EC2'] },
    });
    expect(req.current).toEqual({ services: ['EC2'], accounts: ['111'] });
    expect(req.previous).toEqual({ accounts: ['111'] });
  });
});

describe('resolveSelection', () => {
  it('takes the whole selection from the URL when any f_* param is present', () => {
    const params = new URLSearchParams('f_service=EC2&f_service=S3');
    expect(resolveSelection(['services', 'accounts'], params, { accounts: ['111'] })).toEqual({
      services: ['EC2', 'S3'],
      accounts: [],
    });
  });

  it('falls back to stored values, ignoring junk', () => {
    const stored = { services: ['EC2', 3 as unknown as string, ''], accounts: 'x' as unknown as string[] };
    expect(resolveSelection(['services', 'accounts'], new URLSearchParams(), stored)).toEqual({
      services: ['EC2'],
      accounts: [],
    });
  });
});

describe('summarize', () => {
  it('pluralizes and omits empty filters', () => {
    expect(summarize({ services: ['a', 'b'], accounts: ['c'] })).toBe('Filtered: 2 services, 1 account');
    expect(summarize({ services: [], accounts: [] })).toBe('');
  });
});

describe('multiSelect', () => {
  it('filters options case-insensitively', () => {
    expect(filterOptions(['Amazon EC2', 'Amazon S3', 'Lambda'], 'ec')).toEqual(['Amazon EC2']);
    expect(filterOptions(['a', 'b'], '  ')).toEqual(['a', 'b']);
  });

  it('stages picks while open and commits one change on apply', () => {
    const ms = createMultiSelect({ id: 'f', label: 'Service', plural: 'services' });
    document.body.append(ms.element);
    ms.setOptions(['EC2', 'S3', 'RDS']);
    let changes = 0;
    document.body.addEventListener('change', () => changes++);

    ms.element.querySelector<HTMLElement>('.ms-trigger')!.click();
    for (const value of ['EC2', 'RDS']) {
      const box = ms.element.querySelector<HTMLInputElement>(`input[value="${value}"]`)!;
      box.checked = true;
      box.dispatchEvent(new Event('change', { bubbles: true }));
    }
    expect(changes).toBe(0);
    ms.element.querySelector<HTMLButtonElement>('.ms-apply')!.click();
    expect(changes).toBe(1);
    expect(ms.getSelected()).toEqual(['EC2', 'RDS']);

    ms.element.querySelector<HTMLButtonElement>('.ms-chip-remove[data-value="EC2"]')!.click();
    expect(changes).toBe(2);
    expect(ms.getSelected()).toEqual(['RDS']);

    expect(ms.setOptions(['EC2'])).toBe(true);
    expect(ms.getSelected()).toEqual([]);
    ms.element.remove();
  });
});
