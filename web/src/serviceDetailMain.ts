import './style.css';
import { getFilterValues } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initServicePicker, populateServicePickerOptions } from './shared/servicePicker.ts';
import { initServiceKpi } from './serviceKpi.ts';
import { initServiceTrend } from './serviceTrend.ts';
import { initServiceBreakdowns } from './serviceBreakdowns.ts';
import { initServiceTopResources } from './serviceTopResources.ts';

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
 * breakdowns (Task 3, `serviceBreakdowns.ts`), and the top-resources table
 * (Task 4, `serviceTopResources.ts`) each register their `init*` function
 * into `refreshers` below, following the same pattern as
 * `main.ts`/`explorerMain.ts`. Each of those components is responsible for
 * reading `#service-picker`'s current value itself (via
 * `getSelectedService()`, imported from `shared/servicePicker.ts`) and
 * treating an unselected service (`null`) as "nothing to fetch yet" rather
 * than querying with an empty/invalid filter.
 */

function getServicePicker(): HTMLSelectElement | null {
  return document.querySelector<HTMLSelectElement>('#service-picker');
}

function updatePlaceholderVisibility(service: string | null): void {
  const placeholder = document.querySelector<HTMLElement>('#service-detail-placeholder');
  if (placeholder) placeholder.hidden = service !== null;
}

/**
 * Populates `#service-picker` from `getFilterValues('services')`, sorted
 * alphabetically, via `shared/servicePicker.ts`'s
 * `populateServicePickerOptions` (which honors a `?service=` URL query
 * parameter and otherwise defaults to the first service). On fetch failure
 * (or an empty list) the picker is left showing only its placeholder option
 * and the "select a service" message stays visible, rather than any
 * component attempting to fetch with an empty/invalid service filter.
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
  const initial = populateServicePickerOptions(sorted);
  updatePlaceholderVisibility(initial);
}

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure). The KPI/trend (Task 2), breakdowns (Task 3), and top-resources
 * (Task 4) components register here.
 */
const refreshers: Array<() => Promise<void>> = [
  () => initServiceKpi(updateStatusCurrency),
  () => initServiceTrend(updateStatusCurrency),
  () => initServiceBreakdowns(updateStatusCurrency),
  () => initServiceTopResources(updateStatusCurrency),
];

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();
  initServicePicker(updatePlaceholderVisibility);

  setLoadingIndicatorVisible(true);
  try {
    await populateServicePicker();
    await Promise.allSettled(refreshers.map((refresh) => refresh()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
