/// End-to-end HTTP integration test: proves that a client hitting
/// `POST /api/v1/cost/summary` over real HTTP (via `Router::oneshot`) gets back
/// the correct total, sourced from a FOCUS 1.2 Parquet fixture discovered and
/// registered through the full `build_app` startup path (discovery -> schema
/// detection -> view registration), not just at the repository layer.
use api::build_app;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use data::config::{AppConfig, DataSource, ServerConfig, SourceType};
use data::fixtures::{generate_focus12_fixture, FIXTURE_AMORTIZED_TOTAL_AUG};
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
