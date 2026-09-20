/**
 * Shared "stable handles + `refreshAll()`" orchestration helper (Session 16
 * Task 3), extracted from `entityDetailMain.ts` (Session 11/14) and
 * `tagsMain.ts` (Session 15), which had independently arrived at the exact
 * same ~10-line pattern.
 *
 * This is exactly the code that carried a real, twice-reviewed regression in
 * Session 14: an earlier fix attempt re-invoked `init*` factory functions on
 * every source switch, which re-created a brand-new `RequestGuard` per switch
 * and re-registered a whole new set of `change` listeners each time, causing
 * unbounded listener/fetch accumulation on repeated switches. The fix (and
 * what this helper captures) is: each leaf's `init*` factory performs its
 * ONE-TIME setup (shell render, own `RequestGuard`(s), its single
 * `subscribeToControls` registration) synchronously and returns a stable
 * `{ refresh }` handle WITHOUT triggering the initial fetch itself.
 * `createEntityOrchestrator` calls every factory in `inits` exactly ONCE (at
 * construction time) to build `handles`, then exposes a `refreshAll()` that
 * reuses those same handles — and thus the same guards, and adds no new
 * control listeners — no matter how many times it's called (bootstrap, every
 * later source switch, every later entity/tag change, etc).
 */

export interface EntityHandle {
  refresh: () => Promise<void>;
}

export interface EntityOrchestrator {
  /** Re-invokes every handle's `refresh()`, settling regardless of individual failures. */
  refreshAll: () => Promise<void>;
}

/**
 * Builds the `handles` array exactly once by calling each factory in
 * `inits`, then returns a `refreshAll()` that reuses those same handles on
 * every call. Callers must not re-invoke `inits`' factories themselves —
 * doing so would re-run their one-time setup and reintroduce the Session 14
 * regression this helper exists to prevent.
 */
export function createEntityOrchestrator(inits: Array<() => EntityHandle>): EntityOrchestrator {
  const handles = inits.map((init) => init());

  async function refreshAll(): Promise<void> {
    await Promise.allSettled(handles.map((handle) => handle.refresh()));
  }

  return { refreshAll };
}
