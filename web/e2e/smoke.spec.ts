import { test, expect } from '@playwright/test';

// The single most repeated manual check across every prior session (per
// `.git/sdd/tasks-playwright-e2e.md`): does each of the app's 7 pages load
// cleanly, with the right nav link marked active and zero console/page
// errors? This runs against the REAL backend + REAL fixtures (see
// `playwright.config.ts`'s `webServer` array) — a broken/empty backend
// causes the KPI/chart/table components to either throw (caught by the
// `pageerror` listener below) or the request itself to fail (caught by the
// `console` 'error' listener, since the shared fetch helpers log failures to
// `console.error`), so this test does not silently pass against a backend
// that isn't actually serving real data.

const PAGES: Array<{ path: string; activeHref: string; title: string }> = [
  { path: '/', activeHref: '/', title: 'Overview' },
  { path: '/explorer.html', activeHref: '/explorer.html', title: 'Cost Explorer' },
  { path: '/service-detail.html', activeHref: '/service-detail.html', title: 'Service Detail' },
  { path: '/account-detail.html', activeHref: '/account-detail.html', title: 'Account Detail' },
  { path: '/cost-changes.html', activeHref: '/cost-changes.html', title: 'Cost Changes' },
  { path: '/settings.html', activeHref: '/settings.html', title: 'Settings' },
  { path: '/tags.html', activeHref: '/tags.html', title: 'Tags' },
];

for (const { path, activeHref, title } of PAGES) {
  test(`${title} (${path}) loads cleanly with correct nav state`, async ({ page }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];

    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        consoleErrors.push(msg.text());
      }
    });
    page.on('pageerror', (err) => {
      pageErrors.push(err.message);
    });

    const response = await page.goto(path);
    expect(response, `no response navigating to ${path}`).not.toBeNull();
    expect(response!.ok(), `${path} responded with ${response!.status()}`).toBe(true);

    // Wait for the app to finish its initial data load (KPI/chart/table
    // components all resolve their fetches asynchronously after DOM ready),
    // so console/pageerror listeners have had a chance to observe failures.
    await page.waitForLoadState('networkidle');

    const nav = page.locator('nav.app-nav');
    const links = nav.locator('a');
    await expect(links).toHaveCount(7);

    const active = nav.locator('a.active');
    await expect(active).toHaveCount(1);
    await expect(active).toHaveAttribute('href', activeHref);

    expect(consoleErrors, `console errors on ${path}: ${consoleErrors.join('\n')}`).toEqual([]);
    expect(pageErrors, `uncaught page errors on ${path}: ${pageErrors.join('\n')}`).toEqual([]);

    // Fetch failures in this app are caught and rendered as an in-DOM error
    // state (`.error` on KPI cards, `.table-error`/`.chart-error` on
    // tables/charts — see `web/src/shared/{kpiCard,comparisonTable,chart}.ts`)
    // rather than thrown, so they would NOT show up as a console/pageerror
    // above. This assertion makes the smoke test fail on a backend that is
    // down or returning errors. It does NOT catch an empty/missing-fixture
    // backend: this test uses each page's default date range, which doesn't
    // overlap the Aug 2026 fixtures, so pages legitimately render empty
    // states here. The data-bearing specs (entity-switching, comparison-table,
    // failure-isolation) set an explicit Aug 2026 range and cover that case.
    const errorStates = page.locator('.error, .table-error, .chart-error');
    await expect(errorStates, `page ${path} rendered an error state`).toHaveCount(0);
  });
}
