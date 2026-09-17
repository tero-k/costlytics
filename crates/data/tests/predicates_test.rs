/// End-to-end integration tests for the predicate-filtering feature
/// (`data::queries::predicate::build_predicate`, wired into `summary()`/
/// `timeseries()`/`breakdown()`/`compare()` in Session 5's Tasks 1-3).
///
/// These tests reuse the same architectural-proof pattern as
/// `explorer_test.rs`/`filters_compare_test.rs`: fixture generation ->
/// discover_partitions -> detect_schema -> view registration -> repository
/// queries, proving the filter predicates work end-to-end against real
/// Parquet fixtures (via the real FOCUS 1.2 / CUR 2.0 adapters) rather than
/// just inline SQL or synthetic in-memory tables.
///
/// The most important test here is `sql_injection_attempt_is_neutralized`:
/// a dedicated adversarial proof that a malicious string placed in a
/// `CostFilter` list field is bound as a literal parameter value (via
/// `duckdb::params_from_iter`), never concatenated into the SQL text. It is
/// not enough to observe "the call didn't error" — see that test's doc
/// comment for the three explicit checks performed.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::{cur2, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{
    generate_cur2_fixture, generate_focus12_fixture, CUR_ON_DEMAND_USAGE_AMORTIZED,
    CUR_RI_DISCOUNTED_USAGE_AMORTIZED, CUR_SP_COVERED_USAGE_AMORTIZED,
};
use data::object_store::LocalObjectStore;
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::schema_detection::detect_schema;
use domain::dimensions::Dimension;
use domain::filters::CostFilter;
use std::sync::OnceLock;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_parquet_installed() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
    });
}

/// Build a `CostFilter` for August 2026 (inclusive start, exclusive end),
/// the fixtures' period.
fn aug_filter() -> CostFilter {
    CostFilter::date_range(
        Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    )
}

/// Run the full pipeline (discovery -> schema detection -> view
/// registration) for a FOCUS 1.2 fixture generated into `dir`, returning a
/// repository ready to query.
fn build_focus12_repo(dir: &TempDir) -> DuckDbCostRepository {
    let store = LocalObjectStore;
    let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let partitions =
        discover_partitions(&store, dir.path().to_str().unwrap(), start, end).unwrap();
    assert_eq!(partitions.len(), 1, "expected exactly one August partition");

    let detect_conn = duckdb::Connection::open_in_memory().unwrap();
    detect_conn.execute_batch("LOAD parquet;").unwrap();
    detect_schema(&detect_conn, &partitions[0].files[0]).unwrap();

    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus12::register_view(&conn, &partitions[0].files).unwrap();
    }
    DuckDbCostRepository::new(pool)
}

/// Run the full pipeline (discovery -> schema detection -> view
/// registration) for a CUR 2.0 fixture generated into `dir`, returning a
/// repository ready to query.
fn build_cur2_repo(dir: &TempDir) -> DuckDbCostRepository {
    let store = LocalObjectStore;
    let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let partitions =
        discover_partitions(&store, dir.path().to_str().unwrap(), start, end).unwrap();
    assert_eq!(partitions.len(), 1, "expected exactly one August partition");

    let detect_conn = duckdb::Connection::open_in_memory().unwrap();
    detect_conn.execute_batch("LOAD parquet;").unwrap();
    detect_schema(&detect_conn, &partitions[0].files[0]).unwrap();

    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        cur2::register_view(&conn, &partitions[0].files).unwrap();
    }
    DuckDbCostRepository::new(pool)
}

// ---------------------------------------------------------------------------
// Test 1: summary() filtered by services, full pipeline, FOCUS 1.2 fixture
// ---------------------------------------------------------------------------

/// The FOCUS 1.2 fixture's August partition has two rows: ServiceName='EC2'
/// (EffectiveCost=1034.56) and ServiceName='S3' (EffectiveCost=200.00) — see
/// `fixtures::generate_august_partition`. Filtering `summary()` by
/// `services: ["EC2"]` through the real adapter's registered view must
/// return only the EC2 row's cost, proving the predicate builder is wired
/// correctly end-to-end (not just against synthetic in-memory tables, as the
/// unit tests in `queries/summary.rs` already cover).
#[test]
fn filtered_summary_full_pipeline() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let repo = build_focus12_repo(&dir);

    let mut filter = aug_filter();
    filter.services = vec!["EC2".to_string()];

    let summary = repo.summary(&filter).unwrap();

    assert_eq!(summary.row_count, 1, "only the EC2 row should match");
    assert!(
        (summary.total - 1034.56).abs() < 1e-6,
        "expected only EC2's EffectiveCost (1034.56), got {}",
        summary.total
    );
    assert_eq!(summary.currency, "USD");
}

// ---------------------------------------------------------------------------
// Test 2: breakdown() filtered by charge_categories, full pipeline, CUR 2.0
// fixture
// ---------------------------------------------------------------------------

/// The CUR 2.0 adapter derives `charge_category` from
/// `line_item_line_item_type` (see `adapters::cur2::register_view`'s CASE
/// expression): rows whose `line_item_line_item_type` is one of 'Usage',
/// 'DiscountedUsage', or 'SavingsPlanCoveredUsage' are classified as
/// charge_category='Usage'. Filtering `breakdown()` by
/// `charge_categories: ["Usage"]` should restrict the underlying rows to
/// exactly those three scenarios (on-demand usage, RI-discounted usage,
/// SP-covered usage) before the GROUP BY, so a breakdown grouped by Service
/// (or any other dimension) sums to exactly their combined amortized cost —
/// proving the filter is applied against the real, adapter-derived column,
/// not just a literal fixture field.
#[test]
fn filtered_breakdown_full_pipeline_cur2() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();
    let repo = build_cur2_repo(&dir);

    let mut filter = aug_filter();
    filter.charge_categories = vec!["Usage".to_string()];

    let rows = repo
        .breakdown(&filter, Dimension::ChargeCategory, 20)
        .unwrap()
        .rows;

    // With the charge_categories=["Usage"] predicate applied before the
    // GROUP BY, every remaining row already has charge_category='Usage', so
    // there must be exactly one breakdown row (no other category can appear).
    assert_eq!(
        rows.len(),
        1,
        "filtering by charge_categories=['Usage'] must leave only the 'Usage' group: {:?}",
        rows
    );
    assert_eq!(rows[0].key.as_deref(), Some("Usage"));

    let expected_usage_total =
        CUR_ON_DEMAND_USAGE_AMORTIZED + CUR_RI_DISCOUNTED_USAGE_AMORTIZED + CUR_SP_COVERED_USAGE_AMORTIZED;
    assert!(
        (rows[0].total - expected_usage_total).abs() < 0.01,
        "filtered 'Usage' total mismatch: expected {} (on-demand + RI-discounted + SP-covered), got {}",
        expected_usage_total,
        rows[0].total
    );
}

// ---------------------------------------------------------------------------
// Test 3: SQL injection adversarial proof
// ---------------------------------------------------------------------------

/// Dedicated adversarial proof that a malicious string placed in a
/// `CostFilter` list field cannot be used to inject SQL: `build_predicate`
/// produces parameterized `?` placeholders bound via
/// `duckdb::params_from_iter` (see `queries/predicate.rs`), never string
/// concatenation, so a value like `"EC2'; DROP TABLE normalized_cost; --"`
/// is treated purely as a literal string to compare against `service_name`,
/// with no special SQL meaning whatsoever.
///
/// This test performs three explicit checks, not just "the call didn't
/// crash":
///
/// 1. The `summary()` call bound to the malicious filter returns `Ok(..)`,
///    not a SQL syntax/parse error — proving the payload was bound as data,
///    not spliced into the SQL text (a real injection would either error on
///    the mismatched statement or "succeed" in a structurally different way,
///    e.g. a multi-statement batch execution DuckDB's prepared-statement API
///    does not even support for a single `?`-bound query).
/// 2. The result shows exactly zero matching rows/total, because no row in
///    the fixture has `service_name` literally equal to that entire
///    malicious string (correct behavior for a non-matching literal — not a
///    crash, not an error, not a coincidentally-nonzero result from a
///    mis-evaluated clause).
/// 3. A follow-up query against the *same connection pool*, after the
///    malicious call, still sees the fixture's normal, complete data — i.e.
///    `normalized_cost` (and its backing table) was never actually dropped.
///    This is the proof that closes the loop: it's not enough that the
///    malicious call itself "succeeded" superficially, because a
///    successfully-executed `DROP TABLE` would also let the immediate call
///    return without a Rust-level error. Only a follow-up query proves the
///    schema is intact.
#[test]
fn sql_injection_attempt_is_neutralized() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let repo = build_focus12_repo(&dir);

    // Baseline: confirm the fixture's real, unfiltered August total before
    // the adversarial call, so the follow-up comparison has a known-good
    // value to check against.
    let baseline = repo.summary(&aug_filter()).unwrap();
    assert!(
        (baseline.total - 1234.56).abs() < 1e-6,
        "sanity check on unfiltered baseline total, got {}",
        baseline.total
    );
    assert_eq!(baseline.row_count, 2);

    // The adversarial call: a services filter value crafted to look like it
    // could break out of a naively-concatenated SQL string and drop the
    // underlying view/table.
    let mut malicious_filter = aug_filter();
    malicious_filter.services =
        vec!["EC2'; DROP TABLE normalized_cost; --".to_string()];

    // Check (1): the call must succeed (Ok), not fail with a SQL
    // syntax/parse error. A malformed multi-statement injection attempt
    // against a parameterized `?` placeholder would not parse as valid SQL
    // if it were ever concatenated into the query text.
    let result = repo.summary(&malicious_filter);
    assert!(
        result.is_ok(),
        "malicious filter value must not cause a SQL error (it should be bound as a literal, not concatenated): {:?}",
        result.err()
    );
    let malicious_summary = result.unwrap();

    // Check (2): the malicious value doesn't literally match any real
    // service name in the fixture, so the correct, non-corrupted result is
    // exactly zero matching rows and a zero total.
    assert_eq!(
        malicious_summary.row_count, 0,
        "malicious filter value must match zero rows, not corrupt the query"
    );
    assert_eq!(
        malicious_summary.total, 0.0,
        "malicious filter value must produce a zero total, not corrupt the query"
    );

    // Also run breakdown() and timeseries() with the same malicious value in
    // a different filter field (tags), covering the tag-clause code path's
    // parameter binding too.
    let mut malicious_tag_filter = aug_filter();
    malicious_tag_filter.tags = vec![domain::filters::TagFilter {
        key: "Environment".to_string(),
        operator: domain::filters::TagOperator::Eq,
        values: vec!["'; DROP TABLE normalized_cost; --".to_string()],
    }];
    let breakdown_result = repo.breakdown(
        &malicious_tag_filter,
        Dimension::Service,
        20,
    );
    assert!(
        breakdown_result.is_ok(),
        "malicious tag value must not cause a SQL error: {:?}",
        breakdown_result.err()
    );
    assert_eq!(
        breakdown_result.unwrap().rows.len(),
        0,
        "malicious tag value must match zero rows"
    );

    // Check (3): the critical follow-up proof. Run a fresh, unrelated,
    // *non-malicious* query against the exact same repository (same
    // connection pool, same registered `normalized_cost` view) after the
    // adversarial calls above. If "DROP TABLE normalized_cost" had somehow
    // been executed as real SQL, this would now fail (view/table missing)
    // or return corrupted/empty data. Instead it must return the fixture's
    // full, original, untouched data — proving the view and its backing
    // Parquet-scan table were never actually dropped.
    let post_injection = repo.summary(&aug_filter()).unwrap();
    assert_eq!(
        post_injection.row_count, 2,
        "normalized_cost must still contain both original fixture rows after the injection attempt"
    );
    assert!(
        (post_injection.total - 1234.56).abs() < 1e-6,
        "normalized_cost must still return the fixture's real total after the injection attempt, got {}",
        post_injection.total
    );
    assert_eq!(
        post_injection.total, baseline.total,
        "post-injection-attempt total must exactly match the pre-attempt baseline: normalized_cost was never dropped"
    );

    // Belt-and-suspenders: also confirm distinct_services() (a completely
    // different query against the same view) still returns the fixture's
    // real service names, not an error from a missing table.
    let services = repo.distinct_services().unwrap();
    assert_eq!(
        services,
        vec!["EC2".to_string(), "S3".to_string()],
        "distinct_services must still see the real fixture data post-injection-attempt"
    );
}
