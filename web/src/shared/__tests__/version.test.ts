import { describe, expect, it } from 'vitest';
import { appVersionLabel, formatVersionLabel } from '../version.ts';

describe('formatVersionLabel', () => {
  it('formats a release build', () => {
    expect(formatVersionLabel('0.2.0', 'v0.2.0', false)).toEqual({ label: 'v0.2.0', tooltip: 'Build v0.2.0' });
  });

  it('marks dev builds', () => {
    expect(formatVersionLabel('0.2.0', 'v0.2.0-3-gef66206-dirty', true)).toEqual({
      label: 'v0.2.0-dev',
      tooltip: 'Build v0.2.0-3-gef66206-dirty',
    });
  });

  it('falls back when the build id is empty', () => {
    expect(formatVersionLabel('0.2.0', '', false).tooltip).toBe('Build unknown');
  });
});

describe('appVersionLabel', () => {
  it('does not throw when the build-time constants are not defined', () => {
    expect(appVersionLabel().label).toMatch(/^v\d+\.\d+\.\d+/);
  });
});
