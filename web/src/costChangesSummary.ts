/**
 * Summary KPI cards for the Cost Changes page (plan §30), rendered into
 * `#changes-summary`.
 *
 * Calls `getCompare()` with NO `dimension`, so the API returns a single
 * aggregate row (`key: null`) covering the whole current-vs-previous period
 * pair — unlike `costChangesMovers.ts`, which requests the same endpoint
 * WITH the page's selected dimension. Uses `shared/kpiCard.ts`'s DOM shell
 * (`.kpi-grid` / `.kpi-card`) rather than inventing a new card style.
 *
 * Renders 4 cards: Current total, Previous total, Change (absolute), and
 * Change % ("N/A" when `percentage_change` is `null`, i.e. previous === 0).
 */

import { getCompare } from './api.ts';
import { addDaysIso } from './shared/dates.ts';
import { formatCurrency, formatPercent, formatSignedCurrency, changeClass, errorMessage } from './shared/format.ts';
import { readChangesControls, type ChangesControls } from './shared/controls.ts';
import { RequestGuard } from './shared/requestGuard.ts';
import { renderKpiShell, setKpiLoading, setKpiError, setKpiValue, type KpiCardDef } from './shared/kpiCard.ts';

const CONTAINER_ID = 'changes-summary';

const CARD_DEFS: KpiCardDef[] = [
  { id: 'changes-kpi-current', label: 'Current period total' },
  { id: 'changes-kpi-previous', label: 'Previous period total' },
  { id: 'changes-kpi-change', label: 'Change' },
  { id: 'changes-kpi-change-pct', label: 'Change %' },
];

const refreshGuard = new RequestGuard();

function getContainer(): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#${CONTAINER_ID}`);
}

// ---------------------------------------------------------------------------
// Fetch + render
// ---------------------------------------------------------------------------

async function loadSummary(
  controls: ChangesControls,
  token: number,
  onCurrency?: (currency: string) => void,
): Promise<void> {
  const allIds = CARD_DEFS.map((c) => c.id);
  allIds.forEach(setKpiLoading);

  // The canonical `end` is exclusive; both date pickers' "To" values are
  // inclusive, so both requests' `*_end` are the day after them.
  const currentStart = controls.startIso;
  const currentEnd = addDaysIso(controls.endIsoInclusive, 1);
  const previousStart = controls.previousStartIso;
  const previousEnd = addDaysIso(controls.previousEndIsoInclusive, 1);

  try {
    const result = await getCompare({
      current_start: currentStart,
      current_end: currentEnd,
      previous_start: previousStart,
      previous_end: previousEnd,
      metric: controls.metric,
      // dimension omitted -> a single aggregate row with key: null
    });

    if (!refreshGuard.isCurrent(token)) return;

    const row = result.rows.find((r) => r.key === null) ?? result.rows[0];
    if (!row) {
      throw new Error('compare() returned no rows');
    }

    onCurrency?.(result.currency);
    setKpiValue('changes-kpi-current', formatCurrency(row.current, result.currency));
    setKpiValue('changes-kpi-previous', formatCurrency(row.previous, result.currency));

    const cls = changeClass(row.absolute_change);
    setKpiValue('changes-kpi-change', formatSignedCurrency(row.absolute_change, result.currency), { cardClass: cls });
    const changePctText = row.percentage_change === null ? 'N/A' : formatPercent(row.percentage_change);
    setKpiValue('changes-kpi-change-pct', changePctText, { cardClass: cls });
  } catch (err) {
    if (!refreshGuard.isCurrent(token)) return;
    const message = errorMessage(err);
    allIds.forEach((id) => setKpiError(id, message));
  }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

export function initChangesSummary(onCurrency?: (currency: string) => void): Promise<void> {
  const container = getContainer();
  if (!container) return Promise.resolve();

  renderKpiShell(container, CARD_DEFS);

  const refresh = async (): Promise<void> => {
    const controls = readChangesControls();
    if (!controls) return;
    const token = refreshGuard.next();
    await loadSummary(controls, token, onCurrency);
  };

  return refresh();
}
