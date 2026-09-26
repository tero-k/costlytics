import './style.css';
import { getFilterValues, getTagValues } from './api.ts';
import { setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initAppShell } from './shared/appShell.ts';
import { initPageFilters } from './shared/pageFilters.ts';
import { createDimensionPicker } from './shared/dimensionPicker.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { checkInitialLoad, installCostGuard, type PageQueries } from './shared/costGuard.ts';
import { initEntityKpi } from './entityKpi.ts';
import { initEntityTrend } from './entityTrend.ts';
import { initEntityBreakdowns, type BreakdownDef } from './entityBreakdowns.ts';
import { initEntityTopResources } from './entityTopResources.ts';
import type { EntityConfig } from './shared/entityConfig.ts';
import { createEntityOrchestrator } from './shared/entityOrchestrator.ts';

/**
 * Entry point for the Costlytics Tags drilldown page (plan §26-27's third
 * drilldown page, deferred from Sessions 9-10 — see
 * `.git/sdd/tasks-tags-drilldown.md`'s "Why this session exists").
 *
 * Unlike Service/Account Detail's single flat entity picker, a tag selection
 * is a KEY+VALUE pair, and the value list is conditional on the chosen key
 * (`GET /api/v1/filter-values/tag-values?key=X`), so this page owns TWO
 * independent `DimensionPicker` instances (`shared/dimensionPicker.ts`) —
 * `tag-key-picker` (`?tag_key=`, populated once from `getFilterValues('tag-keys')`)
 * and `tag-value-picker` (`?tag_value=`, REPOPULATED from `getTagValues(key)`
 * every time the key picker's selection changes) — rather than reusing
 * `entityDetailMain.ts`'s single-picker `bootstrapEntityDetailPage`.
 *
 * `dimensionPicker.ts`'s `populateOptions` already clears any options from a
 * prior call before appending the new list (Session 14 fix), so repopulating
 * the value picker on every key change is safe: no accumulating/duplicate
 * `<option>`s.
 *
 * Session 15 Task 3: reuses the SAME generic `initEntityKpi`/`initEntityTrend`/
 * `initEntityBreakdowns`/`initEntityTopResources` leaf modules Service/Account
 * Detail already share (Task 1's `buildFilter`-based `EntityConfig`). This
 * page's `EntityConfig.getSelected()` returns the current tag VALUE (or
 * `null` if either picker is unset), and `buildFilter` reads the CURRENT key
 * selection at call time (`keyPicker.getSelected()`, not a value captured
 * once at setup) since the key can change independently of the value.
 *
 * `EntityConfig.pickerSelector` only covers `#tag-value-picker`: each leaf
 * component's own `subscribeToControls(refresh, { extraIds: [pickerSelector] })`
 * listens for native `change` events, which only fire when the USER directly
 * changes a `<select>` — `dimensionPicker.ts`'s `populateOptions` sets
 * `.value` programmatically and does NOT dispatch `change`. So a tag-KEY
 * change (which reprograms the value picker's options/selection without user
 * interaction on that element) can't rely on that same mechanism; instead
 * `onKeyChange` below explicitly calls `refreshAll()` after repopulating the
 * value picker, mirroring `entityDetailMain.ts`'s `onSourceChange` pattern
 * exactly (repopulate a picker, THEN explicitly refresh every leaf
 * component) rather than introducing a second, different orchestration
 * shape for what is structurally the same problem.
 *
 * Breakdowns: service, account, and region — "what's driving cost under this
 * tag", mirroring the dimensions Service Detail asks of accounts/regions and
 * Account Detail asks of services/regions. Top resources under the selected
 * tag value is also included (`initEntityTopResources`): a tag can span many
 * services/resources, so seeing which specific resources dominate its cost
 * is at least as useful here as it is for a single service/account, and the
 * component is a pure reuse (one more `init*` call plus one more container
 * div) with no new logic to justify leaving it out.
 */

// Renders the shared controls the leaf components (built at module level
// below) subscribe to, so it must run first.
initAppShell();

const TAG_BREAKDOWNS: BreakdownDef[] = [
  { containerId: 'tags-by-service', dimension: 'service', title: 'Cost by service' },
  { containerId: 'tags-by-account', dimension: 'account', title: 'Cost by account' },
  { containerId: 'tags-by-region', dimension: 'region', title: 'Cost by region' },
];

const keyPicker = createDimensionPicker({ paramName: 'tag_key', elementId: 'tag-key-picker', persist: true });
const valuePicker = createDimensionPicker({ paramName: 'tag_value', elementId: 'tag-value-picker', persist: true });

function updatePlaceholderVisibility(): void {
  const placeholder = document.querySelector<HTMLElement>('#tags-placeholder');
  if (placeholder) placeholder.hidden = keyPicker.getSelected() !== null && valuePicker.getSelected() !== null;
}

const entityConfig: EntityConfig = {
  entityNoun: 'tag value',
  idPrefix: 'tags',
  pickerSelector: '#tag-value-picker',
  getSelected: () => (keyPicker.getSelected() === null ? null : valuePicker.getSelected()),
  buildFilter: (selected) => ({
    tags: [{ key: keyPicker.getSelected() ?? '', operator: 'eq', values: [selected] }],
  }),
};

// See `shared/entityOrchestrator.ts` for why `inits` is called exactly once
// here and `refreshAll` must be reused (not rebuilt) for every later source
// or tag key/value switch, matching `entityDetailMain.ts`'s
// post-Session-14-fix pattern (avoiding that session's exact regression:
// unbounded listener/request accumulation on repeated switches).
const { refreshAll } = createEntityOrchestrator([
  () => initEntityKpi(entityConfig, updateStatusCurrency),
  () => initEntityTrend(entityConfig, updateStatusCurrency),
  () => initEntityBreakdowns(entityConfig, TAG_BREAKDOWNS, updateStatusCurrency),
  () => initEntityTopResources(entityConfig, updateStatusCurrency),
]);

/**
 * Populates the tag-value picker from `getTagValues(key)`, sorted
 * alphabetically, honoring `?tag_value=` if it names a value present for
 * this key (else defaulting to the first value). A `null`/absent key (no
 * tag keys available for the active source, or fetch failure) clears the
 * value picker to empty rather than fetching with an invalid key.
 */
async function populateValuePicker(key: string | null): Promise<void> {
  if (key === null) {
    valuePicker.populateOptions([]);
    return;
  }

  let values: string[];
  try {
    values = await getTagValues(key);
  } catch {
    valuePicker.populateOptions([]);
    return;
  }

  const sorted = [...values].sort((a, b) => a.localeCompare(b));
  valuePicker.populateOptions(sorted);
}

/**
 * Populates the tag-key picker from `getFilterValues('tag-keys')`, sorted
 * alphabetically, honoring `?tag_key=` if present (else defaulting to the
 * first key). Returns the resolved initial key, or `null` on fetch failure
 * or an empty key list.
 */
async function populateKeyPicker(): Promise<string | null> {
  let values: string[];
  try {
    values = await getFilterValues('tag-keys');
  } catch {
    return null;
  }

  const sorted = [...values].sort((a, b) => a.localeCompare(b));
  return keyPicker.populateOptions(sorted);
}

/**
 * Re-run when the tag-KEY picker changes: repopulates the tag-VALUE
 * picker's option list for the new key (see module doc comment for why this
 * can't rely on `subscribeToControls`'s native `change`-event mechanism),
 * THEN explicitly re-invokes every leaf component's `refresh()` so they
 * fetch with the new key/value pair rather than the stale one. Mirrors
 * `entityDetailMain.ts`'s `onSourceChange`.
 */
async function onKeyChange(value: string | null): Promise<void> {
  await populateValuePicker(value);
  updatePlaceholderVisibility();
  await refreshAll();
}

/**
 * Re-run when the source picker changes: both the tag-key and tag-value
 * lists are source-relative, so both pickers are fully repopulated, then
 * every leaf component is explicitly refreshed with the new source's
 * resolved key/value — mirrors `entityDetailMain.ts`'s `onSourceChange`.
 */
async function onSourceChange(): Promise<void> {
  const key = await populateKeyPicker();
  await populateValuePicker(key);
  updatePlaceholderVisibility();
  await refreshAll();
}

async function bootstrap(): Promise<void> {

  keyPicker.init((value) => {
    void onKeyChange(value);
  });
  // The value picker's own `change` listener (registered here) only handles
  // URL sync / placeholder visibility for USER-driven selections; the actual
  // refresh for that case is already covered by each leaf component's own
  // `subscribeToControls(refresh, { extraIds: ['#tag-value-picker'] })`
  // listener (registered independently in each `init*` call above) — same
  // division of responsibility Service/Account Detail's single picker uses.
  valuePicker.init(updatePlaceholderVisibility);

  await initSourcePicker(onSourceChange);
  initPageFilters(['services', 'accounts']);
  // KPI summary, trend, breakdowns' shared summary + one per chart, top resources.
  const queries: PageQueries = { current: 4 + TAG_BREAKDOWNS.length, compare: 0 };
  installCostGuard(queries);
  void checkInitialLoad(queries);

  setLoadingIndicatorVisible(true);
  try {
    const initialKey = await populateKeyPicker();
    await populateValuePicker(initialKey);
    updatePlaceholderVisibility();
    await refreshAll();
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
