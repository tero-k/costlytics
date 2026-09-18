import './style.css';
import { getFilterValues } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible } from './shared/statusBar.ts';

/**
 * App shell / orchestrator for the Costlytics Service Detail page.
 *
 * Mirrors `explorerMain.ts`'s bootstrap pattern (shared status bar /
 * date-range defaults, an `allSettled`-based initial-load indicator), plus
 * this page's own `#service-picker`: populated from `getFilterValues`,
 * synced to a `?service=` URL query parameter (so the page is
 * linkable/bookmarkable), and treated as an extra shared control alongside
 * `#date-start`/`#date-end`/`#metric-select` (see `subscribeToControls`'s
 * `extraIds` option).
 *
 * The KPI summary / trend chart (Task 2), the account/region/charge-category
 * breakdowns (Task 3), and the top-resources table (Task 4) each register
 * their `init*` function into `refreshers` below, following the same
 * pattern as `main.ts`/`explorerMain.ts`. Each of those components is
 * responsible for reading `#service-picker`'s current value itself (via
 * `getSelectedService()`) and treating an unselected service (`null`) as
 * "nothing to fetch yet" rather than querying with an empty/invalid filter.
 */

const SERVICE_PARAM = 'service';

function getServicePicker(): HTMLSelectElement | null {
  return document.querySelector<HTMLSelectElement>('#service-picker');
}

/**
 * Currently selected service, or `null` if the placeholder ("Select a
 * service…") option is selected. Later tasks' components read this (or
 * re-implement the same one-line read against `#service-picker`) rather
 * than tracking selection state separately.
 */
export function getSelectedService(): string | null {
  const value = getServicePicker()?.value;
  return value ? value : null;
}

function updateUrlParam(service: string | null): void {
  const url = new URL(window.location.href);
  if (service) {
    url.searchParams.set(SERVICE_PARAM, service);
  } else {
    url.searchParams.delete(SERVICE_PARAM);
  }
  window.history.replaceState(null, '', url);
}

function updatePlaceholderVisibility(service: string | null): void {
  const placeholder = document.querySelector<HTMLElement>('#service-detail-placeholder');
  if (placeholder) placeholder.hidden = service !== null;
}

/**
 * Populates `#service-picker` from `getFilterValues('services')`, sorted
 * alphabetically. Honors a `?service=` URL query parameter if it names a
 * service present in the fetched list, so a shared/bookmarked URL
 * round-trips back to the same selection; otherwise defaults to the first
 * service once the list loads. On fetch failure (or an empty list) the
 * picker is left showing only its placeholder option and the "select a
 * service" message stays visible, rather than any component attempting to
 * fetch with an empty/invalid service filter.
 */
async function populateServicePicker(): Promise<void> {
  const picker = getServicePicker();
  if (!picker) return;

  let services: string[];
  try {
    services = await getFilterValues('services');
  } catch {
    updatePlaceholderVisibility(null);
    return;
  }

  const sorted = [...services].sort((a, b) => a.localeCompare(b));

  const fragment = document.createDocumentFragment();
  for (const service of sorted) {
    const option = document.createElement('option');
    option.value = service;
    option.textContent = service;
    fragment.appendChild(option);
  }
  picker.appendChild(fragment);

  const requested = new URL(window.location.href).searchParams.get(SERVICE_PARAM);
  const initial = requested && sorted.includes(requested) ? requested : (sorted[0] ?? null);

  picker.value = initial ?? '';
  updateUrlParam(initial);
  updatePlaceholderVisibility(initial);
}

/** Keeps the URL query param and placeholder message in sync with the user's picker selection. */
function initServicePicker(): void {
  const picker = getServicePicker();
  if (!picker) return;

  picker.addEventListener('change', () => {
    const service = getSelectedService();
    updateUrlParam(service);
    updatePlaceholderVisibility(service);
  });
}

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure). Empty for now (Task 1); the KPI/trend (Task 2), breakdown
 * (Task 3), and top-resources (Task 4) components register here.
 */
const refreshers: Array<() => Promise<void>> = [];

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();
  initServicePicker();

  setLoadingIndicatorVisible(true);
  try {
    await populateServicePicker();
    await Promise.allSettled(refreshers.map((refresh) => refresh()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
