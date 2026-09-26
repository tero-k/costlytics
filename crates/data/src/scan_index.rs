//! Per-source index of Parquet row groups, used to estimate how much data a
//! date-range query will pull from S3 before running it.
//!
//! Built once at registration from `parquet_metadata()`, which reads only the
//! files' footers (the same footers DuckDB fetches, and caches, for the first
//! query anyway). The estimate mirrors DuckDB's own pruning: a row group is
//! read only if its usage-start min/max statistics overlap the range, and
//! only the columns a query projects are read (see `tests/pushdown_test.rs`).

use chrono::{NaiveDate, NaiveDateTime};
use duckdb::Connection;
use std::collections::{BTreeMap, HashSet};

/// The physical columns one typical cost query reads for a given format: the
/// usage-start date column (listed first), the currency, the columns behind
/// the default (amortized) metric, and one grouping dimension. Queries
/// grouped by a wider column (e.g. resource id) read somewhat more.
pub struct EstimateColumns {
    pub date_column: &'static str,
    pub columns: &'static [&'static str],
}

pub const FOCUS_COLUMNS: EstimateColumns = EstimateColumns {
    date_column: "ChargePeriodStart",
    columns: &["ChargePeriodStart", "BillingCurrency", "EffectiveCost", "ServiceName"],
};

pub const CUR2_COLUMNS: EstimateColumns = EstimateColumns {
    date_column: "line_item_usage_start_date",
    columns: &[
        "line_item_usage_start_date",
        "line_item_currency_code",
        "line_item_line_item_type",
        "line_item_unblended_cost",
        "savings_plan_savings_plan_effective_cost",
        "savings_plan_total_commitment_to_date",
        "savings_plan_used_commitment",
        "reservation_effective_cost",
        "reservation_unused_amortized_upfront_fee_for_billing_period",
        "reservation_unused_recurring_fee",
        "reservation_arn",
        "product_product_name",
    ],
};

/// One row group's usage-start range and the compressed size of the
/// estimate columns within it.
#[derive(Debug, Clone, PartialEq)]
pub struct RowGroupEntry {
    pub file: usize,
    /// `None` when the file carries no (parseable) statistics for the date
    /// column: DuckDB cannot prune such a row group, so it always counts.
    pub min_ts: Option<NaiveDateTime>,
    pub max_ts: Option<NaiveDateTime>,
    pub bytes: u64,
    pub column_chunks: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanIndex {
    pub entries: Vec<RowGroupEntry>,
    pub file_count: usize,
}

/// What one scan over `[start, end)` is expected to read.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScanEstimate {
    pub bytes: u64,
    /// Approximate S3 GET count: one per column chunk read plus two footer
    /// reads (size probe + footer) per touched file.
    pub requests: u64,
    pub row_groups: usize,
    pub files: usize,
    /// Some matched row groups had no date statistics, so they were counted
    /// regardless of range — the estimate is an upper bound.
    pub stats_missing: bool,
}

/// Requests DuckDB issues per touched file to read its footer.
const FOOTER_REQUESTS_PER_FILE: u64 = 2;

impl ScanIndex {
    /// Builds the index from `files`' footers. `conn` must already be able
    /// to read the files (parquet loaded; S3 secret created for `s3://`).
    pub fn build(conn: &Connection, files: &[String], columns: &EstimateColumns) -> Result<Self, duckdb::Error> {
        if files.is_empty() {
            return Ok(Self::default());
        }
        let file_list = files
            .iter()
            .map(|f| format!("'{}'", f.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT file_name, row_group_id, path_in_schema, total_compressed_size, \
                    stats_min_value, stats_max_value \
             FROM parquet_metadata([{file_list}])"
        );
        let wanted: HashSet<String> = columns.columns.iter().map(|c| c.to_ascii_lowercase()).collect();
        let date_column = columns.date_column.to_ascii_lowercase();

        let mut file_ids: BTreeMap<String, usize> = BTreeMap::new();
        let mut groups: BTreeMap<(usize, i64), RowGroupEntry> = BTreeMap::new();
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let file_name: String = row.get(0)?;
            let row_group: i64 = row.get(1)?;
            let path: String = row.get(2)?;
            let size: Option<i64> = row.get(3)?;
            let min: Option<String> = row.get(4)?;
            let max: Option<String> = row.get(5)?;

            // Nested columns (e.g. a tags MAP) report paths like
            // "Tags, key_value, key"; the top-level name is what matters.
            let top = path.split(", ").next().unwrap_or(&path).to_ascii_lowercase();
            if !wanted.contains(&top) {
                continue;
            }
            let next_id = file_ids.len();
            let file = *file_ids.entry(file_name).or_insert(next_id);
            let entry = groups.entry((file, row_group)).or_insert(RowGroupEntry {
                file,
                min_ts: None,
                max_ts: None,
                bytes: 0,
                column_chunks: 0,
            });
            entry.bytes += size.unwrap_or(0).max(0) as u64;
            entry.column_chunks += 1;
            if top == date_column {
                entry.min_ts = min.as_deref().and_then(parse_stat_timestamp);
                entry.max_ts = max.as_deref().and_then(parse_stat_timestamp);
            }
        }
        Ok(Self { entries: groups.into_values().collect(), file_count: files.len() })
    }

    /// Estimates one scan that reads every row group overlapping any of
    /// `ranges` (half-open `[start, end)` dates, as the queries use).
    pub fn estimate(&self, ranges: &[(NaiveDate, NaiveDate)]) -> ScanEstimate {
        let ranges: Vec<(NaiveDateTime, NaiveDateTime)> = ranges
            .iter()
            .map(|(s, e)| (s.and_hms_opt(0, 0, 0).unwrap(), e.and_hms_opt(0, 0, 0).unwrap()))
            .collect();
        let mut out = ScanEstimate::default();
        let mut files = HashSet::new();
        for entry in &self.entries {
            let hit = match (entry.min_ts, entry.max_ts) {
                (Some(min), Some(max)) => ranges.iter().any(|(s, e)| max >= *s && min < *e),
                _ => {
                    if !ranges.is_empty() {
                        out.stats_missing = true;
                    }
                    !ranges.is_empty()
                }
            };
            if hit {
                out.bytes += entry.bytes;
                out.requests += u64::from(entry.column_chunks);
                out.row_groups += 1;
                files.insert(entry.file);
            }
        }
        out.files = files.len();
        out.requests += FOOTER_REQUESTS_PER_FILE * out.files as u64;
        out
    }
}

/// Parses a Parquet statistics value rendered by DuckDB, e.g.
/// `2026-08-01 00:00:00`, `2026-08-01 00:00:00.123+00`, `2026-08-01T00:00:00Z`
/// or a bare `2026-08-01`. Any offset is ignored: billing exports are UTC.
fn parse_stat_timestamp(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if let Some(head) = value.get(..19) {
        let head = head.replacen('T', " ", 1);
        if let Ok(ts) = NaiveDateTime::parse_from_str(&head, "%Y-%m-%d %H:%M:%S") {
            return Some(ts);
        }
    }
    NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn ts(y: i32, m: u32, day: u32) -> Option<NaiveDateTime> {
        d(y, m, day).and_hms_opt(0, 0, 0)
    }

    fn entry(file: usize, min: Option<NaiveDateTime>, max: Option<NaiveDateTime>, bytes: u64) -> RowGroupEntry {
        RowGroupEntry { file, min_ts: min, max_ts: max, bytes, column_chunks: 4 }
    }

    fn index() -> ScanIndex {
        ScanIndex {
            entries: vec![
                entry(0, ts(2026, 7, 1), ts(2026, 7, 31), 100),
                entry(1, ts(2026, 8, 1), ts(2026, 8, 15), 200),
                entry(1, ts(2026, 8, 15), ts(2026, 8, 31), 300),
            ],
            file_count: 2,
        }
    }

    #[test]
    fn one_month_selects_only_that_months_row_groups() {
        let e = index().estimate(&[(d(2026, 8, 1), d(2026, 9, 1))]);
        assert_eq!((e.bytes, e.row_groups, e.files), (500, 2, 1));
        assert_eq!(e.requests, 8 + FOOTER_REQUESTS_PER_FILE);
        assert!(!e.stats_missing);
    }

    #[test]
    fn range_end_is_exclusive() {
        let e = index().estimate(&[(d(2026, 6, 1), d(2026, 7, 1))]);
        assert_eq!(e, ScanEstimate::default());
    }

    #[test]
    fn several_ranges_count_each_row_group_once() {
        let e = index().estimate(&[(d(2026, 7, 1), d(2026, 9, 1)), (d(2026, 8, 1), d(2026, 9, 1))]);
        assert_eq!((e.bytes, e.row_groups, e.files), (600, 3, 2));
    }

    #[test]
    fn missing_stats_always_count_and_flag_upper_bound() {
        let mut idx = index();
        idx.entries.push(entry(2, None, None, 50));
        let e = idx.estimate(&[(d(2020, 1, 1), d(2020, 2, 1))]);
        assert_eq!(e.bytes, 50);
        assert!(e.stats_missing);
    }

    #[test]
    fn parses_duckdb_stat_renderings() {
        let want = d(2026, 8, 1).and_hms_opt(0, 0, 0);
        assert_eq!(parse_stat_timestamp("2026-08-01 00:00:00"), want);
        assert_eq!(parse_stat_timestamp("2026-08-01 00:00:00.123+00"), want);
        assert_eq!(parse_stat_timestamp("2026-08-01T00:00:00Z"), want);
        assert_eq!(parse_stat_timestamp("2026-08-01"), want);
        assert_eq!(parse_stat_timestamp("garbage"), None);
    }
}
