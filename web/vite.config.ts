import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';

const rootDir = fileURLToPath(new URL('.', import.meta.url));

// https://vite.dev/config/
export default defineConfig({
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
      // Four pages: the Overview dashboard (`index.html`), the Cost
      // Explorer (`explorer.html`), the Service Detail drilldown
      // (`service-detail.html`), and the Account Detail drilldown
      // (`account-detail.html`), sharing the same shared/ modules and
      // stylesheet. Vite's default single-entry build only picks up
      // `index.html`, so all four must be listed explicitly here.
      input: {
        index: resolve(rootDir, 'index.html'),
        explorer: resolve(rootDir, 'explorer.html'),
        serviceDetail: resolve(rootDir, 'service-detail.html'),
        accountDetail: resolve(rootDir, 'account-detail.html'),
      },
    },
  },
});
