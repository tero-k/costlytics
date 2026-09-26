/// `ScanIndex` built from real Parquet footers of the generated fixtures
/// (FOCUS 1.2: one file per month, August and September 2026; CUR 2.0).
///
/// All tests are offline (`LOAD parquet;` only).
use chrono::NaiveDate;
use data::discovery::discover_partitions;
use data::fixtures::{generate_cur2_fixture, generate_focus12_fixture};
use data::object_store::LocalObjectStore;
use data::scan_index::{EstimateColumns, ScanIndex, CUR2_COLUMNS, FOCUS_COLUMNS};
use tempfile::TempDir;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn build(
    generate: fn(&std::path::Path) -> Result<(), data::fixtures::FixtureError>,
    columns: &EstimateColumns,
) -> (TempDir, Vec<String>, ScanIndex) {
    let dir = TempDir::new().unwrap();
    generate(dir.path()).unwrap();
    let files: Vec<String> =
        discover_partitions(&LocalObjectStore, dir.path().to_str().unwrap(), d(2026, 1, 1), d(2027, 1, 1))
            .unwrap()
            .into_iter()
            .flat_map(|p| p.files)
            .collect();
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch("LOAD parquet;").unwrap();
    let index = ScanIndex::build(&conn, &files, columns).unwrap();
    (dir, files, index)
}

#[test]
fn focus12_estimate_follows_the_range() {
    let (_dir, files, index) = build(generate_focus12_fixture, &FOCUS_COLUMNS);
    assert_eq!(index.file_count, files.len());
    assert!(index.entries.iter().all(|e| e.min_ts.is_some() && e.max_ts.is_some()), "{index:?}");

    let aug = index.estimate(&[(d(2026, 8, 1), d(2026, 9, 1))]);
    let sep = index.estimate(&[(d(2026, 9, 1), d(2026, 10, 1))]);
    let both = index.estimate(&[(d(2026, 8, 1), d(2026, 10, 1))]);
    let none = index.estimate(&[(d(2025, 1, 1), d(2025, 2, 1))]);

    assert!(aug.bytes > 0 && sep.bytes > 0);
    assert_eq!((aug.files, sep.files, both.files), (1, 1, 2));
    assert_eq!(both.bytes, aug.bytes + sep.bytes);
    assert_eq!(none.bytes, 0);
    assert!(!both.stats_missing);

    // Only the estimate columns are counted, so never more than the files.
    let total: u64 = files.iter().map(|f| std::fs::metadata(f).unwrap().len()).sum();
    assert!(both.bytes < total, "{} vs {total}", both.bytes);
}

#[test]
fn cur2_estimate_finds_date_stats() {
    let (_dir, _files, index) = build(generate_cur2_fixture, &CUR2_COLUMNS);
    assert!(!index.entries.is_empty());
    assert!(index.entries.iter().all(|e| e.min_ts.is_some()), "{index:?}");
    let all = index.estimate(&[(d(2026, 1, 1), d(2027, 1, 1))]);
    assert!(all.bytes > 0);
    assert!(!all.stats_missing);
}
