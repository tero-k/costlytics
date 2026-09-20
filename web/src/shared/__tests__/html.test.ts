import { describe, expect, it } from 'vitest';
import { escapeHtml } from '../html.ts';

describe('escapeHtml', () => {
  // Session 6's whole-branch review used this exact payload to prove the
  // original stored-XSS bug (attacker-controlled cost-data `key` values,
  // e.g. a resource/tag name, interpolated unescaped into ECharts'
  // `tooltip.renderMode: 'html'` formatter output) and its fix.
  it('neutralizes the Session 6 <img onerror> XSS payload', () => {
    const payload = '<img src=x onerror=alert(1)>';
    const escaped = escapeHtml(payload);
    expect(escaped).not.toContain('<');
    expect(escaped).not.toContain('>');
    expect(escaped).toBe('&lt;img src=x onerror=alert(1)&gt;');
  });

  it('neutralizes a <script> tag payload', () => {
    const payload = '<script>alert(1)</script>';
    const escaped = escapeHtml(payload);
    expect(escaped).not.toContain('<script>');
    expect(escaped).not.toMatch(/[<>]/);
    expect(escaped).toBe('&lt;script&gt;alert(1)&lt;/script&gt;');
  });

  it('neutralizes an attribute-breaking double-quote payload', () => {
    const payload = '" onmouseover="alert(1)';
    const escaped = escapeHtml(payload);
    expect(escaped).not.toContain('"');
    expect(escaped).toBe('&quot; onmouseover=&quot;alert(1)');
  });

  it('leaves a benign alphanumeric string unchanged', () => {
    expect(escapeHtml('EC2')).toBe('EC2');
    expect(escapeHtml('us-east-1')).toBe('us-east-1');
  });
});
