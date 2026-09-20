/**
 * Generic entity-picker state/URL-sync logic used by the Service Detail and
 * Account Detail pages' generic orchestrator (`entityDetailMain.ts`'s
 * `bootstrapEntityDetailPage`), which builds one `DimensionPicker` instance
 * per page from its `EntityDetailPageConfig`.
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
 * were ever active on the same page.
 *
 * Session 11 Task 4 deleted the thin per-page wrapper modules
 * (`shared/servicePicker.ts`/`shared/accountPicker.ts`) that used to
 * re-export a `createDimensionPicker` instance's functions under
 * page-specific names: once every leaf component took a generic
 * `EntityConfig` instead of importing page-specific accessor functions
 * directly, those wrappers had no remaining callers besides each page's
 * `*DetailMain.ts`, which now calls `createDimensionPicker` itself (via
 * `entityDetailMain.ts`) instead.
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

    // Remove any options from a PRIOR `populateOptions` call before
    // appending the new list, so calling this repeatedly (e.g. after a
    // source switch) replaces rather than duplicates the option list. Each
    // page's own static placeholder option (`<option value="" selected>...`,
    // defined in the page's HTML, not here) has an empty `value` and is
    // deliberately preserved.
    for (const option of Array.from(picker.querySelectorAll('option'))) {
      if (option.value !== '') option.remove();
    }

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
