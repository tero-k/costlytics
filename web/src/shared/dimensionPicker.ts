/**
 * Generic entity-picker state/URL-sync logic shared by the Service Detail and
 * Account Detail pages' entry points (`serviceDetailMain.ts` /
 * `accountDetailMain.ts`) and their leaf components (`serviceKpi.ts`,
 * `serviceTrend.ts`, `serviceBreakdowns.ts`, `serviceTopResources.ts` and
 * their Account Detail counterparts).
 *
 * Generalized from the original `servicePicker.ts` (Session 9, reviewed
 * twice) once a second page needed the exact same "select an entity from a
 * `getFilterValues`-populated `<select>`, sync it to a URL query param,
 * default to the first entry, notify subscribers on change" pattern — only
 * the URL param name, the DOM element id, and (at the call site) the
 * `getFilterValues` dimension differ between the two pages. Each page creates
 * its own {@link DimensionPicker} instance via {@link createDimensionPicker}
 * (e.g. `createDimensionPicker({ paramName: 'service', elementId:
 * 'service-picker' })`) rather than sharing module-level state, so the two
 * pickers' selections/URL params can never cross-contaminate even if both
 * modules were ever imported into the same page.
 *
 * Pulled out of `serviceDetailMain.ts` (rather than the leaf components
 * importing a picker accessor from there directly) to break the only import
 * cycle in the frontend: `serviceDetailMain.ts` imports all four leaf modules
 * (to register their `init*` functions as refreshers) AND has a top-level
 * `void bootstrap()` side effect, so a leaf module importing anything from it
 * — even just a hoisted function declaration — pulls the whole page's
 * bootstrap logic into that leaf's dependency graph and makes it untestable
 * in isolation.
 */

export interface DimensionPickerConfig {
  /** URL query parameter name, e.g. `'service'` -> `?service=EC2`. */
  paramName: string;
  /** DOM id of the `<select>` element, e.g. `'service-picker'`. */
  elementId: string;
}

export interface DimensionPicker {
  /**
   * Currently selected value, or `null` if the placeholder ("Select a
   * …") option is selected. Every consuming component reads this (rather
   * than tracking selection state separately) to decide what to fetch.
   */
  getSelected(): string | null;
  /** Syncs the URL query parameter to the given selection (or clears it for `null`). */
  updateUrlParam(value: string | null): void;
  /** Reads the URL query parameter, or `null` if absent. */
  getUrlParam(): string | null;
  /**
   * Populates the `<select>` from an already-fetched, alphabetically sorted
   * value list. Honors the URL query parameter if it names a value present
   * in the list, so a shared/bookmarked URL round-trips back to the same
   * selection; otherwise defaults to the first value in the list (or `null`
   * if the list is empty). Returns the resulting selection.
   */
  populateOptions(sortedValues: string[]): string | null;
  /** Wires the `<select>`'s `change` event to sync the URL param and notify `onChange`. */
  init(onChange: (value: string | null) => void): void;
}

/** Creates a {@link DimensionPicker} bound to its own URL param and DOM element. */
export function createDimensionPicker(config: DimensionPickerConfig): DimensionPicker {
  const { paramName, elementId } = config;

  function getElement(): HTMLSelectElement | null {
    return document.querySelector<HTMLSelectElement>(`#${elementId}`);
  }

  function getSelected(): string | null {
    const value = getElement()?.value;
    return value ? value : null;
  }

  function updateUrlParam(value: string | null): void {
    const url = new URL(window.location.href);
    if (value) {
      url.searchParams.set(paramName, value);
    } else {
      url.searchParams.delete(paramName);
    }
    window.history.replaceState(null, '', url);
  }

  function getUrlParam(): string | null {
    return new URL(window.location.href).searchParams.get(paramName);
  }

  function populateOptions(sortedValues: string[]): string | null {
    const picker = getElement();
    if (!picker) return null;

    const fragment = document.createDocumentFragment();
    for (const value of sortedValues) {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = value;
      fragment.appendChild(option);
    }
    picker.appendChild(fragment);

    const requested = getUrlParam();
    const initial = requested && sortedValues.includes(requested) ? requested : (sortedValues[0] ?? null);

    picker.value = initial ?? '';
    updateUrlParam(initial);
    return initial;
  }

  function init(onChange: (value: string | null) => void): void {
    const picker = getElement();
    if (!picker) return;

    picker.addEventListener('change', () => {
      const value = getSelected();
      updateUrlParam(value);
      onChange(value);
    });
  }

  return { getSelected, updateUrlParam, getUrlParam, populateOptions, init };
}
