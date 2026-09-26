import { test, expect, type Page } from '@playwright/test';

/**
 * Resources drilldown (`web/src/resourceDetailMain.ts`): browse the top
 * resources, search by ID (`POST /api/v1/cost/resource-search`), and open
 * one from another drilldown's Top resources table. Uses the demo
 * fixture's Aug 2026 data (`res-{service}-{account}` resource IDs).
 */

async function setAugust2026(page: Page): Promise<void> {
  for (const [id, value] of [
    ['#date-start', '2026-08-01'],
    ['#date-end', '2026-08-31'],
  ]) {
    await page.locator(id).fill(value);
    await page.locator(id).dispatchEvent('change');
  }
  await page.waitForLoadState('networkidle');
}

test('browse, search and select a resource', async ({ page }) => {
  await page.goto('/resource-detail.html');
  await setAugust2026(page);

  const browse = page.locator('#resource-browse-top-resources');
  await expect(browse.locator('tbody tr').first()).toBeVisible();
  await expect(page.locator('#resource-selected')).toBeHidden();

  const input = page.locator('#resource-search-input');
  await input.fill('res-0-');
  const results = page.locator('#resource-search-results li[data-index]');
  await expect(results.first()).toBeVisible();
  const picked = (await results.first().textContent())!;
  expect(picked).toContain('res-0-');
  await input.press('Enter');

  await expect(page.locator('#resource-selected')).toBeVisible();
  await expect(page.locator('#resource-selected-id')).toHaveText(picked);
  await expect(browse).toBeHidden();
  await expect(page.locator('#resource-kpi-total .value')).not.toHaveText('…');
  await expect(page.locator('.error, .table-error, .chart-error')).toHaveCount(0);
  expect(new URL(page.url()).searchParams.get('resource')).toBe(picked);

  await page.locator('#resource-clear').click();
  await expect(browse).toBeVisible();
  await expect(input).toHaveValue('');
});

test('a Top resources link on Service Detail opens the resource', async ({ page }) => {
  await page.goto('/service-detail.html');
  await setAugust2026(page);

  const link = page.locator('#service-top-resources a.resource-link').first();
  await expect(link).toBeVisible();
  const id = (await link.getAttribute('data-resource'))!;
  await link.click();

  await expect(page).toHaveURL(/resource-detail\.html/);
  await expect(page.locator('#resource-selected-id')).toHaveText(id);
  // The shared range came along.
  await expect(page.locator('#date-start')).toHaveValue('2026-08-01');
});
