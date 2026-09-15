use duckdb::Connection;
use thiserror::Error;

/// The detected schema/format of a Parquet file (or set of files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedSchema {
    Focus12,
    Focus10,
    Cur2,
    Unsupported(String),
}

/// Errors that can occur during schema detection.
#[derive(Debug, Error)]
pub enum DetectionError {
    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("No files to inspect")]
    NoFiles,
}

/// Inspect the Parquet schema of the given file URI and return the detected format.
///
/// Uses `DESCRIBE SELECT * FROM read_parquet(...)` to enumerate columns, then
/// checks characteristic column presence to distinguish FOCUS versions from CUR 2.0.
///
/// Detection rules:
/// - FOCUS family: must have `BilledCost`, `EffectiveCost`, `ChargePeriodStart`, `SubAccountId`
///   - FOCUS 1.2: additionally has `ServiceSubcategory`
///   - FOCUS 1.0: FOCUS columns present but no `ServiceSubcategory`
/// - CUR 2.0: has `bill_payer_account_id` AND `line_item_usage_start_date`
/// - Otherwise: `Unsupported`
pub fn detect_schema(conn: &Connection, file_uri: &str) -> Result<DetectedSchema, DetectionError> {
    if file_uri.is_empty() {
        return Err(DetectionError::NoFiles);
    }

    // Escape single quotes in the path for SQL safety
    let escaped = file_uri.replace('\'', "''");
    let sql = format!(
        "SELECT column_name FROM (DESCRIBE SELECT * FROM read_parquet('{}') LIMIT 0)",
        escaped
    );

    let mut stmt = conn.prepare(&sql)?;
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();

    let has = |name: &str| columns.iter().any(|c| c == name);

    // FOCUS family detection
    let is_focus = has("BilledCost")
        && has("EffectiveCost")
        && has("ChargePeriodStart")
        && has("SubAccountId");

    if is_focus {
        if has("ServiceSubcategory") {
            return Ok(DetectedSchema::Focus12);
        } else {
            return Ok(DetectedSchema::Focus10);
        }
    }

    // CUR 2.0 detection
    let is_cur2 = has("bill_payer_account_id") && has("line_item_usage_start_date");
    if is_cur2 {
        return Ok(DetectedSchema::Cur2);
    }

    // Fallback: return the column list as context
    let summary = if columns.is_empty() {
        "no columns found".to_string()
    } else {
        format!("columns: {}", columns.join(", "))
    };
    Ok(DetectedSchema::Unsupported(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;
    use tempfile::TempDir;

    use std::sync::OnceLock;

    /// Ensure the parquet extension is installed to `~/.duckdb` exactly once per
    /// test binary invocation. After this, all connections can `LOAD parquet`
    /// without network access (the file is already cached locally).
    fn ensure_parquet_installed() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("INSTALL parquet; LOAD parquet;").unwrap();
        });
    }

    /// Open an in-memory DuckDB connection with parquet loaded.
    fn open_conn() -> Connection {
        ensure_parquet_installed();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        conn
    }

    /// Write a Parquet file via DuckDB COPY. Returns the dir (to keep it alive) and path string.
    /// We use TempDir + a fixed filename so DuckDB can freely rename its staging file on Windows.
    fn write_parquet(sql_select: &str) -> (TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("data.parquet")
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let conn = open_conn();
        let sql = format!("COPY ({sql_select}) TO '{path}' (FORMAT PARQUET)");
        conn.execute_batch(&sql).unwrap();
        (dir, path)
    }

    fn focus12_select() -> &'static str {
        r#"SELECT
    TIMESTAMP '2026-08-01 00:00:00' AS BillingPeriodStart,
    TIMESTAMP '2026-09-01 00:00:00' AS BillingPeriodEnd,
    TIMESTAMP '2026-08-15 00:00:00' AS ChargePeriodStart,
    TIMESTAMP '2026-08-16 00:00:00' AS ChargePeriodEnd,
    'acct-001'    AS BillingAccountId,
    'Acct Name'   AS BillingAccountName,
    'sub-001'     AS SubAccountId,
    'Sub Name'    AS SubAccountName,
    'AWS'         AS ProviderName,
    'Amazon'      AS PublisherName,
    'EC2'         AS ServiceName,
    'Compute'     AS ServiceCategory,
    'VMs'         AS ServiceSubcategory,
    'us-east-1'   AS RegionName,
    'us-east-1a'  AS AvailabilityZone,
    'i-abc123'    AS ResourceId,
    'my-instance' AS ResourceName,
    'm5.large'    AS ResourceType,
    'Usage'       AS ChargeCategory,
    NULL::VARCHAR AS ChargeClass,
    'Recurring'   AS ChargeFrequency,
    'EC2 usage'   AS ChargeDescription,
    'OnDemand'    AS PricingCategory,
    8.0           AS ConsumedQuantity,
    'Hours'       AS ConsumedUnit,
    1.234         AS BilledCost,
    1.100         AS EffectiveCost,
    1.500         AS ListCost,
    1.200         AS ContractedCost,
    'USD'         AS BillingCurrency,
    NULL::MAP(VARCHAR, VARCHAR) AS Tags,
    NULL::VARCHAR AS CommitmentDiscountId,
    NULL::VARCHAR AS CommitmentDiscountType,
    NULL::VARCHAR AS CommitmentDiscountStatus,
    NULL::VARCHAR AS x_ServiceCode
  FROM (VALUES (NULL)) AS t(dummy)"#
    }

    fn focus10_select() -> &'static str {
        r#"SELECT
    TIMESTAMP '2026-08-01 00:00:00' AS BillingPeriodStart,
    TIMESTAMP '2026-09-01 00:00:00' AS BillingPeriodEnd,
    TIMESTAMP '2026-08-15 00:00:00' AS ChargePeriodStart,
    TIMESTAMP '2026-08-16 00:00:00' AS ChargePeriodEnd,
    'acct-001'  AS BillingAccountId,
    'Acct Name' AS BillingAccountName,
    'sub-001'   AS SubAccountId,
    'Sub Name'  AS SubAccountName,
    'AWS'       AS ProviderName,
    'Amazon'    AS PublisherName,
    'EC2'       AS ServiceName,
    'Compute'   AS ServiceCategory,
    'us-east-1' AS RegionName,
    'i-abc123'  AS ResourceId,
    'Usage'     AS ChargeCategory,
    'Recurring' AS ChargeFrequency,
    8.0         AS ConsumedQuantity,
    'Hours'     AS ConsumedUnit,
    1.234       AS BilledCost,
    1.100       AS EffectiveCost,
    1.500       AS ListCost,
    1.200       AS ContractedCost,
    'USD'       AS BillingCurrency
  FROM (VALUES (NULL)) AS t(dummy)"#
    }

    fn cur2_select() -> &'static str {
        r#"SELECT
    '123456789012' AS bill_payer_account_id,
    '987654321098' AS line_item_usage_account_id,
    TIMESTAMP '2026-08-15 00:00:00' AS line_item_usage_start_date,
    TIMESTAMP '2026-08-16 00:00:00' AS line_item_usage_end_date,
    'AmazonEC2'    AS line_item_product_code,
    1.234          AS line_item_unblended_cost
  FROM (VALUES (NULL)) AS t(dummy)"#
    }

    #[test]
    fn detect_focus12() {
        let (_dir, path) = write_parquet(focus12_select());
        let conn = open_conn();
        let result = detect_schema(&conn, &path).unwrap();
        assert_eq!(result, DetectedSchema::Focus12);
    }

    #[test]
    fn detect_focus10() {
        let (_dir, path) = write_parquet(focus10_select());
        let conn = open_conn();
        let result = detect_schema(&conn, &path).unwrap();
        assert_eq!(result, DetectedSchema::Focus10);
    }

    #[test]
    fn detect_cur2() {
        let (_dir, path) = write_parquet(cur2_select());
        let conn = open_conn();
        let result = detect_schema(&conn, &path).unwrap();
        assert_eq!(result, DetectedSchema::Cur2);
    }

    #[test]
    fn detect_unsupported() {
        let (_dir, path) = write_parquet(
            "SELECT 1 AS foo, 'bar' AS baz FROM (VALUES (NULL)) AS t(d)",
        );
        let conn = open_conn();
        let result = detect_schema(&conn, &path).unwrap();
        assert!(matches!(result, DetectedSchema::Unsupported(_)));
    }

    #[test]
    fn detect_empty_uri_returns_no_files_error() {
        let conn = open_conn();
        let err = detect_schema(&conn, "").unwrap_err();
        assert!(matches!(err, DetectionError::NoFiles));
    }
}
