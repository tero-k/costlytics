use duckdb::Connection;

/// Registers the `normalized_cost` VIEW for a CUR 2.0 / Data Exports dataset.
///
/// `files`: list of absolute URIs or local paths to Parquet files.
/// The VIEW maps AWS CUR 2.0 column names to Costlytics canonical names.
///
/// Call this once per DuckDB connection after any required extension init.
/// The view uses `read_parquet(..., filename=true)` so each row carries its
/// originating file path as `source_file`.
///
/// `product_instance_type` is not present on every CUR line item (e.g. Tax,
/// Credit, Fee rows) — it is read via `TRY_CAST` so missing/absent values
/// yield NULL rather than erroring.
///
/// `amortized_cost` implements AWS-compatible amortization: Savings Plan and
/// Reserved Instance line items are re-mapped onto their effective/unused
/// cost columns instead of the raw unblended cost, so that upfront and
/// recurring commitment fees are spread across the covered usage they pay
/// for. See the CASE expression below for the per-line-item-type mapping.
pub fn register_view(conn: &Connection, files: &[String]) -> Result<(), duckdb::Error> {
    if files.is_empty() {
        return Err(duckdb::Error::InvalidQuery);
    }
    let file_list = build_file_list(files);
    let sql = format!(
        r#"CREATE OR REPLACE VIEW normalized_cost AS
SELECT
    -- Billing period
    CAST(bill_billing_period_start_date AS TIMESTAMP)   AS billing_period_start,
    CAST(bill_billing_period_end_date   AS TIMESTAMP)   AS billing_period_end,

    -- Charge period (canonical usage window)
    CAST(line_item_usage_start_date AS TIMESTAMP)       AS usage_start,
    CAST(line_item_usage_end_date   AS TIMESTAMP)       AS usage_end,

    -- Account hierarchy
    CAST(bill_payer_account_id        AS VARCHAR)       AS billing_account_id,
    CAST(bill_payer_account_name      AS VARCHAR)       AS billing_account_name,
    CAST(line_item_usage_account_id   AS VARCHAR)       AS account_id,
    CAST(line_item_usage_account_name AS VARCHAR)       AS account_name,

    -- Provider
    'AWS'                                                AS provider,
    'Amazon'                                              AS publisher,

    -- Service
    CAST(product_product_name AS VARCHAR)               AS service_name,
    CAST(product_servicecode  AS VARCHAR)               AS service_code,
    NULL::VARCHAR                                        AS service_category,
    NULL::VARCHAR                                        AS service_subcategory,

    -- Location
    CAST(product_region_code        AS VARCHAR)         AS region,
    CAST(line_item_availability_zone AS VARCHAR)        AS availability_zone,

    -- Resource
    CAST(line_item_resource_id AS VARCHAR)              AS resource_id,
    NULL::VARCHAR                                        AS resource_name,
    TRY_CAST(product_instance_type AS VARCHAR)          AS resource_type,

    -- Charge classification
    CASE
        WHEN line_item_line_item_type IN ('Usage', 'DiscountedUsage', 'SavingsPlanCoveredUsage')
            THEN 'Usage'
        WHEN line_item_line_item_type = 'Tax'
            THEN 'Tax'
        WHEN line_item_line_item_type = 'Credit'
            THEN 'Credit'
        WHEN line_item_line_item_type = 'Refund'
            THEN 'Refund'
        WHEN line_item_line_item_type IN ('Fee', 'RIFee', 'SavingsPlanRecurringFee', 'SavingsPlanUpfrontFee')
            THEN 'Purchase'
        WHEN line_item_line_item_type = 'SavingsPlanNegation'
            THEN 'Adjustment'
        ELSE 'Other'
    END                                                  AS charge_category,
    NULL::VARCHAR                                        AS charge_class,
    NULL::VARCHAR                                        AS charge_frequency,
    CAST(line_item_line_item_description AS VARCHAR)    AS charge_description,
    NULL::VARCHAR                                        AS pricing_category,

    -- Usage
    CAST(line_item_usage_amount AS DOUBLE)              AS usage_quantity,
    CAST(pricing_unit           AS VARCHAR)             AS usage_unit,

    -- Cost metrics
    COALESCE(
        CAST(line_item_net_unblended_cost AS DOUBLE),
        CAST(line_item_unblended_cost AS DOUBLE),
        0
    )                                                    AS billed_cost,
    CASE
        WHEN line_item_line_item_type = 'SavingsPlanCoveredUsage'
            THEN CAST(savings_plan_savings_plan_effective_cost AS DOUBLE)

        WHEN line_item_line_item_type = 'SavingsPlanRecurringFee'
            THEN CAST(savings_plan_total_commitment_to_date AS DOUBLE)
                 - CAST(savings_plan_used_commitment AS DOUBLE)

        WHEN line_item_line_item_type IN ('SavingsPlanNegation', 'SavingsPlanUpfrontFee')
            THEN 0

        WHEN line_item_line_item_type = 'DiscountedUsage'
            THEN CAST(reservation_effective_cost AS DOUBLE)

        WHEN line_item_line_item_type = 'RIFee'
            THEN CAST(reservation_unused_amortized_upfront_fee_for_billing_period AS DOUBLE)
                 + CAST(reservation_unused_recurring_fee AS DOUBLE)

        WHEN line_item_line_item_type = 'Fee' AND reservation_arn IS NOT NULL
            THEN 0

        ELSE CAST(line_item_unblended_cost AS DOUBLE)
    END                                                  AS amortized_cost,
    NULL::DOUBLE                                          AS list_cost,
    NULL::DOUBLE                                          AS contracted_cost,

    -- Currency
    CAST(line_item_currency_code AS VARCHAR)            AS currency,

    -- Tags (MAP type, same as FOCUS's Tags column)
    resource_tags                                        AS tags,

    -- Commitment info
    TRY_CAST(reservation_arn AS VARCHAR)                AS commitment_id,
    CASE WHEN reservation_arn IS NOT NULL THEN 'Reservation' ELSE NULL END AS commitment_type,
    NULL::VARCHAR                                        AS commitment_status,

    -- Provenance
    'cur2'                                                AS source_format,
    filename                                              AS source_file

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

    const CUR2_SELECT: &str = r#"SELECT
    TIMESTAMP '2026-08-01 00:00:00'    AS bill_billing_period_start_date,
    TIMESTAMP '2026-09-01 00:00:00'    AS bill_billing_period_end_date,
    TIMESTAMP '2026-08-15 00:00:00'    AS line_item_usage_start_date,
    TIMESTAMP '2026-08-16 00:00:00'    AS line_item_usage_end_date,
    'payer-001'    AS bill_payer_account_id,
    'Payer Name'   AS bill_payer_account_name,
    'acct-001'     AS line_item_usage_account_id,
    'Acct Name'    AS line_item_usage_account_name,
    'Amazon Elastic Compute Cloud' AS product_product_name,
    'AmazonEC2'    AS product_servicecode,
    'us-east-1'    AS product_region_code,
    'us-east-1a'   AS line_item_availability_zone,
    'i-abc123'     AS line_item_resource_id,
    'm5.large'     AS product_instance_type,
    'Usage'        AS line_item_line_item_type,
    'EC2 usage'    AS line_item_line_item_description,
    8.0            AS line_item_usage_amount,
    'Hours'        AS pricing_unit,
    90.0           AS line_item_net_unblended_cost,
    100.0          AS line_item_unblended_cost,
    0.0            AS savings_plan_savings_plan_effective_cost,
    0.0            AS savings_plan_total_commitment_to_date,
    0.0            AS savings_plan_used_commitment,
    0.0            AS reservation_effective_cost,
    0.0            AS reservation_unused_amortized_upfront_fee_for_billing_period,
    0.0            AS reservation_unused_recurring_fee,
    'USD'          AS line_item_currency_code,
    NULL::MAP(VARCHAR, VARCHAR) AS resource_tags,
    NULL::VARCHAR  AS reservation_arn
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
    fn cur2_view_registers_and_queryable() {
        let (_dir, path) = write_parquet(CUR2_SELECT);
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
    fn cur2_view_columns_accessible() {
        let (_dir, path) = write_parquet(CUR2_SELECT);
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
        assert!((billed_cost - 90.0).abs() < 1e-9);
        assert!((amortized_cost - 100.0).abs() < 1e-9);
        assert_eq!(source_format, "cur2");
    }

    #[test]
    fn cur2_charge_category_usage() {
        let (_dir, path) = write_parquet(CUR2_SELECT);
        let conn = open_conn();
        register_view(&conn, &[path]).unwrap();

        let mut stmt = conn
            .prepare("SELECT charge_category FROM normalized_cost LIMIT 1")
            .unwrap();
        let charge_category: String = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(charge_category, "Usage");
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
