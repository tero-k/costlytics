/**
 * Per-module stale-response guard for concurrent refreshes.
 *
 * Each of `kpiCards.ts`, `trendChart.ts`, `topBreakdown.ts` independently
 * attaches `change` listeners to the shared date/metric controls. Rapid
 * control changes can fire overlapping fetches with no cancellation, so a
 * slow earlier response could resolve AFTER a newer one and overwrite it
 * with stale data.
 *
 * Usage: one `RequestGuard` instance per independently-refreshing unit of
 * work (e.g. one for the whole module, or one per chart when a module
 * manages several charts that can be in flight at once). Call `next()` at
 * the start of a refresh to get a token, then check `isCurrent(token)` right
 * before applying the result — if it's no longer current, silently discard.
 */
export class RequestGuard {
  private latest = 0;

  /** Call at the start of a new fetch; returns a token to check on completion. */
  next(): number {
    this.latest += 1;
    return this.latest;
  }

  /** True if `token` is still the most recently issued one (i.e. no newer fetch has started). */
  isCurrent(token: number): boolean {
    return token === this.latest;
  }
}
