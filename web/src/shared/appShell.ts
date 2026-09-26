/**
 * Renders the chrome every page shares — sidebar navigation, the page
 * header with the shared Source / From / To / Metric controls, the quick
 * date-preset chips, and the status bar — from one definition, instead of
 * each HTML page carrying its own copy.
 *
 * A page's HTML provides only:
 *   <div id="app" data-page="overview">
 *     <div class="controls page-controls">…page-only controls…</div>   (optional)
 *     <main>…</main>
 *   </div>
 *
 * Optional `#app` data attributes: `data-shared="false"` (no shared
 * controls/status bar — Settings), `data-from-label` / `data-to-label`
 * (Cost Changes labels its range as the "current" period).
 *
 * The rendered controls keep the ids every other module reads
 * (`#source-picker`, `#date-start`, `#date-end`, `#metric-select`,
 * `#status-*`, `#page-filters`), so `controls.ts`, `costGuard.ts` and the
 * e2e specs work unchanged. `initAppShell()` must run before anything else
 * in a page's bootstrap.
 */

import { initSharedControls } from './appState.ts';
import { DATE_PRESETS, matchingPresets, presetRange, type PresetId } from './datePresets.ts';
import { initStatusBar } from './statusBar.ts';
import { escapeHtml } from './html.ts';
import { appVersionLabel } from './version.ts';

interface NavItem {
  page: string;
  href: string;
  label: string;
  icon: string;
}

interface NavGroup {
  label: string | null;
  items: NavItem[];
}

// 24x24 stroke icons (Lucide-style geometry), drawn with currentColor.
const ICONS = {
  overview: '<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>',
  explorer: '<path d="M3 3v18h18"/><path d="m7 15 4-4 3 3 5-6"/>',
  changes: '<path d="M7 17 17 7"/><path d="M8 7h9v9"/>',
  service: '<path d="M12 2 3 7l9 5 9-5-9-5Z"/><path d="m3 12 9 5 9-5"/><path d="m3 17 9 5 9-5"/>',
  account: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="12" r="2.5"/><path d="M14 10h4M14 14h4"/>',
  resource: '<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2"/>',
  tags: '<path d="M20.6 13.4 13.4 20.6a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8Z"/><circle cx="7.5" cy="7.5" r="1.5"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"/>',
  logo: '<path d="M4 19V9M10 19V5M16 19v-7M22 19H2"/>',
} as const;

export const NAV_GROUPS: NavGroup[] = [
  {
    label: 'Analyze',
    items: [
      { page: 'overview', href: '/', label: 'Overview', icon: ICONS.overview },
      { page: 'explorer', href: '/explorer.html', label: 'Cost Explorer', icon: ICONS.explorer },
      { page: 'cost-changes', href: '/cost-changes.html', label: 'Cost Changes', icon: ICONS.changes },
    ],
  },
  {
    label: 'Drill down',
    items: [
      { page: 'service-detail', href: '/service-detail.html', label: 'Service Detail', icon: ICONS.service },
      { page: 'account-detail', href: '/account-detail.html', label: 'Account Detail', icon: ICONS.account },
      { page: 'resource-detail', href: '/resource-detail.html', label: 'Resources', icon: ICONS.resource },
      { page: 'tags', href: '/tags.html', label: 'Tags', icon: ICONS.tags },
    ],
  },
  {
    label: null,
    items: [{ page: 'settings', href: '/settings.html', label: 'Settings', icon: ICONS.settings }],
  },
];

function svgIcon(paths: string): string {
  return `<svg class="icon" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
}

function renderSidebar(activePage: string): HTMLElement {
  const aside = document.createElement('aside');
  aside.className = 'sidebar';
  const groups = NAV_GROUPS.map((group) => {
    const links = group.items
      .map((item) => {
        const active = item.page === activePage;
        return `<a href="${item.href}"${active ? ' class="active" aria-current="page"' : ''}>${svgIcon(item.icon)}<span>${item.label}</span></a>`;
      })
      .join('');
    const heading = group.label ? `<div class="nav-group-label">${group.label}</div>` : '';
    return `<div class="nav-group${group.label ? '' : ' nav-group-bottom'}">${heading}${links}</div>`;
  }).join('');
  const version = appVersionLabel();
  aside.innerHTML = `
    <a class="brand" href="/">${svgIcon(ICONS.logo)}<span>Costlytics</span></a>
    <nav class="app-nav" aria-label="Main">${groups}</nav>
    <div class="app-version" title="${escapeHtml(version.tooltip)}">${escapeHtml(version.label)}</div>`;
  return aside;
}

function renderSharedControls(app: HTMLElement): string {
  const fromLabel = app.dataset.fromLabel ?? 'From';
  const toLabel = app.dataset.toLabel ?? 'To';
  return `
    <div class="controls shared-controls">
      <label class="control">
        <span>Source</span>
        <select id="source-picker"><option value="" selected>Loading sources&hellip;</option></select>
      </label>
      <label class="control">
        <span>${fromLabel}</span>
        <input type="date" id="date-start" />
      </label>
      <label class="control">
        <span>${toLabel}</span>
        <input type="date" id="date-end" />
      </label>
      <label class="control">
        <span>Metric</span>
        <select id="metric-select">
          <option value="amortized" selected>Amortized</option>
          <option value="billed">Billed</option>
          <option value="list">List</option>
          <option value="contracted">Contracted</option>
        </select>
      </label>
    </div>`;
}

function renderPresets(): string {
  const chips = DATE_PRESETS.map(
    (p) => `<button type="button" class="chip" data-preset="${p.id}" title="${p.title}">${p.label}</button>`,
  ).join('');
  return `<div class="date-presets" role="group" aria-label="Date range presets">${chips}</div>`;
}

const STATUS_BAR = `
  <div class="status-bar" id="status-bar">
    <span class="status-item" id="status-range"></span>
    <span class="status-sep">&middot;</span>
    <span class="status-item" id="status-metric"></span>
    <span class="status-sep">&middot;</span>
    <span class="status-item" id="status-currency">Currency: &hellip;</span>
    <span class="status-sep" id="status-filters-sep" hidden>&middot;</span>
    <span class="status-item status-filters" id="status-filters" hidden></span>
    <span class="status-loading" id="status-loading" hidden>Loading&hellip;</span>
  </div>`;

/**
 * Wires the preset chips. A click sets both date inputs, then dispatches a
 * single `change` — on `#date-start` if it moved, else on `#date-end` — so
 * `costGuard.ts` estimates once and every component re-fetches once (each
 * refresh reads both inputs). The highlighted chip is recomputed only on
 * committed changes, so a range the user cancels in the cost dialog never
 * shows as active.
 */
function initPresets(): void {
  const startInput = document.querySelector<HTMLInputElement>('#date-start');
  const endInput = document.querySelector<HTMLInputElement>('#date-end');
  const chips = Array.from(document.querySelectorAll<HTMLButtonElement>('.date-presets [data-preset]'));
  if (!startInput || !endInput || chips.length === 0) return;

  const highlight = (): void => {
    const active: string[] = matchingPresets(startInput.value, endInput.value, new Date());
    for (const chip of chips) chip.classList.toggle('active', active.includes(chip.dataset.preset ?? ''));
  };

  for (const chip of chips) {
    chip.addEventListener('click', () => {
      const range = presetRange(chip.dataset.preset as PresetId, new Date());
      const startChanged = startInput.value !== range.start;
      const endChanged = endInput.value !== range.end;
      if (!startChanged && !endChanged) return;
      startInput.value = range.start;
      endInput.value = range.end;
      (startChanged ? startInput : endInput).dispatchEvent(new Event('change', { bubbles: true }));
    });
  }
  startInput.addEventListener('change', highlight);
  endInput.addEventListener('change', highlight);
  highlight();
}

/** Renders the shell around `#app` and seeds the shared controls. Call first in every page bootstrap. */
export function initAppShell(): void {
  const app = document.getElementById('app');
  if (!app || app.dataset.shellReady) return;
  app.dataset.shellReady = 'true';

  const page = app.dataset.page ?? '';
  const withShared = app.dataset.shared !== 'false';
  const navItem = NAV_GROUPS.flatMap((g) => g.items).find((item) => item.page === page);

  const layout = document.createElement('div');
  layout.className = 'layout';
  app.before(layout);
  layout.append(renderSidebar(page), app);

  const top = document.createElement('div');
  top.className = 'top-bar';
  top.innerHTML = `
    <header class="app-header">
      <h1>${navItem?.label ?? 'Costlytics'}</h1>
      ${withShared ? renderSharedControls(app) : ''}
    </header>`;

  if (withShared) {
    const toolbar = document.createElement('div');
    toolbar.className = 'toolbar';
    toolbar.innerHTML = renderPresets();
    const pageControls = app.querySelector<HTMLElement>(':scope > .page-controls');
    if (pageControls) toolbar.append(pageControls);
    const filters = document.createElement('div');
    filters.id = 'page-filters';
    filters.className = 'page-filters';
    toolbar.append(filters);
    top.append(toolbar);
  }
  app.prepend(top);

  if (withShared && !document.getElementById('status-bar')) {
    top.insertAdjacentHTML('afterend', STATUS_BAR);
  }

  if (withShared) {
    initSharedControls();
    initStatusBar();
    initPresets();
  }
}
