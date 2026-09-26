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
    generate_cur2_fixture, generate_focus10_fixture, generate_focus12_fixture,
    CUR_GOLDEN_TOTAL_AMORTIZED, CUR_GOLDEN_TOTAL_BILLED, FIXTURE_AMORTIZED_TOTAL_AUG,
    FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn http_cost_summary_matches_fixture_total() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
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

/// End-to-end HTTP integration test for FOCUS 1.0: proves that a client
/// hitting `POST /api/v1/cost/summary` over real HTTP gets back the correct
/// total, sourced from a FOCUS 1.0 Parquet fixture discovered and registered
/// through the full `build_app` startup path (discovery -> schema detection
/// -> view registration), not just at the repository layer.
#[tokio::test]
async fn http_cost_summary_matches_focus10_fixture_total() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus10_fixture(dir.path()).unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-focus10-source".into(),
            name: "Test FOCUS 1.0 fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus10,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
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
        (total - FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG).abs() < 0.01,
        "expected total ~{}, got {} (full response: {})",
        FIXTURE_FOCUS10_AMORTIZED_TOTAL_AUG,
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
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-cur2-source".into(),
            name: "Test CUR 2.0 fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Cur2,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
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
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
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
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            source_type: SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
            ..Default::default()
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

/// End-to-end HTTP integration test for `GET /api/v1/sources` (Session 14,
/// Task 2): proves that a real multi-source `AppConfig`, run through the
/// full `build_app` startup path, produces a `sources_list` response whose
/// per-source `Registered`/`Skipped` shapes reflect what actually happened
/// at startup — not just the handler-level stub coverage in
/// `handlers.rs`'s unit tests.
///
/// Two sources are configured, deliberately in this order:
/// 1. `local-good`, a local FOCUS 1.2 fixture directory that registers
///    successfully (`Registered`).
/// 2. `broken`, a local directory whose only "Parquet" file is garbage, so
///    schema detection fails deterministically (`Skipped`). This used to be
///    an `s3://`-prefixed source (S3 discovery was unconditionally skipped
///    in the old `crates/api`-only world), but `service::register_source`
///    now really tries to connect to S3, which needs network and isn't
///    deterministic/offline-safe for a test — so the deterministic-failure
///    case is exercised locally instead. Note an empty/nonexistent local
///    directory would NOT prove the `Skipped` shape here — that registers as
///    a valid, zero-file `Registered` source instead (see
///    `service::register::register_source`'s doc comment).
///
/// `local-good` is listed first so this test proves SOMETHING about
/// `default_source_id`'s exact value, but note this ordering does NOT by
/// itself distinguish "first configured" from "first registered" semantics:
/// `local-good` happens to be both here, so a hypothetical "first
/// registered" implementation would pass this assertion too. The real
/// distinguishing case (an unregistered source listed first) is documented
/// but not exercised as its own test — `resolve_source`'s actual fallback
/// rule ("first *configured* source", `state.config.sources.first()`, which
/// may point at a Skipped source) is what `handlers.rs`'s `SourcesResponse`
/// doc comment specifies and this test's assertion happens to be consistent
/// with, not what this specific fixture ordering proves in isolation.
#[tokio::test]
async fn http_sources_list_reflects_configured_sources() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();

    // A local source whose only "Parquet" file is garbage → schema detection
    // fails deterministically, offline → Skipped.
    let broken_dir = tempfile::tempdir().unwrap();
    let pd = broken_dir.path().join("BILLING_PERIOD=2026-08");
    std::fs::create_dir(&pd).unwrap();
    std::fs::write(pd.join("Manifest.json"), br#"{"dataFiles":["data.parquet"]}"#).unwrap();
    std::fs::write(pd.join("data.parquet"), b"not parquet").unwrap();

    let config = AppConfig {
        server: ServerConfig::default(),
        cost_guard: Default::default(),
        sources: vec![
            DataSource {
                id: "local-good".into(),
                name: "Local Good Source".into(),
                s3_uri: dir.path().to_str().unwrap().to_string(),
                source_type: SourceType::Focus12,
                aws_region: None,
                aws_profile: None,
                role_arn: None,
                ..Default::default()
            },
            DataSource {
                id: "broken".into(),
                name: "Broken Source".into(),
                s3_uri: broken_dir.path().to_str().unwrap().to_string(),
                source_type: SourceType::Auto,
                ..Default::default()
            },
        ],
    };

    let app = build_app(config).expect("build_app should succeed");

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/sources")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["default_source_id"], "local-good");

    let sources = json["sources"].as_array().expect("sources must be an array");
    assert_eq!(sources.len(), 2);

    let good = &sources[0];
    assert_eq!(good["id"], "local-good");
    assert_eq!(good["name"], "Local Good Source");
    assert_eq!(good["configured_type"], "focus12");
    assert_eq!(good["state"], "registered");
    assert_eq!(good["detected_format"], "focus12");
    assert!(
        good["file_count"].as_u64().unwrap() > 0,
        "expected local-good to have discovered at least one file, got {}",
        good["file_count"]
    );
    assert!(good.get("reason").is_none());

    let broken = &sources[1];
    assert_eq!(broken["id"], "broken");
    assert_eq!(broken["name"], "Broken Source");
    assert_eq!(broken["configured_type"], "auto");
    assert_eq!(broken["state"], "skipped");
    assert!(
        broken["reason"].as_str().unwrap().starts_with("schema detection failed"),
        "{}", broken["reason"]
    );
    assert!(broken.get("detected_format").is_none());
    assert!(broken.get("file_count").is_none());
}

/// Settings over HTTP: add a local source → it registers → list shows it →
/// delete → gone. Runs against the same `service` code the desktop app uses.
#[tokio::test]
async fn http_settings_add_and_delete_source() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let app = build_app(AppConfig::default()).expect("build_app");

    let post = |uri: &str, body: serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };
    let source = serde_json::json!({
        "id": "added", "name": "Added", "s3_uri": dir.path().to_str().unwrap(),
        "source_type": "auto", "auth": { "type": "credential_chain" }
    });

    let resp = app.clone().oneshot(post("/api/v1/settings/source-test", serde_json::json!({ "source": source }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["detected_format"], "focus12");

    let resp = app.clone().oneshot(post("/api/v1/settings/source-save", serde_json::json!({ "source": source, "is_new": true }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["state"], "registered");

    let resp = app.clone().oneshot(post("/api/v1/settings/source-save", serde_json::json!({ "source": source, "is_new": true }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    let resp = app.clone().oneshot(post("/api/v1/settings/source-delete", serde_json::json!({ "id": "added" }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["sources"], serde_json::json!([]));
}

/// `POST /api/v1/cost/estimate` over HTTP: a local source reports its read
/// size but never a warning tier (local reads are free).
#[tokio::test]
async fn http_cost_estimate_for_local_source() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let config = AppConfig {
        server: ServerConfig::default(),
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        }],
    };
    let app = build_app(config).expect("build_app should succeed");

    let range = serde_json::json!([{ "start": "2026-08-01", "end": "2026-09-01" }]);
    let payload = serde_json::json!({ "scans": [range.clone(), range] });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/estimate")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["remote"], false, "{json}");
    assert_eq!(json["known"], true, "{json}");
    assert_eq!(json["tier"], "none", "{json}");
    assert!(json["bytes"].as_u64().unwrap() > 0, "{json}");
    assert_eq!(json["hard_limit_usd"], 1.0, "{json}");
}

/// `POST /api/v1/cost/resource-search` over HTTP: a substring search scoped
/// to the requested date range.
#[tokio::test]
async fn http_resource_search_finds_fixture_resource() {
    let dir = tempfile::tempdir().unwrap();
    generate_focus12_fixture(dir.path()).unwrap();
    let config = AppConfig {
        server: ServerConfig::default(),
        cost_guard: Default::default(),
        sources: vec![DataSource {
            id: "test-source".into(),
            name: "Test fixture source".into(),
            s3_uri: dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        }],
    };
    let app = build_app(config).expect("build_app should succeed");

    let payload = serde_json::json!({ "start": "2026-08-01", "end": "2026-09-01", "q": "ABC" });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/cost/resource-search")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["values"], serde_json::json!(["i-abc123", "bucket-abc"]), "{json}");
}
