/// Guards that queries over the `normalized_cost` view stay pushed down into
/// the Parquet scan: the date-range filter must reach `READ_PARQUET` as a
/// plain column comparison (so row groups outside the range are skipped via
/// Parquet statistics) and only the columns a query uses may be read
/// (projection pushdown). A view change that breaks either — e.g. wrapping
/// the date column in an expression the optimizer cannot see through — would
/// silently turn every query into a full scan of every file, which over S3
/// means downloading whole files.
///
/// All tests are offline (no network calls; `LOAD parquet;` only).
use data::adapters::{cur2, focus10, focus12};
use data::discovery::discover_partitions;
use data::fixtures::{generate_cur2_fixture, generate_focus10_fixture, generate_focus12_fixture};
use data::object_store::LocalObjectStore;
use serde_json::Value;
use tempfile::TempDir;

/// The shape of `CostRepository::summary`'s aggregate query.
const SUMMARY_SQL: &str = "SELECT currency, SUM(amortized_cost), COUNT(*) \
     FROM normalized_cost \
     WHERE usage_start >= CAST('2026-08-01 00:00:00' AS TIMESTAMP) \
       AND usage_start < CAST('2026-09-01 00:00:00' AS TIMESTAMP) \
     GROUP BY currency";

type RegisterView = fn(&duckdb::Connection, &[String]) -> Result<(), duckdb::Error>;

/// Generate a fixture, register its view over every Parquet file, and return
/// the `READ_PARQUET` node's `extra_info` from the summary query's plan.
fn scan_info(generate: fn(&std::path::Path) -> Result<(), data::fixtures::FixtureError>, register: RegisterView) -> String {
    let dir = TempDir::new().unwrap();
    generate(dir.path()).unwrap();
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let end = chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    let files: Vec<String> =
        discover_partitions(&LocalObjectStore, dir.path().to_str().unwrap(), start, end)
            .unwrap()
            .into_iter()
            .flat_map(|p| p.files)
            .collect();
    assert!(!files.is_empty());

    let conn = duckdb::Connection::open_in_memory().unwrap();
    // With statistics propagation on, a filter that the files' min/max stats
    // prove always true (a fixture holding only August rows) is dropped
    // before the plan is shown, leaving nothing to check.
    conn.execute_batch("LOAD parquet; SET disabled_optimizers = 'statistics_propagation';")
        .unwrap();
    register(&conn, &files).unwrap();

    let plan: String = conn
        .query_row(&format!("EXPLAIN (FORMAT json) {SUMMARY_SQL}"), [], |r| r.get(1))
        .unwrap();
    let plan: Value = serde_json::from_str(&plan).unwrap();
    find_scan(&plan).expect("plan has a READ_PARQUET node")
}

fn find_scan(node: &Value) -> Option<String> {
    match node {
        Value::Array(items) => items.iter().find_map(find_scan),
        Value::Object(map) => {
            if map.get("name").and_then(Value::as_str) == Some("READ_PARQUET") {
                return Some(map.get("extra_info").map(Value::to_string).unwrap_or_default());
            }
            map.get("children").and_then(find_scan)
        }
        _ => None,
    }
}

/// The scan's pushed-down filters must compare the bare `date_column` —
/// only a plain column comparison can be checked against row-group min/max
/// statistics. DuckDB also pushes expression filters such as
/// `CAST(CAST(col AS VARCHAR) AS TIMESTAMP) BETWEEN ...` into the scan, but
/// those are evaluated row by row after every row group has been read.
/// `unused` columns (irrelevant to the summary query) must not be read at all.
fn assert_pushed_down(info: &str, date_column: &str, unused: &[&str]) {
    let filters = info
        .split("\"Filters\"")
        .nth(1)
        .unwrap_or_else(|| panic!("no filters pushed into READ_PARQUET: {info}"));
    assert!(
        filters.contains(&format!("{date_column}>=")) && filters.contains(&format!("{date_column}<")),
        "date range on {date_column} not pushed into READ_PARQUET as a statistics-prunable \
         column comparison: {info}"
    );
    for column in unused {
        assert!(
            !info.contains(column),
            "READ_PARQUET reads unused column {column}: {info}"
        );
    }
}

#[test]
fn focus12_summary_pushes_date_filter_and_projection_into_scan() {
    let info = scan_info(generate_focus12_fixture, focus12::register_view);
    assert_pushed_down(&info, "ChargePeriodStart", &["ResourceId", "ChargeDescription", "Tags"]);
}

#[test]
fn focus10_summary_pushes_date_filter_and_projection_into_scan() {
    let info = scan_info(generate_focus10_fixture, focus10::register_view);
    assert_pushed_down(&info, "ChargePeriodStart", &["ResourceId", "ChargeDescription", "Tags"]);
}

#[test]
fn cur2_summary_pushes_date_filter_and_projection_into_scan() {
    let info = scan_info(generate_cur2_fixture, cur2::register_view);
    assert_pushed_down(
        &info,
        "line_item_usage_start_date",
        &["line_item_resource_id", "line_item_line_item_description", "resource_tags"],
    );
}
