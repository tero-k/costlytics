import './style.css';
import { getFilterValues } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initServicePicker, populateServicePickerOptions, serviceEntityConfig } from './shared/servicePicker.ts';
import { initEntityKpi } from './entityKpi.ts';
import { initEntityTrend } from './entityTrend.ts';
import { initServiceBreakdowns } from './serviceBreakdowns.ts';
import { initEntityTopResources } from './entityTopResources.ts';

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
 * The KPI summary / trend chart / top-resources table are the generic
 * `entityKpi.ts`/`entityTrend.ts`/`entityTopResources.ts` components
 * (Session 11), each instantiated here for "service" via
 * `shared/servicePicker.ts`'s `serviceEntityConfig`. The
 * account/region/charge-category breakdowns (`serviceBreakdowns.ts`) remain
 * service-specific for now (Task 3's job). Each component registers its
 * `init*` call into `refreshers` below, following the same pattern as
 * `main.ts`/`explorerMain.ts`. Each of those components is responsible for
 * reading `#service-picker`'s current value itself (via
 * `serviceEntityConfig.getSelected`, i.e. `getSelectedService()` from
 * `shared/servicePicker.ts`) and treating an unselected service (`null`) as
 * "nothing to fetch yet" rather than querying with an empty/invalid filter.
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
 * failure). The generic KPI/trend/top-resources components and the
 * service-specific breakdowns component register here.
 */
const refreshers: Array<() => Promise<void>> = [
  () => initEntityKpi(serviceEntityConfig, updateStatusCurrency),
  () => initEntityTrend(serviceEntityConfig, updateStatusCurrency),
  () => initServiceBreakdowns(updateStatusCurrency),
  () => initEntityTopResources(serviceEntityConfig, updateStatusCurrency),
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
