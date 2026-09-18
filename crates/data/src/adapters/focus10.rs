use duckdb::Connection;

/// Registers the `normalized_cost` VIEW for a FOCUS 1.0 dataset.
///
/// `files`: list of absolute URIs or local paths to Parquet files.
/// The VIEW maps FOCUS 1.0 column names to Costlytics canonical names.
///
/// Call this once per DuckDB connection after any required extension init.
/// The view uses `read_parquet(..., filename=true)` so each row carries its
/// originating file path as `source_file`.
///
/// FOCUS 1.0 is a strict subset of FOCUS 1.2: it lacks `ServiceSubcategory`,
/// `AvailabilityZone`, `ResourceName`, `ResourceType`, `ChargeClass`,
/// `ChargeDescription`, `PricingCategory`, `Tags`, and commitment discount
/// columns, along with the AWS `x_ServiceCode` extension. Those canonical
/// columns are populated with NULL to keep the same column SET as
/// `focus12.rs`'s and `cur2.rs`'s views.
pub fn register_view(conn: &Connection, files: &[String]) -> Result<(), duckdb::Error> {
    if files.is_empty() {
        return Err(duckdb::Error::InvalidQuery);
    }
    let file_list = build_file_list(files);
    let sql = format!(
        r#"CREATE OR REPLACE VIEW normalized_cost AS
SELECT
    -- Billing period
    CAST(BillingPeriodStart AS TIMESTAMP)        AS billing_period_start,
    CAST(BillingPeriodEnd   AS TIMESTAMP)        AS billing_period_end,

    -- Charge period (canonical usage window)
    CAST(ChargePeriodStart  AS TIMESTAMP)        AS usage_start,
    CAST(ChargePeriodEnd    AS TIMESTAMP)        AS usage_end,

    -- Account hierarchy
    CAST(BillingAccountId   AS VARCHAR)          AS billing_account_id,
    CAST(BillingAccountName AS VARCHAR)          AS billing_account_name,
    CAST(SubAccountId       AS VARCHAR)          AS account_id,
    CAST(SubAccountName     AS VARCHAR)          AS account_name,

    -- Provider
    CAST(ProviderName       AS VARCHAR)          AS provider,
    CAST(PublisherName      AS VARCHAR)          AS publisher,

    -- Service
    CAST(ServiceName        AS VARCHAR)          AS service_name,
    NULL::VARCHAR                                AS service_code,
    CAST(ServiceCategory    AS VARCHAR)          AS service_category,
    NULL::VARCHAR                                AS service_subcategory,

    -- Location
    CAST(RegionName         AS VARCHAR)          AS region,
    NULL::VARCHAR                                AS availability_zone,

    -- Resource
    CAST(ResourceId         AS VARCHAR)          AS resource_id,
    NULL::VARCHAR                                AS resource_name,
    NULL::VARCHAR                                AS resource_type,

    -- Charge classification
    CAST(ChargeCategory     AS VARCHAR)          AS charge_category,
    NULL::VARCHAR                                AS charge_class,
    CAST(ChargeFrequency    AS VARCHAR)          AS charge_frequency,
    NULL::VARCHAR                                AS charge_description,
    NULL::VARCHAR                                AS pricing_category,

    -- Usage
    CAST(ConsumedQuantity   AS DOUBLE)           AS usage_quantity,
    CAST(ConsumedUnit       AS VARCHAR)          AS usage_unit,

    -- Cost metrics
    CAST(BilledCost         AS DOUBLE)           AS billed_cost,
    CAST(EffectiveCost      AS DOUBLE)           AS amortized_cost,
    CAST(ListCost           AS DOUBLE)           AS list_cost,
    CAST(ContractedCost     AS DOUBLE)           AS contracted_cost,

    -- Currency
    CAST(BillingCurrency    AS VARCHAR)          AS currency,

    -- Tags (not present in FOCUS 1.0)
    NULL::MAP(VARCHAR, VARCHAR)                  AS tags,

    -- Commitment info (not present in FOCUS 1.0)
    NULL::VARCHAR                                AS commitment_id,
    NULL::VARCHAR                                AS commitment_type,
    NULL::VARCHAR                                AS commitment_status,

    -- Provenance
    'focus10'                                    AS source_format,
    filename                                     AS source_file

FROM read_parquet({file_list}, filename=true, hive_partitioning=false)
"#,
        file_list = file_list
    );
    conn.execute_batch(&sql)?;
    Ok(())
}

/// Build a DuckDB array literal from a list of file paths.
/// Single quotes in paths are escaped as `''`.
fn build_file_list(files: &[String]) -> String {
    let quoted: Vec<String> = files
        .iter()
        .map(|f| format!("'{}'", f.replace('\'', "''")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    const FOCUS10_SELECT: &str = r#"SELECT
    TIMESTAMP '2026-08-01 00:00:00'    AS BillingPeriodStart,
    TIMESTAMP '2026-09-01 00:00:00'    AS BillingPeriodEnd,
    TIMESTAMP '2026-08-15 00:00:00'    AS ChargePeriodStart,
    TIMESTAMP '2026-08-16 00:00:00'    AS ChargePeriodEnd,
    'acct-001'     AS BillingAccountId,
    'Acct Name'    AS BillingAccountName,
    'sub-001'      AS SubAccountId,
    'Sub Name'     AS SubAccountName,
    'AWS'          AS ProviderName,
    'Amazon'       AS PublisherName,
    'EC2'          AS ServiceName,
    'Compute'      AS ServiceCategory,
    'us-east-1'    AS RegionName,
    'i-abc123'     AS ResourceId,
    'Usage'        AS ChargeCategory,
    'Recurring'    AS ChargeFrequency,
    8.0            AS ConsumedQuantity,
    'Hours'        AS ConsumedUnit,
    1.234          AS BilledCost,
    1.100          AS EffectiveCost,
    1.500          AS ListCost,
    1.200          AS ContractedCost,
    'USD'          AS BillingCurrency
  FROM (VALUES (NULL)) AS t(dummy)"#;

    use std::sync::OnceLock;

    /// Ensure the parquet extension is installed to `~/.duckdb` exactly once per
    /// test binary invocation. After this, all connections can `LOAD parquet`
    /// without network access (the file is already cached locally).
    fn ensure_parquet_installed() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("LOAD parquet;").unwrap();
        });
    }

    /// Open an in-memory DuckDB connection with parquet loaded.
    fn open_conn() -> Connection {
        ensure_parquet_installed();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        conn
    }

    /// Write a Parquet file via DuckDB COPY into a TempDir. Returns (dir, path).
    /// TempDir is returned so the caller keeps it alive; the file is inside it.
    fn write_parquet(sql_select: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("data.parquet")
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let conn = open_conn();
        conn.execute_batch(&format!(
            "COPY ({sql_select}) TO '{path}' (FORMAT PARQUET)"
        ))
        .unwrap();
        (dir, path)
    }

    #[test]
    fn focus10_view_registers_and_queryable() {
        let (_dir, path) = write_parquet(FOCUS10_SELECT);
        let conn = open_conn();
        register_view(&conn, &[path]).unwrap();

        let mut stmt = conn.prepare("SELECT COUNT(*) FROM normalized_cost").unwrap();
        let count: i64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert!(count > 0, "expected at least one row in normalized_cost");
    }

    #[test]
    fn focus10_view_columns_accessible() {
        let (_dir, path) = write_parquet(FOCUS10_SELECT);
        let conn = open_conn();
        register_view(&conn, &[path]).unwrap();

        // Verify all key canonical columns are present and readable
        let mut stmt = conn
            .prepare(
                r#"SELECT
                    billing_period_start, billing_period_end,
                    usage_start, usage_end,
                    billing_account_id, billing_account_name,
                    account_id, account_name,
                    provider, publisher,
                    service_name, service_code, service_category, service_subcategory,
                    region, availability_zone,
                    resource_id, resource_name, resource_type,
                    charge_category, charge_class, charge_frequency, charge_description,
                    pricing_category,
                    usage_quantity, usage_unit,
                    billed_cost, amortized_cost, list_cost, contracted_cost,
                    currency,
                    tags,
                    commitment_id, commitment_type, commitment_status,
                    source_format, source_file
                FROM normalized_cost
                LIMIT 1"#,
            )
            .unwrap();

        let rows: Vec<_> = stmt
            .query_map([], |row| {
                let billed_cost: f64 = row.get(26)?;
                let amortized_cost: f64 = row.get(27)?;
                let source_format: String = row.get(35)?;
                Ok((billed_cost, amortized_cost, source_format))
            })
            .unwrap()
            .collect();

        assert_eq!(rows.len(), 1);
        let (billed_cost, amortized_cost, source_format) = rows[0].as_ref().unwrap();
        assert!((billed_cost - 1.234).abs() < 1e-9);
        assert!((amortized_cost - 1.100).abs() < 1e-9);
        assert_eq!(source_format, "focus10");
    }

    #[test]
    fn register_view_empty_files_returns_error() {
        let conn = open_conn();
        let result = register_view(&conn, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn build_file_list_escapes_single_quotes() {
        let files = vec!["path/to/it's.parquet".to_string()];
        let list = build_file_list(&files);
        assert_eq!(list, "['path/to/it''s.parquet']");
    }
}
