import { defineConfig } from 'vitest/config';

// Separate from vite.config.ts so the multi-page `build.rollupOptions.input`
// config (four HTML entry points) stays untouched by test-only concerns.
export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
  },
});
