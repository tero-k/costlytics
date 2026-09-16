/// End-to-end HTTP integration test: proves that a client hitting
/// `POST /api/v1/cost/summary` over real HTTP (via `Router::oneshot`) gets back
/// the correct total, sourced from a FOCUS 1.2 Parquet fixture discovered and
/// registered through the full `build_app` startup path (discovery -> schema
/// detection -> view registration), not just at the repository layer.
use api::build_app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use data::config::{AppConfig, DataSource, ServerConfig, SourceType};
use data::fixtures::{
    generate_cur2_fixture, generate_focus12_fixture, CUR_GOLDEN_TOTAL_AMORTIZED,
    CUR_GOLDEN_TOTAL_BILLED, FIXTURE_AMORTIZED_TOTAL_AUG,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn http_cost_summary_matches_fixture_total() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
        }],
    };

    let app = build_app(config).expect("build_app should succeed");

    let payload = serde_json::json!({
        "start": "2026-08-01",
        "end": "2026-09-01",
        "metric": "amortized"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/summary")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let total = json["total"].as_f64().expect("total must be a number");
    assert!(
        (total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "expected total ~{}, got {} (full response: {})",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        total,
        json
    );
    assert_eq!(json["currency"], "USD");
}

/// Parallel end-to-end HTTP integration test for CUR 2.0: proves that a
/// client hitting `POST /api/v1/cost/summary` over real HTTP gets back the
/// correct amortized and billed totals, sourced from a CUR 2.0 Parquet
/// fixture discovered and registered through the full `build_app` startup
/// path (discovery -> schema detection -> view registration).
async fn cur2_cost_summary_total(metric: &str) -> f64 {
    let dir = tempfile::tempdir().unwrap();
    generate_cur2_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        sources: vec![DataSource {
            id: "test-cur2-source".into(),
            name: "Test CUR 2.0 fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Cur2,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
        }],
    };

    let app = build_app(config).expect("build_app should succeed");

    let payload = serde_json::json!({
        "start": "2026-08-01",
        "end": "2026-09-01",
        "metric": metric
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/summary")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let total = json["total"].as_f64().expect("total must be a number");
    assert_eq!(json["currency"], "USD");
    total
}

#[tokio::test]
async fn http_cost_summary_matches_cur2_fixture_total_amortized() {
    let total = cur2_cost_summary_total("amortized").await;
    assert!(
        (total - CUR_GOLDEN_TOTAL_AMORTIZED).abs() < 0.01,
        "expected amortized total ~{}, got {}",
        CUR_GOLDEN_TOTAL_AMORTIZED,
        total
    );
}

#[tokio::test]
async fn http_cost_summary_matches_cur2_fixture_total_billed() {
    let total = cur2_cost_summary_total("billed").await;
    assert!(
        (total - CUR_GOLDEN_TOTAL_BILLED).abs() < 0.01,
        "expected billed total ~{}, got {}",
        CUR_GOLDEN_TOTAL_BILLED,
        total
    );
}

/// End-to-end HTTP integration test for the Cost Explorer `timeseries`
/// endpoint: proves that `POST /api/v1/cost/timeseries` over real HTTP,
/// sourced from a FOCUS 1.2 Parquet fixture discovered and registered
/// through the full `build_app` startup path, returns the correct monthly
/// ungrouped total for August 2026.
#[tokio::test]
async fn http_timeseries_matches_fixture_total() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
        }],
    };

    let app = build_app(config).expect("build_app should succeed");

    let payload = serde_json::json!({
        "start": "2026-08-01",
        "end": "2026-09-01",
        "metric": "amortized",
        "granularity": "month"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/timeseries")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["currency"], "USD");
    let series = json["series"].as_array().expect("series must be an array");
    assert_eq!(series.len(), 1, "expected exactly one monthly point");

    let total = series[0]["total"]
        .as_f64()
        .expect("total must be a number");
    assert!(
        (total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "expected total ~{}, got {} (full response: {})",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        total,
        json
    );
}

/// End-to-end HTTP integration test for the filter-value lookup and
/// `compare` endpoints (Session 4, Tasks 1-3): proves that
/// `GET /api/v1/filter-values/services` and `POST /api/v1/cost/compare`
/// over real HTTP, sourced from a FOCUS 1.2 Parquet fixture discovered and
/// registered through the full `build_app` startup path, return correct
/// results.
#[tokio::test]
async fn http_filter_values_and_compare_over_real_http() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
        }],
    };

    let app = build_app(config).expect("build_app should succeed");

    // GET /api/v1/filter-values/services
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/filter-values/services")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let values = json["values"].as_array().expect("values must be an array");
    let names: Vec<&str> = values.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        names.contains(&"EC2"),
        "expected 'EC2' among filter values, got {:?}",
        names
    );

    // POST /api/v1/cost/compare: current period covers the fixture's real
    // August 2026 data, previous period has no matching rows.
    let payload = serde_json::json!({
        "current_start": "2026-08-01",
        "current_end": "2026-09-01",
        "previous_start": "2026-01-01",
        "previous_end": "2026-02-01",
        "metric": "amortized"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/compare")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["currency"], "USD");

    let row = &json["rows"][0];
    let current = row["current"].as_f64().expect("current must be a number");
    assert!(
        (current - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "expected current ~{}, got {} (full response: {})",
        FIXTURE_AMORTIZED_TOTAL_AUG,
        current,
        json
    );
    assert_eq!(row["previous"], serde_json::json!(0.0));
    assert_eq!(row["percentage_change"], serde_json::Value::Null);
}
