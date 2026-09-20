import { describe, expect, it } from 'vitest';
import { changeClass, errorMessage, formatCurrency, formatCurrencyCompact, formatPercent, formatSignedCurrency } from '../format.ts';
import { ApiError } from '../../api.ts';

// `sanitizeCurrencyForFallback` is not exported — it's only reachable via
// `formatCurrency`/`formatCurrencyCompact`'s catch-fallback path, which
// triggers whenever `Intl.NumberFormat` rejects `currency` as malformed
// (e.g. anything that isn't a well-formed 3-letter-ish ISO 4217-shaped
// code). All payloads below are intentionally NOT valid Intl currency
// codes, so they exercise the fallback's `sanitizeCurrencyForFallback` call.
describe('formatCurrency fallback currency sanitization (sanitizeCurrencyForFallback)', () => {
  // Session 7's whole-branch review used this exact payload to prove that
  // an unvalidated `currency` value (sourced straight from cost data,
  // FOCUS 1.2 `BillingCurrency`) could reach the fallback string and be
  // embedded as raw HTML by callers (table cells, chart tooltips).
  it('rejects the Session 7 <img onerror> XSS payload entirely (empty fallback, not the raw payload)', () => {
    const result = formatCurrency(12.5, '<img src=x onerror=alert(1)>');
    expect(result).not.toContain('<');
    expect(result).not.toContain('>');
    expect(result).not.toContain('img');
    expect(result).not.toContain('onerror');
    // Value formats via toFixed(2) and the sanitized (empty) currency is
    // trimmed off the end entirely.
    expect(result).toBe('12.50');
  });

  it('rejects the same payload in the compact formatter fallback', () => {
    const result = formatCurrencyCompact(12.5, '<img src=x onerror=alert(1)>');
    expect(result).not.toContain('<');
    expect(result).not.toContain('>');
    expect(result).toBe('13');
  });

  it('formats legitimate currency codes (USD/EUR/GBP) via Intl without ever needing the fallback', () => {
    // These are well-formed ISO 4217 codes, so Intl.NumberFormat handles
    // them directly — confirming the happy path is unaffected by the
    // sanitizer, which only runs when Intl rejects `currency`.
    expect(formatCurrency(10, 'USD')).toMatch(/10/);
    expect(formatCurrency(10, 'EUR')).toMatch(/10/);
    expect(formatCurrency(10, 'GBP')).toMatch(/10/);
  });

  it('passes a legitimate-shaped code through the fallback unchanged when Intl rejects it', () => {
    // "US1" is alphanumeric and matches the sanitizer's allowlist
    // (/^[A-Za-z0-9 ]{0,12}$/), but Intl.NumberFormat rejects it as a
    // currency code (well-formed currency codes are 3 letters, no digits),
    // so it exercises the fallback path. This pins that legitimate-looking
    // strings survive sanitization unchanged (only actually-dangerous
    // payloads get stripped).
    const result = formatCurrency(1, 'US1');
    expect(result).toBe('1.00 US1');
  });

  it('boundary: a 12-character alphanumeric currency string passes the allowlist and appears in the fallback', () => {
    const currency = 'ABCDEFGHIJKL'; // exactly 12 chars, matches /^[A-Za-z0-9 ]{0,12}$/
    expect(currency.length).toBe(12);
    const result = formatCurrency(1, currency);
    expect(result).toBe(`1.00 ${currency}`);
  });

  it('boundary: a 13-character alphanumeric currency string fails the allowlist and is dropped from the fallback', () => {
    const currency = 'ABCDEFGHIJKLM'; // 13 chars, exceeds the {0,12} limit
    expect(currency.length).toBe(13);
    const result = formatCurrency(1, currency);
    expect(result).not.toContain(currency);
    expect(result).toBe('1.00');
  });
});

describe('formatCurrency / formatCurrencyCompact happy path', () => {
  it('formats a normal positive value with a valid currency code', () => {
    expect(formatCurrency(1234.5, 'USD')).toBe(
      new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD' }).format(1234.5),
    );
  });

  it('formats a compact value with a valid currency code', () => {
    expect(formatCurrencyCompact(1234500, 'USD')).toBe(
      new Intl.NumberFormat(undefined, {
        style: 'currency',
        currency: 'USD',
        notation: 'compact',
        maximumFractionDigits: 1,
      }).format(1234500),
    );
  });

  it('formats zero without throwing', () => {
    expect(formatCurrency(0, 'USD')).toMatch(/0/);
  });
});

describe('formatPercent', () => {
  it('prefixes a positive percentage with +', () => {
    expect(formatPercent(37.2)).toBe('+37.2%');
  });

  it('does not prefix a negative percentage (the minus sign is already there)', () => {
    expect(formatPercent(-12.34)).toBe('-12.3%');
  });

  it('does not prefix zero', () => {
    expect(formatPercent(0)).toBe('0.0%');
  });

  it('rounds to one decimal place', () => {
    expect(formatPercent(10.05)).toBe('+10.1%');
  });
});

describe('formatSignedCurrency', () => {
  it('prefixes a positive value with + and formats the absolute value', () => {
    expect(formatSignedCurrency(50, 'USD')).toBe(`+${formatCurrency(50, 'USD')}`);
  });

  it('prefixes a negative value with - and formats the absolute value (not double-negative)', () => {
    const result = formatSignedCurrency(-50, 'USD');
    expect(result).toBe(`-${formatCurrency(50, 'USD')}`);
    expect(result).not.toContain('--');
  });

  it('does not prefix zero', () => {
    expect(formatSignedCurrency(0, 'USD')).toBe(formatCurrency(0, 'USD'));
  });
});

describe('changeClass', () => {
  it('classifies a positive change (cost increase) as bad', () => {
    expect(changeClass(1)).toBe('change-bad');
  });

  it('classifies a negative change (cost decrease) as good', () => {
    expect(changeClass(-1)).toBe('change-good');
  });

  it('classifies zero change as neutral', () => {
    expect(changeClass(0)).toBe('change-neutral');
  });
});

describe('errorMessage', () => {
  it('extracts the message from a real Error', () => {
    expect(errorMessage(new Error('boom'))).toBe('boom');
  });

  it('extracts the message from an ApiError (which also extends Error)', () => {
    const err = new ApiError(404, { error: 'not found' });
    expect(errorMessage(err)).toBe('not found');
  });

  it('falls back to "Unknown error" for a plain string thrown value', () => {
    expect(errorMessage('some string')).toBe('Unknown error');
  });

  it('falls back to "Unknown error" for an unknown object shape', () => {
    expect(errorMessage({ foo: 'bar' })).toBe('Unknown error');
  });

  it('falls back to "Unknown error" for null/undefined', () => {
    expect(errorMessage(null)).toBe('Unknown error');
    expect(errorMessage(undefined)).toBe('Unknown error');
  });
});
