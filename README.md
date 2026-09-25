# Costlytics

A Windows/macOS desktop app (Tauri + Rust) for AWS cost analytics. Costlytics combines efficient data processing with DuckDB and FOCUS 1.2 format support to provide deep insights into AWS spending patterns and trends. The platform delivers real-time analytics, cost breakdowns by dimension, and powerful filtering capabilities for financial optimization.

## Running locally

### 1. Desktop app

Prerequisites: Rust stable, Node 20+, `cargo install tauri-cli --version "^2"`,
and WebView2 on Windows (preinstalled on Windows 11).

For development, run from `crates/app`:

```
cargo tauri dev
```

`cargo tauri build` produces `target/release/bundle/{msi,nsis}` on Windows
and `{dmg,macos}` on macOS. A macOS bundle must be built on a Mac, and
signing/notarization are not configured.

Settings are stored at `<app config dir>/settings.toml` (Windows
`%APPDATA%\app.costlytics.desktop\`, macOS
`~/Library/Application Support/app.costlytics.desktop/`), or wherever
`COSTLYTICS_CONFIG` points.

### 2. Adding an S3 source

- Go to Settings → Add source → S3 bucket.
- Point the URI at the folder containing `BILLING_PERIOD=YYYY-MM/` — for AWS
  Data Exports that's `s3://<bucket>/<prefix>/<export-name>/data`. A
  `Manifest.json` per period is optional; without one, the Parquet files
  under that period are read directly.
- Auth options: AWS profile / credential chain (env vars, `~/.aws`, SSO), or
  an access key. The secret is kept in Windows Credential Manager / macOS
  Keychain, never in `settings.toml`.
- Required IAM: `s3:ListBucket` on the bucket (prefix-scoped is fine) and
  `s3:GetObject` on the prefix.
- On first S3 use, DuckDB downloads the `httpfs`/`aws` extensions (network
  needed, cached in `~/.duckdb/extensions`).
- Data is queried in place over HTTPS range reads, so nothing is copied
  locally. Expect S3 queries to be slower than local ones.

### 3. Fixtures & tests

`config/example.toml` is a documentation example — it points at
`fixtures/focus12` and `fixtures/cur2`, which are not checked into the repo
(`/fixtures/` is gitignored). Generate both with one command from the
workspace root:

```
cargo run -p data --bin generate-fixtures
```

This writes a synthetic FOCUS 1.2 dataset to `fixtures/focus12/` and a
synthetic CUR 2.0 dataset to `fixtures/cur2/` using the same generator
functions (`data::fixtures::generate_focus12_fixture`/`generate_cur2_fixture`)
the test suite relies on, plus a third, larger synthetic FOCUS 1.2 dataset to
`fixtures/demo/` (`data::fixtures::generate_demo_fixture`, not used by any
Rust test) with 22 services, 3 accounts, and some untagged rows — wide
enough to exercise the frontend Overview page's "Other" bucket, top-10
truncation, and `(none)` label against real data. The dev/test HTTP harness,
`cargo run -p api` (binary `costlytics-api`), reads `COSTLYTICS_CONFIG`
(defaulting to `config/example.toml`), which already registers
`local-focus12`, `local-cur2`, and `local-demo` sources pointing at these
directories, so after generating the fixtures all three sources are
immediately queryable. This harness is dev/test-only and is not shipped in
the desktop app; it serves the same `service` code over HTTP that the app
calls in-process via Tauri commands. Alternatively, point a `[[sources]]`
entry at a real FOCUS 1.2 or CUR 2.0 export directory (FOCUS 1.2 layout:
`{base}/BILLING_PERIOD=YYYY-MM/{data.parquet,Manifest.json}`).

The root `cargo test` runs the default members, which exclude the desktop
app crate; `cargo test --workspace` includes it.

Every configured `[[sources]]` entry is either registered successfully at
startup or silently skipped (logged via `tracing::warn!`/`tracing::error!`) —
`GET /api/v1/sources` makes that visible over HTTP as the harness's mirror of
the Settings page's status: it returns every configured source's id, name,
configured type, and one of `{"state": "pending"}` (registration is still in
progress — startup, a save or a reload), `{"state": "registered",
"detected_format", "file_count"}`, or `{"state": "skipped", "reason"}`, plus
`default_source_id`, the source id that requests without an explicit
`source_id` fall back to (the first configured source).

The full startup pipeline (partition discovery -> schema detection -> view
registration -> HTTP query) is exercised end-to-end, against a generated
fixture, by `crates/api/tests/http_integration.rs` — run it with:

```
cargo test -p api --test http_integration
```

The frontend's Vitest suite covers the pure/shared modules in `web/src/shared/*`
(URL/DOM round-trips, XSS/CSV-injection regression pins, date math, formatters)
— run it with:

```
cd web && npm test
```

The Playwright end-to-end suite (`web/e2e/`) runs against the dev/test HTTP
harness (`cargo run -p api`), not the packaged desktop app: it regenerates
the fixtures, copies `config/example.toml` to a scratch file
(`target/e2e-settings.toml`, so the suite's Settings mutations never dirty
the checked-in config), starts the harness against that scratch file and the
Vite dev server itself (reusing them if already running), and covers page
smoke checks, source / entity / tag picker switching, comparison-table sort /
Top-N / CSV export, Settings source management, and per-component failure
isolation. First run needs a Chromium download
(`npx playwright install chromium`); then:

```
cd web && npm run test:e2e
```

If a backend is already listening on port 3000, the suite reuses it and skips
its own fixture regeneration and startup, so stop any old `cargo run -p api`
first to test the current code.

The manual S3 acceptance test (needs a real bucket) is:

```
COSTLYTICS_S3_TEST_URI=s3://… cargo test -p service -- --ignored real_s3
```
