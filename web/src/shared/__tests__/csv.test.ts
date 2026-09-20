import { describe, expect, it } from 'vitest';
import { csvField, toCsv } from '../csv.ts';

// Session 7's whole-branch review named these exact formula-trigger
// characters (`=`, `+`, `-`, `@`, tab, carriage return) as the CSV/formula
// injection vector: RFC 4180 quoting alone does NOT neutralize them, since
// spreadsheet apps strip the surrounding quotes before evaluating a
// leading formula-trigger character. `csvField` must prefix a neutralizing
// `'` before RFC 4180 quoting is applied.
describe('csvField formula-injection guard', () => {
  it.each([
    ['=', '=SUM(A1:A9)'],
    ['+', '+1+1'],
    ['-', '-1+1'],
    ['@', '@SUM(A1:A9)'],
    ['tab', '\tevil'],
  ])('prefixes a field starting with %s with a neutralizing quote', (_label, value) => {
    const result = csvField(value);
    // None of these payloads contain a comma/quote/newline themselves, so
    // no RFC 4180 quoting kicks in — just the neutralizing prefix.
    expect(result).toBe(`'${value}`);
  });

  it('prefixes a field starting with a carriage return with a neutralizing quote (and RFC-4180-quotes it too, since \\r is also a quoting trigger)', () => {
    const result = csvField('\revil');
    expect(result).toBe('"\'\revil"');
  });

  it('does not prefix a normal dimension-name field', () => {
    expect(csvField('Amazon EC2')).toBe('Amazon EC2');
  });

  it('does not prefix a normal currency-amount field', () => {
    expect(csvField('$1,034.56')).toBe('"$1,034.56"');
  });

  it('RFC-4180-quotes a field containing a comma without adding the injection prefix', () => {
    const result = csvField('a,b');
    expect(result).toBe('"a,b"');
    expect(result.startsWith("'")).toBe(false);
  });

  it('RFC-4180-quotes a field containing a double quote, doubling the inner quote', () => {
    expect(csvField('say "hi"')).toBe('"say ""hi"""');
  });

  it('RFC-4180-quotes a field containing a newline', () => {
    expect(csvField('line1\nline2')).toBe('"line1\nline2"');
  });

  it('applies BOTH the injection prefix and RFC 4180 quoting when a field starts with a trigger char and also contains a comma', () => {
    const result = csvField('=SUM(A1),B2');
    // Neutralized first: "'=SUM(A1),B2" — then that whole thing contains a
    // comma, so it gets RFC 4180 quoted too.
    expect(result).toBe('"\'=SUM(A1),B2"');
  });

  it('does not treat a mid-field trigger character as a leading one', () => {
    expect(csvField('a=b')).toBe('a=b');
  });
});

describe('toCsv', () => {
  it('joins rows with CRLF line endings and comma-separated fields, applying csvField quoting per cell', () => {
    const rows = [
      ['Service', 'Amount'],
      ['Amazon EC2', '$1,034.56'],
      ['=SUM(A1)', '-5'],
    ];
    const result = toCsv(rows);
    // Row 3's fields each get the injection prefix but, unlike the "BOTH"
    // test above, neither field contains a comma/quote/newline ITSELF
    // (the comma between them is just the CSV delimiter `toCsv` adds), so
    // neither is RFC-4180-quoted.
    expect(result).toBe('Service,Amount\r\nAmazon EC2,"$1,034.56"\r\n\'=SUM(A1),\'-5');
  });
});
