/// Cross-format integration tests for the Cost Explorer `timeseries()` and
/// `breakdown()` repository queries.
///
/// These tests reuse the same architectural-proof pattern as Session 1's
/// `integration_test.rs` (FOCUS 1.2) and Session 2's `cur2_golden_test.rs`
/// (CUR 2.0): fixture generation -> discover_partitions -> detect_schema ->
/// view registration -> repository queries, proving the explorer endpoints
/// work end-to-end against real Parquet fixtures rather than just inline SQL.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::{cur2, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{
    generate_cur2_fixture, generate_focus12_fixture, CUR_GOLDEN_TOTAL_AMORTIZED,
    CUR_ON_DEMAND_USAGE_AMORTIZED, CUR_RI_DISCOUNTED_USAGE_AMORTIZED,
    CUR_SP_COVERED_USAGE_AMORTIZED, FIXTURE_AMORTIZED_TOTAL_AUG,
};
use data::object_store::LocalObjectStore;
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::schema_detection::detect_schema;
use domain::dimensions::Dimension;
use domain::filters::{CostFilter, TimeGranularity};
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

/// Build a `CostFilter` for August 2026 (inclusive start, exclusive end).
fn aug_filter() -> CostFilter {
    CostFilter::date_range(
        Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    )
}

/// Run the full pipeline (discovery -> schema detection -> view registration)
/// for a FOCUS 1.2 fixture generated into `dir`, returning a repository ready
/// to query.
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

/// Run the full pipeline (discovery -> schema detection -> view registration)
/// for a CUR 2.0 fixture generated into `dir`, returning a repository ready
/// to query.
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
// Test 1: timeseries() full pipeline against FOCUS 1.2 fixture
// ---------------------------------------------------------------------------

#[test]
fn timeseries_full_pipeline_focus12() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let repo = build_focus12_repo(&dir);

    let mut filter = aug_filter();
    filter.granularity = TimeGranularity::Month;

    let points = repo.timeseries(&filter, None).unwrap().points;

    assert_eq!(points.len(), 1, "expected exactly one monthly point");
    assert!(
        (points[0].total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 1e-6,
        "timeseries August total mismatch: expected {}, got {}",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        points[0].total
    );
}

// ---------------------------------------------------------------------------
// Test 2: breakdown() full pipeline against CUR 2.0 fixture
// ---------------------------------------------------------------------------

#[test]
fn breakdown_full_pipeline_cur2() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();
    let repo = build_cur2_repo(&dir);

    let filter = aug_filter();
    let rows = repo
        .breakdown(&filter, Dimension::ChargeCategory, 20)
        .unwrap()
        .rows;

    let total: f64 = rows.iter().map(|r| r.total).sum();
    assert!(
        (total - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 0.01,
        "breakdown rows must sum to CUR_GOLDEN_TOTAL_AMORTIZED: expected {}, got {}",
        CUR_GOLDEN_TOTAL_AMORTIZED,
        total
    );

    // The 'Usage'-classified charge_category rows in the CUR 2.0 adapter's
    // CASE mapping (see adapters::cur2::register_view) are exactly the three
    // scenarios whose line_item_line_item_type is 'Usage', 'DiscountedUsage',
    // or 'SavingsPlanCoveredUsage': on-demand usage, RI-discounted usage, and
    // SP-covered usage. Sum their amortized_cost golden constants precisely.
    let expected_usage_total =
        CUR_ON_DEMAND_USAGE_AMORTIZED + CUR_RI_DISCOUNTED_USAGE_AMORTIZED + CUR_SP_COVERED_USAGE_AMORTIZED;
    assert!(
        (expected_usage_total - 120.0).abs() < 1e-9,
        "sanity check on hand-computed Usage total: {}",
        expected_usage_total
    );

    let usage_row = rows
        .iter()
        .find(|r| r.key.as_deref() == Some("Usage"))
        .expect("expected a 'Usage' charge_category row");
    assert!(
        (usage_row.total - expected_usage_total).abs() < 0.01,
        "'Usage' charge_category total mismatch: expected {} (on-demand + RI-discounted + SP-covered), got {}",
        expected_usage_total,
        usage_row.total
    );
}

// ---------------------------------------------------------------------------
// Test 3: timeseries(), breakdown(), and summary() totals agree
// ---------------------------------------------------------------------------

/// Three different query shapes over the same data must agree: this is a
/// strong correctness cross-check that the aggregation logic (SUM over the
/// metric column, applied in different GROUP BY contexts) is consistent.
#[test]
fn timeseries_and_breakdown_totals_agree_with_summary() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();
    let repo = build_cur2_repo(&dir);

    let filter = aug_filter();

    let summary = repo.summary(&filter).unwrap();

    let mut ts_filter = filter.clone();
    ts_filter.granularity = TimeGranularity::Month;
    let points = repo.timeseries(&ts_filter, None).unwrap().points;
    let timeseries_total: f64 = points.iter().map(|p| p.total).sum();

    let rows = repo
        .breakdown(&filter, Dimension::ChargeCategory, 20)
        .unwrap()
        .rows;
    let breakdown_total: f64 = rows.iter().map(|r| r.total).sum();

    assert!(
        (timeseries_total - summary.total).abs() < 0.01,
        "timeseries total ({}) must agree with summary total ({})",
        timeseries_total,
        summary.total
    );
    assert!(
        (breakdown_total - summary.total).abs() < 0.01,
        "breakdown total ({}) must agree with summary total ({})",
        breakdown_total,
        summary.total
    );
    assert!(
        (timeseries_total - breakdown_total).abs() < 0.01,
        "timeseries total ({}) must agree with breakdown total ({})",
        timeseries_total,
        breakdown_total
    );
    assert!(
        (summary.total - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 0.01,
        "summary total must match the known golden total: expected {}, got {}",
        CUR_GOLDEN_TOTAL_AMORTIZED,
        summary.total
    );
}
