import { test, expect, type Page, type Locator } from '@playwright/test';
import * as fs from 'node:fs';

/**
 * Regression tests for `shared/comparisonTable.ts` (Session 16's
 * consolidated sortable comparison table), exercised through its two page
 * wrappers: Cost Explorer's `#explorer-table` and Cost Changes'
 * `#changes-table`. Covers the three interactive behaviors that have no
 * other automated coverage beyond `comparisonTable.test.ts`'s Vitest unit
 * tests (which exercise the pure sort/Top-N/export functions directly, not
 * the real DOM wiring/click handlers/download flow against a real backend):
 * column-header sort + direction toggle, Top-N "Other" folding, and CSV
 * export producing a real, correct downloaded file.
 *
 * Both pages use `local-demo` (the default source), whose fixture data
 * (`crates/data/src/fixtures.rs::generate_demo_fixture`) lives ENTIRELY in
 * August 2026 (22 services x 3 accounts) -- the default "current month to
 * date" range each page starts with would be empty against `local-demo`, so
 * every test here explicitly sets a date range inside August 2026.
 *
 * The chosen split (current: Aug 17-31, "previous": Aug 2-16) is deliberate,
 * not arbitrary: Explorer auto-derives its "previous" period from
 * `previousPeriod()` (the same-length window immediately preceding
 * "current"), and Cost Changes' previous-period inputs are set to match. If
 * "previous" fell outside August (e.g. July, which has zero fixture data),
 * EVERY row's `previous` would be 0 and EVERY row's `percentage_change`
 * would be `null` -- which would make the sort-by-`previous` and
 * sort-by-`percentageChange` assertions below spuriously pass/fail (an
 * all-tied or all-null column doesn't actually get reordered by
 * `comparisonTable.ts`'s sort, since a comparator that always returns 0
 * leaves `Array.sort`'s stable ordering untouched, and `percentageChange`
 * additionally special-cases `null` rows to always sort last regardless of
 * direction). Keeping both periods inside August gives every column real,
 * distinct, non-null values to actually sort by.
 */

const CURRENT_START = '2026-08-17';
const CURRENT_END = '2026-08-31';
const PREVIOUS_START = '2026-08-02';
const PREVIOUS_END = '2026-08-16';

interface ColumnDef {
  id: string;
  label: string;
}

// Mirrors `shared/comparisonTable.ts`'s `COLUMNS` array (column order = DOM
// `<td>` order, 1-based for `:nth-child`).
const COLUMNS: ColumnDef[] = [
  { id: 'key', label: 'Dimension' },
  { id: 'current', label: 'Cost' },
  { id: 'previous', label: 'Previous Cost' },
  { id: 'absoluteChange', label: 'Difference' },
  { id: 'percentageChange', label: 'Difference %' },
  { id: 'percentageTotal', label: '% Total' },
];

const EXPORT_HEADER = COLUMNS.map((c) => c.label).join(',');

function columnNthChild(colId: string): number {
  const idx = COLUMNS.findIndex((c) => c.id === colId);
  if (idx === -1) throw new Error(`unknown column id: ${colId}`);
  return idx + 1;
}

/**
 * Fills a control and dispatches its `change` event, then waits for the
 * network to go quiet again. Waiting HERE (per field) rather than only once
 * after a batch of fields is deliberate: `page.waitForLoadState('networkidle')`
 * can race a `dispatchEvent()` that fires an async refresh -- if nothing else
 * generates network activity between the dispatch and the wait call, it's
 * possible for the idle check to be satisfied before the triggered fetch has
 * actually reached the network layer, resolving too early against a table
 * still mid-refresh. Waiting after every single field change (proven
 * reliable by `entity-switching.spec.ts`'s equivalent helper) avoids that.
 */
async function fillAndCommit(page: Page, selector: string, value: string): Promise<void> {
  const input = page.locator(selector);
  await input.fill(value);
  await input.dispatchEvent('change');
  await page.waitForLoadState('networkidle');
}

interface PageConfig {
  name: string;
  path: string;
  tableContainerId: string;
  /** Sets the page's date range(s) to the fixed August 2026 window described above. */
  setDateRange: (page: Page) => Promise<void>;
}

const PAGES: PageConfig[] = [
  {
    name: 'Cost Explorer',
    path: '/explorer.html',
    tableContainerId: 'explorer-table',
    setDateRange: async (page) => {
      await fillAndCommit(page, '#date-start', CURRENT_START);
      await fillAndCommit(page, '#date-end', CURRENT_END);
      await page.waitForLoadState('networkidle');
    },
  },
  {
    name: 'Cost Changes',
    path: '/cost-changes.html',
    tableContainerId: 'changes-table',
    setDateRange: async (page) => {
      await fillAndCommit(page, '#date-start', CURRENT_START);
      await fillAndCommit(page, '#date-end', CURRENT_END);
      await fillAndCommit(page, '#prev-date-start', PREVIOUS_START);
      await fillAndCommit(page, '#prev-date-end', PREVIOUS_END);
      await page.waitForLoadState('networkidle');
    },
  },
];

// ---------------------------------------------------------------------------
// DOM read helpers
// ---------------------------------------------------------------------------

/**
 * Waits until the table has actually re-rendered with real rows, as a
 * synchronization barrier more robust than `page.waitForLoadState('networkidle')`
 * alone: under parallel-worker load, it's possible for the 500ms "network
 * quiet" window to elapse and resolve BEFORE a `change`-triggered fetch has
 * actually reached the network layer (observed directly during this test's
 * development -- a one-shot `.count()` right after `networkidle` sometimes
 * read the table mid-"Loading…", not the real, already-fast backend
 * response). `expect(...).not.toHaveCount(0)` retries automatically, so it
 * waits out that race instead of assuming the DOM is already settled.
 */
async function waitForRealRows(container: Locator): Promise<void> {
  await expect(container.locator('tbody tr')).not.toHaveCount(0);
  await expect(container.locator('.table-loading')).toHaveCount(0);
}

async function getColumnTexts(container: Locator, colId: string): Promise<string[]> {
  const nth = columnNthChild(colId);
  return container.locator(`tbody tr td:nth-child(${nth})`).allTextContents();
}

/** Reads the visible sort arrow (`▲`/`▼`) off a column header, or `null` if the column isn't the active sort column. */
async function getHeaderDirection(container: Locator, colId: string): Promise<'asc' | 'desc' | null> {
  const text = await container.locator(`th[data-column="${colId}"]`).textContent();
  if (!text) return null;
  if (text.includes('▲')) return 'asc';
  if (text.includes('▼')) return 'desc';
  return null;
}

/** Parses a formatted currency/percentage cell (e.g. `"$1,234.56"`, `"-$12.30"`, `"12.3%"`) back to a number, or `null` for `"N/A"`. */
function parseCellNumber(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === 'N/A') return null;
  const cleaned = trimmed.replace(/[^0-9.-]/g, '');
  const n = Number.parseFloat(cleaned);
  return Number.isNaN(n) ? null : n;
}

function isNonDecreasing(values: number[]): boolean {
  for (let i = 1; i < values.length; i += 1) {
    if (values[i] < values[i - 1] - 1e-9) return false;
  }
  return true;
}

function isNonIncreasing(values: number[]): boolean {
  for (let i = 1; i < values.length; i += 1) {
    if (values[i] > values[i - 1] + 1e-9) return false;
  }
  return true;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

for (const cfg of PAGES) {
  test.describe(cfg.name, () => {
    test(`${cfg.name}: comparison table renders real rows with no error state`, async ({ page }) => {
      await page.goto(cfg.path);
      await page.waitForLoadState('networkidle');
      await cfg.setDateRange(page);

      const container = page.locator(`#${cfg.tableContainerId}`);
      await waitForRealRows(container);
      await expect(container.locator('.table-error, .table-empty')).toHaveCount(0);

      const rowCount = await container.locator('tbody tr').count();
      expect(rowCount, 'table has no rendered rows').toBeGreaterThan(0);
    });

    test(`${cfg.name}: clicking each sortable column header re-sorts rows, and a second click toggles direction`, async ({
      page,
    }) => {
      await page.goto(cfg.path);
      await page.waitForLoadState('networkidle');
      await cfg.setDateRange(page);

      const container = page.locator(`#${cfg.tableContainerId}`);
      await waitForRealRows(container);
      const rowCount = await container.locator('tbody tr').count();
      // Fewer than 2 rows makes "row order changed" meaningless -- shouldn't
      // happen against the fixed August 2026 range/fixture used here.
      expect(rowCount, 'not enough rows to exercise sorting').toBeGreaterThan(1);

      for (const col of COLUMNS) {
        const beforeKeys = await getColumnTexts(container, 'key');

        await container.locator(`th[data-column="${col.id}"]`).click();

        const dir1 = await getHeaderDirection(container, col.id);
        expect(dir1, `no sort-arrow indicator shown after clicking "${col.label}"`).not.toBeNull();

        const afterFirstKeys = await getColumnTexts(container, 'key');
        expect(afterFirstKeys, `clicking "${col.label}" did not change row order`).not.toEqual(beforeKeys);

        // Verify the CLICKED column's own values are actually ordered
        // correctly for the reported direction, not just "some order
        // changed" -- a real content check, not a proxy.
        const afterFirstColumnTexts = await getColumnTexts(container, col.id);
        if (col.id === 'key') {
          const lower = afterFirstColumnTexts.map((t) => t.toLowerCase());
          const sortedAsc = [...lower].sort((a, b) => a.localeCompare(b));
          expect(lower).toEqual(dir1 === 'asc' ? sortedAsc : [...sortedAsc].reverse());
        } else {
          const numbers = afterFirstColumnTexts.map(parseCellNumber).filter((v): v is number => v !== null);
          expect(dir1 === 'asc' ? isNonDecreasing(numbers) : isNonIncreasing(numbers)).toBe(true);
        }

        // Click the SAME header again: direction must toggle, and (since
        // every column here has real, distinct, non-null values across the
        // August 2026 range used by every test in this file) row order must
        // change again too.
        await container.locator(`th[data-column="${col.id}"]`).click();

        const dir2 = await getHeaderDirection(container, col.id);
        expect(dir2, `sort-arrow disappeared after a second click on "${col.label}"`).not.toBeNull();
        expect(dir2, `direction did not toggle on a second click of "${col.label}"`).not.toBe(dir1);

        const afterSecondKeys = await getColumnTexts(container, 'key');
        expect(afterSecondKeys, `second click on "${col.label}" did not change row order`).not.toEqual(afterFirstKeys);
      }
    });

    test(`${cfg.name}: Top-N control folds extra rows into "Other" and updates the row count`, async ({ page }) => {
      await page.goto(cfg.path);
      await page.waitForLoadState('networkidle');
      await cfg.setDateRange(page);

      const container = page.locator(`#${cfg.tableContainerId}`);
      await waitForRealRows(container);

      // A generous Top N (50) establishes the real distinct-dimension count
      // with no "Other" folding, so the Top-N=2 assertion below isn't
      // hard-coding a fixture number that could silently drift.
      await fillAndCommit(page, '#top-n-input', '50');
      await waitForRealRows(container);

      await expect(container.locator('tr.row-other')).toHaveCount(0);
      const fullRowCount = await container.locator('tbody tr').count();
      expect(fullRowCount, 'need more than 2 distinct rows to exercise Top-N folding').toBeGreaterThan(2);

      await fillAndCommit(page, '#top-n-input', '2');
      await waitForRealRows(container);

      const otherRow = container.locator('tr.row-other');
      await expect(otherRow).toHaveCount(1);
      await expect(otherRow).toContainText('Other');

      const rowCountAtTop2 = await container.locator('tbody tr').count();
      // Top 2 real rows + exactly 1 "Other" row.
      expect(rowCountAtTop2).toBe(3);

      // Raising Top N back past the real count removes "Other" again.
      await fillAndCommit(page, '#top-n-input', '50');
      await waitForRealRows(container);
      await expect(container.locator('tr.row-other')).toHaveCount(0);
      expect(await container.locator('tbody tr').count()).toBe(fullRowCount);
    });

    test(`${cfg.name}: CSV export downloads a file whose content matches the rendered table`, async ({ page }) => {
      await page.goto(cfg.path);
      await page.waitForLoadState('networkidle');
      await cfg.setDateRange(page);

      const container = page.locator(`#${cfg.tableContainerId}`);
      const firstRow = container.locator('tbody tr').first();
      await expect(firstRow).toBeVisible();

      const firstRowLabel = (await firstRow.locator('td').nth(0).textContent())?.trim() ?? '';
      const firstRowCost = (await firstRow.locator('td').nth(1).textContent())?.trim() ?? '';
      expect(firstRowLabel.length, 'first row has no dimension label to cross-check').toBeGreaterThan(0);
      expect(firstRowCost.length, 'first row has no cost value to cross-check').toBeGreaterThan(0);

      const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#csv-export-btn').click()]);

      const downloadPath = await download.path();
      expect(downloadPath, 'download produced no local file path').not.toBeNull();
      const content = fs.readFileSync(downloadPath as string, 'utf-8');
      const lines = content.split('\r\n');

      expect(lines[0]).toBe(EXPORT_HEADER);
      expect(lines.length, 'CSV has no data rows').toBeGreaterThan(1);
      // First data row must correspond to the first RENDERED row (same sort
      // order, no re-fetch/re-sort on export) with matching cell content.
      expect(lines[1]).toContain(firstRowLabel);
      expect(lines[1]).toContain(firstRowCost);
    });
  });
}
