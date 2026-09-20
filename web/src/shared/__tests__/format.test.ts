import { describe, expect, it } from 'vitest';
import { formatCurrency, formatCurrencyCompact } from '../format.ts';

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
