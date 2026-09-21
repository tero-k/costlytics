import { test, expect, type Page } from '@playwright/test';

/**
 * Regression tests for the exact bug class Session 14 introduced then fixed
 * on Service/Account Detail, and that Session 15/16 had to re-verify by hand
 * repeatedly on Tags: `shared/entityOrchestrator.ts`'s `createEntityOrchestrator`
 * must build each leaf component's `{ refresh }` handle exactly ONCE (at
 * bootstrap), then reuse those same handles — and thus the same
 * `RequestGuard`s and `subscribeToControls` listener registrations — on every
 * later `refreshAll()` call (every source switch, every entity/tag switch).
 * The pre-fix bug re-invoked each leaf's `init*` factory on every
 * `refreshAll()`, which re-registered a whole new set of `change` listeners
 * each time WITHOUT removing the old ones, so the Nth source switch fired N
 * times as many requests as the first. See `shared/entityOrchestrator.ts`'s
 * module doc comment and this file's own "proof of teeth" exercise (see
 * `.git/sdd/task-pw-2-report.md`) for how this suite was confirmed to
 * actually catch that regression.
 *
 * Known fixture numbers below (`local-demo`/EC2/Aug 2026 -> $153.00/3 rows,
 * etc.) were re-derived directly against a running backend
 * (`cargo run -p api` + `cargo run -p data --bin generate-fixtures` against
 * `config/example.toml`) rather than copied from a prior session's report,
 * so they're guaranteed current for this checkout.
 */

const AUG_2026_START = '2026-08-01';
const AUG_2026_END = '2026-08-31';

async function setAugust2026Range(page: Page): Promise<void> {
  await page.locator('#date-start').fill(AUG_2026_START);
  await page.locator('#date-start').dispatchEvent('change');
  await page.locator('#date-end').fill(AUG_2026_END);
  await page.locator('#date-end').dispatchEvent('change');
}

function collectPageErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text());
  });
  page.on('pageerror', (err) => {
    errors.push(err.message);
  });
  return errors;
}

async function assertNoErrorState(page: Page): Promise<void> {
  await expect(page.locator('.error, .table-error, .chart-error')).toHaveCount(0);
}

/**
 * Counts `/api/v1/*` requests fired strictly during `action`, using
 * Playwright's `request` event rather than any manual timing. `action` is
 * responsible for waiting until the triggered work has actually settled
 * (e.g. `page.waitForLoadState('networkidle')`) before returning, so the
 * count is a complete, non-flaky per-switch total rather than a snapshot
 * taken too early.
 */
async function countApiRequestsDuring(page: Page, action: () => Promise<void>): Promise<number> {
  let count = 0;
  const onRequest = (req: { url: () => string }): void => {
    if (req.url().includes('/api/v1/')) count += 1;
  };
  page.on('request', onRequest);
  try {
    await action();
  } finally {
    page.off('request', onRequest);
  }
  return count;
}

// ---------------------------------------------------------------------------
// Service Detail
// ---------------------------------------------------------------------------

test('Service Detail: repeated source switches stay flat and end in the correct state', async ({ page }) => {
  const pageErrors = collectPageErrors(page);

  await page.goto('/service-detail.html');
  await page.waitForLoadState('networkidle');

  const sourcePicker = page.locator('#source-picker');
  const servicePicker = page.locator('#service-picker');

  // Backend default_source_id (config/example.toml's first entry) is
  // `local-demo`, which `initSourcePicker` pre-seeds the URL param from.
  await expect(sourcePicker).toHaveValue('local-demo');

  await setAugust2026Range(page);
  await servicePicker.selectOption('EC2');
  await page.waitForLoadState('networkidle');

  await expect(page.locator('#service-kpi-total .value')).toHaveText('$153.00');
  await expect(page.locator('#service-kpi-rows .value')).toHaveText('3');

  // `local-cur2` has no `EC2` service at all (its services are named
  // differently, e.g. "Amazon Elastic Compute Cloud") -- switching to it
  // with `service=EC2` still in the URL must re-narrow the picker to a
  // value that's actually valid for the new source, not error out or keep
  // showing `local-demo`'s stale EC2 numbers. `local-focus12` DOES have an
  // `EC2` entry, so the picker naturally lands back on "EC2" for it
  // (coincidental, not because the stale selection survived un-validated --
  // confirmed separately by the `local-cur2` leg in between).
  const switches: Array<{ source: string; expectedService: string; expectedTotal: string; expectedRows: string }> = [
    { source: 'local-cur2', expectedService: 'Amazon Elastic Compute Cloud', expectedTotal: '$145.00', expectedRows: '12' },
    { source: 'local-focus12', expectedService: 'EC2', expectedTotal: '$1,034.56', expectedRows: '1' },
    { source: 'local-demo', expectedService: 'EC2', expectedTotal: '$153.00', expectedRows: '3' },
  ];

  const requestCounts: number[] = [];
  for (const { source, expectedService, expectedTotal, expectedRows } of switches) {
    const count = await countApiRequestsDuring(page, async () => {
      await sourcePicker.selectOption(source);
      await page.waitForLoadState('networkidle');
    });
    requestCounts.push(count);

    await expect(sourcePicker).toHaveValue(source);
    await expect(servicePicker).toHaveValue(expectedService);
    await expect(page.locator('#service-kpi-total .value')).toHaveText(expectedTotal);
    await expect(page.locator('#service-kpi-rows .value')).toHaveText(expectedRows);
    await assertNoErrorState(page);
  }

  expect(pageErrors, `console/page errors: ${pageErrors.join('\n')}`).toEqual([]);

  // The property that actually matters (Session 14's regression class): the
  // request count per switch does not GROW across repeated switches. We
  // don't assert a specific historical number (e.g. "15 requests"), just
  // non-growth, so this doesn't need updating every time a component's
  // fetch count changes for unrelated reasons.
  expect(requestCounts[1], `request counts across switches: ${requestCounts.join(', ')}`).toBe(requestCounts[0]);
  expect(requestCounts[2], `request counts across switches: ${requestCounts.join(', ')}`).toBe(requestCounts[0]);
});

// ---------------------------------------------------------------------------
// Account Detail
// ---------------------------------------------------------------------------

test('Account Detail: repeated source switches stay flat and end in the correct state', async ({ page }) => {
  const pageErrors = collectPageErrors(page);

  await page.goto('/account-detail.html');
  await page.waitForLoadState('networkidle');

  const sourcePicker = page.locator('#source-picker');
  const accountPicker = page.locator('#account-picker');

  await expect(sourcePicker).toHaveValue('local-demo');

  await setAugust2026Range(page);
  // `local-demo`'s accounts sort as acct-101, acct-102, acct-103 -- the
  // picker defaults to the first one with no `?account=` param yet.
  await expect(accountPicker).toHaveValue('acct-101');
  await page.waitForLoadState('networkidle');

  await expect(page.locator('#account-kpi-total .value')).toHaveText('$6,583.50');
  await expect(page.locator('#account-kpi-rows .value')).toHaveText('23');

  // `local-cur2` and `local-focus12` each have exactly ONE account, neither
  // of which is `acct-101` -- switching must re-narrow to that single valid
  // account, not keep the stale `local-demo` selection or error.
  const switches: Array<{ source: string; expectedAccount: string }> = [
    { source: 'local-cur2', expectedAccount: '987654321098' },
    { source: 'local-focus12', expectedAccount: 'sub-001' },
    { source: 'local-demo', expectedAccount: 'acct-101' },
  ];

  const requestCounts: number[] = [];
  for (const { source, expectedAccount } of switches) {
    const count = await countApiRequestsDuring(page, async () => {
      await sourcePicker.selectOption(source);
      await page.waitForLoadState('networkidle');
    });
    requestCounts.push(count);

    await expect(sourcePicker).toHaveValue(source);
    await expect(accountPicker).toHaveValue(expectedAccount);
    await assertNoErrorState(page);
  }

  // Back on `local-demo`, the final leg's re-narrowed selection round-trips
  // back to the originally-verified KPI numbers.
  await expect(page.locator('#account-kpi-total .value')).toHaveText('$6,583.50');
  await expect(page.locator('#account-kpi-rows .value')).toHaveText('23');

  expect(pageErrors, `console/page errors: ${pageErrors.join('\n')}`).toEqual([]);
  expect(requestCounts[1], `request counts across switches: ${requestCounts.join(', ')}`).toBe(requestCounts[0]);
  expect(requestCounts[2], `request counts across switches: ${requestCounts.join(', ')}`).toBe(requestCounts[0]);
});

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

test('Tags: source and tag-key/value switches stay flat, correct, and never duplicate options', async ({ page }) => {
  const pageErrors = collectPageErrors(page);

  await page.goto('/tags.html');
  await page.waitForLoadState('networkidle');

  const sourcePicker = page.locator('#source-picker');
  const keyPicker = page.locator('#tag-key-picker');
  const valuePicker = page.locator('#tag-value-picker');

  await expect(sourcePicker).toHaveValue('local-demo');
  // `local-demo`'s tag keys sort as Environment, Team -- defaults to the
  // first with no `?tag_key=` yet, and its values (development, production,
  // staging) default to the first alphabetically.
  await expect(keyPicker).toHaveValue('Environment');
  await expect(valuePicker).toHaveValue('development');

  await setAugust2026Range(page);
  await page.waitForLoadState('networkidle');

  await expect(page.locator('#tags-kpi-total .value')).toHaveText('$6,561.50');
  await expect(page.locator('#tags-kpi-rows .value')).toHaveText('22');

  // --- Tag KEY switch: value picker must be repopulated (no stale/dupe options) ---
  const envOptionCountBefore = await valuePicker.locator('option').count();
  expect(envOptionCountBefore).toBe(4); // placeholder + development/production/staging

  await keyPicker.selectOption('Team');
  await page.waitForLoadState('networkidle');

  // Team's values sort as Data, Frontend, Platform, Security -- picker
  // defaults to the first since the URL's stale `tag_value=development`
  // isn't one of them.
  await expect(valuePicker).toHaveValue('Data');
  const teamOptionCount = await valuePicker.locator('option').count();
  expect(teamOptionCount).toBe(5); // placeholder + Data/Frontend/Platform/Security
  const teamOptionValues = await valuePicker.locator('option').evaluateAll((opts) =>
    opts.map((o) => (o as HTMLOptionElement).value),
  );
  expect(new Set(teamOptionValues).size, 'duplicate <option> values after a key switch').toBe(teamOptionValues.length);
  expect(teamOptionValues.sort()).toEqual(['', 'Data', 'Frontend', 'Platform', 'Security'].sort());

  await expect(page.locator('#tags-kpi-total .value')).toHaveText('$5,571.00');
  await expect(page.locator('#tags-kpi-rows .value')).toHaveText('18');
  await assertNoErrorState(page);

  // Switch the key back and forth a couple more times: still no
  // accumulating/duplicate options, still lands on a valid, non-stale value.
  await keyPicker.selectOption('Environment');
  await page.waitForLoadState('networkidle');
  await expect(valuePicker).toHaveValue('development');
  let optionCount = await valuePicker.locator('option').count();
  expect(optionCount).toBe(4);

  await keyPicker.selectOption('Team');
  await page.waitForLoadState('networkidle');
  await expect(valuePicker).toHaveValue('Data');
  optionCount = await valuePicker.locator('option').count();
  expect(optionCount).toBe(5);
  await assertNoErrorState(page);

  // --- Source switches: local-cur2 has NO tag keys at all for this fixture ---
  const sourceSwitches: Array<{ source: string; expectedKey: string | null; expectedValue: string | null }> = [
    { source: 'local-cur2', expectedKey: null, expectedValue: null },
    { source: 'local-demo', expectedKey: 'Environment', expectedValue: 'development' },
    { source: 'local-cur2', expectedKey: null, expectedValue: null },
  ];

  const requestCounts: number[] = [];
  for (const { source, expectedKey, expectedValue } of sourceSwitches) {
    const count = await countApiRequestsDuring(page, async () => {
      await sourcePicker.selectOption(source);
      await page.waitForLoadState('networkidle');
    });
    requestCounts.push(count);

    await expect(sourcePicker).toHaveValue(source);
    await expect(keyPicker).toHaveValue(expectedKey ?? '');
    await expect(valuePicker).toHaveValue(expectedValue ?? '');

    // Empty-key case: picker should show ONLY its static placeholder option,
    // never a leftover option from the previous source.
    if (expectedKey === null) {
      await expect(keyPicker.locator('option')).toHaveCount(1);
      await expect(valuePicker.locator('option')).toHaveCount(1);
    }

    await assertNoErrorState(page);
  }

  expect(pageErrors, `console/page errors: ${pageErrors.join('\n')}`).toEqual([]);
  // Two of the three switches above are identical repeats of the SAME
  // source-with-no-tag-keys case (local-cur2, first and third): their
  // request counts must be equal to each other -- the flat-count property
  // Session 14's bug would have violated.
  expect(requestCounts[2], `request counts across switches: ${requestCounts.join(', ')}`).toBe(requestCounts[0]);
});
