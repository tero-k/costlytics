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
}
