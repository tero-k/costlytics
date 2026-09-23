/**
 * Pure helpers for the single transport seam in `api.ts`. Inside the
 * desktop app (Tauri) every `/api/v1/<a>/<b-c>` request becomes the Rust
 * command `<a>_<b_c>` (`crates/app/src/commands.rs`), invoked with
 * `{ req: <body or query params> }`. Outside Tauri, `api.ts` uses `fetch`
 * against the dev/test HTTP harness (`crates/api`), which serves the same
 * paths — so page modules never know which one they're talking to.
 */

const PREFIX = '/api/v1/';

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function commandName(path: string): string {
  const pathname = path.split('?')[0];
  return pathname.slice(PREFIX.length).replace(/[/-]/g, '_');
}

export function argsFromPath(path: string): Record<string, string> {
  const query = path.includes('?') ? path.slice(path.indexOf('?') + 1) : '';
  return Object.fromEntries(new URLSearchParams(query));
}

/** Maps `service::ErrorKind` to the status the HTTP harness would return. */
export function statusForKind(kind: unknown): number {
  switch (kind) {
    case 'bad_request':
      return 400;
    case 'not_found':
      return 404;
    case 'conflict':
      return 409;
    default:
      return 500;
  }
}
