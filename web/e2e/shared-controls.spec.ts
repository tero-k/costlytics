import { test, expect, type Page } from '@playwright/test';

/**
 * Source / From / To / Metric are shared across every tab and persisted
 * (`web/src/shared/appState.ts`): a change on one page is what every other
 * page opens with, including after a reload. Each Playwright test gets a
 * fresh browser context, so localStorage starts empty here.
 */

async function setControl(page: Page, selector: string, value: string): Promise<void> {
  const el = page.locator(selector);
  if (selector === '#metric-select') await el.selectOption(value);
  else {
    await el.fill(value);
    await el.dispatchEvent('change');
  }
}

test('range and metric set on Overview carry to Cost Explorer and survive a reload', async ({ page }) => {
  await page.goto('/');
  await page.waitForLoadState('networkidle');
  await setControl(page, '#date-start', '2026-08-01');
  await setControl(page, '#date-end', '2026-08-31');
  await setControl(page, '#metric-select', 'billed');
  await page.waitForLoadState('networkidle');

  // Navigate via the sidebar (no query params on the link).
  await page.locator('nav.app-nav a[href="/explorer.html"]').click();
  await expect(page.locator('#date-start')).toHaveValue('2026-08-01');
  await expect(page.locator('#date-end')).toHaveValue('2026-08-31');
  await expect(page.locator('#metric-select')).toHaveValue('billed');
  expect(new URL(page.url()).searchParams.get('metric')).toBe('billed');

  await page.reload();
  await expect(page.locator('#date-start')).toHaveValue('2026-08-01');
  await expect(page.locator('#metric-select')).toHaveValue('billed');
});

test('URL params win over the stored state', async ({ page }) => {
  await page.goto('/');
  await setControl(page, '#metric-select', 'billed');
  await page.goto('/tags.html?from=2026-07-01&to=2026-07-31&metric=list');
  await expect(page.locator('#date-start')).toHaveValue('2026-07-01');
  await expect(page.locator('#metric-select')).toHaveValue('list');
});

test('a date preset sets both inputs with a single refresh', async ({ page }) => {
  await page.goto('/explorer.html');
  await page.waitForLoadState('networkidle');

  const timeseriesRequests: string[] = [];
  page.on('request', (req) => {
    if (req.url().includes('/api/v1/cost/timeseries')) timeseriesRequests.push(req.url());
  });

  await page.locator('.date-presets [data-preset="last-year"]').click();
  await page.waitForLoadState('networkidle');

  const year = new Date().getFullYear() - 1;
  await expect(page.locator('#date-start')).toHaveValue(`${year}-01-01`);
  await expect(page.locator('#date-end')).toHaveValue(`${year}-12-31`);
  await expect(page.locator('.date-presets [data-preset="last-year"]')).toHaveClass(/active/);
  expect(timeseriesRequests).toHaveLength(1);
});

test('the selected source carries across pages', async ({ page }) => {
  await page.goto('/');
  const picker = page.locator('#source-picker');
  await expect(picker.locator('option[value="local-focus12"]')).toHaveCount(1);
  await picker.selectOption('local-focus12');
  await page.waitForLoadState('networkidle');

  await page.locator('nav.app-nav a[href="/cost-changes.html"]').click();
  await expect(page.locator('#source-picker')).toHaveValue('local-focus12');
});
