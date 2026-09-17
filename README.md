# Costlytics

A self-hosted AWS cost analytics dashboard built in Rust. Costlytics combines efficient data processing with DuckDB and FOCUS 1.2 format support to provide deep insights into AWS spending patterns and trends. The platform delivers real-time analytics, cost breakdowns by dimension, and powerful filtering capabilities for financial optimization.

## Running locally

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
truncation, and `(none)` label against real data. `cargo run -p api` loads
`config/example.toml` directly, which already registers `local-focus12`,
`local-cur2`, and `local-demo` sources pointing at these directories, so
after generating the fixtures all three sources are immediately queryable.
Alternatively, point a `[[sources]]` entry at a real FOCUS 1.2 or CUR 2.0
export directory (FOCUS 1.2 layout:
`{base}/BILLING_PERIOD=YYYY-MM/{data.parquet,Manifest.json}`).

The full startup pipeline (partition discovery -> schema detection -> view
registration -> HTTP query) is exercised end-to-end, against a generated
fixture, by `crates/api/tests/http_integration.rs` — run it with:

```
cargo test -p api --test http_integration
```
