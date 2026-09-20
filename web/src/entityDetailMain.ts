import { getFilterValues, type FilterFields, type FilterValuesDimension } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { createDimensionPicker } from './shared/dimensionPicker.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { initEntityKpi } from './entityKpi.ts';
import { initEntityTrend } from './entityTrend.ts';
import { initEntityBreakdowns, type BreakdownDef } from './entityBreakdowns.ts';
import { initEntityTopResources } from './entityTopResources.ts';
import type { EntityConfig } from './shared/entityConfig.ts';

/**
 * Generic app shell / orchestrator for a "detail page" entity (plan §26) —
 * Service Detail and Account Detail (Session 11 Task 4). Both pages'
 * `*DetailMain.ts` entry points are now just a `BreakdownDef[]` plus a call
 * to {@link bootstrapEntityDetailPage} with this config; a future Tags
 * drilldown page adds a THIRD such call, not a fifth near-copy of this file.
 *
 * Subsumes what used to be each page's own bootstrap function AND its
 * `shared/servicePicker.ts`/`shared/accountPicker.ts` wrapper (both deleted
 * in this session): this module builds its own `DimensionPicker` directly
 * from `shared/dimensionPicker.ts`'s `createDimensionPicker` and its own
 * `EntityConfig` from the same handful of primitives (`paramName`,
 * `elementId`, `filterValuesDimension`, `buildFilter`, `entityNoun`,
 * `idPrefix`), since nothing else in the codebase imports those two picker
 * modules anymore now that the KPI / trend / breakdowns / top-resources
 * components all take a generic `EntityConfig` rather than page-specific
 * accessor functions.
 *
 * Mirrors `explorerMain.ts`'s/`main.ts`'s bootstrap pattern (shared status
 * bar / date-range defaults, an `allSettled`-based initial-load indicator),
 * plus this page family's own entity picker: populated from
 * `getFilterValues(config.filterValuesDimension)`, synced to a `?{paramName}=` URL query
 * parameter (so the page is linkable/bookmarkable), and treated as an extra
 * shared control alongside `#date-start`/`#date-end`/`#metric-select` (via
 * each generic component's `subscribeToControls(..., { extraIds: [...] })`).
 *
 * Each generic component is responsible for reading `config.getSelected()`
 * itself and treating an unselected entity (`null`) as "nothing to fetch
 * yet" rather than querying with an empty/invalid filter.
 *
 * Session 15 Task 1: `filterValuesDimension` (which POPULATES the picker's
 * option list via `getFilterValues`) and `buildFilter` (which turns a
 * SELECTED value into the `Partial<FilterFields>` slice a `/cost/*` request
 * carries, passed straight through to `EntityConfig`) are deliberately two
 * separate fields here, even though for Service/Account Detail they happen
 * to correspond 1:1 (`'services'` populates from and filters on the same
 * concept). A future Tags page needs them to diverge — its picker
 * population is a two-step tag-key/tag-value fetch with no single
 * `FilterValuesDimension`, while its `buildFilter` still needs to produce a
 * `{tags: [...]}` filter shaped like this same `EntityConfig`. Keeping them
 * separate now avoids re-deriving one from the other later.
 */
export interface EntityDetailPageConfig {
  /** URL query parameter name, e.g. `'service'` -> `?service=EC2`. */
  paramName: string;
  /** DOM id of the entity `<select>`, e.g. `'service-picker'`. */
  elementId: string;
  /** DOM id of the "select an entity" placeholder element, hidden once one is selected. */
  placeholderId: string;
  /** `getFilterValues` dimension to populate the entity picker's option list from. */
  filterValuesDimension: FilterValuesDimension;
  /** Builds the `Partial<FilterFields>` slice a `/cost/*` request should carry for the selected value; passed straight through to `EntityConfig.buildFilter`. */
  buildFilter: (selected: string) => Partial<FilterFields>;
  /** Lowercase singular noun for this entity, used in user-facing copy. */
  entityNoun: string;
  /** DOM id prefix for this page's per-entity component containers. */
  idPrefix: string;
  /** This page's breakdown-by-dimension charts, e.g. Service Detail's account/region/charge_category vs. Account Detail's service/region. */
  breakdowns: BreakdownDef[];
}

/**
 * Bootstraps a detail page for the given entity config: wires the shared
 * status bar / date-range defaults, the entity picker (populate, URL
 * round-trip, change handling), and the four generic leaf components (KPI,
 * trend, breakdowns, top-resources), then resolves once their initial load
 * has settled (success or failure).
 */
export async function bootstrapEntityDetailPage(config: EntityDetailPageConfig): Promise<void> {
  const picker = createDimensionPicker({ paramName: config.paramName, elementId: config.elementId });

  const entityConfig: EntityConfig = {
    entityNoun: config.entityNoun,
    idPrefix: config.idPrefix,
    buildFilter: config.buildFilter,
    pickerSelector: `#${config.elementId}`,
    getSelected: picker.getSelected,
  };

  function updatePlaceholderVisibility(selected: string | null): void {
    const placeholder = document.querySelector<HTMLElement>(`#${config.placeholderId}`);
    if (placeholder) placeholder.hidden = selected !== null;
  }

  /**
   * Populates the picker from `getFilterValues(config.filterValuesDimension)`,
   * sorted alphabetically. On fetch failure (or an empty list) the picker is
   * left showing only its placeholder option and the "select an entity"
   * message stays visible, rather than any component attempting to fetch
   * with an empty/invalid filter.
   */
  async function populatePicker(): Promise<void> {
    const el = document.querySelector<HTMLSelectElement>(`#${config.elementId}`);
    if (!el) return;

    let values: string[];
    try {
      values = await getFilterValues(config.filterValuesDimension);
    } catch {
      updatePlaceholderVisibility(null);
      return;
    }

    const sorted = [...values].sort((a, b) => a.localeCompare(b));
    const initial = picker.populateOptions(sorted);
    updatePlaceholderVisibility(initial);
  }

  // Each `init*` call performs its ONE-TIME setup (shell render, own
  // `RequestGuard`(s), and its single `subscribeToControls` registration)
  // synchronously and returns a stable `{ refresh }` handle — it does NOT
  // trigger the initial load itself. `handles` is built exactly once, so
  // every subsequent `refreshAll()` call (bootstrap AND every later source
  // switch) reuses the SAME guards and adds NO new control listeners,
  // unlike the pre-fix version which re-ran these factories (and thus
  // `subscribeToControls`) on every source switch, leaking listeners and
  // decoupling old/new in-flight requests' guards from each other.
  const handles = [
    initEntityKpi(entityConfig, updateStatusCurrency),
    initEntityTrend(entityConfig, updateStatusCurrency),
    initEntityBreakdowns(entityConfig, config.breakdowns, updateStatusCurrency),
    initEntityTopResources(entityConfig, updateStatusCurrency),
  ];

  async function refreshAll(): Promise<void> {
    await Promise.allSettled(handles.map((handle) => handle.refresh()));
  }

  /**
   * Re-run when the source picker changes: repopulates the entity picker's
   * option list from the NEW source's `getFilterValues` (so it stops
   * showing the old source's entities) and resolves/URL-syncs the
   * selection, THEN re-invokes every leaf component's `refresh()` so they
   * end up fetching with the corrected, new-source-relative entity rather
   * than the stale one their own `#source-picker` listener (registered
   * after this one — see `initSourcePicker`'s doc comment) fired with.
   * That automatic, stale-entity fetch is a known, harmless transient: each
   * leaf's `RequestGuard` ensures the LATER `refresh()` call triggered here
   * always wins once it resolves, regardless of arrival order — and since
   * `handles` is reused rather than rebuilt, that guard is the SAME guard
   * the stale, auto-triggered fetch used, so invalidation actually works.
   */
  async function onSourceChange(): Promise<void> {
    await populatePicker();
    await refreshAll();
  }

  initDateRangeDefaults();
  initStatusBar();
  picker.init(updatePlaceholderVisibility);
  await initSourcePicker(onSourceChange);

  setLoadingIndicatorVisible(true);
  try {
    await populatePicker();
    await refreshAll();
  } finally {
    setLoadingIndicatorVisible(false);
  }
}
