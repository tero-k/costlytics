//! `CostRepository::daily_series` against real Parquet fixtures (FOCUS 1.2
//! anomaly fixture, CUR 2.0 golden fixture) and an inline multi-currency table.
use chrono::{Duration, NaiveDate};
use data::adapters::{cur2, focus12};
use data::discovery::discover_partitions;
use data::duckdb_pool;
use data::fixtures::{
    anomaly_fixture_rows, generate_anomaly_fixture, generate_cur2_fixture, ANOMALY_FIXTURE_DAYS,
    ANOMALY_NEW_SERVICE, ANOMALY_SPIKE_ACCOUNT, ANOMALY_SPIKE_SERVICE, ANOMALY_SUPPORT_SERVICE,
    CUR_ON_DEMAND_USAGE_AMORTIZED, CUR_RI_DISCOUNTED_USAGE_AMORTIZED,
    CUR_SP_COVERED_USAGE_AMORTIZED,
};
use data::object_store::LocalObjectStore;
use data::queries::summary::{CostRepository, DuckDbCostRepository, QueryError};
use domain::anomaly::ScopeKind;
use domain::cost::CostMetric;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn repo_for(
    dir: &std::path::Path,
    register: fn(&duckdb::Connection, &[String]) -> Result<(), duckdb::Error>,
) -> DuckDbCostRepository {
    let partitions = discover_partitions(
        &LocalObjectStore,
        dir.to_str().unwrap(),
        d(2020, 1, 1),
        d(2030, 1, 1),
    )
    .unwrap();
    let files: Vec<String> = partitions.into_iter().flat_map(|p| p.files).collect();
    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch("LOAD parquet;").unwrap();
        register(&conn, &files).unwrap();
    }
    DuckDbCostRepository::new(pool)
}

#[test]
fn anomaly_fixture_series_match_generated_rows() {
    let end = d(2026, 9, 30);
    let dir = tempfile::tempdir().unwrap();
    generate_anomaly_fixture(dir.path(), end).unwrap();
    let repo = repo_for(dir.path(), focus12::register_view);

    let start = end - Duration::days(ANOMALY_FIXTURE_DAYS - 1);
    let rows = repo
        .daily_series(start, end + Duration::days(1), CostMetric::Amortized)
        .unwrap();

    let get = |kind: ScopeKind, key: &str, day: NaiveDate| {
        rows.iter()
            .find(|r| r.scope_kind == kind && r.scope_key == key && r.day == day)
            .map(|r| r.cost)
    };
    let spike_day = end - Duration::days(4);
    assert_eq!(
        get(ScopeKind::Service, ANOMALY_SPIKE_SERVICE, spike_day),
        Some(420.0)
    );
    // Account 111 = EC2 420 + S3 40 on a spike day.
    assert_eq!(
        get(ScopeKind::Account, ANOMALY_SPIKE_ACCOUNT, spike_day),
        Some(460.0)
    );
    assert_eq!(
        get(
            ScopeKind::Service,
            ANOMALY_NEW_SERVICE,
            end - Duration::days(10)
        ),
        None
    );

    // Support (Purchase) and Tax rows on the 1st are excluded from detection.
    let first = d(2026, 9, 1);
    assert!(rows
        .iter()
        .all(|r| r.scope_key != ANOMALY_SUPPORT_SERVICE && r.scope_key != "Tax"));
    let ec2_first = get(ScopeKind::Service, ANOMALY_SPIKE_SERVICE, first).unwrap();
    assert_eq!(
        get(ScopeKind::Account, ANOMALY_SPIKE_ACCOUNT, first),
        Some(ec2_first + 40.0)
    );

    let fixture_total: f64 = anomaly_fixture_rows(end)
        .iter()
        .filter(|r| r.charge_category == "Usage")
        .map(|r| r.cost)
        .sum();
    let service_total: f64 = rows
        .iter()
        .filter(|r| r.scope_kind == ScopeKind::Service)
        .map(|r| r.cost)
        .sum();
    let account_total: f64 = rows
        .iter()
        .filter(|r| r.scope_kind == ScopeKind::Account)
        .map(|r| r.cost)
        .sum();
    assert!((service_total - fixture_total).abs() < 1e-6);
    assert!((account_total - fixture_total).abs() < 1e-6);
}

#[test]
fn cur2_fixture_series_cover_usage_lines_only() {
    let dir = tempfile::tempdir().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();
    let repo = repo_for(dir.path(), cur2::register_view);
    let rows = repo
        .daily_series(d(2026, 8, 1), d(2026, 9, 1), CostMetric::Amortized)
        .unwrap();
    // Usage + DiscountedUsage + SavingsPlanCoveredUsage map to 'Usage'; credits,
    // refunds, tax, fees and SP/RI purchases are excluded.
    let usage = CUR_ON_DEMAND_USAGE_AMORTIZED
        + CUR_RI_DISCOUNTED_USAGE_AMORTIZED
        + CUR_SP_COVERED_USAGE_AMORTIZED;
    for kind in [ScopeKind::Service, ScopeKind::Account] {
        let total: f64 = rows
            .iter()
            .filter(|r| r.scope_kind == kind)
            .map(|r| r.cost)
            .sum();
        assert!((total - usage).abs() < 1e-6, "{kind:?}: {total}");
    }
}

#[test]
fn null_keys_become_none_and_multiple_currencies_are_refused() {
    let pool = duckdb_pool::build_pool().unwrap();
    {
        let conn = pool.get().unwrap();
        conn.execute_batch(
            "CREATE TABLE normalized_cost AS SELECT * FROM (VALUES \
               (TIMESTAMP '2026-09-01 00:00:00', NULL, 'a1', 'USD', 'Usage', 5.0), \
               (TIMESTAMP '2026-09-01 00:00:00', 'EC2', NULL, 'USD', 'Usage', 7.0), \
               (TIMESTAMP '2026-09-01 00:00:00', 'EC2', NULL, 'USD', 'Discount', -1.0), \
               (TIMESTAMP '2026-09-01 00:00:00', 'EC2', NULL, 'USD', 'Tax', 100.0) \
             ) AS t(usage_start, service_name, account_id, currency, charge_category, amortized_cost)",
        )
        .unwrap();
    }
    let repo = DuckDbCostRepository::new(pool.clone());
    let rows = repo
        .daily_series(d(2026, 9, 1), d(2026, 9, 2), CostMetric::Amortized)
        .unwrap();
    assert!(rows
        .iter()
        .any(|r| r.scope_kind == ScopeKind::Service && r.scope_key == "(none)" && r.cost == 5.0));
    // EC2 / (none) account = 7 usage - 1 negotiated discount; the 100 tax is excluded.
    assert!(rows
        .iter()
        .any(|r| r.scope_kind == ScopeKind::Service && r.scope_key == "EC2" && r.cost == 6.0));
    assert!(rows
        .iter()
        .any(|r| r.scope_kind == ScopeKind::Account && r.scope_key == "(none)" && r.cost == 6.0));

    pool.get()
        .unwrap()
        .execute_batch("INSERT INTO normalized_cost VALUES (TIMESTAMP '2026-09-01 00:00:00', 'EC2', 'a1', 'EUR', 'Usage', 1.0)")
        .unwrap();
    let err = repo
        .daily_series(d(2026, 9, 1), d(2026, 9, 2), CostMetric::Amortized)
        .unwrap_err();
    assert!(matches!(err, QueryError::MultipleCurrencies(_)), "{err:?}");
}
