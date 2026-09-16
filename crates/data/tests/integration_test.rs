/// End-to-end integration tests for the data crate.
///
/// These tests exercise the full stack:
///   fixture generation → discovery → schema detection → adapter → query
///
/// All tests are offline (no network calls, no AWS credentials required).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::focus12;
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{
    generate_focus12_fixture, FIXTURE_AMORTIZED_TOTAL_AUG, FIXTURE_BILLED_TOTAL_AUG,
};
use data::object_store::LocalObjectStore;
use data::object_store::YearMonth;
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::schema_detection::{detect_schema, DetectedSchema};
use domain::cost::CostMetric;
use domain::filters::CostFilter;
use std::sync::OnceLock;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Ensure the DuckDB parquet extension is installed exactly once for the test
/// binary (it hits disk, not the network, after the first install).
fn ensure_parquet_installed() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
    });
}

/// Build a `CostFilter` for August 2026 (inclusive start, exclusive end).
fn aug_filter() -> CostFilter {
    CostFilter::date_range(
        Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    )
}

/// Generate the FOCUS 1.2 fixture into a TempDir and return the dir (keeps it
/// alive for the duration of the test) together with a list of all Parquet
/// paths (both partitions).
fn setup_fixture() -> (TempDir, Vec<String>) {
    ensure_parquet_installed();
    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    let aug_path = dir
        .path()
        .join("BILLING_PERIOD=2026-08")
        .join("data.parquet")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let sep_path = dir
        .path()
        .join("BILLING_PERIOD=2026-09")
        .join("data.parquet")
        .to_str()
        .unwrap()
        .replace('\\', "/");

    (dir, vec![aug_path, sep_path])
}

/// Build a pool whose in-memory DuckDB instance already has:
///   - the parquet extension loaded
///   - the `normalized_cost` VIEW registered over `files`
///
/// This follows the pattern documented in the task brief: register the view
/// on one connection before handing the pool to the repository, because all
/// pool connections share the same underlying in-memory database.
fn build_test_pool(files: &[String]) -> duckdb_pool::DbPool {
    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus12::register_view(&conn, files).unwrap();
    }
    pool
}

// ---------------------------------------------------------------------------
// Test 1: golden amortized cost for August 2026
// ---------------------------------------------------------------------------

/// Full end-to-end golden-value test.
///
/// Steps: fixture → discovery → schema detection → register_view → summary
/// Assert: total matches `FIXTURE_AMORTIZED_TOTAL_AUG` within 0.01.
#[test]
fn golden_amortized_cost_august() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    // ---- 1. Discovery ----
    let store = LocalObjectStore;
    let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let partitions =
        discover_partitions(&store, dir.path().to_str().unwrap(), start, end).unwrap();

    assert_eq!(partitions.len(), 1, "expected exactly one August partition");
    assert!(
        !partitions[0].files.is_empty(),
        "August partition must have at least one file"
    );
    assert_eq!(
        partitions[0].billing_period,
        YearMonth::new(2026, 8),
        "billing period must be 2026-08"
    );

    // ---- 2. Schema detection ----
    // Open a fresh connection just for schema detection (does not share state
    // with the pool we create below).
    let detect_conn = duckdb::Connection::open_in_memory().unwrap();
    detect_conn.execute_batch("LOAD parquet;").unwrap();
    let detected = detect_schema(&detect_conn, &partitions[0].files[0]).unwrap();
    assert_eq!(
        detected,
        DetectedSchema::Focus12,
        "fixture parquet should be detected as FOCUS 1.2"
    );

    // ---- 3. Register view and query ----
    let pool = build_test_pool(&partitions[0].files);
    let repo = DuckDbCostRepository::new(pool);

    let filter = aug_filter();
    let summary = repo.summary(&filter).unwrap();

    assert_eq!(summary.currency, "USD", "currency must be USD");
    assert_eq!(
        summary.metric,
        CostMetric::Amortized,
        "default metric must be Amortized"
    );
    assert!(
        (summary.total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "golden assertion failed: expected {}, got {}",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        summary.total
    );
}

// ---------------------------------------------------------------------------
// Test 2: metric switch — amortized vs billed differ
// ---------------------------------------------------------------------------

/// Verify that switching from Amortized to Billed yields a different total.
///
/// The fixture has Aug amortized=1234.56 and Aug billed=1100.00 — they must
/// differ.  The test does not hard-code which is larger, only that they differ
/// by more than the floating-point epsilon used elsewhere.
#[test]
fn metric_switch_billed_vs_amortized() {
    let (_dir, files) = setup_fixture();
    let pool = build_test_pool(&files);
    let repo = DuckDbCostRepository::new(pool);

    let amortized_filter = aug_filter();
    let amortized_summary = repo.summary(&amortized_filter).unwrap();

    let mut billed_filter = aug_filter();
    billed_filter.metric = CostMetric::Billed;
    let billed_summary = repo.summary(&billed_filter).unwrap();

    // Sanity-check that each total is individually correct.
    assert!(
        (amortized_summary.total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "amortized total mismatch: expected {}, got {}",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        amortized_summary.total
    );
    assert!(
        (billed_summary.total - FIXTURE_BILLED_TOTAL_AUG).abs() < 0.01,
        "billed total mismatch: expected {}, got {}",
        FIXTURE_BILLED_TOTAL_AUG,
        billed_summary.total
    );

    // The key assertion: amortized and billed must differ.
    assert!(
        (amortized_summary.total - billed_summary.total).abs() > 0.01,
        "amortized ({}) and billed ({}) totals must differ",
        amortized_summary.total,
        billed_summary.total
    );

    // Also verify the metric fields are correctly set on the summaries.
    assert_eq!(amortized_summary.metric, CostMetric::Amortized);
    assert_eq!(billed_summary.metric, CostMetric::Billed);
}

// ---------------------------------------------------------------------------
// Test 3: date filter — August query must not include September data
// ---------------------------------------------------------------------------

/// Confirm that filtering to Aug 2026 excludes September rows.
///
/// The fixture has a September row (EffectiveCost=200.00).  Querying with the
/// August filter must return exactly `FIXTURE_AMORTIZED_TOTAL_AUG` — NOT the
/// combined total that would result if the Sep row leaked in.
#[test]
fn date_filter_excludes_september_data() {
    let (_dir, files) = setup_fixture();
    // Register both partitions in the view so the September data IS present in
    // the database — the filter is the only thing keeping it out.
    let pool = build_test_pool(&files);
    let repo = DuckDbCostRepository::new(pool);

    let filter = aug_filter(); // start=2026-08-01, end=2026-09-01
    let summary = repo.summary(&filter).unwrap();

    let combined_total = FIXTURE_AMORTIZED_TOTAL_AUG + data::fixtures::FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL;

    // Must match August total exactly, not the combined total.
    assert!(
        (summary.total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "August filter returned wrong total: expected {}, got {} \
         (combined would be {})",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        summary.total,
        combined_total
    );

    // Belt-and-suspenders: total must NOT include September data.
    assert!(
        (summary.total - combined_total).abs() > 0.01,
        "September data leaked into August query: total={}, combined={}",
        summary.total,
        combined_total
    );
}

// ---------------------------------------------------------------------------
// Test 4: partitions_for_range returns [YearMonth(2026, 8)] for August range
// ---------------------------------------------------------------------------

/// Unit-level check that `partitions_for_range` maps the August date window to
/// exactly one billing period: August 2026.
///
/// This is already covered by unit tests in `discovery.rs`, but the task brief
/// requires it to appear here as well.
#[test]
fn partitions_for_range_august_only() {
    let start = chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let periods = data::discovery::partitions_for_range(start, end);

    assert_eq!(
        periods,
        vec![YearMonth::new(2026, 8)],
        "range 2026-08-01..2026-09-01 must map to exactly [YearMonth(2026, 8)]"
    );
}
