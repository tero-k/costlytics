/**
 * HTML-escaping helper. Required anywhere a dynamic string (ultimately
 * traceable back to `breakdown()`'s/`compare()`'s `key` field — i.e.
 * attacker-controlled cost-data values such as service/account/resource/tag
 * names) is interpolated into an HTML string handed to ECharts, since
 * ECharts' default `tooltip.renderMode: 'html'` injects a `formatter`'s
 * returned string as raw HTML rather than escaping it (stored-XSS risk).
 */
export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}
