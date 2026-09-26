import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const rootDir = fileURLToPath(new URL('.', import.meta.url));

// The Cargo workspace version is the single source of truth for the app
// version (see README "Releasing"); `package.json`'s version is unused.
function cargoWorkspaceVersion(): string {
  const toml = readFileSync(resolve(rootDir, '../Cargo.toml'), 'utf8');
  const section = toml.split(/^\[workspace\.package\]\s*$/m)[1]?.split(/^\[/m)[0] ?? '';
  const match = section.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) throw new Error('version not found in [workspace.package] of ../Cargo.toml');
  return match[1];
}

function gitDescribe(): string {
  try {
    return execSync('git describe --tags --always --dirty', { cwd: rootDir, stdio: ['ignore', 'pipe', 'ignore'] })
      .toString()
      .trim();
  } catch {
    return 'unknown';
  }
}

// https://vite.dev/config/
export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(cargoWorkspaceVersion()),
    __APP_BUILD__: JSON.stringify(gitDescribe()),
  },
  server: {
    proxy: {
      // Same-origin proxy to the Axum backend during `npm run dev`, so
      // `fetch('/api/v1/...')` works without CORS (the backend also has a
      // permissive CORS layer, but same-origin via proxy is simpler).
      '/api': {
        target: 'http://127.0.0.1:3000',
        changeOrigin: true,
      },
    },
  },
  build: {
    rollupOptions: {
      // Eight pages: the Overview dashboard (`index.html`), the Cost
      // Explorer (`explorer.html`), the Service Detail drilldown
      // (`service-detail.html`), the Account Detail drilldown
      // (`account-detail.html`), the Cost Changes page
      // (`cost-changes.html`), the Tags drilldown page (`tags.html`,
      // Session 15 Task 2), the Resources drilldown
      // (`resource-detail.html`), and the Settings page (`settings.html`), which
      // replaced the Data Sources page, sharing the same shared/ modules
      // and stylesheet. Vite's default single-entry build only picks up
      // `index.html`, so all eight must be listed explicitly here.
      input: {
        index: resolve(rootDir, 'index.html'),
        explorer: resolve(rootDir, 'explorer.html'),
        serviceDetail: resolve(rootDir, 'service-detail.html'),
        accountDetail: resolve(rootDir, 'account-detail.html'),
        costChanges: resolve(rootDir, 'cost-changes.html'),
        settings: resolve(rootDir, 'settings.html'),
        tags: resolve(rootDir, 'tags.html'),
        resourceDetail: resolve(rootDir, 'resource-detail.html'),
      },
    },
  },
});
