import './style.css';
import { bootstrapEntityDetailPage } from './entityDetailMain.ts';
import type { BreakdownDef } from './entityBreakdowns.ts';

/**
 * Entry point for the Costlytics Account Detail page. All orchestration
 * logic (status bar, picker wiring, the four generic leaf components) lives
 * in the generic `entityDetailMain.ts`'s `bootstrapEntityDetailPage` — this
 * file is just this page's config: which URL param / DOM ids to use, and its
 * breakdown dimensions (plan §26: cost by service and by region — two
 * charts, no charge-category breakdown for accounts).
 *
 * Account values are rendered plainly (raw `account_id` strings, as
 * `getFilterValues('accounts')` / `distinct_accounts` currently return — see
 * `crates/data/src/queries/summary.rs`): there is no display-name concept in
 * the backend yet, so this page doesn't invent one.
 */
const ACCOUNT_BREAKDOWNS: BreakdownDef[] = [
  { containerId: 'account-by-service', dimension: 'service', title: 'Cost by service' },
  { containerId: 'account-by-region', dimension: 'region', title: 'Cost by region' },
];

void bootstrapEntityDetailPage({
  paramName: 'account',
  elementId: 'account-picker',
  placeholderId: 'account-detail-placeholder',
  filterValuesDimension: 'accounts',
  buildFilter: (selected) => ({ accounts: [selected] }),
  entityNoun: 'account',
  idPrefix: 'account',
  breakdowns: ACCOUNT_BREAKDOWNS,
});
