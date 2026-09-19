/**
 * `#service-picker` state/URL-sync logic shared by the Service Detail page's
 * entry point (`serviceDetailMain.ts`) and its four leaf components
 * (`serviceKpi.ts`, `serviceTrend.ts`, `serviceBreakdowns.ts`,
 * `serviceTopResources.ts`).
 *
 * Thin, page-specific instantiation of the generic `dimensionPicker.ts`
 * (`?service=` URL param, `#service-picker` element) — see that module's doc
 * comment for why the picker pattern was generalized (Session 10, once the
 * Account Detail page needed the identical pattern for `?account=` /
 * `#account-picker`). Exports the same function names as before the
 * generalization so none of this module's five callers had to change.
 */

import { createDimensionPicker } from './dimensionPicker.ts';
import type { EntityConfig } from './entityConfig.ts';

const servicePicker = createDimensionPicker({ paramName: 'service', elementId: 'service-picker' });

/**
 * Currently selected service, or `null` if the placeholder ("Select a
 * service…") option is selected. Every Service Detail component reads this
 * (rather than tracking selection state separately) to decide what to fetch.
 */
export const getSelectedService = servicePicker.getSelected;

/** Syncs the `?service=` URL query parameter to the given selection (or clears it for `null`). */
export const updateUrlParam = servicePicker.updateUrlParam;

/** Reads the `?service=` URL query parameter, or `null` if absent. */
export const getServiceUrlParam = servicePicker.getUrlParam;

/**
 * Populates `#service-picker` from an already-fetched, alphabetically sorted
 * service list. Honors a `?service=` URL query parameter if it names a
 * service present in the list, so a shared/bookmarked URL round-trips back to
 * the same selection; otherwise defaults to the first service in the list (or
 * `null` if the list is empty). Returns the resulting selection.
 */
export const populateServicePickerOptions = servicePicker.populateOptions;

/** Wires `#service-picker`'s `change` event to sync the URL param and notify `onChange`. */
export const initServicePicker = servicePicker.init;

/**
 * {@link EntityConfig} instantiation for the Service Detail page, consumed by
 * the generic `entityKpi.ts`/`entityTrend.ts`/`entityTopResources.ts`
 * components (via `serviceDetailMain.ts`) so they can fetch/filter/render for
 * "service" without any service-specific code of their own.
 */
export const serviceEntityConfig: EntityConfig = {
  entityNoun: 'service',
  idPrefix: 'service',
  filterKey: 'services',
  pickerSelector: '#service-picker',
  getSelected: getSelectedService,
};
