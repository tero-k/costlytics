use chrono::{DateTime, NaiveDateTime, Utc};
use domain::cost::{
    BreakdownResult, BreakdownRow, CompareResult, CompareRow, CostSummary, TimeSeriesPoint,
    TimeSeriesResult, COL_ACCOUNT_ID, COL_REGION, COL_SERVICE_NAME, COL_TAGS,
};
use domain::dimensions::Dimension;
use domain::filters::CostFilter;
use thiserror::Error;

/// Safety cap on the number of points a single `timeseries()` call may
/// return, mirroring `MAX_BREAKDOWN_LIMIT` in the API layer. A caller asking
/// for a wide date range at fine granularity with a high-cardinality
/// `group_by` could otherwise ask DuckDB (and this process) to materialize
/// millions of rows. Results are ordered by `period` ascending, so a
/// truncated result is still a sensible prefix (earliest periods first).
const MAX_TIMESERIES_POINTS: usize = 10_000;

/// Safety cap on the number of distinct values returned by the
/// `distinct_*` filter-value lookup methods (used to populate dropdown
/// filters in the UI). These queries are not scoped to a date range, so an
/// unbounded dataset with high-cardinality columns (e.g. resource IDs
/// masquerading as tag values) could otherwise return an unbounded result.
const MAX_FILTER_VALUES: usize = 1000;

/// Safety cap on the number of rows a single `compare()` call may return
/// when grouped by dimension, mirroring `MAX_BREAKDOWN_LIMIT`/`breakdown()`'s
/// own cap. Rows are ordered by the magnitude of their change (largest
/// first), so a truncated result still surfaces the most significant deltas.
const MAX_COMPARE_ROWS: usize = 1000;

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
    ) -> Result<TimeSeriesResult, QueryError>;

    fn breakdown(
        &self,
        filter: &CostFilter,
        dimension: Dimension,
        limit: usize,
    ) -> Result<BreakdownResult, QueryError>;

    /// Period-over-period comparison: aggregates `current` and `previous`
    /// independently (each using its own filter's date range and shared
    /// metric), then combines them either as a single aggregate row
    /// (`dimension: None`) or grouped by a dimension, with rows present in
    /// only one period still appearing (0 on the missing side).
    fn compare(
        &self,
        current: &CostFilter,
        previous: &CostFilter,
        dimension: Option<Dimension>,
    ) -> Result<CompareResult, QueryError>;

    /// Distinct, sorted service names across the whole dataset (not scoped to
    /// a date range) — used to populate dropdown filters.
    fn distinct_services(&self) -> Result<Vec<String>, QueryError>;

    /// Distinct, sorted account IDs across the whole dataset.
    fn distinct_accounts(&self) -> Result<Vec<String>, QueryError>;

    /// Distinct, sorted regions across the whole dataset.
    fn distinct_regions(&self) -> Result<Vec<String>, QueryError>;

    /// Distinct, sorted tag keys across the whole dataset.
    fn distinct_tag_keys(&self) -> Result<Vec<String>, QueryError>;

    /// Distinct, sorted tag values for a single tag key across the whole
    /// dataset.
    fn distinct_tag_values(&self, key: &str) -> Result<Vec<String>, QueryError>;
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
    ) -> Result<TimeSeriesResult, QueryError> {
        let conn = self.pool.get()?;

        let metric_col = filter.metric.column_name();
        let unit = filter.granularity.date_trunc_unit();

        let start_str = filter.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str = filter.end.format("%Y-%m-%d %H:%M:%S").to_string();

        let currency = check_single_currency(&conn, &start_str, &end_str)?;

        let points = match grouping {
            None => {
                let sql = format!(
                    "SELECT CAST(date_trunc('{unit}', usage_start) AS VARCHAR) AS period, \
                     SUM({metric}) AS total, COUNT(*) AS row_count \
                     FROM normalized_cost \
                     WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
                     GROUP BY period \
                     ORDER BY period \
                     LIMIT ?",
                    unit = unit,
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let rows: Vec<(String, f64, i64)> = stmt
                    .query_map(
                        duckdb::params![start_str, end_str, MAX_TIMESERIES_POINTS as i64],
                        |row| {
                            let period: String = row.get(0)?;
                            let total: f64 = row.get(1)?;
                            let row_count: i64 = row.get(2)?;
                            Ok((period, total, row_count))
                        },
                    )?
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
                     ORDER BY period, grp \
                     LIMIT ?",
                    unit = unit,
                    dim_col = dim_col,
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let rows: Vec<(String, Option<String>, f64, i64)> = stmt
                    .query_map(
                        duckdb::params![start_str, end_str, MAX_TIMESERIES_POINTS as i64],
                        |row| {
                            let period: String = row.get(0)?;
                            let grp: Option<String> = row.get(1)?;
                            let total: f64 = row.get(2)?;
                            let row_count: i64 = row.get(3)?;
                            Ok((period, grp, total, row_count))
                        },
                    )?
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

        Ok(TimeSeriesResult { currency, points })
    }

    fn breakdown(
        &self,
        filter: &CostFilter,
        dimension: Dimension,
        limit: usize,
    ) -> Result<BreakdownResult, QueryError> {
        let conn = self.pool.get()?;

        let metric_col = filter.metric.column_name();
        let dim_col = dimension.sql_column();

        let start_str = filter.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let end_str = filter.end.format("%Y-%m-%d %H:%M:%S").to_string();

        let currency = check_single_currency(&conn, &start_str, &end_str)?;

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

        Ok(BreakdownResult { currency, rows })
    }

    fn compare(
        &self,
        current: &CostFilter,
        previous: &CostFilter,
        dimension: Option<Dimension>,
    ) -> Result<CompareResult, QueryError> {
        let conn = self.pool.get()?;

        // Both periods are compared using the current filter's metric — the
        // two periods must be measured the same way for the delta to mean
        // anything.
        let metric_col = current.metric.column_name();

        let current_start_str = current.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let current_end_str = current.end.format("%Y-%m-%d %H:%M:%S").to_string();
        let previous_start_str = previous.start.format("%Y-%m-%d %H:%M:%S").to_string();
        let previous_end_str = previous.end.format("%Y-%m-%d %H:%M:%S").to_string();

        // Check single-currency for each period independently, then confirm
        // the two periods agree with each other — otherwise the comparison
        // would silently mix currencies. A period with zero matching rows
        // (e.g. a synthetic "no data yet" previous period, or a genuinely
        // empty window) reports an empty currency string rather than a real
        // one; that's not a currency conflict, just an absence of data, so
        // it's exempted from the mismatch check and the other period's
        // (non-empty) currency is used.
        let current_currency = check_single_currency(&conn, &current_start_str, &current_end_str)?;
        let previous_currency =
            check_single_currency(&conn, &previous_start_str, &previous_end_str)?;
        let currency = if current_currency.is_empty() {
            previous_currency
        } else if previous_currency.is_empty() || previous_currency == current_currency {
            current_currency
        } else {
            return Err(QueryError::MultipleCurrencies(vec![
                previous_currency,
                current_currency,
            ]));
        };

        let rows = match dimension {
            None => {
                let sql = format!(
                    "SELECT SUM(CASE WHEN usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) THEN {metric} END) AS current_total, \
                     SUM(CASE WHEN usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) THEN {metric} END) AS previous_total \
                     FROM normalized_cost \
                     WHERE (usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP)) \
                        OR (usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP))",
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let (current_total, previous_total): (Option<f64>, Option<f64>) = stmt
                    .query_row(
                        duckdb::params![
                            current_start_str,
                            current_end_str,
                            previous_start_str,
                            previous_end_str,
                            current_start_str,
                            current_end_str,
                            previous_start_str,
                            previous_end_str,
                        ],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;

                let current_total = current_total.unwrap_or(0.0);
                let previous_total = previous_total.unwrap_or(0.0);

                vec![build_compare_row(None, current_total, previous_total)]
            }
            Some(dim) => {
                let dim_col = dim.sql_column();
                let sql = format!(
                    "WITH current_agg AS ( \
                        SELECT {dim_col} AS key, SUM({metric}) AS total \
                        FROM normalized_cost \
                        WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
                        GROUP BY key \
                     ), \
                     previous_agg AS ( \
                        SELECT {dim_col} AS key, SUM({metric}) AS total \
                        FROM normalized_cost \
                        WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
                        GROUP BY key \
                     ) \
                     SELECT \
                        COALESCE(c.key, p.key) AS key, \
                        COALESCE(c.total, 0) AS current, \
                        COALESCE(p.total, 0) AS previous \
                     FROM current_agg c \
                     FULL OUTER JOIN previous_agg p ON c.key IS NOT DISTINCT FROM p.key \
                     ORDER BY ABS(COALESCE(c.total, 0) - COALESCE(p.total, 0)) DESC \
                     LIMIT ?",
                    dim_col = dim_col,
                    metric = metric_col
                );

                let mut stmt = conn.prepare(&sql)?;
                let raw_rows: Vec<(Option<String>, f64, f64)> = stmt
                    .query_map(
                        duckdb::params![
                            current_start_str,
                            current_end_str,
                            previous_start_str,
                            previous_end_str,
                            MAX_COMPARE_ROWS as i64,
                        ],
                        |row| {
                            let key: Option<String> = row.get(0)?;
                            let current_total: f64 = row.get(1)?;
                            let previous_total: f64 = row.get(2)?;
                            Ok((key, current_total, previous_total))
                        },
                    )?
                    .collect::<Result<_, _>>()?;

                raw_rows
                    .into_iter()
                    .map(|(key, current_total, previous_total)| {
                        build_compare_row(key, current_total, previous_total)
                    })
                    .collect()
            }
        };

        Ok(CompareResult { currency, rows })
    }

    fn distinct_services(&self) -> Result<Vec<String>, QueryError> {
        self.distinct_column_values(COL_SERVICE_NAME)
    }

    fn distinct_accounts(&self) -> Result<Vec<String>, QueryError> {
        self.distinct_column_values(COL_ACCOUNT_ID)
    }

    fn distinct_regions(&self) -> Result<Vec<String>, QueryError> {
        self.distinct_column_values(COL_REGION)
    }

    fn distinct_tag_keys(&self) -> Result<Vec<String>, QueryError> {
        let conn = self.pool.get()?;

        let sql = format!(
            "SELECT DISTINCT k FROM ( \
                SELECT UNNEST(map_keys({tags})) AS k \
                FROM normalized_cost WHERE {tags} IS NOT NULL \
             ) \
             ORDER BY k \
             LIMIT ?",
            tags = COL_TAGS
        );

        let mut stmt = conn.prepare(&sql)?;
        let keys: Vec<String> = stmt
            .query_map(duckdb::params![MAX_FILTER_VALUES as i64], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(keys)
    }

    fn distinct_tag_values(&self, key: &str) -> Result<Vec<String>, QueryError> {
        let conn = self.pool.get()?;

        let sql = format!(
            "SELECT DISTINCT v FROM ( \
                SELECT UNNEST(map_keys({tags})) AS k, UNNEST(map_values({tags})) AS v \
                FROM normalized_cost WHERE {tags} IS NOT NULL \
             ) \
             WHERE k = ? \
             ORDER BY v \
             LIMIT ?",
            tags = COL_TAGS
        );

        let mut stmt = conn.prepare(&sql)?;
        let values: Vec<String> = stmt
            .query_map(duckdb::params![key, MAX_FILTER_VALUES as i64], |row| {
                row.get(0)
            })?
            .collect::<Result<_, _>>()?;

        Ok(values)
    }
}

impl DuckDbCostRepository {
    /// Shared implementation for `distinct_services`/`distinct_accounts`/
    /// `distinct_regions`: distinct, sorted, non-null values of a single
    /// column across the whole dataset, capped at `MAX_FILTER_VALUES`.
    fn distinct_column_values(&self, column: &str) -> Result<Vec<String>, QueryError> {
        let conn = self.pool.get()?;

        let sql = format!(
            "SELECT DISTINCT {column} FROM normalized_cost \
             WHERE {column} IS NOT NULL \
             ORDER BY {column} \
             LIMIT ?",
            column = column
        );

        let mut stmt = conn.prepare(&sql)?;
        let values: Vec<String> = stmt
            .query_map(duckdb::params![MAX_FILTER_VALUES as i64], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(values)
    }
}

/// Detect a multi-currency situation for the given window: refuse to silently
/// report a partial total. Shared by `timeseries()` and `breakdown()` (`summary()`
/// computes currency totals directly since it needs the per-currency breakdown).
///
/// Returns the single currency present in the window, or `""` if the window
/// contains no rows at all (consistent with `summary()`'s empty-range behavior).
fn check_single_currency(
    conn: &duckdb::Connection,
    start_str: &str,
    end_str: &str,
) -> Result<String, QueryError> {
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
    Ok(currencies.into_iter().next().unwrap_or_default())
}

/// Build a `CompareRow` from raw current/previous totals, computing
/// `absolute_change` and `percentage_change` in Rust (simple per-row
/// arithmetic on already-aggregated numbers, not raw-row aggregation).
fn build_compare_row(key: Option<String>, current: f64, previous: f64) -> CompareRow {
    let absolute_change = current - previous;
    let percentage_change = if previous != 0.0 {
        Some(absolute_change / previous * 100.0)
    } else {
        None
    };
    CompareRow {
        key,
        current,
        previous,
        absolute_change,
        percentage_change,
    }
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

        let result = repo.timeseries(&filter, None).unwrap();
        assert_eq!(result.currency, "USD");
        let points = result.points;

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

        let points = repo.timeseries(&filter, None).unwrap().points;

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
            .unwrap()
            .points;

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

        let result = repo
            .breakdown(&filter, domain::dimensions::Dimension::Service, 10)
            .unwrap();
        assert_eq!(result.currency, "USD");
        let rows = result.rows;

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
            .unwrap()
            .rows;

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

    // -----------------------------------------------------------------------
    // distinct_* filter-value lookup tests
    // -----------------------------------------------------------------------

    /// Three rows carrying real tag data (unlike the shared fixtures above,
    /// whose `Tags` column is always NULL) plus varied service/account/region
    /// values, to exercise `distinct_services`/`distinct_accounts`/
    /// `distinct_regions`/`distinct_tag_keys`/`distinct_tag_values`.
    const TAGGED_SELECT: &str = r#"
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
            map {'Environment': 'production', 'Team': 'platform'} AS Tags,
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
            'acct-002', 'Acct Name 2', 'sub-002', 'Sub Name 2',
            'AWS', 'Amazon',
            'S3', 'Storage', 'Object Storage',
            'eu-west-1', 'eu-west-1a',
            'bucket-abc', 'my-bucket', 'S3Bucket',
            'Usage', NULL::VARCHAR, 'Recurring', 'S3 usage', 'OnDemand',
            100.0, 'GB',
            0.00, 200.00, 0.00, 0.00,
            'USD',
            map {'Environment': 'staging'},
            NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR,
            NULL::VARCHAR
        UNION ALL
        SELECT
            TIMESTAMP '2026-08-01 00:00:00',
            TIMESTAMP '2026-09-01 00:00:00',
            TIMESTAMP '2026-08-25 00:00:00',
            TIMESTAMP '2026-08-26 00:00:00',
            'acct-001', 'Acct Name', 'sub-001', 'Sub Name',
            'AWS', 'Amazon',
            'EC2', 'Compute', 'VMs',
            'us-east-1', 'us-east-1a',
            'i-def456', 'my-instance-2', 'm5.large',
            'Usage', NULL::VARCHAR, 'Recurring', 'EC2 usage', 'OnDemand',
            4.0, 'Hours',
            50.00, 50.00, 50.00, 50.00,
            'USD',
            NULL::MAP(VARCHAR, VARCHAR),
            NULL::VARCHAR, NULL::VARCHAR, NULL::VARCHAR,
            NULL::VARCHAR
    "#;

    #[test]
    fn distinct_services_returns_sorted_unique() {
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let services = repo.distinct_services().unwrap();
        assert_eq!(services, vec!["EC2".to_string(), "S3".to_string()]);
    }

    #[test]
    fn distinct_accounts_returns_sorted_unique() {
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        // account_id maps from SubAccountId (see focus12 adapter), not
        // BillingAccountId.
        let accounts = repo.distinct_accounts().unwrap();
        assert_eq!(accounts, vec!["sub-001".to_string(), "sub-002".to_string()]);
    }

    #[test]
    fn distinct_regions_returns_sorted_unique() {
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let regions = repo.distinct_regions().unwrap();
        assert_eq!(
            regions,
            vec!["eu-west-1".to_string(), "us-east-1".to_string()]
        );
    }

    #[test]
    fn distinct_tag_keys_returns_all_keys_across_rows() {
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let keys = repo.distinct_tag_keys().unwrap();
        assert_eq!(keys, vec!["Environment".to_string(), "Team".to_string()]);
    }

    #[test]
    fn distinct_tag_values_filters_by_key() {
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let env_values = repo.distinct_tag_values("Environment").unwrap();
        assert_eq!(
            env_values,
            vec!["production".to_string(), "staging".to_string()]
        );

        let team_values = repo.distinct_tag_values("Team").unwrap();
        assert_eq!(team_values, vec!["platform".to_string()]);

        // A nonexistent key returns an empty list, not an error.
        let missing = repo.distinct_tag_values("NoSuchKey").unwrap();
        assert!(missing.is_empty());
    }

    // -----------------------------------------------------------------------
    // compare() tests
    // -----------------------------------------------------------------------

    /// "Previous period" fixture: July 2026, EC2 only at 900.00 amortized
    /// (no S3 row) — the asymmetric-presence case that exercises the
    /// `FULL OUTER JOIN` in `compare()`'s grouped path.
    const PREVIOUS_PERIOD_SELECT: &str = r#"
        SELECT
            TIMESTAMP '2026-07-01 00:00:00'  AS BillingPeriodStart,
            TIMESTAMP '2026-08-01 00:00:00'  AS BillingPeriodEnd,
            TIMESTAMP '2026-07-10 00:00:00'  AS ChargePeriodStart,
            TIMESTAMP '2026-07-11 00:00:00'  AS ChargePeriodEnd,
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
            950.00          AS BilledCost,
            900.00          AS EffectiveCost,
            1000.00         AS ListCost,
            950.00          AS ContractedCost,
            'USD'           AS BillingCurrency,
            NULL::MAP(VARCHAR, VARCHAR) AS Tags,
            NULL::VARCHAR   AS CommitmentDiscountId,
            NULL::VARCHAR   AS CommitmentDiscountType,
            NULL::VARCHAR   AS CommitmentDiscountStatus,
            NULL::VARCHAR   AS x_ServiceCode
    "#;

    /// Build a test pool backed by both the current-period (August, FOCUS
    /// 1.2 INLINE_SELECT) and previous-period (July, PREVIOUS_PERIOD_SELECT)
    /// fixtures, registered together as one `normalized_cost` view.
    fn build_compare_pool() -> (tempfile::TempDir, tempfile::TempDir, duckdb_pool::DbPool) {
        let (dir1, path1) = write_parquet(INLINE_SELECT);
        let (dir2, path2) = write_parquet(PREVIOUS_PERIOD_SELECT);
        let pool = build_test_pool(&[path1, path2]);
        (dir1, dir2, pool)
    }

    #[test]
    fn compare_no_dimension_current_vs_previous() {
        let (_dir1, _dir2, pool) = build_compare_pool();
        let repo = DuckDbCostRepository::new(pool);

        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        );

        let result = repo.compare(&current, &previous, None).unwrap();
        assert_eq!(result.currency, "USD");
        assert_eq!(result.rows.len(), 1);

        let row = &result.rows[0];
        assert_eq!(row.key, None);
        assert!((row.current - 1234.56).abs() < 1e-6, "current: {}", row.current);
        assert!((row.previous - 900.00).abs() < 1e-6, "previous: {}", row.previous);
        assert!(
            (row.absolute_change - 334.56).abs() < 1e-6,
            "absolute_change: {}",
            row.absolute_change
        );
        let pct = row.percentage_change.expect("percentage_change present");
        assert!((pct - 37.173333333).abs() < 1e-3, "percentage_change: {}", pct);
    }

    #[test]
    fn compare_by_service_includes_service_missing_from_previous() {
        let (_dir1, _dir2, pool) = build_compare_pool();
        let repo = DuckDbCostRepository::new(pool);

        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        );

        let result = repo
            .compare(&current, &previous, Some(Dimension::Service))
            .unwrap();
        assert_eq!(result.currency, "USD");
        assert_eq!(result.rows.len(), 2);

        let ec2 = result
            .rows
            .iter()
            .find(|r| r.key.as_deref() == Some("EC2"))
            .expect("EC2 row present");
        assert!((ec2.current - 1034.56).abs() < 1e-6, "EC2 current: {}", ec2.current);
        assert!((ec2.previous - 900.00).abs() < 1e-6, "EC2 previous: {}", ec2.previous);
        assert!(
            (ec2.absolute_change - 134.56).abs() < 1e-6,
            "EC2 absolute_change: {}",
            ec2.absolute_change
        );

        let s3 = result
            .rows
            .iter()
            .find(|r| r.key.as_deref() == Some("S3"))
            .expect("S3 row present (missing from previous period)");
        assert!((s3.current - 200.00).abs() < 1e-6, "S3 current: {}", s3.current);
        assert_eq!(s3.previous, 0.0, "S3 previous should be 0.0 (no July row)");
        assert!(
            (s3.absolute_change - 200.00).abs() < 1e-6,
            "S3 absolute_change: {}",
            s3.absolute_change
        );
        assert_eq!(
            s3.percentage_change, None,
            "S3 percentage_change should be None (previous == 0)"
        );
    }

    #[test]
    fn compare_zero_previous_total_gives_none_percentage() {
        let (_dir1, _dir2, pool) = build_compare_pool();
        let repo = DuckDbCostRepository::new(pool);

        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        );

        let result = repo
            .compare(&current, &previous, Some(Dimension::Service))
            .unwrap();

        let s3 = result
            .rows
            .iter()
            .find(|r| r.key.as_deref() == Some("S3"))
            .expect("S3 row present");
        assert_eq!(s3.previous, 0.0);
        assert_eq!(s3.percentage_change, None);
    }

    /// A previous period with zero matching rows (no data at all, not just
    /// a zero total for one dimension key) must not be mistaken for a
    /// currency conflict: `check_single_currency` returns `""` for an empty
    /// result set, which is an absence of data, not a second currency.
    #[test]
    fn compare_empty_previous_period_does_not_error() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );
        // January 2026: no rows in the fixture at all.
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
        );

        let result = repo.compare(&current, &previous, None).unwrap();
        assert_eq!(result.currency, "USD");
        assert_eq!(result.rows.len(), 1);

        let row = &result.rows[0];
        assert_eq!(row.previous, 0.0);
        assert_eq!(row.percentage_change, None);
        assert!((row.current - 1234.56).abs() < 1e-6);
    }

    #[test]
    fn compare_multi_currency_returns_error() {
        let (_dir, path) = write_parquet(TWO_CURRENCY_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let current = CostFilter::date_range(start, end);
        let previous = CostFilter::date_range(start, end);

        let err = repo.compare(&current, &previous, None).unwrap_err();
        match err {
            QueryError::MultipleCurrencies(currencies) => {
                assert_eq!(currencies, vec!["EUR", "USD"]);
            }
            other => panic!("expected MultipleCurrencies error, got {other:?}"),
        }
    }

    /// A current period with zero matching rows (previous period has real
    /// data) must resolve to the previous period's currency, not error and
    /// not be mistaken for a currency conflict. This exercises the
    /// `current_currency.is_empty()` branch of compare()'s currency
    /// resolution, the mirror image of
    /// `compare_empty_previous_period_does_not_error`.
    #[test]
    fn compare_empty_current_period_does_not_error() {
        let (_dir, path) = write_parquet(INLINE_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        // January 2026: no rows in the fixture at all.
        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
        );
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );

        let result = repo.compare(&current, &previous, None).unwrap();
        assert_eq!(result.currency, "USD");
        assert_eq!(result.rows.len(), 1);

        let row = &result.rows[0];
        assert_eq!(row.current, 0.0);
        assert!((row.previous - 1234.56).abs() < 1e-6, "previous: {}", row.previous);
        let pct = row.percentage_change.expect("percentage_change present");
        assert!((pct - (-100.0)).abs() < 1e-6, "percentage_change: {}", pct);
    }

    // -----------------------------------------------------------------------
    // EUR-only fixture (July 2026), used together with INLINE_SELECT
    // (USD-only, August 2026) to exercise a genuine *cross-period* currency
    // mismatch in compare() — as opposed to TWO_CURRENCY_SELECT, which mixes
    // USD and EUR within a single period and so trips check_single_currency's
    // own internal single-period guard before compare()'s cross-period
    // comparison logic is ever reached.
    // -----------------------------------------------------------------------
    const EUR_ONLY_JULY_SELECT: &str = r#"
            SELECT
                TIMESTAMP '2026-07-01 00:00:00' AS BillingPeriodStart,
                TIMESTAMP '2026-08-01 00:00:00' AS BillingPeriodEnd,
                TIMESTAMP '2026-07-10 00:00:00' AS ChargePeriodStart,
                TIMESTAMP '2026-07-11 00:00:00' AS ChargePeriodEnd,
                'acct-001' AS BillingAccountId, 'Acct' AS BillingAccountName,
                'sub-001' AS SubAccountId, 'Sub' AS SubAccountName,
                'AWS' AS ProviderName, 'Amazon' AS PublisherName,
                'EC2' AS ServiceName, 'Compute' AS ServiceCategory, 'VMs' AS ServiceSubcategory,
                'us-east-1' AS RegionName, 'us-east-1a' AS AvailabilityZone,
                'r-3' AS ResourceId, 'r' AS ResourceName, 't' AS ResourceType,
                'Usage' AS ChargeCategory, NULL::VARCHAR AS ChargeClass,
                'Recurring' AS ChargeFrequency, 'desc' AS ChargeDescription, 'OnDemand' AS PricingCategory,
                1.0 AS ConsumedQuantity, 'Hours' AS ConsumedUnit,
                300.0 AS BilledCost, 300.0 AS EffectiveCost,
                300.0 AS ListCost, 300.0 AS ContractedCost,
                'EUR' AS BillingCurrency,
                NULL::MAP(VARCHAR, VARCHAR) AS Tags,
                NULL::VARCHAR AS CommitmentDiscountId,
                NULL::VARCHAR AS CommitmentDiscountType,
                NULL::VARCHAR AS CommitmentDiscountStatus,
                NULL::VARCHAR AS x_ServiceCode
        "#;

    /// A genuine cross-period currency mismatch: the current period (August,
    /// USD-only) and previous period (July, EUR-only) are each internally
    /// single-currency, so `check_single_currency` accepts both individually
    /// — the conflict can only be caught by compare()'s cross-period
    /// comparison, which is exactly the `else` branch this test targets.
    #[test]
    fn compare_cross_period_currency_mismatch_returns_error() {
        let (_dir1, path1) = write_parquet(INLINE_SELECT);
        let (_dir2, path2) = write_parquet(EUR_ONLY_JULY_SELECT);
        let pool = build_test_pool(&[path1, path2]);
        let repo = DuckDbCostRepository::new(pool);

        let current = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        );
        let previous = CostFilter::date_range(
            Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
        );

        let err = repo.compare(&current, &previous, None).unwrap_err();
        match err {
            QueryError::MultipleCurrencies(currencies) => {
                assert_eq!(currencies, vec!["EUR", "USD"]);
            }
            other => panic!("expected MultipleCurrencies error, got {other:?}"),
        }
    }

    #[test]
    fn distinct_services_respects_limit_cap() {
        // Sanity check that the bound LIMIT ? is actually wired up: with
        // MAX_FILTER_VALUES == 1000 and only 2 distinct services in this
        // fixture, the cap has no visible effect here, but confirms the
        // query executes correctly with the bound parameter present and
        // returns the full (uncapped-in-practice) result.
        let (_dir, path) = write_parquet(TAGGED_SELECT);
        let pool = build_test_pool(&[path]);
        let repo = DuckDbCostRepository::new(pool);

        let services = repo.distinct_services().unwrap();
        assert!(services.len() <= MAX_FILTER_VALUES);
        assert_eq!(services.len(), 2);
    }
}
