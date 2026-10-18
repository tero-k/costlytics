//! Daily cost per service and per account: the anomaly detector's input.
//! One scan of `normalized_cost` via GROUPING SETS (cost guard estimates it
//! as a single scan of the window).

use std::collections::BTreeSet;

use chrono::NaiveDate;
use domain::anomaly::{DailySeriesRow, ScopeKind};
use domain::cost::CostMetric;
use duckdb::Connection;

use super::summary::QueryError;

/// Key for rows whose service/account column is NULL.
pub const NONE_KEY: &str = "(none)";

/// Charge categories detection looks at: usage, net of negotiated discounts
/// (EDP, private rates), which accrue daily with usage. Tax, purchases
/// (Support, Savings Plan / RI fees), credits, refunds and adjustments are
/// billed in lumps, typically on the 1st, and would fire every month.
pub const DETECTED_CHARGE_CATEGORIES: &[&str] = &["Usage", "Discount"];

/// `[start, end)` by `usage_start` date, `DETECTED_CHARGE_CATEGORIES` only.
/// Errors with `MultipleCurrencies` rather than summing different currencies
/// into one series.
pub fn query(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
    metric: CostMetric,
) -> Result<Vec<DailySeriesRow>, QueryError> {
    let sql = format!(
        "WITH t AS ( \
           SELECT CAST(usage_start AS DATE) AS day, service_name, account_id, currency, {metric} AS cost \
           FROM normalized_cost \
           WHERE usage_start >= CAST(? AS TIMESTAMP) AND usage_start < CAST(? AS TIMESTAMP) \
             AND charge_category IN ({categories}) \
         ) \
         SELECT GROUPING(service_name) AS by_account, service_name, account_id, currency, \
                strftime(day, '%Y-%m-%d') AS day_str, COALESCE(SUM(cost), 0) AS total \
         FROM t \
         GROUP BY GROUPING SETS ((day, service_name, currency), (day, account_id, currency)) \
         ORDER BY by_account, service_name, account_id, day_str",
        metric = metric.column_name(),
        categories = DETECTED_CHARGE_CATEGORIES.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(", ")
    );
    let start_str = format!("{start} 00:00:00");
    let end_str = format!("{end} 00:00:00");
    let mut stmt = conn.prepare(&sql)?;
    let raw: Vec<(
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        f64,
    )> = stmt
        .query_map(duckdb::params![start_str, end_str], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let currencies: BTreeSet<String> = raw.iter().filter_map(|r| r.3.clone()).collect();
    if currencies.len() > 1 {
        return Err(QueryError::MultipleCurrencies(
            currencies.into_iter().collect(),
        ));
    }

    raw.into_iter()
        .map(|(by_account, service, account, _currency, day, cost)| {
            let (scope_kind, key) = if by_account == 0 {
                (ScopeKind::Service, service)
            } else {
                (ScopeKind::Account, account)
            };
            let day = NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                .map_err(|_| QueryError::InvalidTimestamp(day.clone()))?;
            Ok(DailySeriesRow {
                scope_kind,
                scope_key: key.unwrap_or_else(|| NONE_KEY.to_string()),
                day,
                cost,
            })
        })
        .collect()
}
