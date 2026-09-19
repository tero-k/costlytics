import './style.css';
import { getFilterValues } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initAccountPicker, populateAccountPickerOptions, accountEntityConfig } from './shared/accountPicker.ts';
import { initEntityKpi } from './entityKpi.ts';
import { initEntityTrend } from './entityTrend.ts';
import { initAccountBreakdowns } from './accountBreakdowns.ts';
import { initEntityTopResources } from './entityTopResources.ts';

/**
 * App shell / orchestrator for the Costlytics Account Detail page.
 *
 * Mirrors `serviceDetailMain.ts`'s bootstrap pattern (shared status bar /
 * date-range defaults, an `allSettled`-based initial-load indicator), plus
 * this page's own `#account-picker`: populated from `getFilterValues`,
 * synced to a `?account=` URL query parameter (so the page is
 * linkable/bookmarkable), via the generalized `shared/dimensionPicker.ts`
 * (instantiated for this page as `shared/accountPicker.ts`) — the same
 * pattern `serviceDetailMain.ts` uses for `#service-picker`.
 *
 * Account values are rendered plainly (raw `account_id` strings, as
 * `getFilterValues('accounts')` / `distinct_accounts` currently return — see
 * `crates/data/src/queries/summary.rs`): there is no display-name concept in
 * the backend yet, so this page doesn't invent one.
 *
 * The KPI summary / trend chart / top-resources table are the generic
 * `entityKpi.ts`/`entityTrend.ts`/`entityTopResources.ts` components
 * (Session 11), each instantiated here for "account" via
 * `shared/accountPicker.ts`'s `accountEntityConfig` — mirrors
 * `serviceDetailMain.ts`'s use of `serviceEntityConfig`. The service/region
 * breakdowns (`accountBreakdowns.ts`) remain account-specific for now
 * (Task 3's job).
 */

function getAccountPicker(): HTMLSelectElement | null {
  return document.querySelector<HTMLSelectElement>('#account-picker');
}

function updatePlaceholderVisibility(account: string | null): void {
  const placeholder = document.querySelector<HTMLElement>('#account-detail-placeholder');
  if (placeholder) placeholder.hidden = account !== null;
}

/**
 * Populates `#account-picker` from `getFilterValues('accounts')`, sorted
 * alphabetically, via `shared/accountPicker.ts`'s
 * `populateAccountPickerOptions` (which honors a `?account=` URL query
 * parameter and otherwise defaults to the first account). On fetch failure
 * (or an empty list) the picker is left showing only its placeholder option
 * and the "select an account" message stays visible.
 */
async function populateAccountPicker(): Promise<void> {
  const picker = getAccountPicker();
  if (!picker) return;

  let accounts: string[];
  try {
    accounts = await getFilterValues('accounts');
  } catch {
    updatePlaceholderVisibility(null);
    return;
  }

  const sorted = [...accounts].sort((a, b) => a.localeCompare(b));
  const initial = populateAccountPickerOptions(sorted);
  updatePlaceholderVisibility(initial);
}

/**
 * Component initializers for this page, each returning a promise that
 * resolves once that component's first load has settled (success or
 * failure). The generic KPI/trend/top-resources components and the
 * account-specific breakdowns component register here.
 */
const refreshers: Array<() => Promise<void>> = [
  () => initEntityKpi(accountEntityConfig, updateStatusCurrency),
  () => initEntityTrend(accountEntityConfig, updateStatusCurrency),
  () => initAccountBreakdowns(updateStatusCurrency),
  () => initEntityTopResources(accountEntityConfig, updateStatusCurrency),
];

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();
  initAccountPicker(updatePlaceholderVisibility);

  setLoadingIndicatorVisible(true);
  try {
    await populateAccountPicker();
    await Promise.allSettled(refreshers.map((refresh) => refresh()));
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
