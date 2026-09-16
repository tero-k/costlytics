use chrono::{DateTime, NaiveDateTime, Utc};
use domain::cost::{BreakdownRow, CostSummary, TimeSeriesPoint};
use domain::dimensions::Dimension;
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
    #[error("invalid timestamp returned by DuckDB: {0}")]
    InvalidTimestamp(String),
}

/// Abstraction over cost queries.
pub trait CostRepository: Send + Sync {
    fn summary(&self, filter: &CostFilter) -> Result<CostSummary, QueryError>;

    fn timeseries(
        &self,
        filter: &CostFilter,
        grouping: Option<Dimension>,
    ) -> Result<Vec<TimeSeriesPoint>, QueryError>;

    fn breakdown(
        &self,
        filter: &CostFilter,
        dimension: Dimension,
        limit: usize,
    ) -> Result<Vec<BreakdownRow>, QueryError>;
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

        // Detect multi-currency situation: refuse to silently report a partial total.
        let currencies: Vec<String> = rows.iter().map(|(c, _, _)| c.clone()).collect();
        if currencies.len() > 1 {
            return Err(QueryError::MultipleCurrencies(currencies));
        }

        // At most one currency remains. If there are no rows, return zeros.
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
            start: filter.start,
            end: filter.end,
        })
    }

    fn timeseries(
        &self,
        filter: &CostFilter,
        grouping: Option<Dimension>,
    ) -> Result<Vec<TimeSeriesPoint>, QueryError> {
        let conn = self.pool.get()?;

        let metric_col = filter.metric.column_name();
        let unit = filter.granularity.date_trunc_unit();

        let start_str = filter.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str = filter.end.format("%Y-%m-%d %H:%M:%S").to_string();

        check_single_currency(&conn, &start_str, &end_str)?;

        let points = match grouping {
            None => {
                let sql = format!(
                    "SELECT CAST(date_trunc('{unit}', usage_start) AS VARCHAR) AS period, \
                     SUM({metric}) AS total, COUNT(*) AS row_count \
                     FROM normalized_cost \
                     WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
                     GROUP BY period \
                     ORDER BY period",
                    unit = unit,
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let rows: Vec<(String, f64, i64)> = stmt
                    .query_map(duckdb::params![start_str, end_str], |row| {
                        let period: String = row.get(0)?;
                        let total: f64 = row.get(1)?;
                        let row_count: i64 = row.get(2)?;
                        Ok((period, total, row_count))
                    })?
                    .collect::<Result<_, _>>()?;

                rows.into_iter()
                    .map(|(period, total, row_count)| {
                        Ok(TimeSeriesPoint {
                            period: parse_duckdb_timestamp(&period)?,
                            group: None,
                            total,
                            row_count: row_count as u64,
                        })
                    })
                    .collect::<Result<Vec<_>, QueryError>>()?
            }
            Some(dim) => {
                let dim_col = dim.sql_column();
                let sql = format!(
                    "SELECT CAST(date_trunc('{unit}', usage_start) AS VARCHAR) AS period, \
                     {dim_col} AS grp, SUM({metric}) AS total, COUNT(*) AS row_count \
                     FROM normalized_cost \
                     WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
                     GROUP BY period, grp \
                     ORDER BY period, grp",
                    unit = unit,
                    dim_col = dim_col,
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let rows: Vec<(String, Option<String>, f64, i64)> = stmt
                    .query_map(duckdb::params![start_str, end_str], |row| {
                        let period: String = row.get(0)?;
                        let grp: Option<String> = row.get(1)?;
                        let total: f64 = row.get(2)?;
                        let row_count: i64 = row.get(3)?;
                        Ok((period, grp, total, row_count))
                    })?
                    .collect::<Result<_, _>>()?;

                rows.into_iter()
                    .map(|(period, grp, total, row_count)| {
                        Ok(TimeSeriesPoint {
                            period: parse_duckdb_timestamp(&period)?,
                            group: grp,
                            total,
                            row_count: row_count as u64,
                        })
                    })
                    .collect::<Result<Vec<_>, QueryError>>()?
            }
        };

        Ok(points)
    }

    fn breakdown(
        &self,
        filter: &CostFilter,
        dimension: Dimension,
        limit: usize,
    ) -> Result<Vec<BreakdownRow>, QueryError> {
        let conn = self.pool.get()?;

        let metric_col = filter.metric.column_name();
        let dim_col = dimension.sql_column();

        let start_str = filter.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str = filter.end.format("%Y-%m-%d %H:%M:%S").to_string();

        check_single_currency(&conn, &start_str, &end_str)?;

        let sql = format!(
            "SELECT {dim_col} AS grp, SUM({metric}) AS total, COUNT(*) AS row_count \
             FROM normalized_cost \
             WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
             GROUP BY grp \
             ORDER BY total DESC \
             LIMIT ?",
            dim_col = dim_col,
            metric = metric_col
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<BreakdownRow> = stmt
            .query_map(duckdb::params![start_str, end_str, limit as i64], |row| {
                let key: Option<String> = row.get(0)?;
                let total: f64 = row.get(1)?;
                let row_count: i64 = row.get(2)?;
                Ok(BreakdownRow {
                    key,
                    total,
                    row_count: row_count as u64,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }
}

/// Detect a multi-currency situation for the given window: refuse to silently
/// report a partial total. Shared by `timeseries()` and `breakdown()` (`summary()`
/// computes currency totals directly since it needs the per-currency breakdown).
fn check_single_currency(
    conn: &duckdb::Connection,
    start_str: &str,
    end_str: &str,
) -> Result<(), QueryError> {
    let sql = "SELECT DISTINCT currency FROM normalized_cost \
               WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
               ORDER BY currency";
    let mut stmt = conn.prepare(sql)?;
    let currencies: Vec<String> = stmt
        .query_map(duckdb::params![start_str, end_str], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    if currencies.len() > 1 {
        return Err(QueryError::MultipleCurrencies(currencies));
    }
    Ok(())
}

/// Parse a timestamp string produced by `CAST(... AS VARCHAR)` on a DuckDB
/// TIMESTAMP column (e.g. `"2026-08-01 00:00:00"`) into a UTC `DateTime`.
/// DuckDB stores TIMESTAMP without a timezone; normalized_cost's `usage_start`
/// is always UTC by construction (see adapters), so this is a direct reinterpretation.
fn parse_duckdb_timestamp(s: &str) -> Result<DateTime<Utc>, QueryError> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|naive| naive.and_utc())
        .map_err(|_| QueryError::InvalidTimestamp(s.to_string()))
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
            conn.execute_batch("LOAD parquet;").unwrap();
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
        assert_eq!(summary.metric, CostMetric::Amortized);
        assert_eq!(summary.source_format.as_deref(), Some("focus12"));
        assert_eq!(summary.start, start);
        assert_eq!(summary.end, end);
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
    }

    // -----------------------------------------------------------------------
    // Two-currency fixture (USD + EUR), reused by all multi-currency tests.
    // -----------------------------------------------------------------------
    const TWO_CURRENCY_SELECT: &str = r#"
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

    #[test]
    fn summary_multi_currency_returns_error() {
        let (_dir, path) = write_parquet(TWO_CURRENCY_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let err = repo.summary(&filter).unwrap_err();
        match err {
            QueryError::MultipleCurrencies(currencies) => {
                assert_eq!(currencies, vec!["EUR", "USD"]);
            }
            other => panic!("expected MultipleCurrencies error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // timeseries() tests
    // -----------------------------------------------------------------------

    #[test]
    fn timeseries_ungrouped_monthly() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let mut filter = CostFilter::date_range(start, end);
        filter.granularity = domain::filters::TimeGranularity::Month;

        let points = repo.timeseries(&filter, None).unwrap();

        assert_eq!(points.len(), 1);
        let p = &points[0];
        assert_eq!(p.period, Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap());
        assert_eq!(p.group, None);
        assert!(
            (p.total - 1234.56).abs() < 1e-6,
            "monthly total mismatch: {}",
            p.total
        );
        assert_eq!(p.row_count, 2);
    }

    #[test]
    fn timeseries_ungrouped_daily() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let mut filter = CostFilter::date_range(start, end);
        filter.granularity = domain::filters::TimeGranularity::Day;

        let points = repo.timeseries(&filter, None).unwrap();

        assert_eq!(points.len(), 2);

        let p0 = &points[0];
        assert_eq!(
            p0.period,
            Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap()
        );
        assert!((p0.total - 1034.56).abs() < 1e-6);
        assert_eq!(p0.row_count, 1);

        let p1 = &points[1];
        assert_eq!(
            p1.period,
            Utc.with_ymd_and_hms(2026, 8, 20, 0, 0, 0).unwrap()
        );
        assert!((p1.total - 200.00).abs() < 1e-6);
        assert_eq!(p1.row_count, 1);
    }

    #[test]
    fn timeseries_grouped_by_service() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let mut filter = CostFilter::date_range(start, end);
        filter.granularity = domain::filters::TimeGranularity::Month;

        let points = repo
            .timeseries(&filter, Some(domain::dimensions::Dimension::Service))
            .unwrap();

        assert_eq!(points.len(), 2);

        let ec2 = points
            .iter()
            .find(|p| p.group.as_deref() == Some("EC2"))
            .expect("EC2 point present");
        assert!((ec2.total - 1034.56).abs() < 1e-6);
        assert_eq!(ec2.row_count, 1);

        let s3 = points
            .iter()
            .find(|p| p.group.as_deref() == Some("S3"))
            .expect("S3 point present");
        assert!((s3.total - 200.00).abs() < 1e-6);
        assert_eq!(s3.row_count, 1);
    }

    #[test]
    fn timeseries_multi_currency_returns_error() {
        let (_dir, path) = write_parquet(TWO_CURRENCY_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let err = repo.timeseries(&filter, None).unwrap_err();
        match err {
            QueryError::MultipleCurrencies(currencies) => {
                assert_eq!(currencies, vec!["EUR", "USD"]);
            }
            other => panic!("expected MultipleCurrencies error, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // breakdown() tests
    // -----------------------------------------------------------------------

    #[test]
    fn breakdown_by_service() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let rows = repo
            .breakdown(&filter, domain::dimensions::Dimension::Service, 10)
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key.as_deref(), Some("EC2"));
        assert!((rows[0].total - 1034.56).abs() < 1e-6);
        assert_eq!(rows[0].row_count, 1);

        assert_eq!(rows[1].key.as_deref(), Some("S3"));
        assert!((rows[1].total - 200.00).abs() < 1e-6);
        assert_eq!(rows[1].row_count, 1);
    }

    #[test]
    fn breakdown_respects_limit() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let rows = repo
            .breakdown(&filter, domain::dimensions::Dimension::Service, 1)
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key.as_deref(), Some("EC2"));
    }

    #[test]
    fn breakdown_multi_currency_returns_error() {
        let (_dir, path) = write_parquet(TWO_CURRENCY_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let filter = CostFilter::date_range(start, end);

        let err = repo
            .breakdown(&filter, domain::dimensions::Dimension::Service, 10)
            .unwrap_err();
        match err {
            QueryError::MultipleCurrencies(currencies) => {
                assert_eq!(currencies, vec!["EUR", "USD"]);
            }
            other => panic!("expected MultipleCurrencies error, got {other:?}"),
        }
    }
}
