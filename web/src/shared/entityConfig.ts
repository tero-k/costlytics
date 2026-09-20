import type { FilterFields } from '../api.ts';

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
 * `buildFilter`/`paramName`/`elementId`), rather than being exported by a
 * page-specific picker wrapper module (the now-deleted
 * `shared/servicePicker.ts`/`shared/accountPicker.ts`) — one fewer file per
 * page, since `dimensionPicker.ts`'s `createDimensionPicker` already covers
 * the URL/DOM wiring those wrappers used to re-export.
 *
 * Session 15 Task 1 replaced the flat `filterKey: 'services' | 'accounts'`
 * field with `buildFilter`, a callback from the selected value to the
 * `Partial<FilterFields>` slice a request should carry. Service/Account
 * Detail's selection is a single flat value (`{services: [selected]}` /
 * `{accounts: [selected]}`), but a future Tags page's selection is a
 * key+value PAIR (`{tags: [{key, operator: 'eq', values: [selected]}]}`)
 * that can't be expressed as `{[key]: [selected]}` at all — `buildFilter`
 * lets each page decide its own request shape while the four leaf modules
 * below stay 100% generic. This is deliberately a DIFFERENT concern from
 * `getFilterValues`'s dimension argument (which populates a picker's option
 * list, not a request filter) — see `EntityDetailPageConfig` in
 * `entityDetailMain.ts` for how the two are kept separate.
 */

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
  /**
   * Builds the `Partial<FilterFields>` (`api.ts`) slice a `/cost/*` request
   * should carry for the given selected value, e.g.
   * `(selected) => ({services: [selected]})`. Called fresh on every request
   * rather than memoized, so a page whose filter depends on additional
   * state beyond `selected` (e.g. a future Tags page's currently selected
   * tag KEY) can read that state at call time.
   */
  buildFilter: (selected: string) => Partial<FilterFields>;
  /** CSS selector for this page's picker `<select>`, passed to `subscribeToControls`'s `extraIds`. */
  pickerSelector: string;
  /** Currently selected entity value, or `null` if nothing is selected yet. */
  getSelected: () => string | null;
}
