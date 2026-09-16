use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{NaiveDate, TimeZone, Utc};
use data::queries::summary::QueryError;
use domain::{
    cost::{BreakdownRow, CostMetric, TimeSeriesPoint},
    dimensions::Dimension,
    filters::{CostFilter, TimeGranularity},
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

pub async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/summary
// ---------------------------------------------------------------------------

/// Allowable metric strings as sent by clients.
fn parse_metric(s: &str) -> Option<CostMetric> {
    match s.to_ascii_lowercase().as_str() {
        "amortized" => Some(CostMetric::Amortized),
        "billed" => Some(CostMetric::Billed),
        "list" => Some(CostMetric::List),
        "contracted" => Some(CostMetric::Contracted),
        _ => None,
    }
}

#[derive(Deserialize)]
pub struct SummaryRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn bad_request(msg: impl Into<String>) -> impl IntoResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn internal_error(msg: impl Into<String>) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn conflict(msg: impl Into<String>) -> impl IntoResponse {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse { error: msg.into() }),
    )
}

pub async fn cost_summary(
    State(state): State<AppState>,
    Json(body): Json<SummaryRequest>,
) -> impl IntoResponse {
    // Validate: start must be before end
    if body.start >= body.end {
        return bad_request("start must be before end").into_response();
    }

    // Validate: date range must not exceed 5 years (~1827 days)
    let days = (body.end - body.start).num_days();
    if days > 5 * 366 {
        return bad_request("date range must not exceed 5 years").into_response();
    }

    // Parse metric
    let metric = if let Some(ref m) = body.metric {
        match parse_metric(m) {
            Some(metric) => metric,
            None => return bad_request(format!("unknown metric '{}'", m)).into_response(),
        }
    } else {
        CostMetric::default()
    };

    // Resolve source_id: use provided or fall back to first configured source
    let source_id = if let Some(ref id) = body.source_id {
        id.clone()
    } else {
        match state.config.sources.first() {
            Some(src) => src.id.clone(),
            None => return bad_request("no sources configured").into_response(),
        }
    };

    // Look up repo
    let repo = match state.repos.get(&source_id) {
        Some(r) => r.clone(),
        None => return bad_request(format!("unknown source_id '{}'", source_id)).into_response(),
    };

    // Convert NaiveDate to DateTime<Utc> at midnight UTC
    let start = Utc
        .from_utc_datetime(&body.start.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc
        .from_utc_datetime(&body.end.and_hms_opt(0, 0, 0).unwrap());

    let mut filter = CostFilter::date_range(start, end);
    filter.metric = metric;

    // Call repo — this is a blocking DuckDB call; run it on the blocking thread pool
    let result = tokio::task::spawn_blocking(move || repo.summary(&filter)).await;

    match result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_summary task panicked");
            internal_error("internal server error").into_response()
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        ))
        .into_response(),
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_summary query failed");
            internal_error("internal server error").into_response()
        }
        Ok(Ok(summary)) => (StatusCode::OK, Json(summary)).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/timeseries
// ---------------------------------------------------------------------------

/// Allowable granularity strings as sent by clients.
fn parse_granularity(s: &str) -> Option<TimeGranularity> {
    match s.to_ascii_lowercase().as_str() {
        "day" => Some(TimeGranularity::Day),
        "month" => Some(TimeGranularity::Month),
        "year" => Some(TimeGranularity::Year),
        _ => None,
    }
}

/// Allowable dimension strings as sent by clients. Reuses `Dimension`'s own
/// `snake_case` `Deserialize` impl rather than duplicating a match arm.
fn parse_dimension(s: &str) -> Option<Dimension> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

#[derive(Deserialize)]
pub struct TimeseriesRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
    pub granularity: Option<String>,
    pub group_by: Option<String>,
}

#[derive(Serialize)]
pub struct TimeseriesResponse {
    pub metric: CostMetric,
    pub currency: String,
    pub granularity: TimeGranularity,
    pub series: Vec<TimeSeriesPoint>,
}

pub async fn cost_timeseries(
    State(state): State<AppState>,
    Json(body): Json<TimeseriesRequest>,
) -> impl IntoResponse {
    // Validate: start must be before end
    if body.start >= body.end {
        return bad_request("start must be before end").into_response();
    }

    // Validate: date range must not exceed 5 years (~1827 days)
    let days = (body.end - body.start).num_days();
    if days > 5 * 366 {
        return bad_request("date range must not exceed 5 years").into_response();
    }

    // Parse metric
    let metric = if let Some(ref m) = body.metric {
        match parse_metric(m) {
            Some(metric) => metric,
            None => return bad_request(format!("unknown metric '{}'", m)).into_response(),
        }
    } else {
        CostMetric::default()
    };

    // Parse granularity
    let granularity = if let Some(ref g) = body.granularity {
        match parse_granularity(g) {
            Some(g) => g,
            None => return bad_request(format!("unknown granularity '{}'", g)).into_response(),
        }
    } else {
        TimeGranularity::default()
    };

    // Parse group_by (optional)
    let group_by = if let Some(ref d) = body.group_by {
        match parse_dimension(d) {
            Some(dim) => Some(dim),
            None => return bad_request(format!("unknown group_by '{}'", d)).into_response(),
        }
    } else {
        None
    };

    // Resolve source_id: use provided or fall back to first configured source
    let source_id = if let Some(ref id) = body.source_id {
        id.clone()
    } else {
        match state.config.sources.first() {
            Some(src) => src.id.clone(),
            None => return bad_request("no sources configured").into_response(),
        }
    };

    // Look up repo
    let repo = match state.repos.get(&source_id) {
        Some(r) => r.clone(),
        None => return bad_request(format!("unknown source_id '{}'", source_id)).into_response(),
    };

    // Convert NaiveDate to DateTime<Utc> at midnight UTC
    let start = Utc.from_utc_datetime(&body.start.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&body.end.and_hms_opt(0, 0, 0).unwrap());

    let mut filter = CostFilter::date_range(start, end);
    filter.metric = metric;
    filter.granularity = granularity;

    // `timeseries()` guarantees the window is single-currency (or errors), but
    // doesn't return the currency itself. Rather than plumb a new return type
    // through `CostRepository`, fetch it via the existing `summary()` call —
    // it's a cheap aggregate query and avoids any new SQL living in this
    // handler layer.
    let repo_for_summary = repo.clone();
    let filter_for_summary = filter.clone();
    let summary_result =
        tokio::task::spawn_blocking(move || repo_for_summary.summary(&filter_for_summary)).await;

    let currency = match summary_result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_timeseries currency lookup task panicked");
            return internal_error("internal server error").into_response();
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => {
            return conflict(format!(
                "multiple currencies present: [{}]; currency filtering is not yet supported",
                currencies.join(", ")
            ))
            .into_response();
        }
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_timeseries currency lookup failed");
            return internal_error("internal server error").into_response();
        }
        Ok(Ok(summary)) => summary.currency,
    };

    // Call repo — this is a blocking DuckDB call; run it on the blocking thread pool
    let result =
        tokio::task::spawn_blocking(move || repo.timeseries(&filter, group_by)).await;

    match result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_timeseries task panicked");
            internal_error("internal server error").into_response()
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        ))
        .into_response(),
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_timeseries query failed");
            internal_error("internal server error").into_response()
        }
        Ok(Ok(series)) => (
            StatusCode::OK,
            Json(TimeseriesResponse {
                metric,
                currency,
                granularity,
                series,
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/breakdown
// ---------------------------------------------------------------------------

/// Default and maximum Top-N result size for breakdown queries.
const DEFAULT_BREAKDOWN_LIMIT: usize = 50;
const MAX_BREAKDOWN_LIMIT: usize = 10_000;

#[derive(Deserialize)]
pub struct BreakdownRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
    pub dimension: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct BreakdownResponse {
    pub metric: CostMetric,
    pub currency: String,
    pub dimension: Dimension,
    pub rows: Vec<BreakdownRow>,
}

pub async fn cost_breakdown(
    State(state): State<AppState>,
    Json(body): Json<BreakdownRequest>,
) -> impl IntoResponse {
    // Validate: start must be before end
    if body.start >= body.end {
        return bad_request("start must be before end").into_response();
    }

    // Validate: date range must not exceed 5 years (~1827 days)
    let days = (body.end - body.start).num_days();
    if days > 5 * 366 {
        return bad_request("date range must not exceed 5 years").into_response();
    }

    // Parse metric
    let metric = if let Some(ref m) = body.metric {
        match parse_metric(m) {
            Some(metric) => metric,
            None => return bad_request(format!("unknown metric '{}'", m)).into_response(),
        }
    } else {
        CostMetric::default()
    };

    // Parse dimension (required)
    let dimension = match body.dimension {
        Some(ref d) => match parse_dimension(d) {
            Some(dim) => dim,
            None => return bad_request(format!("unknown dimension '{}'", d)).into_response(),
        },
        None => return bad_request("dimension is required").into_response(),
    };

    // Validate limit
    let limit = body.limit.unwrap_or(DEFAULT_BREAKDOWN_LIMIT);
    if limit == 0 {
        return bad_request("limit must be greater than 0").into_response();
    }
    if limit > MAX_BREAKDOWN_LIMIT {
        return bad_request(format!(
            "limit must not exceed {}",
            MAX_BREAKDOWN_LIMIT
        ))
        .into_response();
    }

    // Resolve source_id: use provided or fall back to first configured source
    let source_id = if let Some(ref id) = body.source_id {
        id.clone()
    } else {
        match state.config.sources.first() {
            Some(src) => src.id.clone(),
            None => return bad_request("no sources configured").into_response(),
        }
    };

    // Look up repo
    let repo = match state.repos.get(&source_id) {
        Some(r) => r.clone(),
        None => return bad_request(format!("unknown source_id '{}'", source_id)).into_response(),
    };

    // Convert NaiveDate to DateTime<Utc> at midnight UTC
    let start = Utc.from_utc_datetime(&body.start.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&body.end.and_hms_opt(0, 0, 0).unwrap());

    let mut filter = CostFilter::date_range(start, end);
    filter.metric = metric;

    // Same rationale as `cost_timeseries`: fetch currency via the existing
    // `summary()` call rather than plumbing a new return type through
    // `CostRepository`.
    let repo_for_summary = repo.clone();
    let filter_for_summary = filter.clone();
    let summary_result =
        tokio::task::spawn_blocking(move || repo_for_summary.summary(&filter_for_summary)).await;

    let currency = match summary_result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_breakdown currency lookup task panicked");
            return internal_error("internal server error").into_response();
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => {
            return conflict(format!(
                "multiple currencies present: [{}]; currency filtering is not yet supported",
                currencies.join(", ")
            ))
            .into_response();
        }
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_breakdown currency lookup failed");
            return internal_error("internal server error").into_response();
        }
        Ok(Ok(summary)) => summary.currency,
    };

    // Call repo — this is a blocking DuckDB call; run it on the blocking thread pool
    let result =
        tokio::task::spawn_blocking(move || repo.breakdown(&filter, dimension, limit)).await;

    match result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_breakdown task panicked");
            internal_error("internal server error").into_response()
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        ))
        .into_response(),
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_breakdown query failed");
            internal_error("internal server error").into_response()
        }
        Ok(Ok(rows)) => (
            StatusCode::OK,
            Json(BreakdownResponse {
                metric,
                currency,
                dimension,
                rows,
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::build_router;
    use axum::{body::Body, http::Request};
    use domain::cost::CostSummary;
    use http_body_util::BodyExt;
    use std::collections::HashMap;
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
        ) -> Result<Vec<domain::cost::TimeSeriesPoint>, data::queries::summary::QueryError> {
            Ok(vec![domain::cost::TimeSeriesPoint {
                period: Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(),
                group: grouping.map(|_| "EC2".to_string()),
                total: 42.0,
                row_count: 1,
            }])
        }

        fn breakdown(
            &self,
            _filter: &CostFilter,
            _dimension: domain::dimensions::Dimension,
            _limit: usize,
        ) -> Result<Vec<domain::cost::BreakdownRow>, data::queries::summary::QueryError> {
            Ok(vec![domain::cost::BreakdownRow {
                key: Some("EC2".to_string()),
                total: 42.0,
                row_count: 1,
            }])
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

    fn make_state() -> AppState {
        // Minimal config with one source
        let source = data::config::DataSource {
            id: "test-source".into(),
            name: "Test".into(),
            s3_uri: "fixtures/focus12".into(),
            source_type: data::config::SourceType::Focus12,
            aws_region: None,
            aws_profile: None,
            role_arn: None,
        };
        let config = data::config::AppConfig {
            server: data::config::ServerConfig::default(),
            sources: vec![source],
        };

        let repo: Arc<dyn data::queries::summary::CostRepository> =
            Arc::new(StubRepo { summary: stub_summary() });

        let mut repos = HashMap::new();
        repos.insert("test-source".to_string(), repo);

        AppState {
            config,
            pools: HashMap::new(),
            repos,
        }
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
}
