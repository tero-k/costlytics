# Costlytics

A self-hosted AWS cost analytics dashboard built in Rust. Costlytics combines efficient data processing with DuckDB and FOCUS 1.2 format support to provide deep insights into AWS spending patterns and trends. The platform delivers real-time analytics, cost breakdowns by dimension, and powerful filtering capabilities for financial optimization.

## Running locally

`config/example.toml` is a documentation example — it points `s3_uri` at
`fixtures/focus12`, which is not checked into the repo. `cargo run -p api`
will start fine, but with no data source registered unless you first generate
that fixture directory yourself, e.g. from a one-off script or test using
`data::fixtures::generate_focus12_fixture`, or point a `[[sources]]` entry at
a real FOCUS 1.2 export directory (layout:
`{base}/BILLING_PERIOD=YYYY-MM/{data.parquet,Manifest.json}`).

The full startup pipeline (partition discovery -> schema detection -> view
registration -> HTTP query) is exercised end-to-end, against a generated
fixture, by `crates/api/tests/http_integration.rs` — run it with:

```
cargo test -p api --test http_integration
```
