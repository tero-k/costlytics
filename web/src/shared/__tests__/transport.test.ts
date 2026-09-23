import { describe, expect, it } from 'vitest';
import { argsFromPath, commandName, statusForKind } from '../../transport.ts';

describe('commandName', () => {
  it.each([
    ['/api/v1/cost/summary', 'cost_summary'],
    ['/api/v1/filter-values/tag-values?key=Team', 'filter_values_tag_values'],
    ['/api/v1/filter-values/services?source_id=a', 'filter_values_services'],
    ['/api/v1/sources', 'sources'],
    ['/api/v1/settings', 'settings'],
    ['/api/v1/settings/source-save', 'settings_source_save'],
  ])('%s -> %s', (path, expected) => {
    expect(commandName(path)).toBe(expected);
  });
});

describe('argsFromPath', () => {
  it('turns the query string into an object', () => {
    expect(argsFromPath('/api/v1/filter-values/tag-values?key=a%20b&source_id=s1')).toEqual({
      key: 'a b',
      source_id: 's1',
    });
  });
  it('is empty without a query string', () => {
    expect(argsFromPath('/api/v1/sources')).toEqual({});
  });
});

describe('statusForKind', () => {
  it('mirrors the HTTP harness mapping', () => {
    expect(statusForKind('bad_request')).toBe(400);
    expect(statusForKind('not_found')).toBe(404);
    expect(statusForKind('conflict')).toBe(409);
    expect(statusForKind('internal')).toBe(500);
    expect(statusForKind(undefined)).toBe(500);
  });
});
