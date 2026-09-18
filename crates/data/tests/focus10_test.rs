/// Integration tests for the FOCUS 1.0 adapter: a full-pipeline golden test
/// (mirroring `cur2_golden_total_via_repository` in `cur2_golden_test.rs`)
/// plus the three-way cross-format parity test that extends Session 2's
/// two-way FOCUS 1.2 / CUR 2.0 parity proof
/// (`cross_format_parity_focus_vs_cur`) to all three now-supported formats:
/// FOCUS 1.0, FOCUS 1.2, and CUR 2.0.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use chrono::TimeZone;
use chrono::Utc;
use data::adapters::{cur2, focus10, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{generate_focus10_fixture, FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG};
use data::object_store::LocalObjectStore;
use data::object_store::YearMonth;
use data::queries::summary::{CostRepository, DuckDbCostRepository};
use data::schema_detection::{detect_schema, DetectedSchema};
use domain::cost::CostMetric;
use domain::filters::CostFilter;
use std::sync::OnceLock;

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

// ---------------------------------------------------------------------------
// Full pipeline golden test (discovery -> schema detection -> view
// registration -> repository.summary())
// ---------------------------------------------------------------------------

/// Full end-to-end golden-value test for FOCUS 1.0, mirroring
/// `cur2_golden_total_via_repository` in `cur2_golden_test.rs`.
#[test]
fn focus10_full_pipeline() {
    ensure_parquet_installed();

    let dir = tempfile::TempDir::new().unwrap();
    generate_focus10_fixture(dir.path()).unwrap();

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
        DetectedSchema::Focus10,
        "fixture parquet should be detected as FOCUS 1.0"
    );

    // ---- 3. Register view and query (Amortized) ----
    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus10::register_view(&conn, &partitions[0].files).unwrap();
    }
    let repo = DuckDbCostRepository::new(pool);

    let filter = aug_filter();
    let summary = repo.summary(&filter).unwrap();

    assert_eq!(
        summary.metric,
        CostMetric::Amortized,
        "default metric must be Amortized"
    );
    assert!(
        (summary.total - FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "golden amortized assertion failed: expected {}, got {}",
        FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG,
        summary.total
    );
}

// ---------------------------------------------------------------------------
// Three-way cross-format parity test
// ---------------------------------------------------------------------------

/// The architectural proof: minimal single-row FOCUS 1.0, FOCUS 1.2, and CUR
/// 2.0 fixtures, all representing the exact same real-world charge ("$100 of
/// on-demand EC2 usage, no RI/SP discount involved"), must produce IDENTICAL
/// `summary()` totals via the canonical `normalized_cost` view — for both the
/// Amortized and Billed metrics.
///
/// This extends Session 2's two-way `cross_format_parity_focus_vs_cur` test
/// (in `cur2_golden_test.rs`) to all three now-supported formats, proving the
/// canonical model genuinely generalizes: dashboard queries operate against
/// the canonical model, never directly against CUR/FOCUS column names, so the
/// SAME query code produces the SAME answer regardless of source format.
#[test]
fn cross_format_parity_three_ways() {
    ensure_parquet_installed();

    // ---- FOCUS 1.0: one row, BilledCost = EffectiveCost = 100.0 (no discount) ----
    let focus10_dir = tempfile::TempDir::new().unwrap();
    let focus10_path = focus10_dir
        .path()
        .join("focus10.parquet")
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
                    'us-east-1'     AS RegionName,
                    'i-parity'      AS ResourceId,
                    'Usage'         AS ChargeCategory,
                    'Recurring'     AS ChargeFrequency,
                    8.0             AS ConsumedQuantity,
                    'Hours'         AS ConsumedUnit,
                    100.0           AS BilledCost,
                    100.0           AS EffectiveCost,
                    100.0           AS ListCost,
                    100.0           AS ContractedCost,
                    'USD'           AS BillingCurrency
            ) TO '{focus10_path}' (FORMAT PARQUET)"#
        );
        conn.execute_batch(&sql).unwrap();
    }

    // ---- FOCUS 1.2: one row, BilledCost = EffectiveCost = 100.0 (no discount) ----
    let focus12_dir = tempfile::TempDir::new().unwrap();
    let focus12_path = focus12_dir
        .path()
        .join("focus12.parquet")
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
            ) TO '{focus12_path}' (FORMAT PARQUET)"#
        );
        conn.execute_batch(&sql).unwrap();
    }

    // ---- CUR 2.0: one row, line_item_unblended_cost = line_item_net_unblended_cost = 100.0 ----
    let cur_dir = tempfile::TempDir::new().unwrap();
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

    // ---- Register each in its own pool/connection (each creates a view named
    //      `normalized_cost`, so they must not share an in-memory database). ----
    let focus10_pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = focus10_pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus10::register_view(&conn, &[focus10_path]).unwrap();
    }
    let focus10_repo = DuckDbCostRepository::new(focus10_pool);

    let focus12_pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = focus12_pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        focus12::register_view(&conn, &[focus12_path]).unwrap();
    }
    let focus12_repo = DuckDbCostRepository::new(focus12_pool);

    let cur_pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = cur_pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        cur2::register_view(&conn, &[cur_path]).unwrap();
    }
    let cur_repo = DuckDbCostRepository::new(cur_pool);

    // ---- Amortized metric: all three must equal 100.0 ----
    let filter = aug_filter();
    let focus10_amortized = focus10_repo.summary(&filter).unwrap();
    let focus12_amortized = focus12_repo.summary(&filter).unwrap();
    let cur_amortized = cur_repo.summary(&filter).unwrap();

    assert!(
        (focus10_amortized.total - 100.0).abs() < 0.01,
        "FOCUS 1.0 amortized total diverged from expected 100.0: got {}",
        focus10_amortized.total
    );
    assert!(
        (focus12_amortized.total - 100.0).abs() < 0.01,
        "FOCUS 1.2 amortized total diverged from expected 100.0: got {}",
        focus12_amortized.total
    );
    assert!(
        (cur_amortized.total - 100.0).abs() < 0.01,
        "CUR 2.0 amortized total diverged from expected 100.0: got {}",
        cur_amortized.total
    );
    assert!(
        (focus10_amortized.total - focus12_amortized.total).abs() < 0.01
            && (focus12_amortized.total - cur_amortized.total).abs() < 0.01,
        "three-way cross-format parity FAILED (Amortized): FOCUS 1.0 total={} vs \
         FOCUS 1.2 total={} vs CUR 2.0 total={} — the canonical model diverged between \
         formats for an equivalent no-discount charge",
        focus10_amortized.total,
        focus12_amortized.total,
        cur_amortized.total
    );

    // ---- Billed metric: all three must equal 100.0 ----
    let mut billed_filter = aug_filter();
    billed_filter.metric = CostMetric::Billed;
    let focus10_billed = focus10_repo.summary(&billed_filter).unwrap();
    let focus12_billed = focus12_repo.summary(&billed_filter).unwrap();
    let cur_billed = cur_repo.summary(&billed_filter).unwrap();

    assert!(
        (focus10_billed.total - 100.0).abs() < 0.01,
        "FOCUS 1.0 billed total diverged from expected 100.0: got {}",
        focus10_billed.total
    );
    assert!(
        (focus12_billed.total - 100.0).abs() < 0.01,
        "FOCUS 1.2 billed total diverged from expected 100.0: got {}",
        focus12_billed.total
    );
    assert!(
        (cur_billed.total - 100.0).abs() < 0.01,
        "CUR 2.0 billed total diverged from expected 100.0: got {}",
        cur_billed.total
    );
    assert!(
        (focus10_billed.total - focus12_billed.total).abs() < 0.01
            && (focus12_billed.total - cur_billed.total).abs() < 0.01,
        "three-way cross-format parity FAILED (Billed): FOCUS 1.0 total={} vs \
         FOCUS 1.2 total={} vs CUR 2.0 total={} — the canonical model diverged between \
         formats for an equivalent no-discount charge",
        focus10_billed.total,
        focus12_billed.total,
        cur_billed.total
    );
}
