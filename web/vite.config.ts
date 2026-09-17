import { defineConfig } from 'vite';

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
});
