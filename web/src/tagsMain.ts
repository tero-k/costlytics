import './style.css';
import { getFilterValues, getTagValues } from './api.ts';
import { initDateRangeDefaults, initStatusBar, setLoadingIndicatorVisible } from './shared/statusBar.ts';
import { createDimensionPicker } from './shared/dimensionPicker.ts';
import { initSourcePicker } from './shared/sourcePicker.ts';

/**
 * Entry point for the Costlytics Tags drilldown page (plan §26-27's third
 * drilldown page, deferred from Sessions 9-10 — see
 * `.git/sdd/tasks-tags-drilldown.md`'s "Why this session exists").
 *
 * Unlike Service/Account Detail's single flat entity picker, a tag selection
 * is a KEY+VALUE pair, and the value list is conditional on the chosen key
 * (`GET /api/v1/filter-values/tag-values?key=X`), so this page owns TWO
 * independent `DimensionPicker` instances (`shared/dimensionPicker.ts`) —
 * `tag-key-picker` (`?tag_key=`, populated once from `getFilterValues('tag-keys')`)
 * and `tag-value-picker` (`?tag_value=`, REPOPULATED from `getTagValues(key)`
 * every time the key picker's selection changes) — rather than reusing
 * `entityDetailMain.ts`'s single-picker `bootstrapEntityDetailPage`.
 *
 * `dimensionPicker.ts`'s `populateOptions` already clears any options from a
 * prior call before appending the new list (Session 14 fix), so repopulating
 * the value picker on every key change is safe: no accumulating/duplicate
 * `<option>`s.
 *
 * This task builds the page shell and two-level picker only. `#tags-kpi`,
 * `#tags-trend`, `#tags-breakdowns` are empty containers wired up by a later
 * task (Session 15 Task 3), which will build a `tags`-specific `EntityConfig`
 * (`shared/entityConfig.ts`) whose `getSelected()` returns the current tag
 * VALUE and whose `buildFilter` reads the CURRENT key selection at call time
 * (`(selected) => ({ tags: [{ key: keyPicker.getSelected() ?? '', operator:
 * 'eq', values: [selected] }] })`), reusing the same generic
 * `initEntityKpi`/`initEntityTrend`/`initEntityBreakdowns` leaf modules
 * Service/Account Detail already share.
 */

const keyPicker = createDimensionPicker({ paramName: 'tag_key', elementId: 'tag-key-picker' });
const valuePicker = createDimensionPicker({ paramName: 'tag_value', elementId: 'tag-value-picker' });

function updatePlaceholderVisibility(): void {
  const placeholder = document.querySelector<HTMLElement>('#tags-placeholder');
  if (placeholder) placeholder.hidden = keyPicker.getSelected() !== null && valuePicker.getSelected() !== null;
}

/**
 * Populates the tag-value picker from `getTagValues(key)`, sorted
 * alphabetically, honoring `?tag_value=` if it names a value present for
 * this key (else defaulting to the first value). A `null`/absent key (no
 * tag keys available for the active source, or fetch failure) clears the
 * value picker to empty rather than fetching with an invalid key.
 */
async function populateValuePicker(key: string | null): Promise<void> {
  if (key === null) {
    valuePicker.populateOptions([]);
    return;
  }

  let values: string[];
  try {
    values = await getTagValues(key);
  } catch {
    valuePicker.populateOptions([]);
    return;
  }

  const sorted = [...values].sort((a, b) => a.localeCompare(b));
  valuePicker.populateOptions(sorted);
}

/**
 * Populates the tag-key picker from `getFilterValues('tag-keys')`, sorted
 * alphabetically, honoring `?tag_key=` if present (else defaulting to the
 * first key). Returns the resolved initial key, or `null` on fetch failure
 * or an empty key list.
 */
async function populateKeyPicker(): Promise<string | null> {
  let values: string[];
  try {
    values = await getFilterValues('tag-keys');
  } catch {
    return null;
  }

  const sorted = [...values].sort((a, b) => a.localeCompare(b));
  return keyPicker.populateOptions(sorted);
}

/**
 * Re-run when the source picker changes: both the tag-key and tag-value
 * lists are source-relative, so both pickers are fully repopulated (mirrors
 * `entityDetailMain.ts`'s `onSourceChange`, minus the leaf-component
 * `refreshAll()` this task doesn't yet have).
 */
async function onSourceChange(): Promise<void> {
  const key = await populateKeyPicker();
  await populateValuePicker(key);
  updatePlaceholderVisibility();
}

async function bootstrap(): Promise<void> {
  initDateRangeDefaults();
  initStatusBar();

  keyPicker.init((value) => {
    void (async () => {
      await populateValuePicker(value);
      updatePlaceholderVisibility();
    })();
  });
  valuePicker.init(updatePlaceholderVisibility);

  await initSourcePicker(onSourceChange);

  setLoadingIndicatorVisible(true);
  try {
    const initialKey = await populateKeyPicker();
    await populateValuePicker(initialKey);
    updatePlaceholderVisibility();
  } finally {
    setLoadingIndicatorVisible(false);
  }
}

void bootstrap();
