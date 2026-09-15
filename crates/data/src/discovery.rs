use chrono::{Datelike, NaiveDate};
use thiserror::Error;

use crate::manifests::{parse_manifest, DatasetPartition};
use crate::object_store::{ObjectStore, YearMonth};

/// Errors that can occur during partition discovery.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("Object store error: {0}")]
    ObjectStore(#[from] crate::object_store::ObjectStoreError),
    #[error("Manifest parse error: {0}")]
    Manifest(#[from] crate::manifests::ManifestError),
}

/// Return the billing-period months that overlap the range `[start, end_exclusive)`.
///
/// A partition is included if ANY day of that calendar month falls within the range.
pub fn partitions_for_range(start: NaiveDate, end_exclusive: NaiveDate) -> Vec<YearMonth> {
    if start >= end_exclusive {
        return Vec::new();
    }

    let start_ym = YearMonth::new(start.year(), start.month());
    // end_exclusive is the first day NOT in range; but we still include the month
    // it falls in if the range reaches into that month at all (i.e. end_exclusive > first day of that month).
    // Actually: a month overlaps iff its start_date < end_exclusive AND its end_date_exclusive > start.
    // The last month to consider: the month containing (end_exclusive - 1 day), or the month just
    // before end_exclusive if end_exclusive is the first of the month.
    let last_day_in_range = end_exclusive - chrono::Duration::days(1);
    let end_ym = YearMonth::new(last_day_in_range.year(), last_day_in_range.month());

    let mut result = Vec::new();
    let mut current = start_ym;
    loop {
        result.push(current.clone());
        if current == end_ym {
            break;
        }
        // Advance to next month
        current = if current.month == 12 {
            YearMonth::new(current.year + 1, 1)
        } else {
            YearMonth::new(current.year, current.month + 1)
        };
    }
    result
}

/// Discover all manifest partitions covering the given date range.
///
/// For each billing period that overlaps `[start, end_exclusive)`, this function
/// looks for a `Manifest.json` file at `{base_uri}/BILLING_PERIOD={YYYY-MM}/Manifest.json`.
/// Periods where no manifest is found are skipped with a `tracing::warn`.
pub fn discover_partitions(
    store: &dyn ObjectStore,
    base_uri: &str,
    start: NaiveDate,
    end_exclusive: NaiveDate,
) -> Result<Vec<DatasetPartition>, DiscoveryError> {
    let periods = partitions_for_range(start, end_exclusive);
    let mut partitions = Vec::new();

    for period in periods {
        let partition_dir = format!(
            "{}/BILLING_PERIOD={}",
            base_uri.trim_end_matches('/'),
            period.to_ym_string()
        );
        let manifest_uri = format!("{}/Manifest.json", partition_dir);

        match store.read_file(&manifest_uri) {
            Ok(content) => {
                match parse_manifest(period.clone(), &manifest_uri, &content, &partition_dir) {
                    Ok(partition) => partitions.push(partition),
                    Err(e) => {
                        tracing::warn!(
                            period = %period,
                            manifest_uri = %manifest_uri,
                            error = %e,
                            "Failed to parse Manifest.json for billing period"
                        );
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    period = %period,
                    manifest_uri = %manifest_uri,
                    "No Manifest.json found for billing period, skipping"
                );
            }
        }
    }

    Ok(partitions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use crate::object_store::LocalObjectStore;
    use std::fs;
    use tempfile::TempDir;

    // ── partitions_for_range tests (required by brief) ──────────────────────

    #[test]
    fn partitions_for_range_single_month() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(); // exclusive
        let periods = partitions_for_range(start, end);
        assert_eq!(periods, vec![YearMonth::new(2026, 8)]);
    }

    #[test]
    fn partitions_for_range_spans_three_months() {
        let start = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 16).unwrap();
        let periods = partitions_for_range(start, end);
        assert_eq!(
            periods,
            vec![
                YearMonth::new(2026, 7),
                YearMonth::new(2026, 8),
                YearMonth::new(2026, 9),
            ]
        );
    }

    #[test]
    fn partitions_for_range_mid_month_start_and_end() {
        // Range is Aug 15 to Sep 15 — should still include both Aug and Sep partitions
        let start = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        let periods = partitions_for_range(start, end);
        assert_eq!(periods.len(), 2);
        assert!(periods.contains(&YearMonth::new(2026, 8)));
        assert!(periods.contains(&YearMonth::new(2026, 9)));
    }

    #[test]
    fn year_month_end_date_exclusive_december() {
        let ym = YearMonth::new(2026, 12);
        let end = ym.end_date_exclusive();
        assert_eq!(end.year(), 2027);
        assert_eq!(end.month(), 1);
        assert_eq!(end.day(), 1);
    }

    // ── edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn partitions_for_range_empty_when_start_equals_end() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let periods = partitions_for_range(date, date);
        assert!(periods.is_empty());
    }

    #[test]
    fn partitions_for_range_single_day() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 8, 16).unwrap();
        let periods = partitions_for_range(start, end);
        assert_eq!(periods, vec![YearMonth::new(2026, 8)]);
    }

    #[test]
    fn partitions_for_range_year_boundary() {
        let start = NaiveDate::from_ymd_opt(2025, 11, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let periods = partitions_for_range(start, end);
        assert_eq!(
            periods,
            vec![
                YearMonth::new(2025, 11),
                YearMonth::new(2025, 12),
                YearMonth::new(2026, 1),
            ]
        );
    }

    #[test]
    fn partitions_for_range_end_exclusive_is_first_of_month() {
        // Range ends exactly at the first of a month; that month should NOT be included.
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let periods = partitions_for_range(start, end);
        assert_eq!(periods, vec![YearMonth::new(2026, 8)]);
        assert!(!periods.contains(&YearMonth::new(2026, 9)));
    }

    // ── discover_partitions tests ────────────────────────────────────────────

    fn make_manifest_json(files: &[&str]) -> Vec<u8> {
        let json = serde_json::json!({ "dataFiles": files });
        serde_json::to_vec(&json).unwrap()
    }

    #[test]
    fn discover_partitions_finds_manifests() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        // Create BILLING_PERIOD=2026-08/Manifest.json
        let partition_dir = dir.path().join("BILLING_PERIOD=2026-08");
        fs::create_dir(&partition_dir).unwrap();
        let manifest = make_manifest_json(&["data/part-00001.parquet"]);
        fs::write(partition_dir.join("Manifest.json"), &manifest).unwrap();

        let store = LocalObjectStore;
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();

        let partitions = discover_partitions(&store, &base, start, end).unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].billing_period, YearMonth::new(2026, 8));
        assert_eq!(partitions[0].files.len(), 1);
    }

    #[test]
    fn discover_partitions_skips_missing_manifests() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        // Create BILLING_PERIOD=2026-08 but NO Manifest.json
        fs::create_dir(dir.path().join("BILLING_PERIOD=2026-08")).unwrap();
        // Create BILLING_PERIOD=2026-09 WITH a Manifest.json
        let partition_dir_09 = dir.path().join("BILLING_PERIOD=2026-09");
        fs::create_dir(&partition_dir_09).unwrap();
        let manifest = make_manifest_json(&["data/part.parquet"]);
        fs::write(partition_dir_09.join("Manifest.json"), &manifest).unwrap();

        let store = LocalObjectStore;
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

        let partitions = discover_partitions(&store, &base, start, end).unwrap();
        // Only 2026-09 should be returned; 2026-08 is skipped
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].billing_period, YearMonth::new(2026, 9));
    }

    #[test]
    fn discover_partitions_multiple_months() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().to_str().unwrap().to_string();

        for month in [7u32, 8, 9] {
            let pd = dir.path().join(format!("BILLING_PERIOD=2026-{:02}", month));
            fs::create_dir(&pd).unwrap();
            let manifest = make_manifest_json(&[&format!("data/part-{:02}.parquet", month)]);
            fs::write(pd.join("Manifest.json"), &manifest).unwrap();
        }

        let store = LocalObjectStore;
        let start = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

        let partitions = discover_partitions(&store, &base, start, end).unwrap();
        assert_eq!(partitions.len(), 3);
        let bps: Vec<_> = partitions.iter().map(|p| p.billing_period.clone()).collect();
        assert!(bps.contains(&YearMonth::new(2026, 7)));
        assert!(bps.contains(&YearMonth::new(2026, 8)));
        assert!(bps.contains(&YearMonth::new(2026, 9)));
    }
}
