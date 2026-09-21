import { test, expect, type Page, type Route } from '@playwright/test';

/**
 * Regression tests for the independent-failure-isolation pattern every
 * session since Session 6 has manually re-verified by hand (per
 * `.git/sdd/tasks-playwright-e2e.md`): each page's components fetch
 * independently (`Promise.allSettled`, or each owning its own
 * `RequestGuard`/try-catch), so ONE component's API call failing renders
 * THAT component's own error state (`.error`/`.table-error`/`.chart-error`)
 * without blanking or erroring its siblings on the same page.
 *
 * Each test forces exactly one endpoint (or, for Cost Changes, one specific
 * REQUEST SHAPE on a shared endpoint) to fail via Playwright route
 * interception with a fulfilled 500 response (not `route.abort()` --
 * aborting causes the browser to log its own `net::ERR_FAILED` console
 * error, which would be indistinguishable from a real regression in a
 * console-error-based test; a fulfilled 500 is handled entirely by the
 * app's own try/catch, exactly like a real backend 500 would be).
 *
 * Route interception is registered AFTER the initial page load (once the
 * page is already on a real, non-empty date range) and BEFORE the date
 * range is changed to the fixed August 2026 window every test in this file
 * uses -- see `comparison-table.spec.ts`'s doc comment for why `local-demo`
 * needs an explicit August 2026 range (its fixture data lives entirely in
 * that month). This makes the failure fire during one deterministic,
 * awaited refresh cycle rather than racing the initial bootstrap load.
 */

const DATE_START = '2026-08-17';
const DATE_END = '2026-08-31';

/**
 * Fills a control and dispatches its `change` event, then waits for the
 * network to go quiet. Waiting per-field (not just once after a batch) avoids
 * a race between `dispatchEvent()` firing an async refresh and
 * `waitForLoadState('networkidle')` being satisfied before that refresh's
 * fetch actually reaches the network layer -- see `comparison-table.spec.ts`'s
 * equivalent helper for the full explanation (found by direct observation
 * while developing this suite).
 */
async function fillAndCommit(page: Page, selector: string, value: string): Promise<void> {
  const input = page.locator(selector);
  await input.fill(value);
  await input.dispatchEvent('change');
  await page.waitForLoadState('networkidle');
}

function forcedFailureResponse(route: Route): Promise<void> {
  return route.fulfill({
    status: 500,
    contentType: 'application/json',
    body: JSON.stringify({ error: 'Forced failure for failure-isolation test' }),
  });
}

// ---------------------------------------------------------------------------
// Overview: trend chart (`/cost/timeseries`) vs. KPI cards (`/cost/compare` +
// `/cost/summary`) vs. top-services/top-accounts charts (`/cost/breakdown` +
// `/cost/summary`) -- three genuinely distinct endpoint groups on one page.
// ---------------------------------------------------------------------------

test('Overview: breaking the trend chart request leaves KPI cards and top-breakdown charts unaffected', async ({
  page,
}) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  await page.route('**/api/v1/cost/timeseries', forcedFailureResponse);

  await fillAndCommit(page, '#date-start', DATE_START);
  await fillAndCommit(page, '#date-end', DATE_END);
  await page.waitForLoadState('networkidle');

  // Target: the trend chart shows its error overlay.
  await expect(page.locator('#trend-chart .chart-error')).toHaveCount(1);

  // Siblings: KPI cards (from `/cost/compare` + `/cost/summary`) rendered
  // real values, no error state.
  await expect(page.locator('#overview .kpi-card.error')).toHaveCount(0);
  await expect(page.locator('#kpi-current .value')).not.toHaveText('Error');
  await expect(page.locator('#kpi-current .value')).not.toHaveText('…');

  // Siblings: top-services/top-accounts charts (`/cost/breakdown` +
  // `/cost/summary`) rendered real bar charts, no error overlay.
  await expect(page.locator('#top-services .chart-error, #top-services .chart-empty')).toHaveCount(0);
  await expect(page.locator('#top-accounts .chart-error, #top-accounts .chart-empty')).toHaveCount(0);
  await expect(page.locator('#top-services canvas')).toHaveCount(1);
  await expect(page.locator('#top-accounts canvas')).toHaveCount(1);
});

// ---------------------------------------------------------------------------
// Cost Explorer: comparison table (`/cost/compare`) vs. grouped trend chart
// (`/cost/timeseries`) -- two distinct endpoints on one page.
// ---------------------------------------------------------------------------

test('Cost Explorer: breaking the comparison table request leaves the trend chart unaffected', async ({ page }) => {
  await page.goto('/explorer.html');
  await page.waitForLoadState('networkidle');

  await page.route('**/api/v1/cost/compare', forcedFailureResponse);

  await fillAndCommit(page, '#date-start', DATE_START);
  await fillAndCommit(page, '#date-end', DATE_END);
  await page.waitForLoadState('networkidle');

  // Target: the comparison table shows its error state.
  await expect(page.locator('#explorer-table .table-error')).toHaveCount(1);

  // Sibling: the grouped trend chart (`/cost/timeseries`, untouched) still
  // rendered real data.
  await expect(page.locator('#explorer-trend .chart-error, #explorer-trend .chart-empty')).toHaveCount(0);
  await expect(page.locator('#explorer-trend canvas')).toHaveCount(1);
});

// ---------------------------------------------------------------------------
// Cost Changes: summary cards, biggest-movers lists, and the comparison
// table ALL call the SAME endpoint (`/cost/compare`), differing only in
// whether the request body includes a `dimension` field -- the summary cards
// omit it (a single aggregate row), while movers/table always include it.
// Route interception here inspects the request BODY, not just the URL, to
// isolate the summary cards' specific requests without touching movers/table.
// ---------------------------------------------------------------------------

test('Cost Changes: breaking the summary cards request leaves movers and the comparison table unaffected', async ({
  page,
}) => {
  await page.goto('/cost-changes.html');
  await page.waitForLoadState('networkidle');

  await page.route('**/api/v1/cost/compare', async (route) => {
    let body: unknown;
    try {
      body = route.request().postDataJSON();
    } catch {
      body = undefined;
    }
    const dimension = (body as { dimension?: unknown } | undefined)?.dimension;
    if (dimension === undefined) {
      // The summary cards' aggregate request (no `dimension`) -- fail it.
      await forcedFailureResponse(route);
    } else {
      // Movers'/table's dimension-scoped requests -- pass through untouched.
      await route.continue();
    }
  });

  await fillAndCommit(page, '#date-start', DATE_START);
  await fillAndCommit(page, '#date-end', DATE_END);
  await fillAndCommit(page, '#prev-date-start', '2026-08-02');
  await fillAndCommit(page, '#prev-date-end', '2026-08-16');
  await page.waitForLoadState('networkidle');

  // Target: every summary card shows its error state.
  await expect(page.locator('#changes-summary .kpi-card')).toHaveCount(4);
  await expect(page.locator('#changes-summary .kpi-card.error')).toHaveCount(4);

  // Sibling: biggest movers rendered real increase/decrease entries, no
  // error state.
  await expect(page.locator('#changes-movers .table-error')).toHaveCount(0);
  await expect(page.locator('#changes-movers .movers-item')).not.toHaveCount(0);

  // Sibling: the comparison table rendered real rows, no error state.
  await expect(page.locator('#changes-table .table-error')).toHaveCount(0);
  await expect(page.locator('#changes-table tbody tr')).not.toHaveCount(0);
});
