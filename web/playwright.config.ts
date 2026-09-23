import { defineConfig, devices } from '@playwright/test';

// Design decision (Session 17 Task 1): a SINGLE `npx playwright test` (or
// `npm run test:e2e`) invocation must bring up both the Rust backend
// (`cargo run -p api`, serving on 127.0.0.1:3000 per `config/example.toml`)
// and the Vite frontend dev server (`npm run dev`, serving on
// http://localhost:5173), against real fixture data, with no operator
// pre-steps.
//
// We use Playwright's `webServer` option as an ARRAY of two entries rather
// than a wrapper/global-setup script. As of the installed version
// (`@playwright/test` 1.63.0 — see `node_modules/playwright/types/test.d.ts`,
// which documents `webServer?: TestConfigWebServer | TestConfigWebServer[]`
// with a worked multi-entry example), Playwright natively starts multiple
// servers in parallel, waits for each one's readiness URL, tears both down
// together at the end of the run, and supports a per-entry `cwd` — which is
// exactly what's needed here, since `cargo run -p api` hardcodes the config
// path `config/example.toml` relative to the *workspace root*
// (`crates/api/src/main.rs`), while `npm run dev` must run from `web/`. A
// hand-rolled wrapper script would have to reimplement this readiness
// polling, parallel start, and graceful-teardown logic that Playwright
// already provides, so the array form is strictly simpler here.
//
// The backend entry's command first (re)generates the local fixtures via
// `cargo run -p data --bin generate-fixtures` (idempotent — it regenerates
// deterministic synthetic data each run, so this always leaves a real,
// non-empty backend for the suite to run against, never a stale/missing
// fixture silently producing an empty-but-"passing" smoke test) and then
// starts the API server. Both commands run relative to the workspace root
// (`cwd: '..'`), matching how a human would run them per the README.
//
// `webServer` as an array requires an explicit `baseURL` (see the same type
// doc comment), which we set to the frontend's URL so specs can use
// relative paths like `page.goto('/explorer.html')`.
const CI = !!process.env.CI;

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: CI,
  retries: CI ? 1 : 0,
  workers: CI ? 1 : undefined,
  reporter: 'list',
  use: {
    baseURL: 'http://localhost:5173',
    trace: 'on-first-retry',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: [
    {
      // Repo-root-relative: regenerate fixtures, then serve the API against
      // a SCRATCH COPY of the example config (`target/e2e-settings.toml`).
      // The Settings e2e spec saves/deletes sources through the real
      // backend, which persists those mutations back to whatever file
      // `COSTLYTICS_CONFIG` points at — running against the checked-in
      // `config/example.toml` directly would let the suite modify (and
      // dirty git's view of) that file.
      command:
        'cargo run -p data --bin generate-fixtures && node -e "require(\'fs\').copyFileSync(\'config/example.toml\',\'target/e2e-settings.toml\')" && cargo run -p api',
      cwd: '..',
      env: { COSTLYTICS_CONFIG: 'target/e2e-settings.toml' },
      url: 'http://127.0.0.1:3000/api/v1/sources',
      // First-time debug build + fixture generation can be slow; generous
      // timeout so a cold `cargo` cache doesn't spuriously fail the suite.
      timeout: 300_000,
      reuseExistingServer: !CI,
      stdout: 'pipe',
      stderr: 'pipe',
    },
    {
      command: 'npm run dev',
      cwd: '.',
      url: 'http://localhost:5173',
      timeout: 60_000,
      reuseExistingServer: !CI,
      stdout: 'pipe',
      stderr: 'pipe',
    },
  ],
});
