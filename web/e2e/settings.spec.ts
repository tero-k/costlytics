import { test, expect } from '@playwright/test';

// Settings round-trip against the real harness backend: add a local-folder
// source (a second copy of the FOCUS 1.2 fixture), test it, save it, see it
// registered and selectable, then delete it again (two-click confirm).
test('add, test, save and delete a local-folder source', async ({ page }) => {
  await page.goto('/settings.html');
  await page.getByRole('button', { name: 'Add source' }).click();

  await page.getByLabel('Name').fill('E2E Focus Copy');
  await page.getByLabel('Location type').selectOption('local');
  await page.getByLabel('Folder path').fill('fixtures/focus12');

  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect(page.locator('#source-test-result')).toContainText('focus12');

  await page.getByRole('button', { name: 'Save' }).click();
  const row = page.locator('#sources-table tr[data-id="e2e-focus-copy"]');
  await expect(row).toContainText('Registered');

  await page.goto('/?source=e2e-focus-copy');
  await expect(page.locator('#source-picker')).toHaveValue('e2e-focus-copy');

  await page.goto('/settings.html');
  const again = page.locator('#sources-table tr[data-id="e2e-focus-copy"]');
  await again.getByRole('button', { name: 'Delete' }).click();
  await again.getByRole('button', { name: 'Confirm delete' }).click();
  await expect(page.locator('#sources-table tr[data-id="e2e-focus-copy"]')).toHaveCount(0);
});

test('invalid S3 URI is rejected with a readable error', async ({ page }) => {
  await page.goto('/settings.html');
  await page.getByRole('button', { name: 'Add source' }).click();
  await page.getByLabel('Name').fill('Bad');
  await page.getByLabel('S3 URI').fill('s3://');
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.locator('#source-test-result')).toContainText('S3 URI must include a bucket');
});
