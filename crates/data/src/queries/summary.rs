use domain::cost::CostSummary;
use domain::filters::CostFilter;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueryError {
    #[error("DuckDB error: {0}")]
    DuckDb(#[from] duckdb::Error),
    #[error("Pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("Multiple currencies: {0:?}")]
    MultipleCurrencies(Vec<String>),
}

/// Abstraction over cost queries.
pub trait CostRepository: Send + Sync {
    fn summary(&self, filter: &CostFilter) -> Result<CostSummary, QueryError>;
}

pub struct DuckDbCostRepository {
    pool: crate::duckdb_pool::DbPool,
}

impl DuckDbCostRepository {
    pub fn new(pool: crate::duckdb_pool::DbPool) -> Self {
        Self { pool }
    }
}

impl CostRepository for DuckDbCostRepository {
    fn summary(&self, filter: &CostFilter) -> Result<CostSummary, QueryError> {
        let conn = self.pool.get()?;
        let start_instant = std::time::Instant::now();

        let metric_col = filter.metric.column_name();

        // Bind timestamps as strings and cast in SQL — duckdb-rs does not support
        // DateTime<Utc> directly.
        let start_str = filter.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str = filter.end.format("%Y-%m-%d %H:%M:%S").to_string();

        // Aggregate per currency in DuckDB — never pull raw rows into Rust.
        let sql = format!(
            "SELECT currency, SUM({metric}) AS total, COUNT(*) AS row_count \
             FROM normalized_cost \
             WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
             GROUP BY currency \
             ORDER BY currency",
            metric = metric_col
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<(String, f64, i64)> = stmt
            .query_map(duckdb::params![start_str, end_str], |row| {
                let currency: String = row.get(0)?;
                let total: f64 = row.get(1)?;
                let row_count: i64 = row.get(2)?;
                Ok((currency, total, row_count))
            })?
            .collect::<Result<_, _>>()?;

        // Detect multi-currency situation.
        let currencies: Vec<String> = rows.iter().map(|(c, _, _)| c.clone()).collect();

        let multi_currency_warning = if currencies.len() > 1 {
            Some(currencies.clone())
        } else {
            None
        };

        // Use the first (lexicographically smallest) currency and its total.
        // If there are no rows, return zeros.
        let (currency, total, row_count) = rows
            .into_iter()
            .next()
            .map(|(c, t, r)| (c, t, r as u64))
            .unwrap_or_else(|| (String::new(), 0.0, 0u64));

        // Fetch source_format (best-effort, LIMIT 1).
        let source_format = fetch_source_format(&conn, &start_str, &end_str);

        let query_ms = start_instant.elapsed().as_millis() as u64;

        Ok(CostSummary {
            metric: filter.metric,
            currency,
            total,
            row_count,
            source_format,
            query_ms,
            multi_currency_warning,
        })
    }
}

/// Fetch the `source_format` value for the given time window (LIMIT 1).
/// Returns `None` on any error or if no rows are present.
fn fetch_source_format(
    conn: &duckdb::Connection,
    start_str: &str,
    end_str: &str,
) -> Option<String> {
    let sql = "SELECT source_format FROM normalized_cost \
               WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
               LIMIT 1";
    let mut stmt = conn.prepare(sql).ok()?;
    stmt.query_map(duckdb::params![start_str, end_str], |row| row.get(0))
        .ok()?
        .next()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::focus12;
    use crate::duckdb_pool;
    use chrono::TimeZone;
    use chrono::Utc;
    use domain::cost::CostMetric;
    use domain::filters::CostFilter;
    use duckdb::Connection;
    use std::sync::OnceLock;

    // -----------------------------------------------------------------------
    // Inline FOCUS 1.2 rows used by unit tests (no fixture files on disk).
    // August 2026: two rows summing to 1234.56 amortized / 1100.00 billed.
    // -----------------------------------------------------------------------
    const INLINE_SELECT: &str = r#"
        SELECT
            TIMESTAMP '2026-08-01 00:00:00'  AS BillingPeriodStart,
            TIMESTAMP '2026-09-01 00:00:00'  AS BillingPeriodEnd,
            TIMESTAMP '2026-08-10 00:00:00'  AS ChargePeriodStart,
            TIMESTAMP '2026-08-11 00:00:00'  AS ChargePeriodEnd,
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
            NULL::VARCHAR   AS x_ServiceCode
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
            NULL::VARCHAR
    "#;

    fn ensure_parquet_installed() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("INSTALL parquet; LOAD parquet;").unwrap();
        });
    }

    fn open_conn() -> Connection {
        ensure_parquet_installed();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        conn
    }

    /// Write an inline SELECT to a Parquet file and return (TempDir, path).
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

    /// Build a pool whose underlying DuckDB instance already has the parquet
    /// extension loaded and the normalized_cost VIEW registered.
    fn build_test_pool(files: &[String]) -> duckdb_pool::DbPool {
        let pool = duckdb_pool::build_pool().unwrap();
        {
            let conn = pool.get().unwrap();
            conn.execute_batch("LOAD parquet;").unwrap();
            focus12::register_view(&conn, files).unwrap();
        }
        pool
    }

    #[test]
    fn summary_amortized_august_2026() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);
        // default metric is Amortized

        let summary = repo.summary(&filter).unwrap();

        // 1034.56 + 200.00 = 1234.56
        assert!(
            (summary.total - 1234.56).abs() < 1e-6,
            "amortized total mismatch: {}",
            summary.total
        );
        assert_eq!(summary.currency, "USD");
        assert_eq!(summary.row_count, 2);
        assert!(summary.multi_currency_warning.is_none());
        assert_eq!(summary.metric, CostMetric::Amortized);
        assert_eq!(summary.source_format.as_deref(), Some("focus12"));
    }

    #[test]
    fn summary_billed_august_2026() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let mut filter = CostFilter::date_range(start, end);
        filter.metric = CostMetric::Billed;

        let summary = repo.summary(&filter).unwrap();

        // 1100.00 + 0.00 = 1100.00
        assert!(
            (summary.total - 1100.00).abs() < 1e-6,
            "billed total mismatch: {}",
            summary.total
        );
        assert_eq!(summary.metric, CostMetric::Billed);
    }

    #[test]
    fn summary_empty_range_returns_zeros() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        // July 2026 — no data
        let start = Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let summary = repo.summary(&filter).unwrap();

        assert_eq!(summary.total, 0.0);
        assert_eq!(summary.row_count, 0);
        assert_eq!(summary.currency, "");
        assert!(summary.multi_currency_warning.is_none());
    }

    #[test]
    fn summary_multi_currency_warning() {
        // Build a two-currency dataset by UNIONing a USD row and a EUR row.
        let two_currency_select = r#"
            SELECT
                TIMESTAMP '2026-08-01 00:00:00' AS BillingPeriodStart,
                TIMESTAMP '2026-09-01 00:00:00' AS BillingPeriodEnd,
                TIMESTAMP '2026-08-10 00:00:00' AS ChargePeriodStart,
                TIMESTAMP '2026-08-11 00:00:00' AS ChargePeriodEnd,
                'acct-001' AS BillingAccountId, 'Acct' AS BillingAccountName,
                'sub-001' AS SubAccountId, 'Sub' AS SubAccountName,
                'AWS' AS ProviderName, 'Amazon' AS PublisherName,
                'EC2' AS ServiceName, 'Compute' AS ServiceCategory, 'VMs' AS ServiceSubcategory,
                'us-east-1' AS RegionName, 'us-east-1a' AS AvailabilityZone,
                'r-1' AS ResourceId, 'r' AS ResourceName, 't' AS ResourceType,
                'Usage' AS ChargeCategory, NULL::VARCHAR AS ChargeClass,
                'Recurring' AS ChargeFrequency, 'desc' AS ChargeDescription, 'OnDemand' AS PricingCategory,
                1.0 AS ConsumedQuantity, 'Hours' AS ConsumedUnit,
                100.0 AS BilledCost, 100.0 AS EffectiveCost,
                100.0 AS ListCost, 100.0 AS ContractedCost,
                'EUR' AS BillingCurrency,
                NULL::MAP(VARCHAR, VARCHAR) AS Tags,
                NULL::VARCHAR AS CommitmentDiscountId,
                NULL::VARCHAR AS CommitmentDiscountType,
                NULL::VARCHAR AS CommitmentDiscountStatus,
                NULL::VARCHAR AS x_ServiceCode
            UNION ALL
            SELECT
                TIMESTAMP '2026-08-01 00:00:00', TIMESTAMP '2026-09-01 00:00:00',
                TIMESTAMP '2026-08-15 00:00:00', TIMESTAMP '2026-08-16 00:00:00',
                'acct-001', 'Acct', 'sub-001', 'Sub',
                'AWS', 'Amazon', 'S3', 'Storage', 'Object',
                'us-east-1', 'us-east-1a', 'r-2', 'r', 't',
                'Usage', NULL::VARCHAR, 'Recurring', 'desc', 'OnDemand',
                1.0, 'GB',
                200.0, 200.0, 200.0, 200.0,
                'USD',
                NULL::MAP(VARCHAR, VARCHAR),
                NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR
        "#;

        let (_dir, path) = write_parquet(two_currency_select);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let summary = repo.summary(&filter).unwrap();

        // First alphabetically is EUR
        assert_eq!(summary.currency, "EUR");
        assert!(
            (summary.total - 100.0).abs() < 1e-6,
            "expected EUR total 100.0, got {}",
            summary.total
        );
        assert!(summary.multi_currency_warning.is_some());
        let warning = summary.multi_currency_warning.unwrap();
        assert_eq!(warning, vec!["EUR", "USD"]);
    }
}
