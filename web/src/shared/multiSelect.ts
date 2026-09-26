/**
 * Searchable multi-select filter widget (per-page Service / Account
 * filters, `pageFilters.ts`).
 *
 * A trigger button shows the selection as chips ("All" when empty); it
 * opens a panel with a search box and a checkbox list. Selections made in
 * the panel are STAGED and committed when the panel closes (outside click,
 * Escape, or Apply), so ticking five services fires one refresh, not five.
 * Removing a chip commits immediately.
 *
 * A commit dispatches one bubbling `change` event from the widget's root.
 * Native `change` events from the inner checkboxes / search input are
 * stopped at the root, so listeners above it (e.g. `subscribeToControls`
 * on `#page-filters`) only ever see committed selections.
 */

import { escapeHtml } from './html.ts';

export interface MultiSelectOptions {
  /** Root element id, e.g. `filter-services`. */
  id: string;
  /** Visible label, e.g. `Service`. */
  label: string;
  /** Plural noun for summaries, e.g. `services`. */
  plural: string;
}

export interface MultiSelect {
  readonly element: HTMLElement;
  /** Replaces the option list. Selected values not in `values` are dropped (returns whether any were). */
  setOptions(values: string[]): boolean;
  getSelected(): string[];
  /** Sets the selection without dispatching `change`. Unknown values are kept until the next `setOptions`. */
  setSelected(values: string[]): void;
}

const MAX_CHIPS = 2;
const MAX_RENDERED_OPTIONS = 300;

/** Case-insensitive substring filter used by the search box. @internal exported for tests */
export function filterOptions(options: string[], query: string): string[] {
  const q = query.trim().toLowerCase();
  return q ? options.filter((o) => o.toLowerCase().includes(q)) : options;
}

export function createMultiSelect(opts: MultiSelectOptions): MultiSelect {
  let options: string[] = [];
  let selected: string[] = [];
  let staged = new Set<string>();

  const root = document.createElement('div');
  root.className = 'multi-select control';
  root.id = opts.id;
  root.innerHTML = `
    <span class="ms-label">${escapeHtml(opts.label)}</span>
    <div class="ms-trigger" role="button" tabindex="0" aria-haspopup="listbox" aria-expanded="false"></div>
    <div class="ms-panel" hidden>
      <input type="search" class="ms-search" placeholder="Search ${escapeHtml(opts.plural)}&hellip;" aria-label="Search ${escapeHtml(opts.plural)}" />
      <ul class="ms-options" role="listbox" aria-multiselectable="true"></ul>
      <div class="ms-footer">
        <button type="button" class="ms-clear">Clear</button>
        <button type="button" class="ms-apply button-primary">Apply</button>
      </div>
    </div>`;

  const trigger = root.querySelector<HTMLElement>('.ms-trigger')!;
  const panel = root.querySelector<HTMLElement>('.ms-panel')!;
  const search = root.querySelector<HTMLInputElement>('.ms-search')!;
  const list = root.querySelector<HTMLUListElement>('.ms-options')!;

  // Only committed selections leave the widget as `change` events.
  root.addEventListener('change', (event) => {
    if (event.target !== root) event.stopPropagation();
  });

  function commit(next: string[]): void {
    const same = next.length === selected.length && next.every((v) => selected.includes(v));
    selected = next;
    renderTrigger();
    if (!same) root.dispatchEvent(new Event('change', { bubbles: true }));
  }

  function renderTrigger(): void {
    if (selected.length === 0) {
      trigger.innerHTML = `<span class="ms-all">All ${escapeHtml(opts.plural)}</span>`;
    } else {
      const chips = selected
        .slice(0, MAX_CHIPS)
        .map(
          (v) =>
            `<span class="ms-chip" title="${escapeHtml(v)}"><span class="ms-chip-text">${escapeHtml(v)}</span><button type="button" class="ms-chip-remove" data-value="${escapeHtml(v)}" aria-label="Remove ${escapeHtml(v)}">&times;</button></span>`,
        )
        .join('');
      const more = selected.length > MAX_CHIPS ? `<span class="ms-more">+${selected.length - MAX_CHIPS}</span>` : '';
      trigger.innerHTML = chips + more;
    }
    root.classList.toggle('has-selection', selected.length > 0);
  }

  function renderOptions(): void {
    const visible = filterOptions(options, search.value);
    // Selected-first so a staged pick stays in view while searching.
    const ordered = [...visible.filter((v) => staged.has(v)), ...visible.filter((v) => !staged.has(v))];
    const shown = ordered.slice(0, MAX_RENDERED_OPTIONS);
    list.innerHTML =
      shown
        .map(
          (v) =>
            `<li role="option" aria-selected="${staged.has(v)}"><label><input type="checkbox" value="${escapeHtml(v)}"${staged.has(v) ? ' checked' : ''} /><span>${escapeHtml(v)}</span></label></li>`,
        )
        .join('') +
      (ordered.length > shown.length
        ? `<li class="ms-note">${ordered.length - shown.length} more &mdash; refine the search</li>`
        : '') +
      (ordered.length === 0 ? `<li class="ms-note">No matches</li>` : '');
  }

  function open(): void {
    if (!panel.hidden) return;
    staged = new Set(selected);
    search.value = '';
    renderOptions();
    panel.hidden = false;
    trigger.setAttribute('aria-expanded', 'true');
    root.classList.add('open');
    search.focus();
    document.addEventListener('pointerdown', onOutside, true);
  }

  function close(apply: boolean): void {
    if (panel.hidden) return;
    panel.hidden = true;
    trigger.setAttribute('aria-expanded', 'false');
    root.classList.remove('open');
    document.removeEventListener('pointerdown', onOutside, true);
    // Keep the committed order stable; append newly staged values.
    if (apply) commit([...selected.filter((v) => staged.has(v)), ...options.filter((v) => staged.has(v) && !selected.includes(v))]);
  }

  function onOutside(event: PointerEvent): void {
    if (!root.contains(event.target as Node)) close(true);
  }

  trigger.addEventListener('click', (event) => {
    const remove = (event.target as HTMLElement).closest<HTMLButtonElement>('.ms-chip-remove');
    if (remove) {
      event.stopPropagation();
      commit(selected.filter((v) => v !== remove.dataset.value));
      return;
    }
    if (panel.hidden) open();
    else close(true);
  });
  trigger.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' || event.key === ' ' || event.key === 'ArrowDown') {
      event.preventDefault();
      open();
    }
  });
  search.addEventListener('input', renderOptions);
  search.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      list.querySelector<HTMLInputElement>('input')?.focus();
    }
  });
  list.addEventListener('change', (event) => {
    const box = event.target as HTMLInputElement;
    if (box.checked) staged.add(box.value);
    else staged.delete(box.value);
    box.closest('li')?.setAttribute('aria-selected', String(box.checked));
  });
  list.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const boxes = Array.from(list.querySelectorAll<HTMLInputElement>('input'));
    const idx = boxes.indexOf(document.activeElement as HTMLInputElement);
    const next = event.key === 'ArrowDown' ? idx + 1 : idx - 1;
    if (next < 0) search.focus();
    else boxes[Math.min(next, boxes.length - 1)]?.focus();
  });
  panel.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      close(false);
      trigger.focus();
    } else if (event.key === 'Enter' && event.target === search) {
      event.preventDefault();
      close(true);
    }
  });
  root.querySelector('.ms-clear')!.addEventListener('click', () => {
    staged.clear();
    renderOptions();
  });
  root.querySelector('.ms-apply')!.addEventListener('click', () => close(true));

  renderTrigger();

  return {
    element: root,
    setOptions(values: string[]): boolean {
      options = values;
      const kept = selected.filter((v) => values.includes(v));
      const dropped = kept.length !== selected.length;
      selected = kept;
      renderTrigger();
      if (!panel.hidden) renderOptions();
      return dropped;
    },
    getSelected: () => [...selected],
    setSelected(values: string[]): void {
      selected = [...values];
      renderTrigger();
    },
  };
}
