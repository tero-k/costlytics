import './style.css';
import { searchResources } from './api.ts';
import { setLoadingIndicatorVisible, updateStatusCurrency } from './shared/statusBar.ts';
import { initAppShell } from './shared/appShell.ts';
import { initPageFilters } from './shared/pageFilters.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';
import { checkInitialLoad, installCostGuard, type PageQueries } from './shared/costGuard.ts';
import { readControls } from './shared/controls.ts';
import { addDaysIso } from './shared/dates.ts';
import { createSearchPicker } from './shared/searchPicker.ts';
import { initEntityKpi } from './entityKpi.ts';
import { initEntityTrend } from './entityTrend.ts';
import { initEntityBreakdowns, type BreakdownDef } from './entityBreakdowns.ts';
import { initEntityTopResources } from './entityTopResources.ts';
import type { EntityConfig } from './shared/entityConfig.ts';
import { createEntityOrchestrator } from './shared/entityOrchestrator.ts';

/**
 * Entry point for the Resources drilldown page: one resource's KPIs, trend
 * and breakdowns, reusing the same generic leaf components as Service /
 * Account Detail and Tags.
 *
 * Resource IDs are far too numerous for a `<select>`, so the entity picker
 * is a server-side type-ahead (`shared/searchPicker.ts` →
 * `POST /api/v1/cost/resource-search`, scoped to the current range, metric
 * and page filters). With nothing selected the page instead shows the
 * range's most expensive resources — the generic top-resources table, fed
 * by a second `EntityConfig` that is "selected" exactly when the resource
 * picker is NOT — and its resource links pick in-page rather than
 * navigating (the same links navigate here from the other drilldowns).
 */

const RESOURCE_BREAKDOWNS: BreakdownDef[] = [
  { containerId: 'resource-by-category', dimension: 'charge_category', title: 'Cost by charge category' },
  { containerId: 'resource-by-pricing', dimension: 'pricing_category', title: 'Cost by pricing category' },
  { containerId: 'resource-by-region', dimension: 'region', title: 'Cost by region' },
];

// Renders the shared controls every leaf component below subscribes to, so
// it must run before they are built.
initAppShell();

const SEARCH_LIMIT = 20;
const PICKER_SELECTOR = '#resource-search';

const picker = createSearchPicker({
  rootId: 'resource-search',
  inputId: 'resource-search-input',
  resultsId: 'resource-search-results',
  paramName: 'resource',
  search: async (q) => {
    const controls = readControls();
    if (!controls) return [];
    return searchResources({
      start: controls.startIso,
      end: addDaysIso(controls.endIsoInclusive, 1),
      metric: controls.metric,
      q,
      limit: SEARCH_LIMIT,
    });
  },
});

const resourceConfig: EntityConfig = {
  entityNoun: 'resource',
  idPrefix: 'resource',
  pickerSelector: PICKER_SELECTOR,
  getSelected: picker.getSelected,
  buildFilter: (selected) => ({ resource_ids: [selected] }),
};

/** "Selected" (with a dummy value) only while no resource is picked, so the browse table goes idle once one is. */
const browseConfig: EntityConfig = {
  entityNoun: 'resource',
  idPrefix: 'resource-browse',
  pickerSelector: PICKER_SELECTOR,
  getSelected: () => (picker.getSelected() === null ? 'all' : null),
  buildFilter: () => ({}),
};

function updateView(): void {
  const selected = picker.getSelected();
  const detail = document.querySelector<HTMLElement>('#resource-selected');
  const browse = document.querySelector<HTMLElement>('#resource-browse-top-resources');
  const idEl = document.querySelector<HTMLElement>('#resource-selected-id');
  if (detail) detail.hidden = selected === null;
  if (browse) browse.hidden = selected !== null;
  if (idEl) idEl.textContent = selected ?? '';
}

const { refreshAll } = createEntityOrchestrator([
  () => initEntityKpi(resourceConfig, updateStatusCurrency),
  () => initEntityTrend(resourceConfig, updateStatusCurrency),
  () => initEntityBreakdowns(resourceConfig, RESOURCE_BREAKDOWNS, updateStatusCurrency),
  () => initEntityTopResources(browseConfig, updateStatusCurrency),
]);

async function bootstrap(): Promise<void> {
  // The leaf components re-fetch on the picker's `change` themselves; this
  // listener only swaps which half of the page is visible.
  document.querySelector(PICKER_SELECTOR)?.addEventListener('change', updateView);
  document.querySelector('#resource-clear')?.addEventListener('click', () => picker.select(null));
  document.querySelector('#resource-browse-top-resources')?.addEventListener('click', (event) => {
    const link = (event.target as HTMLElement).closest<HTMLAnchorElement>('a.resource-link');
    if (!link?.dataset.resource) return;
    event.preventDefault();
    picker.select(link.dataset.resource);
  });
  updateView();

  await initSourcePicker();
  initPageFilters(['services', 'accounts']);
  // KPI summary, trend, breakdowns' shared summary + one per chart (the
  // browse table only runs while those don't).
  const queries: PageQueries = { current: 3 + RESOURCE_BREAKDOWNS.length, compare: 0 };
  installCostGuard(queries);
  void checkInitialLoad(queries);

  setLoadingIndicatorVisible(true);
  try {
    await refreshAll();
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
