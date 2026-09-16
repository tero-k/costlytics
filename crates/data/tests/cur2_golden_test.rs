/// Golden per-scenario tests for the CUR 2.0 adapter's `amortized_cost` CASE
/// expression, plus an aggregate golden-total test and a cross-format parity
/// test proving that FOCUS 1.2 and CUR 2.0 produce identical `summary()`
/// results for an equivalent no-discount charge.
///
/// These tests exist because the aggregate total assertions in
/// `crates/data/tests/integration_test.rs` (and in `fixtures.rs`'s own unit
/// tests) sum over all 11 scenario rows — two compensating formula bugs could
/// cancel out and still pass those. This file pins individual scenario rows
/// directly against the `normalized_cost` VIEW.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::{cur2, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{generate_cur2_fixture, CUR_GOLDEN_TOTAL_AMORTIZED, CUR_GOLDEN_TOTAL_BILLED};
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

/// Generate the CUR 2.0 golden fixture into a fresh TempDir and return the
/// dir (keeps it alive for the duration of the test) plus the path to the
/// August partition's single parquet file.
fn setup_cur2_fixture() -> (TempDir, String) {
    ensure_parquet_installed();
    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();

    let aug_path = dir
        .path()
        .join("BILLING_PERIOD=2026-08")
        .join("data.parquet")
        .to_str()
        .unwrap()
        .replace('\\', "/");

    (dir, aug_path)
}

/// Open an in-memory DuckDB connection with parquet loaded and the CUR 2.0
/// `normalized_cost` view registered over the given files.
fn open_conn_with_cur2_view(files: &[String]) -> duckdb::Connection {
    ensure_parquet_installed();
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch("LOAD parquet;").unwrap();
    cur2::register_view(&conn, files).unwrap();
    conn
}

/// Query `billed_cost, amortized_cost` from `normalized_cost` for the single
/// row matching `resource_id`.
fn query_scenario(conn: &duckdb::Connection, resource_id: &str) -> (f64, f64) {
    let mut stmt = conn
        .prepare("SELECT billed_cost, amortized_cost FROM normalized_cost WHERE resource_id = ?")
        .unwrap();
    stmt.query_row([resource_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap_or_else(|e| panic!("no row found for resource_id={resource_id}: {e}"))
}

/// Build a pool whose in-memory DuckDB instance already has the parquet
/// extension loaded and the CUR 2.0 `normalized_cost` VIEW registered.
fn build_cur2_test_pool(files: &[String]) -> duckdb_pool::DbPool {
    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        cur2::register_view(&conn, files).unwrap();
    }
    pool
}

// ---------------------------------------------------------------------------
// Per-scenario golden tests
// ---------------------------------------------------------------------------

/// RI Fee row (`res-ri-fee`): proves the two-term RI fee sum
/// (unused amortized upfront + unused recurring fee).
#[test]
fn cur2_golden_ri_fee() {
    let (_dir, aug_path) = setup_cur2_fixture();
    let conn = open_conn_with_cur2_view(&[aug_path]);

    let (billed, amortized) = query_scenario(&conn, "res-ri-fee");

    assert!(
        (billed - 50.0).abs() < 0.01,
        "res-ri-fee billed_cost: expected 50.0, got {billed}"
    );
    assert!(
        (amortized - 5.0).abs() < 0.01,
        "res-ri-fee amortized_cost: expected 5.0, got {amortized}"
    );
}

/// SP Recurring Fee row (`res-sp-recurring`): proves the commitment-minus-
/// used-commitment subtraction.
#[test]
fn cur2_golden_sp_recurring_fee() {
    let (_dir, aug_path) = setup_cur2_fixture();
    let conn = open_conn_with_cur2_view(&[aug_path]);

    let (billed, amortized) = query_scenario(&conn, "res-sp-recurring");

    assert!(
        (billed - 100.0).abs() < 0.01,
        "res-sp-recurring billed_cost: expected 100.0, got {billed}"
    );
    assert!(
        (amortized - 40.0).abs() < 0.01,
        "res-sp-recurring amortized_cost: expected 40.0, got {amortized}"
    );
}

/// SP Negation row (`res-sp-negation`): proves the zero-out branch — a
/// negative billed cost that must not flow through to amortized_cost.
#[test]
fn cur2_golden_sp_negation() {
    let (_dir, aug_path) = setup_cur2_fixture();
    let conn = open_conn_with_cur2_view(&[aug_path]);

    let (billed, amortized) = query_scenario(&conn, "res-sp-negation");

    assert!(
        (billed - (-30.0)).abs() < 0.01,
        "res-sp-negation billed_cost: expected -30.0, got {billed}"
    );
    assert_eq!(
        amortized, 0.0,
        "res-sp-negation amortized_cost: expected exactly 0.0, got {amortized}"
    );
}

/// RI-Discounted Usage row (`res-ri-discounted`): proves the
/// `reservation_effective_cost` passthrough.
#[test]
fn cur2_golden_ri_discounted_usage() {
    let (_dir, aug_path) = setup_cur2_fixture();
    let conn = open_conn_with_cur2_view(&[aug_path]);

    let (billed, amortized) = query_scenario(&conn, "res-ri-discounted");

    assert_eq!(
        billed, 0.0,
        "res-ri-discounted billed_cost: expected exactly 0.0, got {billed}"
    );
    assert!(
        (amortized - 8.0).abs() < 0.01,
        "res-ri-discounted amortized_cost: expected 8.0, got {amortized}"
    );
}

// ---------------------------------------------------------------------------
// Aggregate golden test (full pipeline: discovery -> schema detection ->
// view registration -> repository.summary())
// ---------------------------------------------------------------------------

/// Full end-to-end golden-value test for CUR 2.0, mirroring
/// `golden_amortized_cost_august` in `integration_test.rs`.
#[test]
fn cur2_golden_total_via_repository() {
    ensure_parquet_installed();

    let dir = TempDir::new().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();

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
    let detect_conn = duckdb::Connection::open_in_memory().unwrap();
    detect_conn.execute_batch("LOAD parquet;").unwrap();
    let detected = detect_schema(&detect_conn, &partitions[0].files[0]).unwrap();
    assert_eq!(
        detected,
        DetectedSchema::Cur2,
        "fixture parquet should be detected as CUR 2.0"
    );

    // ---- 3. Register view and query (Amortized) ----
    let pool = build_cur2_test_pool(&partitions[0].files);
    let repo = DuckDbCostRepository::new(pool);

    let filter = aug_filter();
    let summary = repo.summary(&filter).unwrap();

    assert_eq!(
        summary.metric,
        CostMetric::Amortized,
        "default metric must be Amortized"
    );
    assert!(
        (summary.total - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 0.01,
        "golden amortized assertion failed: expected {}, got {}",
        CUR_GOLDEN_TOTAL_AMORTIZED,
        summary.total
    );

    // ---- 4. Billed metric ----
    let mut billed_filter = aug_filter();
    billed_filter.metric = CostMetric::Billed;
    let billed_summary = repo.summary(&billed_filter).unwrap();

    assert!(
        (billed_summary.total - CUR_GOLDEN_TOTAL_BILLED).abs() < 0.01,
        "golden billed assertion failed: expected {}, got {}",
        CUR_GOLDEN_TOTAL_BILLED,
        billed_summary.total
    );
}

// ---------------------------------------------------------------------------
// Cross-format parity test
// ---------------------------------------------------------------------------

/// The architectural proof: a minimal single-row FOCUS 1.2 fixture and a
/// minimal single-row CUR 2.0 fixture, both representing the exact same
/// real-world charge ("$100 of on-demand EC2 usage, no RI/SP discount
/// involved"), must produce IDENTICAL `summary()` totals via the canonical
/// `normalized_cost` view — for both the Amortized and Billed metrics.
///
/// This is the test that proves the plan's core claim: dashboard queries
/// operate against the canonical model, never directly against CUR or FOCUS
/// column names, so the SAME query code produces the SAME answer regardless
/// of source format.
#[test]
fn cross_format_parity_focus_vs_cur() {
    ensure_parquet_installed();

    // ---- FOCUS 1.2: one row, BilledCost = EffectiveCost = 100.0 (no discount) ----
    let focus_dir = TempDir::new().unwrap();
    let focus_path = focus_dir
        .path()
        .join("focus.parquet")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        let sql = format!(
            r#"COPY (
                SELECT
                    TIMESTAMP '2026-08-01 00:00:00' AS BillingPeriodStart,
                    TIMESTAMP '2026-09-01 00:00:00' AS BillingPeriodEnd,
                    TIMESTAMP '2026-08-10 00:00:00' AS ChargePeriodStart,
                    TIMESTAMP '2026-08-11 00:00:00' AS ChargePeriodEnd,
                    'acct-001'      AS BillingAccountId,
                    'Acct Name'     AS BillingAccountName,
                    'sub-001'       AS SubAccountId,
                    'Sub Name'      AS SubAccountName,
                    'AWS'           AS ProviderName,
                    'Amazon'        AS PublisherName,
                    'EC2'           AS ServiceName,
                    'Compute'       AS ServiceCategory,
                    'VMs'           AS ServiceSubcategory,
                    'us-east-1'     AS RegionName,
                    'us-east-1a'    AS AvailabilityZone,
                    'i-parity'      AS ResourceId,
                    'my-instance'   AS ResourceName,
                    'm5.large'      AS ResourceType,
                    'Usage'         AS ChargeCategory,
                    NULL::VARCHAR   AS ChargeClass,
                    'Recurring'     AS ChargeFrequency,
                    'EC2 usage'     AS ChargeDescription,
                    'OnDemand'      AS PricingCategory,
                    8.0             AS ConsumedQuantity,
                    'Hours'         AS ConsumedUnit,
                    100.0           AS BilledCost,
                    100.0           AS EffectiveCost,
                    100.0           AS ListCost,
                    100.0           AS ContractedCost,
                    'USD'           AS BillingCurrency,
                    NULL::MAP(VARCHAR, VARCHAR) AS Tags,
                    NULL::VARCHAR   AS CommitmentDiscountId,
                    NULL::VARCHAR   AS CommitmentDiscountType,
                    NULL::VARCHAR   AS CommitmentDiscountStatus,
                    'AmazonEC2'     AS x_ServiceCode
            ) TO '{focus_path}' (FORMAT PARQUET)"#
        );
        conn.execute_batch(&sql).unwrap();
    }

    // ---- CUR 2.0: one row, line_item_unblended_cost = line_item_net_unblended_cost = 100.0 ----
    let cur_dir = TempDir::new().unwrap();
    let cur_path = cur_dir
        .path()
        .join("cur.parquet")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        let sql = format!(
            r#"COPY (
                SELECT
                    TIMESTAMP '2026-08-01 00:00:00' AS bill_billing_period_start_date,
                    TIMESTAMP '2026-09-01 00:00:00' AS bill_billing_period_end_date,
                    TIMESTAMP '2026-08-10 00:00:00' AS line_item_usage_start_date,
                    TIMESTAMP '2026-08-11 00:00:00' AS line_item_usage_end_date,
                    '123456789012'  AS bill_payer_account_id,
                    'Payer Name'    AS bill_payer_account_name,
                    '987654321098'  AS line_item_usage_account_id,
                    'Acct Name'     AS line_item_usage_account_name,
                    'Amazon Elastic Compute Cloud' AS product_product_name,
                    'AmazonEC2'     AS product_servicecode,
                    'us-east-1'     AS product_region_code,
                    'us-east-1a'    AS line_item_availability_zone,
                    'i-parity'      AS line_item_resource_id,
                    'm5.large'      AS product_instance_type,
                    'Usage'         AS line_item_line_item_type,
                    'EC2 usage'     AS line_item_line_item_description,
                    8.0             AS line_item_usage_amount,
                    'Hours'         AS pricing_unit,
                    100.0           AS line_item_net_unblended_cost,
                    100.0           AS line_item_unblended_cost,
                    0.0             AS savings_plan_savings_plan_effective_cost,
                    0.0             AS savings_plan_total_commitment_to_date,
                    0.0             AS savings_plan_used_commitment,
                    0.0             AS reservation_effective_cost,
                    0.0             AS reservation_unused_amortized_upfront_fee_for_billing_period,
                    0.0             AS reservation_unused_recurring_fee,
                    'USD'           AS line_item_currency_code,
                    NULL::MAP(VARCHAR, VARCHAR) AS resource_tags,
                    NULL::VARCHAR   AS reservation_arn
            ) TO '{cur_path}' (FORMAT PARQUET)"#
        );
        conn.execute_batch(&sql).unwrap();
    }

    // ---- Register each in its own pool/connection (both create a view named
    //      `normalized_cost`, so they must not share an in-memory database). ----
    let focus_pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = focus_pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus12::register_view(&conn, &[focus_path]).unwrap();
    }
    let focus_repo = DuckDbCostRepository::new(focus_pool);

    let cur_pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = cur_pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        cur2::register_view(&conn, &[cur_path]).unwrap();
    }
    let cur_repo = DuckDbCostRepository::new(cur_pool);

    // ---- Amortized metric: both must equal 100.0 ----
    let filter = aug_filter();
    let focus_amortized = focus_repo.summary(&filter).unwrap();
    let cur_amortized = cur_repo.summary(&filter).unwrap();

    assert!(
        (focus_amortized.total - 100.0).abs() < 0.01,
        "FOCUS amortized total diverged from expected 100.0: got {}",
        focus_amortized.total
    );
    assert!(
        (cur_amortized.total - 100.0).abs() < 0.01,
        "CUR amortized total diverged from expected 100.0: got {}",
        cur_amortized.total
    );
    assert!(
        (focus_amortized.total - cur_amortized.total).abs() < 0.01,
        "cross-format parity FAILED (Amortized): FOCUS 1.2 total={} vs CUR 2.0 total={} \
         (difference={}) — the canonical model diverged between formats for an \
         equivalent no-discount charge",
        focus_amortized.total,
        cur_amortized.total,
        (focus_amortized.total - cur_amortized.total).abs()
    );

    // ---- Billed metric: both must equal 100.0 ----
    let mut billed_filter = aug_filter();
    billed_filter.metric = CostMetric::Billed;
    let focus_billed = focus_repo.summary(&billed_filter).unwrap();
    let cur_billed = cur_repo.summary(&billed_filter).unwrap();

    assert!(
        (focus_billed.total - 100.0).abs() < 0.01,
        "FOCUS billed total diverged from expected 100.0: got {}",
        focus_billed.total
    );
    assert!(
        (cur_billed.total - 100.0).abs() < 0.01,
        "CUR billed total diverged from expected 100.0: got {}",
        cur_billed.total
    );
    assert!(
        (focus_billed.total - cur_billed.total).abs() < 0.01,
        "cross-format parity FAILED (Billed): FOCUS 1.2 total={} vs CUR 2.0 total={} \
         (difference={}) — the canonical model diverged between formats for an \
         equivalent no-discount charge",
        focus_billed.total,
        cur_billed.total,
        (focus_billed.total - cur_billed.total).abs()
    );
}
