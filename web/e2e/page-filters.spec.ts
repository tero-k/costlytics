import { test, expect, type Page, type Request } from '@playwright/test';

/**
 * Per-page Service / Account filters (`web/src/shared/pageFilters.ts`):
 * a committed selection reaches every `/cost/*` request on that page, is
 * reflected in the status bar, and stays independent of other pages.
 */

async function pickService(page: Page, service: string): Promise<void> {
  const filter = page.locator('#filter-services');
  await filter.locator('.ms-trigger').click();
  await filter.locator('.ms-search').fill(service);
  await filter.locator(`input[type="checkbox"][value="${service}"]`).check();
  await filter.locator('.ms-apply').click();
}

function costRequestBodies(page: Page): Array<Record<string, unknown>> {
  const bodies: Array<Record<string, unknown>> = [];
  page.on('request', (req: Request) => {
    if (req.url().includes('/api/v1/cost/') && !req.url().includes('/estimate')) {
      bodies.push(JSON.parse(req.postData() ?? '{}') as Record<string, unknown>);
    }
  });
  return bodies;
}

test('a service filter on Overview scopes every cost request and shows in the status bar', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  // Options load from the demo fixture's service list.
  await page.locator('#filter-services .ms-trigger').click();
  const firstOption = page.locator('#filter-services .ms-options input').first();
  await expect(firstOption).toBeVisible();
  const service = (await firstOption.getAttribute('value'))!;
  await page.keyboard.press('Escape');

  const bodies = costRequestBodies(page);
  await pickService(page, service);
  await page.waitForLoadState('networkidle');

  expect(bodies.length).toBeGreaterThan(0);
  for (const body of bodies) {
    const scoped = 'current' in body ? (body.current as Record<string, unknown>) : body;
    expect(scoped.services).toEqual([service]);
  }
  await expect(page.locator('#status-filters')).toHaveText('Filtered: 1 service');
  await expect(page.locator('#filter-services .ms-chip')).toHaveCount(1);
  expect(new URL(page.url()).searchParams.getAll('f_service')).toEqual([service]);

  // Another page keeps its own (empty) filter.
  await page.locator('nav.app-nav a[href="/explorer.html"]').click();
  await expect(page.locator('#filter-services .ms-all')).toBeVisible();
  await expect(page.locator('#status-filters')).toBeHidden();

  // Coming back restores Overview's filter.
  await page.locator('nav.app-nav a[href="/"]').click();
  await expect(page.locator('#filter-services .ms-chip')).toHaveCount(1);
});

test('Service Detail offers only an account filter', async ({ page }) => {
  await page.goto('/service-detail.html');
  await expect(page.locator('#filter-accounts')).toHaveCount(1);
  await expect(page.locator('#filter-services')).toHaveCount(0);
});
