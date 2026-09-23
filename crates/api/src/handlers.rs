//! Axum wrappers over `service` — all validation/query logic lives there.

use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use service::app::{SaveSourceRequest, SourceIdRequest, TestSourceRequest};
use service::cost::{
    self, BreakdownRequest, CompareRequest, FilterDimension, FilterValuesQuery, SummaryRequest,
    TagValuesQuery, TimeseriesRequest,
};
use service::{ErrorKind, ServiceError};

use crate::state::AppState;

fn error_response(err: ServiceError) -> Response {
    let status = match err.kind {
        ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        ErrorKind::Conflict => StatusCode::CONFLICT,
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": err.message }))).into_response()
}

/// Runs a blocking service call on the blocking pool and maps the result.
async fn run<T, F>(f: F) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T, ServiceError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(Err(err)) => error_response(err),
        Err(join_err) => {
            tracing::error!(error = %join_err, "handler task panicked");
            error_response(ServiceError::internal("internal server error"))
        }
    }
}

pub async fn health() -> impl IntoResponse {
    Json(cost::health())
}

pub async fn cost_summary(State(s): State<AppState>, Json(req): Json<SummaryRequest>) -> Response {
    run(move || cost::cost_summary(&s.registry, req)).await
}

pub async fn cost_timeseries(State(s): State<AppState>, Json(req): Json<TimeseriesRequest>) -> Response {
    run(move || cost::cost_timeseries(&s.registry, req)).await
}

pub async fn cost_breakdown(State(s): State<AppState>, Json(req): Json<BreakdownRequest>) -> Response {
    run(move || cost::cost_breakdown(&s.registry, req)).await
}

pub async fn cost_compare(State(s): State<AppState>, Json(req): Json<CompareRequest>) -> Response {
    run(move || cost::cost_compare(&s.registry, req)).await
}

async fn values(s: AppState, dim: FilterDimension, q: FilterValuesQuery) -> Response {
    run(move || cost::filter_values(&s.registry, dim, q)).await
}

pub async fn filter_values_services(State(s): State<AppState>, Query(q): Query<FilterValuesQuery>) -> Response {
    values(s, FilterDimension::Services, q).await
}

pub async fn filter_values_accounts(State(s): State<AppState>, Query(q): Query<FilterValuesQuery>) -> Response {
    values(s, FilterDimension::Accounts, q).await
}

pub async fn filter_values_regions(State(s): State<AppState>, Query(q): Query<FilterValuesQuery>) -> Response {
    values(s, FilterDimension::Regions, q).await
}

pub async fn filter_values_tag_keys(State(s): State<AppState>, Query(q): Query<FilterValuesQuery>) -> Response {
    values(s, FilterDimension::TagKeys, q).await
}

pub async fn filter_values_tag_values(State(s): State<AppState>, Query(q): Query<TagValuesQuery>) -> Response {
    run(move || cost::tag_values(&s.registry, q)).await
}

pub async fn sources_list(State(s): State<AppState>) -> Response {
    run(move || Ok(s.sources())).await
}

pub async fn settings_get(State(s): State<AppState>) -> Response {
    run(move || Ok(s.settings())).await
}

pub async fn settings_source_save(State(s): State<AppState>, Json(req): Json<SaveSourceRequest>) -> Response {
    run(move || s.save_source(req)).await
}

pub async fn settings_source_delete(State(s): State<AppState>, Json(req): Json<SourceIdRequest>) -> Response {
    run(move || s.delete_source(req)).await
}

pub async fn settings_source_test(State(s): State<AppState>, Json(req): Json<TestSourceRequest>) -> Response {
    run(move || s.test_source(req)).await
}

pub async fn settings_source_reload(State(s): State<AppState>, Json(req): Json<SourceIdRequest>) -> Response {
    run(move || s.reload_source(req)).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::build_router;
    use axum::{body::Body, http::Request};
    use chrono::{TimeZone, Utc};
    use domain::cost::CostSummary;
    use domain::{cost::CostMetric, filters::CostFilter};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tower::ServiceExt;

    // -----------------------------------------------------------------------
    // Stub repository for offline handler tests
    // -----------------------------------------------------------------------

    struct StubRepo {
        summary: CostSummary,
    }

    impl data::queries::summary::CostRepository for StubRepo {
        fn summary(
            &self,
            _filter: &CostFilter,
        ) -> Result<CostSummary, data::queries::summary::QueryError> {
            Ok(self.summary.clone())
        }

        fn timeseries(
            &self,
            _filter: &CostFilter,
            grouping: Option<domain::dimensions::Dimension>,
        ) -> Result<domain::cost::TimeSeriesResult, data::queries::summary::QueryError> {
            Ok(domain::cost::TimeSeriesResult {
                currency: self.summary.currency.clone(),
                points: vec![domain::cost::TimeSeriesPoint {
                    period: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
                    group: grouping.map(|_| "EC2".to_string()),
                    total: 42.0,
                    row_count: 1,
                }],
            })
        }

        fn breakdown(
            &self,
            _filter: &CostFilter,
            _dimension: domain::dimensions::Dimension,
            _limit: usize,
        ) -> Result<domain::cost::BreakdownResult, data::queries::summary::QueryError> {
            Ok(domain::cost::BreakdownResult {
                currency: self.summary.currency.clone(),
                rows: vec![domain::cost::BreakdownRow {
                    key: Some("EC2".to_string()),
                    total: 42.0,
                    row_count: 1,
                }],
            })
        }

        fn compare(
            &self,
            _current: &CostFilter,
            _previous: &CostFilter,
            dimension: Option<domain::dimensions::Dimension>,
        ) -> Result<domain::cost::CompareResult, data::queries::summary::QueryError> {
            Ok(domain::cost::CompareResult {
                currency: self.summary.currency.clone(),
                rows: vec![domain::cost::CompareRow {
                    key: dimension.map(|_| "EC2".to_string()),
                    current: 42.0,
                    previous: 40.0,
                    absolute_change: 2.0,
                    percentage_change: Some(5.0),
                }],
            })
        }

        fn distinct_services(&self) -> Result<Vec<String>, data::queries::summary::QueryError> {
            Ok(vec!["EC2".to_string(), "S3".to_string()])
        }

        fn distinct_accounts(&self) -> Result<Vec<String>, data::queries::summary::QueryError> {
            Ok(vec!["acct-001".to_string()])
        }

        fn distinct_regions(&self) -> Result<Vec<String>, data::queries::summary::QueryError> {
            Ok(vec!["us-east-1".to_string()])
        }

        fn distinct_tag_keys(&self) -> Result<Vec<String>, data::queries::summary::QueryError> {
            Ok(vec!["Environment".to_string(), "Team".to_string()])
        }

        fn distinct_tag_values(
            &self,
            _key: &str,
        ) -> Result<Vec<String>, data::queries::summary::QueryError> {
            Ok(vec!["production".to_string()])
        }
    }

    fn stub_summary() -> CostSummary {
        CostSummary {
            metric: CostMetric::Amortized,
            currency: "USD".into(),
            total: 42.0,
            row_count: 1,
            source_format: Some("focus12".into()),
            query_ms: 0,
            start: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
        }
    }

    fn service_with(sources: Vec<data::config::DataSource>) -> service::CostlyticsService {
        let config = data::config::AppConfig { server: Default::default(), sources };
        service::CostlyticsService::new(config, None, Arc::new(service::secrets::MemorySecretStore::default()))
    }

    fn stub_registered(file_count: usize) -> service::register::Registered {
        service::register::Registered {
            repo: Arc::new(StubRepo { summary: stub_summary() }),
            detected_format: "focus12".into(),
            file_count,
            billing_periods: vec![],
        }
    }

    fn make_state() -> AppState {
        let svc = service_with(vec![data::config::DataSource {
            id: "test-source".into(),
            name: "Test".into(),
            s3_uri: "fixtures/focus12".into(),
            source_type: data::config::SourceType::Focus12,
            ..Default::default()
        }]);
        svc.registry.set_result("test-source", Ok(stub_registered(3)));
        Arc::new(svc)
    }

    fn make_state_with_mixed_sources() -> AppState {
        let svc = service_with(vec![
            data::config::DataSource {
                id: "good-source".into(),
                name: "Good Source".into(),
                s3_uri: "fixtures/focus12".into(),
                ..Default::default()
            },
            data::config::DataSource {
                id: "broken-source".into(),
                name: "Broken Source".into(),
                s3_uri: "fixtures/does-not-exist".into(),
                ..Default::default()
            },
        ]);
        svc.registry.set_result("good-source", Ok(stub_registered(5)));
        svc.registry.set_result("broken-source", Err("no Parquet files found via discovery".into()));
        Arc::new(svc)
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn health_returns_200() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn summary_start_equals_end_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-08-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/summary")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn summary_start_after_end_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-09-01",
            "end": "2026-08-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/summary")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn summary_invalid_dates_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "not-a-date",
            "end": "2026-09-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/summary")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn summary_valid_request_returns_200() {
        let app = build_router(make_state());
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
        assert_eq!(json["currency"], "USD");
    }

    /// Session 5, Task 3: the request body deserializes correctly with the
    /// six predicate fields present (`StubRepo` returns fixed data regardless
    /// of filter contents, so this mainly proves the handler doesn't reject
    /// an extended body).
    #[tokio::test]
    async fn summary_with_service_filter_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "metric": "amortized",
            "services": ["EC2", "S3"],
            "accounts": [],
            "regions": ["us-east-1"],
            "charge_categories": [],
            "resource_ids": [],
            "tags": [
                {"key": "Environment", "operator": "eq", "values": ["production"]}
            ]
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
        assert_eq!(json["currency"], "USD");
    }

    /// A predicate list field longer than `MAX_PREDICATE_VALUES` is rejected
    /// with 400 rather than being handed to the query engine unbounded.
    #[tokio::test]
    async fn summary_with_oversized_service_list_returns_400() {
        let app = build_router(make_state());
        let services: Vec<String> = (0..1001)
            .map(|i| format!("svc-{i}"))
            .collect();
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "services": services,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/summary")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// `compare`'s per-period oversized-list check applies independently to
    /// `previous` too, not just `current`.
    #[tokio::test]
    async fn compare_with_oversized_previous_account_list_returns_400() {
        let app = build_router(make_state());
        let accounts: Vec<String> = (0..1001)
            .map(|i| format!("acct-{i}"))
            .collect();
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01",
            "previous": {"accounts": accounts},
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/compare")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn timeseries_valid_request_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "metric": "amortized",
            "granularity": "month",
            "group_by": "service"
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
        assert_eq!(json["granularity"], "month");
        assert_eq!(json["series"][0]["group"], "EC2");
    }

    #[tokio::test]
    async fn timeseries_invalid_granularity_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "granularity": "fortnight"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/timeseries")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn breakdown_valid_request_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "metric": "amortized",
            "dimension": "service",
            "limit": 10
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/breakdown")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["currency"], "USD");
        assert_eq!(json["dimension"], "service");
        assert_eq!(json["rows"][0]["key"], "EC2");
    }

    #[tokio::test]
    async fn breakdown_missing_dimension_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/breakdown")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn breakdown_invalid_dimension_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "start": "2026-08-01",
            "end": "2026-09-01",
            "dimension": "not_a_dimension"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/breakdown")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn filter_values_services_returns_200() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/filter-values/services")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["values"], serde_json::json!(["EC2", "S3"]));
    }

    #[tokio::test]
    async fn filter_values_tag_values_missing_key_returns_400() {
        let app = build_router(make_state());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/filter-values/tag-values")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compare_valid_request_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01",
            "metric": "amortized",
            "dimension": "service"
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
        assert_eq!(json["dimension"], "service");
        assert_eq!(json["rows"][0]["key"], "EC2");
        assert_eq!(json["rows"][0]["current"], 42.0);
        assert_eq!(json["rows"][0]["previous"], 40.0);
    }

    /// The `current`/`previous` nested filter objects deserialize
    /// independently and don't require matching values on both sides.
    #[tokio::test]
    async fn compare_with_per_period_filters_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01",
            "metric": "amortized",
            "dimension": "service",
            "current": {"services": ["EC2"]},
            "previous": {"services": ["EC2", "S3"]}
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
        assert_eq!(json["current_filters"]["services"], serde_json::json!(["EC2"]));
        assert_eq!(
            json["previous_filters"]["services"],
            serde_json::json!(["EC2", "S3"])
        );
    }

    /// When a client omits `previous` entirely, the echoed `previous_filters`
    /// must show the empty defaults that were actually applied — making the
    /// "forgot to set previous" footgun self-diagnosing in the response
    /// rather than silently comparing a filtered current period against an
    /// unfiltered previous one.
    #[tokio::test]
    async fn compare_with_omitted_previous_echoes_empty_defaults() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01",
            "current": {"services": ["EC2"]}
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
        assert_eq!(json["current_filters"]["services"], serde_json::json!(["EC2"]));
        assert_eq!(json["previous_filters"]["services"], serde_json::json!([]));
        assert_eq!(json["previous_filters"]["accounts"], serde_json::json!([]));
        assert_eq!(json["previous_filters"]["tags"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn compare_invalid_current_range_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-09-01",
            "current_end": "2026-08-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/compare")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compare_invalid_previous_range_returns_400() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-08-01",
            "previous_end": "2026-07-01"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/cost/compare")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compare_without_dimension_returns_200() {
        let app = build_router(make_state());
        let payload = serde_json::json!({
            "current_start": "2026-08-01",
            "current_end": "2026-09-01",
            "previous_start": "2026-07-01",
            "previous_end": "2026-08-01"
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
        assert_eq!(json["dimension"], serde_json::Value::Null);
        assert_eq!(json["rows"][0]["key"], serde_json::Value::Null);
    }

    // -----------------------------------------------------------------------
    // GET /api/v1/sources
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn sources_list_returns_200_with_mixed_statuses() {
        let app = build_router(make_state_with_mixed_sources());
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/sources")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // default_source_id mirrors resolve_source(None)'s fallback: the
        // first configured source, regardless of registration outcome.
        assert_eq!(json["default_source_id"], "good-source");

        let sources = json["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);

        let good = &sources[0];
        assert_eq!(good["id"], "good-source");
        assert_eq!(good["name"], "Good Source");
        assert_eq!(good["configured_type"], "auto");
        assert_eq!(good["state"], "registered");
        assert_eq!(good["detected_format"], "focus12");
        assert_eq!(good["file_count"], 5);

        let broken = &sources[1];
        assert_eq!(broken["id"], "broken-source");
        assert_eq!(broken["state"], "skipped");
        assert_eq!(
            broken["reason"],
            "no Parquet files found via discovery"
        );
        // Skipped entries carry no detected_format/file_count fields.
        assert!(broken.get("detected_format").is_none());
        assert!(broken.get("file_count").is_none());
    }

    #[tokio::test]
    async fn sources_list_empty_config_returns_null_default() {
        let state: AppState = Arc::new(service_with(vec![]));
        let app = build_router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/sources")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["default_source_id"], serde_json::Value::Null);
        assert_eq!(json["sources"], serde_json::json!([]));
    }
}
