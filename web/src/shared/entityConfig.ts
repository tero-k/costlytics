/**
 * Shared config type for the "detail page" entity components
 * (`entityKpi.ts`, `entityTrend.ts`, `entityTopResources.ts`).
 *
 * Session 10's whole-branch review found `serviceKpi.ts`/`accountKpi.ts` and
 * `serviceTrend.ts`/`accountTrend.ts` had ZERO code differences beyond
 * entity-noun substitution (`service` -> `account`, `services` ->
 * `accounts`, `#service-picker` -> `#account-picker`, etc.). This type
 * captures exactly those substitution points so the three generic modules
 * can be written once and instantiated per entity, the same way
 * `dimensionPicker.ts` generalized the original `servicePicker.ts`.
 *
 * As of Session 11 Task 4, each page's `EntityConfig` is built inline by the
 * generic `entityDetailMain.ts`'s `bootstrapEntityDetailPage` from its
 * `EntityDetailPageConfig` (which supplies `entityNoun`/`idPrefix`/
 * `filterKey`/`paramName`/`elementId`), rather than being exported by a
 * page-specific picker wrapper module (the now-deleted
 * `shared/servicePicker.ts`/`shared/accountPicker.ts`) — one fewer file per
 * page, since `dimensionPicker.ts`'s `createDimensionPicker` already covers
 * the URL/DOM wiring those wrappers used to re-export.
 */

/** Request-body filter key from `FilterFields` (`api.ts`) this entity filters cost queries on. */
export type EntityFilterKey = 'services' | 'accounts';

export interface EntityConfig {
  /**
   * Lowercase singular noun for this entity, used in user-facing copy (e.g.
   * `'service'` -> "Select a service to view its cost trend.").
   */
  entityNoun: string;
  /**
   * DOM id prefix for this page's per-entity elements, e.g. `'service'` ->
   * container ids `service-kpi`, `service-trend`, `service-top-resources`
   * and card/chart ids derived from those.
   */
  idPrefix: string;
  /** `FilterFields` key to populate with `[selected]` on `/cost/*` requests. */
  filterKey: EntityFilterKey;
  /** CSS selector for this page's picker `<select>`, passed to `subscribeToControls`'s `extraIds`. */
  pickerSelector: string;
  /** Currently selected entity value, or `null` if nothing is selected yet. */
  getSelected: () => string | null;
}
