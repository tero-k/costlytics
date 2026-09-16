use duckdb::Connection;
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Errors that can occur while generating the synthetic FOCUS 1.2 fixture dataset.
#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Known totals for the fixture dataset (used as golden values in tests).
pub const FIXTURE_AMORTIZED_TOTAL_AUG: f64 = 1234.56;  // August 2026
pub const FIXTURE_BILLED_TOTAL_AUG: f64 = 1100.00;     // August 2026
pub const FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL: f64 = 200.00; // Sep 1–15 2026
pub const FIXTURE_AMORTIZED_TOTAL_COMBINED: f64 =
    FIXTURE_AMORTIZED_TOTAL_AUG + FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL;

/// Generate a synthetic FOCUS 1.2 Parquet dataset in the given directory.
///
/// Layout:
/// ```text
/// {base_dir}/BILLING_PERIOD=2026-08/data.parquet
/// {base_dir}/BILLING_PERIOD=2026-08/Manifest.json
/// {base_dir}/BILLING_PERIOD=2026-09/data.parquet
/// {base_dir}/BILLING_PERIOD=2026-09/Manifest.json
/// ```
///
/// # Totals
/// - August 2026: `FIXTURE_AMORTIZED_TOTAL_AUG` amortized, `FIXTURE_BILLED_TOTAL_AUG` billed.
/// - September 2026 (rows on Sep 1–15): `FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL` amortized.
///
/// The query layer filters on `usage_start` (mapped from `ChargePeriodStart`), not
/// `BillingPeriodStart`.
pub fn generate_focus12_fixture(base_dir: &Path) -> Result<(), FixtureError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("LOAD parquet;")?;

    generate_august_partition(&conn, base_dir)?;
    generate_september_partition(&conn, base_dir)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// August 2026 partition
// Two rows:
//   Row 1: EffectiveCost=1034.56, BilledCost=1100.00  (Aug 10)
//   Row 2: EffectiveCost=200.00,  BilledCost=0.00     (Aug 20)
// Totals: amortized=1234.56, billed=1100.00
// ---------------------------------------------------------------------------
fn generate_august_partition(conn: &Connection, base_dir: &Path) -> Result<(), FixtureError> {
    let dir = base_dir.join("BILLING_PERIOD=2026-08");
    fs::create_dir_all(&dir)?;
    let parquet_path = dir.join("data.parquet");
    let parquet_str = parquet_path.to_str().unwrap().replace('\\', "/");

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
                'i-abc123'      AS ResourceId,
                'my-instance'   AS ResourceName,
                'm5.large'      AS ResourceType,
                'Usage'         AS ChargeCategory,
                NULL::VARCHAR   AS ChargeClass,
                'Recurring'     AS ChargeFrequency,
                'EC2 usage'     AS ChargeDescription,
                'OnDemand'      AS PricingCategory,
                8.0             AS ConsumedQuantity,
                'Hours'         AS ConsumedUnit,
                1100.00         AS BilledCost,
                1034.56         AS EffectiveCost,
                1500.00         AS ListCost,
                1200.00         AS ContractedCost,
                'USD'           AS BillingCurrency,
                NULL::MAP(VARCHAR, VARCHAR) AS Tags,
                NULL::VARCHAR   AS CommitmentDiscountId,
                NULL::VARCHAR   AS CommitmentDiscountType,
                NULL::VARCHAR   AS CommitmentDiscountStatus,
                'AmazonEC2'     AS x_ServiceCode
            UNION ALL
            SELECT
                TIMESTAMP '2026-08-01 00:00:00',
                TIMESTAMP '2026-09-01 00:00:00',
                TIMESTAMP '2026-08-20 00:00:00',
                TIMESTAMP '2026-08-21 00:00:00',
                'acct-001', 'Acct Name', 'sub-001', 'Sub Name',
                'AWS', 'Amazon',
                'S3', 'Storage', 'Object Storage',
                'us-east-1', 'us-east-1a',
                'bucket-abc', 'my-bucket', 'S3Bucket',
                'Usage', NULL::VARCHAR, 'Recurring', 'S3 usage', 'OnDemand',
                100.0, 'GB',
                0.00, 200.00, 0.00, 0.00,
                'USD',
                NULL::MAP(VARCHAR, VARCHAR),
                NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR,
                'AmazonS3'
        ) TO '{parquet_str}' (FORMAT PARQUET)"#,
        parquet_str = parquet_str
    );
    conn.execute_batch(&sql)?;

    let manifest = serde_json::json!({
        "dataFiles": ["data.parquet"],
        "billingPeriod": {
            "start": "2026-08-01T00:00:00Z",
            "end": "2026-09-01T00:00:00Z"
        }
    });
    fs::write(
        dir.join("Manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// September 2026 partition (partial: Sep 1–15 only)
// One row: EffectiveCost=200.00  (Sep 5, inside the partial window)
// Total: amortized=200.00
// ---------------------------------------------------------------------------
fn generate_september_partition(conn: &Connection, base_dir: &Path) -> Result<(), FixtureError> {
    let dir = base_dir.join("BILLING_PERIOD=2026-09");
    fs::create_dir_all(&dir)?;
    let parquet_path = dir.join("data.parquet");
    let parquet_str = parquet_path.to_str().unwrap().replace('\\', "/");

    let sql = format!(
        r#"COPY (
            SELECT
                TIMESTAMP '2026-09-01 00:00:00' AS BillingPeriodStart,
                TIMESTAMP '2026-10-01 00:00:00' AS BillingPeriodEnd,
                TIMESTAMP '2026-09-05 00:00:00' AS ChargePeriodStart,
                TIMESTAMP '2026-09-06 00:00:00' AS ChargePeriodEnd,
                'acct-001'      AS BillingAccountId,
                'Acct Name'     AS BillingAccountName,
                'sub-001'       AS SubAccountId,
                'Sub Name'      AS SubAccountName,
                'AWS'           AS ProviderName,
                'Amazon'        AS PublisherName,
                'RDS'           AS ServiceName,
                'Database'      AS ServiceCategory,
                'Relational'    AS ServiceSubcategory,
                'eu-west-1'     AS RegionName,
                'eu-west-1a'    AS AvailabilityZone,
                'db-xyz'        AS ResourceId,
                'my-db'         AS ResourceName,
                'db.t3.micro'   AS ResourceType,
                'Usage'         AS ChargeCategory,
                NULL::VARCHAR   AS ChargeClass,
                'Recurring'     AS ChargeFrequency,
                'RDS usage'     AS ChargeDescription,
                'OnDemand'      AS PricingCategory,
                24.0            AS ConsumedQuantity,
                'Hours'         AS ConsumedUnit,
                0.00            AS BilledCost,
                200.00          AS EffectiveCost,
                250.00          AS ListCost,
                210.00          AS ContractedCost,
                'USD'           AS BillingCurrency,
                NULL::MAP(VARCHAR, VARCHAR) AS Tags,
                NULL::VARCHAR   AS CommitmentDiscountId,
                NULL::VARCHAR   AS CommitmentDiscountType,
                NULL::VARCHAR   AS CommitmentDiscountStatus,
                'AmazonRDS'     AS x_ServiceCode
        ) TO '{parquet_str}' (FORMAT PARQUET)"#,
        parquet_str = parquet_str
    );
    conn.execute_batch(&sql)?;

    let manifest = serde_json::json!({
        "dataFiles": ["data.parquet"],
        "billingPeriod": {
            "start": "2026-09-01T00:00:00Z",
            "end": "2026-10-01T00:00:00Z"
        }
    });
    fs::write(
        dir.join("Manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// CUR 2.0 golden fixture: 11 scenario rows exercising every branch of the
// `amortized_cost` CASE expression in `adapters::cur2::register_view`.
// See `.git/sdd/task-cur2-2-brief.md` for the source-of-truth table.
// ---------------------------------------------------------------------------

/// Golden expected values per CUR 2.0 scenario row (billed_cost / amortized_cost
/// as computed by the `normalized_cost` VIEW's CASE expression).
pub const CUR_ON_DEMAND_USAGE_BILLED: f64 = 90.0;
pub const CUR_ON_DEMAND_USAGE_AMORTIZED: f64 = 100.0;

pub const CUR_CREDIT_BILLED: f64 = -20.0;
pub const CUR_CREDIT_AMORTIZED: f64 = -20.0;

pub const CUR_REFUND_BILLED: f64 = -15.0;
pub const CUR_REFUND_AMORTIZED: f64 = -15.0;

pub const CUR_TAX_BILLED: f64 = 5.0;
pub const CUR_TAX_AMORTIZED: f64 = 5.0;

pub const CUR_RI_DISCOUNTED_USAGE_BILLED: f64 = 0.0;
pub const CUR_RI_DISCOUNTED_USAGE_AMORTIZED: f64 = 8.0;

pub const CUR_RI_FEE_BILLED: f64 = 50.0;
pub const CUR_RI_FEE_AMORTIZED: f64 = 5.0;

pub const CUR_SP_COVERED_USAGE_BILLED: f64 = 0.0;
pub const CUR_SP_COVERED_USAGE_AMORTIZED: f64 = 12.0;

pub const CUR_SP_RECURRING_FEE_BILLED: f64 = 100.0;
pub const CUR_SP_RECURRING_FEE_AMORTIZED: f64 = 40.0;

pub const CUR_SP_NEGATION_BILLED: f64 = -30.0;
pub const CUR_SP_NEGATION_AMORTIZED: f64 = 0.0;

pub const CUR_SP_UPFRONT_FEE_BILLED: f64 = 25.0;
pub const CUR_SP_UPFRONT_FEE_AMORTIZED: f64 = 0.0;

pub const CUR_FEE_WITH_RESERVATION_BILLED: f64 = 40.0;
pub const CUR_FEE_WITH_RESERVATION_AMORTIZED: f64 = 0.0;

/// Sum of all 11 scenarios' billed_cost.
pub const CUR_GOLDEN_TOTAL_BILLED: f64 = 245.0;
/// Sum of all 11 scenarios' amortized_cost.
pub const CUR_GOLDEN_TOTAL_AMORTIZED: f64 = 135.0;

/// Generate a synthetic CUR 2.0 Parquet dataset covering every branch of the
/// `amortized_cost` CASE expression in `adapters::cur2::register_view`.
///
/// Layout:
/// ```text
/// {base_dir}/BILLING_PERIOD=2026-08/data.parquet
/// {base_dir}/BILLING_PERIOD=2026-08/Manifest.json
/// ```
///
/// Contains 11 rows (one per scenario), each with a distinct
/// `line_item_resource_id` so tests can filter individual scenarios.
///
/// # Totals
/// - `CUR_GOLDEN_TOTAL_BILLED` (245.0) summed over `billed_cost`.
/// - `CUR_GOLDEN_TOTAL_AMORTIZED` (135.0) summed over `amortized_cost`.
pub fn generate_cur2_fixture(base_dir: &Path) -> Result<(), FixtureError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("LOAD parquet;")?;

    let dir = base_dir.join("BILLING_PERIOD=2026-08");
    fs::create_dir_all(&dir)?;
    let parquet_path = dir.join("data.parquet");
    let parquet_str = parquet_path.to_str().unwrap().replace('\\', "/");

    // Common column defaults, overridden per-scenario below:
    //   line_item_net_unblended_cost / line_item_unblended_cost -> per scenario
    //   savings_plan_* / reservation_effective_cost / reservation_unused_* -> 0.0
    //   reservation_arn -> NULL unless scenario specifies it
    let sql = format!(
        r#"COPY (
            WITH t AS (
              SELECT * FROM (VALUES
                -- (resource_id, line_item_type, description, net_unblended, unblended,
                --  sp_effective_cost, sp_total_commitment, sp_used_commitment,
                --  reservation_effective_cost, reservation_unused_upfront, reservation_unused_recurring,
                --  reservation_arn)
                ('res-ondemand',      'Usage',                   'On-demand EC2 usage',   90.0,   100.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-credit',        'Credit',                  'Promotional credit',   -20.0,  -20.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-refund',        'Refund',                  'Refund',               -15.0,  -15.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-tax',           'Tax',                     'Tax',                    5.0,    5.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-ri-discounted', 'DiscountedUsage',         'RI-discounted usage',   0.0,    0.0,  0.0,  0.0,   0.0,  8.0, 0.0, 0.0, NULL),
                ('res-ri-fee',        'RIFee',                   'RI unused fee',         50.0,   50.0,  0.0,  0.0,   0.0,  0.0, 3.0, 2.0, NULL),
                ('res-sp-covered',    'SavingsPlanCoveredUsage', 'SP-covered usage',       0.0,    0.0, 12.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-sp-recurring',  'SavingsPlanRecurringFee', 'SP recurring fee',     100.0,  100.0,  0.0, 100.0, 60.0,  0.0, 0.0, 0.0, NULL),
                ('res-sp-negation',   'SavingsPlanNegation',     'SP negation',          -30.0,  -30.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-sp-upfront',    'SavingsPlanUpfrontFee',   'SP upfront fee',        25.0,   25.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, NULL),
                ('res-fee-reservation','Fee',                    'Reservation fee',       40.0,   40.0,  0.0,  0.0,   0.0,  0.0, 0.0, 0.0, 'arn:aws:ec2:us-east-1:123456789012:reserved-instances/abc123')
            ) AS t(
                line_item_resource_id, line_item_line_item_type, line_item_line_item_description,
                line_item_net_unblended_cost, line_item_unblended_cost,
                savings_plan_savings_plan_effective_cost, savings_plan_total_commitment_to_date, savings_plan_used_commitment,
                reservation_effective_cost, reservation_unused_amortized_upfront_fee_for_billing_period, reservation_unused_recurring_fee,
                reservation_arn
            )
            ),
        rows AS (
            SELECT
                TIMESTAMP '2026-08-01 00:00:00' AS bill_billing_period_start_date,
                TIMESTAMP '2026-09-01 00:00:00' AS bill_billing_period_end_date,
                TIMESTAMP '2026-08-15 00:00:00' AS line_item_usage_start_date,
                TIMESTAMP '2026-08-16 00:00:00' AS line_item_usage_end_date,
                '123456789012'  AS bill_payer_account_id,
                'Payer Name'    AS bill_payer_account_name,
                '987654321098'  AS line_item_usage_account_id,
                'Acct Name'     AS line_item_usage_account_name,
                'Amazon Elastic Compute Cloud' AS product_product_name,
                'AmazonEC2'     AS product_servicecode,
                'us-east-1'     AS product_region_code,
                'us-east-1a'    AS line_item_availability_zone,
                line_item_resource_id,
                'm5.large'      AS product_instance_type,
                line_item_line_item_type,
                line_item_line_item_description,
                1.0             AS line_item_usage_amount,
                'Hours'         AS pricing_unit,
                line_item_net_unblended_cost,
                line_item_unblended_cost,
                savings_plan_savings_plan_effective_cost,
                savings_plan_total_commitment_to_date,
                savings_plan_used_commitment,
                reservation_effective_cost,
                reservation_unused_amortized_upfront_fee_for_billing_period,
                reservation_unused_recurring_fee,
                'USD'           AS line_item_currency_code,
                NULL::MAP(VARCHAR, VARCHAR) AS resource_tags,
                reservation_arn
            FROM t
        )
        SELECT * FROM rows
        ) TO '{parquet_str}' (FORMAT PARQUET)"#,
        parquet_str = parquet_str
    );
    conn.execute_batch(&sql)?;

    let manifest = serde_json::json!({
        "dataFiles": ["data.parquet"],
        "billingPeriod": {
            "start": "2026-08-01T00:00:00Z",
            "end": "2026-09-01T00:00:00Z"
        }
    });
    fs::write(
        dir.join("Manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::OnceLock;

    fn ensure_parquet_installed() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("LOAD parquet;").unwrap();
        });
    }

    #[test]
    fn fixture_constants_consistent() {
        assert!(
            (FIXTURE_AMORTIZED_TOTAL_COMBINED
                - (FIXTURE_AMORTIZED_TOTAL_AUG + FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL))
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn generate_fixture_creates_files() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_focus12_fixture(dir.path()).unwrap();

        let aug_parquet = dir.path().join("BILLING_PERIOD=2026-08").join("data.parquet");
        let aug_manifest = dir.path().join("BILLING_PERIOD=2026-08").join("Manifest.json");
        let sep_parquet = dir.path().join("BILLING_PERIOD=2026-09").join("data.parquet");
        let sep_manifest = dir.path().join("BILLING_PERIOD=2026-09").join("Manifest.json");

        assert!(aug_parquet.exists(), "Aug parquet missing");
        assert!(aug_manifest.exists(), "Aug manifest missing");
        assert!(sep_parquet.exists(), "Sep parquet missing");
        assert!(sep_manifest.exists(), "Sep manifest missing");
    }

    #[test]
    fn fixture_parquet_readable_by_duckdb() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_focus12_fixture(dir.path()).unwrap();

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();

        let aug_path = dir
            .path()
            .join("BILLING_PERIOD=2026-08")
            .join("data.parquet")
            .to_str()
            .unwrap()
            .replace('\\', "/");

        let mut stmt = conn
            .prepare(&format!(
                "SELECT COUNT(*) FROM read_parquet('{aug_path}')"
            ))
            .unwrap();
        let count: i64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(count, 2, "August partition should have 2 rows");
    }

    #[test]
    fn fixture_august_totals_match_constants() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_focus12_fixture(dir.path()).unwrap();

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();

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

        // August amortized (EffectiveCost)
        let mut stmt = conn
            .prepare(&format!(
                "SELECT SUM(EffectiveCost) FROM read_parquet('{aug_path}')"
            ))
            .unwrap();
        let amortized_aug: f64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert!(
            (amortized_aug - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 1e-6,
            "Aug amortized mismatch: {} vs {}",
            amortized_aug,
            FIXTURE_AMORTIZED_TOTAL_AUG
        );

        // August billed (BilledCost)
        let mut stmt = conn
            .prepare(&format!(
                "SELECT SUM(BilledCost) FROM read_parquet('{aug_path}')"
            ))
            .unwrap();
        let billed_aug: f64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert!(
            (billed_aug - FIXTURE_BILLED_TOTAL_AUG).abs() < 1e-6,
            "Aug billed mismatch: {} vs {}",
            billed_aug,
            FIXTURE_BILLED_TOTAL_AUG
        );

        // September partial amortized
        let mut stmt = conn
            .prepare(&format!(
                "SELECT SUM(EffectiveCost) FROM read_parquet('{sep_path}')"
            ))
            .unwrap();
        let amortized_sep: f64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert!(
            (amortized_sep - FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL).abs() < 1e-6,
            "Sep amortized mismatch: {} vs {}",
            amortized_sep,
            FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL
        );
    }

    #[test]
    fn fixture_manifest_json_format() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_focus12_fixture(dir.path()).unwrap();

        let manifest_path = dir.path().join("BILLING_PERIOD=2026-08").join("Manifest.json");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(
            parsed["dataFiles"].is_array(),
            "Manifest must have dataFiles array"
        );
        assert_eq!(parsed["dataFiles"][0], "data.parquet");
    }

    #[test]
    fn fixture_view_registers_and_totals_match() {
        use crate::adapters::focus12;
        use crate::duckdb_pool;
        use chrono::TimeZone;
        use chrono::Utc;
        use crate::queries::summary::{CostRepository, DuckDbCostRepository};
        use domain::cost::CostMetric;
        use domain::filters::CostFilter;

        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
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

        let pool = duckdb_pool::build_pool().unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch("LOAD parquet;").unwrap();
            focus12::register_view(&conn, &[aug_path, sep_path]).unwrap();
        }
        let repo = DuckDbCostRepository::new(pool);

        // August amortized
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);
        let summary = repo.summary(&filter).unwrap();
        assert!(
            (summary.total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 1e-6,
            "aug amortized via repository: {} vs {}",
            summary.total,
            FIXTURE_AMORTIZED_TOTAL_AUG
        );

        // August billed
        let mut filter_billed = CostFilter::date_range(start, end);
        filter_billed.metric = CostMetric::Billed;
        let summary_billed = repo.summary(&filter_billed).unwrap();
        assert!(
            (summary_billed.total - FIXTURE_BILLED_TOTAL_AUG).abs() < 1e-6,
            "aug billed via repository: {} vs {}",
            summary_billed.total,
            FIXTURE_BILLED_TOTAL_AUG
        );

        // September partial amortized
        let sep_start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let sep_end = Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, 0).unwrap();
        let sep_filter = CostFilter::date_range(sep_start, sep_end);
        let sep_summary = repo.summary(&sep_filter).unwrap();
        assert!(
            (sep_summary.total - FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL).abs() < 1e-6,
            "sep partial amortized via repository: {} vs {}",
            sep_summary.total,
            FIXTURE_AMORTIZED_TOTAL_SEP_PARTIAL
        );
    }

    #[test]
    fn cur_golden_totals_consistent() {
        let billed_sum = CUR_ON_DEMAND_USAGE_BILLED
            + CUR_CREDIT_BILLED
            + CUR_REFUND_BILLED
            + CUR_TAX_BILLED
            + CUR_RI_DISCOUNTED_USAGE_BILLED
            + CUR_RI_FEE_BILLED
            + CUR_SP_COVERED_USAGE_BILLED
            + CUR_SP_RECURRING_FEE_BILLED
            + CUR_SP_NEGATION_BILLED
            + CUR_SP_UPFRONT_FEE_BILLED
            + CUR_FEE_WITH_RESERVATION_BILLED;
        let amortized_sum = CUR_ON_DEMAND_USAGE_AMORTIZED
            + CUR_CREDIT_AMORTIZED
            + CUR_REFUND_AMORTIZED
            + CUR_TAX_AMORTIZED
            + CUR_RI_DISCOUNTED_USAGE_AMORTIZED
            + CUR_RI_FEE_AMORTIZED
            + CUR_SP_COVERED_USAGE_AMORTIZED
            + CUR_SP_RECURRING_FEE_AMORTIZED
            + CUR_SP_NEGATION_AMORTIZED
            + CUR_SP_UPFRONT_FEE_AMORTIZED
            + CUR_FEE_WITH_RESERVATION_AMORTIZED;
        assert!(
            (billed_sum - CUR_GOLDEN_TOTAL_BILLED).abs() < 1e-9,
            "billed sum mismatch: {} vs {}",
            billed_sum,
            CUR_GOLDEN_TOTAL_BILLED
        );
        assert!(
            (amortized_sum - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 1e-9,
            "amortized sum mismatch: {} vs {}",
            amortized_sum,
            CUR_GOLDEN_TOTAL_AMORTIZED
        );
    }

    #[test]
    fn cur2_fixture_creates_valid_parquet() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_cur2_fixture(dir.path()).unwrap();

        let parquet_path = dir
            .path()
            .join("BILLING_PERIOD=2026-08")
            .join("data.parquet");
        assert!(parquet_path.exists(), "CUR 2.0 parquet missing");

        let parquet_str = parquet_path.to_str().unwrap().replace('\\', "/");
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT COUNT(*) FROM read_parquet('{parquet_str}')"
            ))
            .unwrap();
        let count: i64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(count, 11, "CUR 2.0 fixture should have 11 rows");
    }

    #[test]
    fn cur2_fixture_manifest_json_format() {
        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_cur2_fixture(dir.path()).unwrap();

        let manifest_path = dir.path().join("BILLING_PERIOD=2026-08").join("Manifest.json");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(
            parsed["dataFiles"].is_array(),
            "Manifest must have dataFiles array"
        );
        assert_eq!(parsed["dataFiles"][0], "data.parquet");
    }

    #[test]
    fn cur2_fixture_view_registers_and_golden_totals_match() {
        use crate::adapters::cur2;

        ensure_parquet_installed();
        let dir = tempfile::tempdir().unwrap();
        generate_cur2_fixture(dir.path()).unwrap();

        let parquet_path = dir
            .path()
            .join("BILLING_PERIOD=2026-08")
            .join("data.parquet")
            .to_str()
            .unwrap()
            .replace('\\', "/");

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        cur2::register_view(&conn, &[parquet_path]).unwrap();

        let mut stmt = conn
            .prepare("SELECT SUM(billed_cost), SUM(amortized_cost) FROM normalized_cost")
            .unwrap();
        let (billed_total, amortized_total): (f64, f64) = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();

        assert!(
            (billed_total - CUR_GOLDEN_TOTAL_BILLED).abs() < 1e-6,
            "billed total via view: {} vs {}",
            billed_total,
            CUR_GOLDEN_TOTAL_BILLED
        );
        assert!(
            (amortized_total - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 1e-6,
            "amortized total via view: {} vs {}",
            amortized_total,
            CUR_GOLDEN_TOTAL_AMORTIZED
        );
    }
}
