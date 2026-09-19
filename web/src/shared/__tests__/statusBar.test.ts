import { beforeEach, describe, expect, it } from 'vitest';
import { updateStatusCurrency } from '../statusBar.ts';

describe('updateStatusCurrency', () => {
  beforeEach(() => {
    document.body.innerHTML = '<span id="status-currency">Currency: —</span>';
  });

  it('updates the displayed text to reflect the given currency', () => {
    updateStatusCurrency('USD');
    const el = document.querySelector('#status-currency');
    expect(el?.textContent).toBe('Currency: USD');
  });

  it('does not change the displayed text when given an empty string (Session 9 regression)', () => {
    // Reproduces the Session 9 bug scenario: a zero-row query result whose
    // `currency` field comes back as "" must not blank out whatever
    // currency was last shown.
    const el = document.querySelector('#status-currency');
    expect(el).not.toBeNull();
    const before = el!.textContent;

    updateStatusCurrency('');

    expect(el!.textContent).toBe(before);
  });

  it('does not change the displayed text for other falsy values (undefined)', () => {
    const el = document.querySelector('#status-currency');
    const before = el!.textContent;

    // The implementation's guard is a plain `if (!currency) return;`, so
    // any falsy value — not just "" — must be ignored. TypeScript's
    // declared parameter type is `string`, so we simulate what actually
    // reaches the function at runtime (e.g. from an API response missing
    // the field) via an explicit cast rather than assuming.
    updateStatusCurrency(undefined as unknown as string);

    expect(el!.textContent).toBe(before);
  });

  it('does nothing (does not throw) when the status-currency element is missing', () => {
    document.body.innerHTML = '';
    expect(() => updateStatusCurrency('USD')).not.toThrow();
  });
});
