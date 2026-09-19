/**
 * `#account-picker` state/URL-sync logic shared by the Account Detail page's
 * entry point (`accountDetailMain.ts`) and its leaf components.
 *
 * Thin, page-specific instantiation of the generic `dimensionPicker.ts`
 * (`?account=` URL param, `#account-picker` element) — mirrors
 * `servicePicker.ts`'s instantiation for the Service Detail page. See
 * `dimensionPicker.ts`'s doc comment for why the picker pattern was
 * generalized.
 */

import { createDimensionPicker } from './dimensionPicker.ts';
import type { EntityConfig } from './entityConfig.ts';

const accountPicker = createDimensionPicker({ paramName: 'account', elementId: 'account-picker' });

/**
 * Currently selected account, or `null` if the placeholder ("Select an
 * account…") option is selected. Every Account Detail component reads this
 * (rather than tracking selection state separately) to decide what to fetch.
 */
export const getSelectedAccount = accountPicker.getSelected;

/** Syncs the `?account=` URL query parameter to the given selection (or clears it for `null`). */
export const updateUrlParam = accountPicker.updateUrlParam;

/** Reads the `?account=` URL query parameter, or `null` if absent. */
export const getAccountUrlParam = accountPicker.getUrlParam;

/**
 * Populates `#account-picker` from an already-fetched, alphabetically sorted
 * account list. Honors a `?account=` URL query parameter if it names an
 * account present in the list, so a shared/bookmarked URL round-trips back to
 * the same selection; otherwise defaults to the first account in the list (or
 * `null` if the list is empty). Returns the resulting selection.
 */
export const populateAccountPickerOptions = accountPicker.populateOptions;

/** Wires `#account-picker`'s `change` event to sync the URL param and notify `onChange`. */
export const initAccountPicker = accountPicker.init;

/**
 * {@link EntityConfig} instantiation for the Account Detail page, consumed by
 * the generic `entityKpi.ts`/`entityTrend.ts`/`entityTopResources.ts`
 * components (via `accountDetailMain.ts`) so they can fetch/filter/render for
 * "account" without any account-specific code of their own.
 */
export const accountEntityConfig: EntityConfig = {
  entityNoun: 'account',
  idPrefix: 'account',
  filterKey: 'accounts',
  pickerSelector: '#account-picker',
  getSelected: getSelectedAccount,
};
