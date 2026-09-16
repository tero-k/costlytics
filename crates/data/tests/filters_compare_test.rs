/// Cross-format integration tests for the filter-value lookup queries
/// (`distinct_services()`/`distinct_regions()`) and the `compare()`
/// period-over-period query, added in this session's Tasks 1-3.
///
/// These tests reuse the same architectural-proof pattern as
/// `explorer_test.rs`: fixture generation -> discover_partitions ->
/// detect_schema -> view registration -> repository queries, proving the
/// new query surface works end-to-end against real Parquet fixtures rather
/// than just inline SQL.
///
/// `compare()` operates on ONE `CostRepository` instance (one DuckDB
/// connection / one `normalized_cost` view), so it inherently cannot compare
/// data across two different adapters' views in a single call. To still
/// prove format-agnosticism, `compare_full_pipeline_focus12` and
/// `compare_full_pipeline_cur2` below run the exact same `compare()` code
/// path twice — once against a FOCUS-1.2-backed repository and once against
/// a CUR-2.0-backed repository — each comparing the fixture's real August
/// 2026 data (current period) against a synthetic empty previous period
/// (no matching rows). Both must produce internally-consistent, correct
/// `absolute_change`/`percentage_change` results via the same Rust function.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::{cur2, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{
    generate_cur2_fixture, generate_focus12_fixture, CUR_GOLDEN_TOTAL_AMORTIZED,
    FIXTURE_AMORTIZED_TOTAL_AUG,
};
use data::object_store::LocalObjectStore;
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::schema_detection::detect_schema;
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
/// the fixture data's period.
fn aug_filter() -> domain::filters::CostFilter {
    domain::filters::CostFilter::date_range(
        Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
    )
}

/// A date range with no matching rows in either fixture, used as a
/// synthetic "empty previous period" for `compare()`.
fn empty_previous_filter() -> domain::filters::CostFilter {
    domain::filters::CostFilter::date_range(
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
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
// Test 1: distinct_services()/distinct_regions() full pipeline against
// FOCUS 1.2 fixture
// ---------------------------------------------------------------------------

#[test]
fn filter_values_full_pipeline_focus12() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let repo = build_focus12_repo(&dir);

    // The August partition (the only one registered by build_focus12_repo)
    // has two rows: ServiceName='EC2'/RegionName='us-east-1' and
    // ServiceName='S3'/RegionName='us-east-1' (see
    // `fixtures::generate_august_partition`).
    let services = repo.distinct_services().unwrap();
    assert_eq!(
        services,
        vec!["EC2".to_string(), "S3".to_string()],
        "distinct_services should return the fixture's known service names, sorted"
    );

    let regions = repo.distinct_regions().unwrap();
    assert_eq!(
        regions,
        vec!["us-east-1".to_string()],
        "distinct_regions should return the fixture's known region name"
    );
}

// ---------------------------------------------------------------------------
// Test 2: compare() full pipeline, run against both adapters
// ---------------------------------------------------------------------------

/// `compare()` against a FOCUS-1.2-backed repository: current period is the
/// fixture's real August 2026 data, previous period is a synthetic empty
/// window (no matching rows in the fixture).
#[test]
fn compare_full_pipeline_focus12() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let repo = build_focus12_repo(&dir);

    let current = aug_filter();
    let previous = empty_previous_filter();

    let result = repo.compare(&current, &previous, None).unwrap();
    assert_eq!(result.currency, "USD");
    assert_eq!(result.rows.len(), 1);

    let row = &result.rows[0];
    assert_eq!(row.key, None);
    assert!(
        (row.current - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 1e-6,
        "current: expected {}, got {}",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        row.current
    );
    assert_eq!(row.previous, 0.0, "previous period has no data");
    assert!(
        (row.absolute_change - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 1e-6,
        "absolute_change should equal current when previous is 0"
    );
    assert_eq!(
        row.percentage_change, None,
        "percentage_change should be None when previous total is 0"
    );
}

/// `compare()` against a CUR-2.0-backed repository, run through the exact
/// same `CostRepository::compare()` code path as
/// `compare_full_pipeline_focus12`, proving format-agnosticism.
#[test]
fn compare_full_pipeline_cur2() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();
    let repo = build_cur2_repo(&dir);

    let current = aug_filter();
    let previous = empty_previous_filter();

    let result = repo.compare(&current, &previous, None).unwrap();
    assert_eq!(result.currency, "USD");
    assert_eq!(result.rows.len(), 1);

    let row = &result.rows[0];
    assert_eq!(row.key, None);
    assert!(
        (row.current - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 1e-6,
        "current: expected {}, got {}",
        CUR_GOLDEN_TOTAL_AMORTIZED,
        row.current
    );
    assert_eq!(row.previous, 0.0, "previous period has no data");
    assert!(
        (row.absolute_change - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 1e-6,
        "absolute_change should equal current when previous is 0"
    );
    assert_eq!(
        row.percentage_change, None,
        "percentage_change should be None when previous total is 0"
    );
}
