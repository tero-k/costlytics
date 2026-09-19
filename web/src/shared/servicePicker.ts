/**
 * `#service-picker` state/URL-sync logic shared by the Service Detail page's
 * entry point (`serviceDetailMain.ts`) and its four leaf components
 * (`serviceKpi.ts`, `serviceTrend.ts`, `serviceBreakdowns.ts`,
 * `serviceTopResources.ts`).
 *
 * Pulled out of `serviceDetailMain.ts` (rather than the leaf components
 * importing `getSelectedService` from there directly) to break the only
 * import cycle in the frontend: `serviceDetailMain.ts` imports all four leaf
 * modules (to register their `init*` functions as refreshers) AND has a
 * top-level `void bootstrap()` side effect, so a leaf module importing
 * anything from it — even just a hoisted function declaration — pulls the
 * whole page's bootstrap logic into that leaf's dependency graph and makes
 * it untestable in isolation.
 */

const SERVICE_PARAM = 'service';

function getServicePicker(): HTMLSelectElement | null {
  return document.querySelector<HTMLSelectElement>('#service-picker');
}

/**
 * Currently selected service, or `null` if the placeholder ("Select a
 * service…") option is selected. Every Service Detail component reads this
 * (rather than tracking selection state separately) to decide what to fetch.
 */
export function getSelectedService(): string | null {
  const value = getServicePicker()?.value;
  return value ? value : null;
}

/** Syncs the `?service=` URL query parameter to the given selection (or clears it for `null`). */
export function updateUrlParam(service: string | null): void {
  const url = new URL(window.location.href);
  if (service) {
    url.searchParams.set(SERVICE_PARAM, service);
  } else {
    url.searchParams.delete(SERVICE_PARAM);
  }
  window.history.replaceState(null, '', url);
}

/** Reads the `?service=` URL query parameter, or `null` if absent. */
export function getServiceUrlParam(): string | null {
  return new URL(window.location.href).searchParams.get(SERVICE_PARAM);
}

/**
 * Populates `#service-picker` from an already-fetched, alphabetically sorted
 * service list. Honors a `?service=` URL query parameter if it names a
 * service present in the list, so a shared/bookmarked URL round-trips back to
 * the same selection; otherwise defaults to the first service in the list (or
 * `null` if the list is empty). Returns the resulting selection.
 */
export function populateServicePickerOptions(sortedServices: string[]): string | null {
  const picker = getServicePicker();
  if (!picker) return null;

  const fragment = document.createDocumentFragment();
  for (const service of sortedServices) {
    const option = document.createElement('option');
    option.value = service;
    option.textContent = service;
    fragment.appendChild(option);
  }
  picker.appendChild(fragment);

  const requested = getServiceUrlParam();
  const initial = requested && sortedServices.includes(requested) ? requested : (sortedServices[0] ?? null);

  picker.value = initial ?? '';
  updateUrlParam(initial);
  return initial;
}

/** Wires `#service-picker`'s `change` event to sync the URL param and notify `onChange`. */
export function initServicePicker(onChange: (service: string | null) => void): void {
  const picker = getServicePicker();
  if (!picker) return;

  picker.addEventListener('change', () => {
    const service = getSelectedService();
    updateUrlParam(service);
    onChange(service);
  });
}
