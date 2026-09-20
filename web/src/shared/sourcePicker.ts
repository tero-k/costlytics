/**
 * Shared source-selector control (`#source-picker`), wired into every page
 * (Session 14 Task 3). Reuses `dimensionPicker.ts`'s generic
 * `createDimensionPicker` directly — same "sorted string list + URL param +
 * `<select>`" pattern the Service/Account Detail pages already use for their
 * entity pickers — rather than a bespoke picker module.
 *
 * Populated from `getSources()`'s REGISTERED sources only: a skipped source
 * isn't queryable via `source_id` (the backend would 400/404), so it isn't
 * offered as a selection here. (Task 4's diagnostics page is where skipped
 * sources' status becomes visible, not this picker.)
 *
 * `createDimensionPicker` sets both an `<option>`'s `value` AND its
 * displayed text to the same string, so this picker's option list is
 * populated with source IDs (not display names) — the ID is what has to
 * round-trip through the URL param and `setActiveSourceId`/`source_id`, and
 * reusing the picker as-is (per this task's plan) means the displayed label
 * and the value can't differ. IDs are still human-legible in practice
 * (`config/example.toml` gives them descriptive slugs).
 *
 * On init, the resolved initial selection is pushed into `setActiveSourceId`
 * and this function's promise doesn't resolve until that happens — callers
 * MUST `await initSourcePicker()` before triggering their page's first
 * fetch, so no request ever races an unset source. Selection precedence
 * (highest first):
 *   1. `?source=` URL query param, if it names a registered source
 *      (bookmarked/shared links round-trip, same as the entity pickers).
 *   2. The backend's `default_source_id` (`resolve_source(None)`'s own
 *      fallback), IF it's registered — this is what makes a fresh page load
 *      match the backend's default, satisfying this task's "default source
 *      pre-selected" requirement.
 *   3. The first registered source alphabetically, as a safe fallback for
 *      the case `default_source_id` names a SKIPPED source (it's allowed to
 *      — see `SourcesResponse.default_source_id`'s doc comment in `api.ts`).
 *      Selecting an unselectable default would leave every request either
 *      silently using the backend's own further fallback or erroring
 *      outright, so the picker never does that.
 *
 * Implementation note: (2) is realized by pre-seeding the `?source=` URL
 * param with `default_source_id` (via the picker's own `updateUrlParam`)
 * BEFORE calling `populateOptions`, when no URL param is already present —
 * `populateOptions` then picks it up as case (1) naturally, reusing
 * `dimensionPicker.ts`'s existing precedence logic rather than
 * reimplementing it here.
 *
 * If `getSources()` fails entirely (e.g. backend down), the picker is left
 * empty and `setActiveSourceId(null)` is called — every request is then
 * sent without `source_id`, which is exactly what happened before this
 * task existed (the backend's `resolve_source(state, None)` fallback), so
 * this is a no-worse-than-before degradation rather than a hard failure.
 */

import { getSources, setActiveSourceId } from '../api.ts';
import { createDimensionPicker } from './dimensionPicker.ts';

export async function initSourcePicker(
  onSourceChange?: (sourceId: string | null) => void,
): Promise<void> {
  const picker = createDimensionPicker({ paramName: 'source', elementId: 'source-picker' });

  let ids: string[] = [];
  try {
    const { sources, default_source_id } = await getSources();
    ids = sources
      .filter((s) => s.state === 'registered')
      .map((s) => s.id)
      .sort((a, b) => a.localeCompare(b));

    if (!picker.getUrlParam() && default_source_id && ids.includes(default_source_id)) {
      picker.updateUrlParam(default_source_id);
    }
  } catch {
    // Leave `ids` empty; the picker shows only its placeholder option and
    // every request goes out without `source_id` (backend fallback).
  }

  const initial = picker.populateOptions(ids);
  setActiveSourceId(initial);

  // `subscribeToControls`'s own `#source-picker` listener (added by every
  // fetching module) re-fetches on the same `change` event; this listener
  // just needs to update the active source BEFORE those fire, which holds
  // because DOM listeners fire in registration order and this page's
  // `initSourcePicker()` call always completes (this function is `await`ed)
  // before any component's `subscribeToControls(...)` call runs.
  //
  // `onSourceChange`, if given, is invoked from INSIDE this same listener,
  // right after `setActiveSourceId` — this guarantees it runs (and any
  // `await` inside it starts) before every leaf component's OWN
  // `#source-picker` listener has a chance to run its `change` handler
  // synchronously to completion, since this listener was registered first
  // (see `entityDetailMain.ts`, which `await`s `initSourcePicker()` before
  // its leaf components' `subscribeToControls` calls run). It's used by
  // pages (e.g. Service/Account Detail) that own an entity picker whose
  // option list/selection is source-dependent and must be refreshed before
  // those components' automatic re-fetch is allowed to be treated as final.
  picker.init((value) => {
    setActiveSourceId(value);
    onSourceChange?.(value);
  });
}
