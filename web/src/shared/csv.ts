/**
 * CSV field quoting/escaping and browser download helpers, shared wherever
 * the app needs to export tabular data as a downloadable `.csv` file (Cost
 * Explorer's `#csv-export-btn`, `explorerMain.ts`).
 *
 * This is a DIFFERENT escaping concern from `shared/html.ts`'s
 * `escapeHtml`: CSV has its own quoting rules (RFC 4180) — a field must be
 * wrapped in double quotes if it contains a comma, a double quote, or a
 * newline, and any double quote inside it must be doubled. Reusing the HTML
 * escaper here would be both wrong (it doesn't handle commas/newlines) and
 * misleading (it'd HTML-entity-encode characters that don't need it in a
 * CSV context, e.g. turning a literal `"` into `&quot;` instead of `""`).
 */

/** Quotes a single CSV field per RFC 4180 if it contains a comma, quote, or newline. */
export function csvField(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

/** Joins already-stringified rows (including the header row, if any) into CSV text using CRLF line endings. */
export function toCsv(rows: string[][]): string {
  return rows.map((row) => row.map(csvField).join(',')).join('\r\n');
}

/** Triggers a browser download of `content` as a file named `filename`, via a `Blob` + temporary `<a download>` element. */
export function downloadTextFile(filename: string, content: string, mimeType = 'text/csv;charset=utf-8'): void {
  const blob = new Blob([content], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  // Revoke on a delay rather than immediately after `click()` — some
  // browsers process the download asynchronously, and revoking the object
  // URL too early can cancel it.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
