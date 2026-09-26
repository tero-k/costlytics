/**
 * Per-page Service / Account filters, rendered into the shell's
 * `#page-filters` slot (`appShell.ts`). Each page keeps its own selection:
 * persisted in localStorage per page (`costlytics.filters.<page>.v1`) and
 * mirrored into the URL (`?f_service=EC2&f_service=S3&f_account=…`, URL
 * wins on load).
 *
 * The selection reaches requests through `api.ts`'s
 * `setPageFilterProvider` choke point, and a committed change bubbles a
 * `change` event up to `#page-filters`, which `subscribeToControls`
 * listens on for every page — so no fetching module is page-filter-aware.
 *
 * Option lists come from `getFilterValues` and are reloaded when the
 * source changes; selected values the new source doesn't have are dropped.
 */

import { getFilterValues, setPageFilterProvider, type FilterFields, type FilterValuesDimension } from '../api.ts';
import { createMultiSelect, type MultiSelect } from './multiSelect.ts';
import { updateStatusFilters } from './statusBar.ts';

export type PageFilterKey = 'services' | 'accounts';

interface FilterDef {
  label: string;
  singular: string;
  plural: string;
  param: string;
  dimension: FilterValuesDimension;
}

const DEFS: Record<PageFilterKey, FilterDef> = {
  services: { label: 'Service', singular: 'service', plural: 'services', param: 'f_service', dimension: 'services' },
  accounts: { label: 'Account', singular: 'account', plural: 'accounts', param: 'f_account', dimension: 'accounts' },
};

type Selection = Partial<Record<PageFilterKey, string[]>>;

function storageKey(page: string): string {
  return `costlytics.filters.${page}.v1`;
}

function readStored(page: string): Selection {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(storageKey(page)) ?? '{}');
    return parsed && typeof parsed === 'object' ? (parsed as Selection) : {};
  } catch {
    return {};
  }
}

function writeStored(page: string, selection: Selection): void {
  try {
    localStorage.setItem(storageKey(page), JSON.stringify(selection));
  } catch {
    // Storage unavailable: the filters just aren't remembered.
  }
}

/**
 * URL params win over storage, as a whole: a link carrying any `f_*` param
 * describes the complete filter state, so stored values for other keys
 * aren't mixed in. @internal exported for tests
 */
export function resolveSelection(keys: PageFilterKey[], params: URLSearchParams, stored: Selection): Selection {
  const fromUrl = keys.some((k) => params.has(DEFS[k].param));
  const selection: Selection = {};
  for (const key of keys) {
    const raw = fromUrl ? params.getAll(DEFS[key].param) : stored[key];
    selection[key] = Array.isArray(raw) ? raw.filter((v): v is string => typeof v === 'string' && v !== '') : [];
  }
  return selection;
}

/** e.g. "Filtered: 2 services, 1 account" — empty when nothing is filtered. @internal exported for tests */
export function summarize(selection: Selection): string {
  const parts = (Object.keys(DEFS) as PageFilterKey[])
    .filter((k) => (selection[k]?.length ?? 0) > 0)
    .map((k) => {
      const n = selection[k]!.length;
      return `${n} ${n === 1 ? DEFS[k].singular : DEFS[k].plural}`;
    });
  return parts.length ? `Filtered: ${parts.join(', ')}` : '';
}

/**
 * Renders the given filters into `#page-filters`, restores their selection
 * and registers the request filter provider — synchronously, so call this
 * after `initSourcePicker()` resolves and before any component's first
 * fetch. Option lists load in the background.
 */
export function initPageFilters(keys: PageFilterKey[]): void {
  const container = document.querySelector<HTMLElement>('#page-filters');
  const page = document.getElementById('app')?.dataset.page ?? 'page';
  if (!container || keys.length === 0) return;

  const widgets = new Map<PageFilterKey, MultiSelect>();
  const initial = resolveSelection(keys, new URL(window.location.href).searchParams, readStored(page));
  for (const key of keys) {
    const def = DEFS[key];
    const widget = createMultiSelect({ id: `filter-${key}`, label: def.label, plural: def.plural });
    widget.setSelected(initial[key] ?? []);
    widgets.set(key, widget);
    container.append(widget.element);
  }

  const current = (): Selection => Object.fromEntries([...widgets].map(([k, w]) => [k, w.getSelected()]));

  const record = (): void => {
    const selection = current();
    writeStored(page, selection);
    const url = new URL(window.location.href);
    for (const key of keys) {
      url.searchParams.delete(DEFS[key].param);
      for (const value of selection[key] ?? []) url.searchParams.append(DEFS[key].param, value);
    }
    window.history.replaceState(null, '', url);
    updateStatusFilters(summarize(selection));
  };

  setPageFilterProvider((): Partial<FilterFields> => current());
  // Widgets dispatch `change` only on committed selections (see `multiSelect.ts`).
  container.addEventListener('change', record);
  record();

  const loadOptions = async (): Promise<void> => {
    await Promise.all(
      [...widgets].map(async ([key, widget]) => {
        let values: string[];
        try {
          values = await getFilterValues(DEFS[key].dimension);
        } catch {
          return; // Keep the current selection; it still applies to requests.
        }
        const sorted = [...values].sort((a, b) => a.localeCompare(b));
        if (widget.setOptions(sorted)) widget.element.dispatchEvent(new Event('change', { bubbles: true }));
      }),
    );
  };

  document.querySelector('#source-picker')?.addEventListener('change', () => void loadOptions());
  void loadOptions();
}
