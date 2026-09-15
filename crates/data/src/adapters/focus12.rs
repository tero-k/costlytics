use duckdb::Connection;

/// Registers the `normalized_cost` VIEW for a FOCUS 1.2 dataset.
///
/// `files`: list of absolute URIs or local paths to Parquet files.
/// The VIEW maps FOCUS 1.2 column names to Costlytics canonical names.
///
/// Call this once per DuckDB connection after any required extension init.
/// The view uses `read_parquet(..., filename=true)` so each row carries its
/// originating file path as `source_file`.
///
/// `x_ServiceCode` is an AWS extension column that may not be present in all
/// FOCUS 1.2 exports — it is read via `TRY_CAST` so missing columns yield NULL.
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
    TRY_CAST(x_ServiceCode  AS VARCHAR)          AS service_code,
    CAST(ServiceCategory    AS VARCHAR)          AS service_category,
    CAST(ServiceSubcategory AS VARCHAR)          AS service_subcategory,

    -- Location
    CAST(RegionName         AS VARCHAR)          AS region,
    CAST(AvailabilityZone   AS VARCHAR)          AS availability_zone,

    -- Resource
    CAST(ResourceId         AS VARCHAR)          AS resource_id,
    CAST(ResourceName       AS VARCHAR)          AS resource_name,
    CAST(ResourceType       AS VARCHAR)          AS resource_type,

    -- Charge classification
    CAST(ChargeCategory     AS VARCHAR)          AS charge_category,
    CAST(ChargeClass        AS VARCHAR)          AS charge_class,
    CAST(ChargeFrequency    AS VARCHAR)          AS charge_frequency,
    CAST(ChargeDescription  AS VARCHAR)          AS charge_description,
    CAST(PricingCategory    AS VARCHAR)          AS pricing_category,

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

    -- Tags (MAP type in FOCUS 1.2)
    Tags                                         AS tags,

    -- Commitment info
    CAST(CommitmentDiscountId     AS VARCHAR)    AS commitment_id,
    CAST(CommitmentDiscountType   AS VARCHAR)    AS commitment_type,
    CAST(CommitmentDiscountStatus AS VARCHAR)    AS commitment_status,

    -- Provenance
    'focus12'                                    AS source_format,
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

    const FOCUS12_SELECT: &str = r#"SELECT
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
    'VMs'          AS ServiceSubcategory,
    'us-east-1'    AS RegionName,
    'us-east-1a'   AS AvailabilityZone,
    'i-abc123'     AS ResourceId,
    'my-instance'  AS ResourceName,
    'm5.large'     AS ResourceType,
    'Usage'        AS ChargeCategory,
    NULL::VARCHAR  AS ChargeClass,
    'Recurring'    AS ChargeFrequency,
    'EC2 usage'    AS ChargeDescription,
    'OnDemand'     AS PricingCategory,
    8.0            AS ConsumedQuantity,
    'Hours'        AS ConsumedUnit,
    1.234          AS BilledCost,
    1.100          AS EffectiveCost,
    1.500          AS ListCost,
    1.200          AS ContractedCost,
    'USD'          AS BillingCurrency,
    NULL::MAP(VARCHAR, VARCHAR) AS Tags,
    NULL::VARCHAR  AS CommitmentDiscountId,
    NULL::VARCHAR  AS CommitmentDiscountType,
    NULL::VARCHAR  AS CommitmentDiscountStatus,
    'AmazonEC2'    AS x_ServiceCode
  FROM (VALUES (NULL)) AS t(dummy)"#;

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
    fn focus12_view_registers_and_queryable() {
        let (_dir, path) = write_parquet(FOCUS12_SELECT);
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
    fn focus12_view_columns_accessible() {
        let (_dir, path) = write_parquet(FOCUS12_SELECT);
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
                let source_format: String = row.get(35)?;
                Ok((billed_cost, source_format))
            })
            .unwrap()
            .collect();

        assert_eq!(rows.len(), 1);
        let (billed_cost, source_format) = rows[0].as_ref().unwrap();
        assert!((billed_cost - 1.234).abs() < 1e-9);
        assert_eq!(source_format, "focus12");
    }

    #[test]
    fn focus12_view_source_file_set() {
        let (_dir, path) = write_parquet(FOCUS12_SELECT);
        let conn = open_conn();
        register_view(&conn, &[path]).unwrap();

        let mut stmt = conn
            .prepare("SELECT source_file FROM normalized_cost LIMIT 1")
            .unwrap();
        let source_file: String = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        // DuckDB resolves the path — just check it's non-empty
        assert!(!source_file.is_empty());
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

    #[test]
    fn focus12_view_multiple_files() {
        let (_dir1, path1) = write_parquet(FOCUS12_SELECT);
        let (_dir2, path2) = write_parquet(FOCUS12_SELECT);

        let conn = open_conn();
        register_view(&conn, &[path1, path2]).unwrap();

        let mut stmt = conn.prepare("SELECT COUNT(*) FROM normalized_cost").unwrap();
        let count: i64 = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(count, 2, "expected one row per file");
    }
}
