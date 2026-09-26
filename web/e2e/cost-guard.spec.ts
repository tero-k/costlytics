import { test, expect, type Page, type Route } from '@playwright/test';

// S3 cost guard (`src/shared/costGuard.ts`). The fixture sources are local,
// so the real backend always answers `tier: "none"` — the flows below mock
// `/cost/estimate` per page (parallel-safe: no shared settings change) and
// check that the rest of the page reacts correctly. One test also hits the
// real endpoint to pin its contract.

function estimateBody(tier: 'none' | 'soft' | 'hard') {
  return {
    remote: true,
    known: true,
    bytes: 42_000_000_000,
    requests: 12_000,
    usd: tier === 'hard' ? 3.78 : tier === 'soft' ? 0.4 : 0.01,
    tier,
    stats_missing: false,
    soft_limit_usd: 0.1,
    hard_limit_usd: 1,
  };
}

async function mockEstimate(page: Page, tier: () => 'none' | 'soft' | 'hard'): Promise<void> {
  await page.route('**/api/v1/cost/estimate', (route: Route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(estimateBody(tier())) }),
  );
}

/** Fills a date input and fires `change`, without waiting for the network (a dialog may hold it). */
async function commit(page: Page, selector: string, value: string): Promise<void> {
  const input = page.locator(selector);
  await input.fill(value);
  await input.dispatchEvent('change');
}

test('hard tier: Cancel restores the range and fetches nothing', async ({ page }) => {
  let tier: 'none' | 'hard' = 'none';
  await mockEstimate(page, () => tier);
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  const before = await page.locator('#date-start').inputValue();

  let summaryRequests = 0;
  page.on('request', (req) => {
    if (req.url().includes('/api/v1/cost/summary')) summaryRequests++;
  });

  tier = 'hard';
  await commit(page, '#date-start', '2022-01-01');
  const dialog = page.locator('dialog.cost-guard-dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('42.0 GB');
  await expect(dialog).toContainText('$3.78');

  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('#date-start')).toHaveValue(before);
  await page.waitForLoadState('networkidle');
  expect(summaryRequests).toBe(0);
});

test('hard tier: Load anyway fetches and shows the banner', async ({ page }) => {
  let tier: 'none' | 'hard' = 'none';
  await mockEstimate(page, () => tier);
  await page.goto('/');
  await page.waitForLoadState('networkidle');

  tier = 'hard';
  await commit(page, '#date-start', '2022-01-01');
  const summary = page.waitForRequest('**/api/v1/cost/summary');
  await page.locator('dialog.cost-guard-dialog').getByRole('button', { name: 'Load anyway' }).click();
  await summary;
  await expect(page.locator('#date-start')).toHaveValue('2022-01-01');
  await expect(page.locator('#cost-guard-banner')).toContainText('42.0 GB');
});

test('soft tier: loads immediately with a dismissible banner', async ({ page }) => {
  let tier: 'none' | 'soft' = 'none';
  await mockEstimate(page, () => tier);
  await page.goto('/explorer.html');
  await page.waitForLoadState('networkidle');

  tier = 'soft';
  const timeseries = page.waitForRequest('**/api/v1/cost/timeseries');
  await commit(page, '#date-start', '2025-01-01');
  await timeseries;
  await expect(page.locator('dialog.cost-guard-dialog')).toHaveCount(0);
  const banner = page.locator('#cost-guard-banner');
  await expect(banner).toContainText('$0.40');
  await banner.getByRole('button', { name: 'Dismiss' }).click();
  await expect(banner).toHaveCount(0);
});

test('real estimate endpoint: a local fixture source never warns', async ({ request }) => {
  const res = await request.post('http://127.0.0.1:3000/api/v1/cost/estimate', {
    data: { scans: [[{ start: '2026-08-01', end: '2026-09-01' }]] },
  });
  expect(res.ok()).toBe(true);
  const body = await res.json();
  expect(body.remote).toBe(false);
  expect(body.tier).toBe('none');
  expect(body.known).toBe(true);
});
