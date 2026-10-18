import { test, expect } from '@playwright/test';

// Anomalies inbox against the real harness. `local-anomaly`'s fixture
// (`data::fixtures::anomaly_fixture_rows`, regenerated relative to today on
// every run) yields exactly four anomalies; the harness checks at startup.
// Serial: acknowledge/mute mutate the shared backend's inbox.
test.describe.configure({ mode: 'serial' });

const EC2 = 'Amazon Elastic Compute Cloud';
const SAGEMAKER = 'Amazon SageMaker';

test('lists the fixture anomalies, and only those', async ({ page }) => {
  await page.goto('/anomalies.html');
  const rows = page.locator('#anomaly-list tbody tr');
  await expect(rows).toHaveCount(4);
  await expect(page.locator('#anomaly-list')).toContainText(EC2);
  await expect(page.locator('#anomaly-list')).toContainText(SAGEMAKER);
  await expect(page.locator('#anomaly-list')).toContainText('New spend');
  await expect(page.locator('#anomaly-list')).not.toContainText('AWS Lambda');
  await expect(page.locator('#anomaly-list')).not.toContainText('Amazon Relational Database Service');
  await expect(page.locator('#nav-badge-anomalies')).toHaveText('4');
  await expect(page.locator('#run-status')).toContainText('Local anomaly fixture');
});

test('acknowledge hides the anomaly and decrements the badge', async ({ page }) => {
  await page.goto('/anomalies.html');
  const row = page.locator('#anomaly-list tbody tr', { hasText: EC2 });
  await row.getByRole('button', { name: 'Acknowledge' }).click();
  await expect(page.locator('#anomaly-list tbody tr', { hasText: EC2 })).toHaveCount(0);
  await expect(page.locator('#nav-badge-anomalies')).toHaveText('3');

  await page.locator('#show-all').check();
  await expect(page.locator('#anomaly-list tbody tr', { hasText: EC2 })).toContainText('Acknowledged');
});

test('mute hides a scope until it expires', async ({ page }) => {
  await page.goto('/anomalies.html');
  const row = page.locator('#anomaly-list tbody tr', { hasText: SAGEMAKER });
  await row.locator('select[data-action="mute"]').selectOption('30');
  await expect(page.locator('#anomaly-list tbody tr', { hasText: SAGEMAKER })).toHaveCount(0);
  await expect(page.locator('#nav-badge-anomalies')).toHaveText('2');
});

test('Open links to the detail page for the anomaly dates', async ({ page }) => {
  await page.goto('/anomalies.html');
  await page.locator('#show-all').check();
  const link = page.locator('#anomaly-list tbody tr', { hasText: EC2 }).locator('a.anomaly-open');
  await expect(link).toHaveAttribute('href', /\/service-detail\.html\?service=Amazon\+Elastic\+Compute\+Cloud&from=.*&source=local-anomaly/);
});

test('Check now reports the run', async ({ page }) => {
  await page.goto('/anomalies.html');
  await page.locator('#check-now').click();
  await expect(page.locator('#anomaly-message')).toContainText('Check finished');
});

test('anomaly rules persist and invalid values are rejected', async ({ page }) => {
  await page.goto('/settings.html');
  const minDelta = page.locator('#an-min-delta');
  await expect(minDelta).toHaveValue('20');
  await minDelta.fill('25');
  await page.getByRole('button', { name: 'Save anomaly rules' }).click();
  await expect(page.locator('#anomaly-result')).toContainText('Saved');
  await page.reload();
  await expect(page.locator('#an-min-delta')).toHaveValue('25');

  await page.locator('#an-lookback').fill('5');
  await page.getByRole('button', { name: 'Save anomaly rules' }).click();
  await expect(page.locator('#anomaly-result')).toContainText('lookback_days must be between 14 and 90');

  // Restore the default for later runs against a reused server.
  await page.reload();
  await page.locator('#an-min-delta').fill('20');
  await page.getByRole('button', { name: 'Save anomaly rules' }).click();
  await expect(page.locator('#anomaly-result')).toContainText('Saved');
});
