import './style.css';
import { bootstrapEntityDetailPage } from './entityDetailMain.ts';
import type { BreakdownDef } from './entityBreakdowns.ts';

/**
 * Entry point for the Costlytics Service Detail page. All orchestration
 * logic (status bar, picker wiring, the four generic leaf components) lives
 * in the generic `entityDetailMain.ts`'s `bootstrapEntityDetailPage` — this
 * file is just this page's config: which URL param / DOM ids to use, and its
 * breakdown dimensions (plan §26: cost by account, by region, and by charge
 * category — three charts, vs. Account Detail's two).
 */
const SERVICE_BREAKDOWNS: BreakdownDef[] = [
  { containerId: 'service-by-account', dimension: 'account', title: 'Cost by account' },
  { containerId: 'service-by-region', dimension: 'region', title: 'Cost by region' },
  { containerId: 'service-by-category', dimension: 'charge_category', title: 'Cost by charge category' },
];

void bootstrapEntityDetailPage({
  paramName: 'service',
  elementId: 'service-picker',
  placeholderId: 'service-detail-placeholder',
  filterKey: 'services',
  entityNoun: 'service',
  idPrefix: 'service',
  breakdowns: SERVICE_BREAKDOWNS,
});
