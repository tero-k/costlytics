/**
 * Type-ahead entity picker for dimensions too large to list up front
 * (resource IDs). The counterpart of `dimensionPicker.ts`'s `<select>`
 * picker: same `?{paramName}=` URL round-trip and localStorage fallback,
 * but options come from a debounced server search as the user types.
 *
 * Picking (click / Enter) commits the selection and dispatches one bubbling
 * `change` from the root element — leaf components subscribe to the root's
 * selector via `EntityConfig.pickerSelector`, exactly like a `<select>`.
 * The inner search input's own native `change` (fired on blur) is stopped
 * at the root so it never triggers a refresh.
 */

import { escapeHtml } from './html.ts';

export interface SearchPickerConfig {
  /** Wrapper element id (receives the committed `change` event). */
  rootId: string;
  inputId: string;
  resultsId: string;
  /** URL query parameter, e.g. `resource`. */
  paramName: string;
  /** Runs the search; only called for queries of at least `MIN_QUERY` chars. */
  search: (query: string) => Promise<string[]>;
}

export interface SearchPicker {
  getSelected(): string | null;
  /** Commits a selection (or clears it with `null`) and dispatches `change`. */
  select(value: string | null): void;
}

const DEBOUNCE_MS = 250;
const MIN_QUERY = 2;

export function createSearchPicker(config: SearchPickerConfig): SearchPicker {
  const root = document.getElementById(config.rootId);
  const input = document.getElementById(config.inputId) as HTMLInputElement | null;
  const results = document.getElementById(config.resultsId) as HTMLUListElement | null;
  const storageKey = `costlytics.picker.${config.paramName}.v1`;

  let selected: string | null = null;
  let items: string[] = [];
  let active = -1;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let searchSeq = 0;

  function readStored(): string | null {
    try {
      return localStorage.getItem(storageKey);
    } catch {
      return null;
    }
  }

  function record(value: string | null): void {
    const url = new URL(window.location.href);
    if (value) url.searchParams.set(config.paramName, value);
    else url.searchParams.delete(config.paramName);
    window.history.replaceState(null, '', url);
    try {
      if (value) localStorage.setItem(storageKey, value);
      else localStorage.removeItem(storageKey);
    } catch {
      // Storage unavailable: the selection just isn't remembered.
    }
  }

  function hideResults(): void {
    if (!results || !input) return;
    results.hidden = true;
    input.setAttribute('aria-expanded', 'false');
    active = -1;
  }

  function renderResults(message?: string): void {
    if (!results || !input) return;
    results.innerHTML = message
      ? `<li class="search-note">${escapeHtml(message)}</li>`
      : items
          .map(
            (id, i) =>
              `<li role="option" id="${config.resultsId}-${i}" data-index="${i}" aria-selected="${i === active}"${i === active ? ' class="active"' : ''}>${escapeHtml(id)}</li>`,
          )
          .join('');
    results.hidden = false;
    input.setAttribute('aria-expanded', 'true');
    if (active >= 0) input.setAttribute('aria-activedescendant', `${config.resultsId}-${active}`);
    else input.removeAttribute('aria-activedescendant');
  }

  async function runSearch(query: string): Promise<void> {
    const seq = ++searchSeq;
    renderResults('Searching…');
    try {
      const found = await config.search(query);
      if (seq !== searchSeq) return;
      items = found;
      active = found.length > 0 ? 0 : -1;
      if (found.length === 0) renderResults('No matching resources in this date range.');
      else renderResults();
    } catch {
      if (seq !== searchSeq) return;
      items = [];
      renderResults('Search failed.');
    }
  }

  function select(value: string | null): void {
    selected = value;
    if (input) input.value = value ?? '';
    searchSeq++;
    hideResults();
    record(value);
    root?.dispatchEvent(new Event('change', { bubbles: true }));
  }

  if (root && input && results) {
    root.addEventListener('change', (event) => {
      if (event.target !== root) event.stopPropagation();
    });

    input.addEventListener('input', () => {
      clearTimeout(timer);
      const query = input.value.trim();
      if (query.length < MIN_QUERY) {
        searchSeq++;
        items = [];
        if (query.length === 0) hideResults();
        else renderResults(`Type at least ${MIN_QUERY} characters.`);
        return;
      }
      timer = setTimeout(() => void runSearch(query), DEBOUNCE_MS);
    });

    input.addEventListener('keydown', (event) => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        if (items.length === 0) return;
        event.preventDefault();
        const delta = event.key === 'ArrowDown' ? 1 : -1;
        active = (active + delta + items.length) % items.length;
        renderResults();
      } else if (event.key === 'Enter') {
        event.preventDefault();
        if (!results.hidden && active >= 0 && items[active]) select(items[active]);
        else if (input.value.trim() === '' && selected !== null) select(null);
      } else if (event.key === 'Escape') {
        hideResults();
        input.value = selected ?? '';
      }
    });

    // `mousedown` (not `click`) so the pick lands before the input's blur hides the list.
    results.addEventListener('mousedown', (event) => {
      const li = (event.target as HTMLElement).closest<HTMLLIElement>('li[data-index]');
      if (!li) return;
      event.preventDefault();
      const value = items[Number(li.dataset.index)];
      if (value) select(value);
    });

    input.addEventListener('blur', () => {
      hideResults();
      input.value = selected ?? '';
    });
  }

  // Initial selection: URL param, then the last one picked on this page.
  const initial = new URL(window.location.href).searchParams.get(config.paramName) || readStored();
  if (initial) {
    selected = initial;
    if (input) input.value = initial;
    record(initial);
  }

  return { getSelected: () => selected, select };
}
