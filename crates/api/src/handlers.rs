use axum::{
    extract::{Json, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use data::queries::summary::{CostRepository, QueryError};
use domain::{
    cost::{BreakdownRow, CostMetric, TimeSeriesPoint},
    dimensions::Dimension,
    filters::{CostFilter, TagFilter, TimeGranularity},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::state::AppState;

/// Maximum allowable query date range, in days (~5 years).
const MAX_DATE_RANGE_DAYS: i64 = 5 * 366;

/// Maximum number of values allowed in any single predicate list field
/// (`accounts`/`services`/`regions`/`charge_categories`/`resource_ids`), or
/// in the total `tags` vec, on any `/cost/*` request. Reuses
/// `MAX_FILTER_VALUES`'s cardinality/rationale from the `data` crate: the
/// `filter-values` lookup endpoints can never return more than that many
/// distinct values, so no legitimate client needs a longer predicate list.
/// Enforced here, at the request-validation layer, rather than inside
/// `data::queries::predicate` — that keeps the predicate builder a simple,
/// policy-free translator and matches how `MAX_DATE_RANGE_DAYS` and
/// `breakdown`'s `limit` cap are already validated in this file.
const MAX_PREDICATE_VALUES: usize = 1000;

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

/// The six `CostFilter` predicate fields a client may supply on any
/// `/cost/*` request, flattened directly into each request struct so the
/// JSON shape stays a single flat object (see the brief's example body).
/// All fields are optional and default to empty ("no filter on this
/// dimension"), matching `CostFilter::date_range`'s own defaults.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct FilterFields {
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub charge_categories: Vec<String>,
    #[serde(default)]
    pub resource_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<TagFilter>,
}

impl FilterFields {
    /// Copies these fields onto a `CostFilter`'s corresponding predicate
    /// fields (`filter.start`/`end`/`metric`/`granularity` are left
    /// untouched — this only ever sets the predicate fields).
    fn apply(&self, filter: &mut CostFilter) {
        filter.accounts = self.accounts.clone();
        filter.services = self.services.clone();
        filter.regions = self.regions.clone();
        filter.charge_categories = self.charge_categories.clone();
        filter.resource_ids = self.resource_ids.clone();
        filter.tags = self.tags.clone();
    }
}

#[derive(Deserialize)]
pub struct SummaryRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
    #[serde(flatten)]
    pub filters: FilterFields,
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

/// Outcome of the preamble shared by all three `/cost/*` handlers: a resolved
/// repository plus the `CostFilter` built from the validated request.
struct ResolvedRequest {
    repo: Arc<dyn CostRepository>,
    filter: CostFilter,
}

/// Validates a single `(start, end)` date pair — `start` before `end`, and
/// the range not exceeding [`MAX_DATE_RANGE_DAYS`] — and converts both bounds
/// to UTC `DateTime`s at midnight.
///
/// Factored out of `resolve_common` so that handlers needing more than one
/// independent date-range pair (namely `cost_compare`, with its `current` and
/// `previous` ranges) can validate each pair through this single code path
/// instead of hand-copying the checks.
fn validate_date_range(
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Box<Response>> {
    // Validate: start must be before end
    if start >= end {
        return Err(Box::new(bad_request("start must be before end").into_response()));
    }

    // Validate: date range must not exceed 5 years (~1827 days)
    let days = (end - start).num_days();
    if days > MAX_DATE_RANGE_DAYS {
        return Err(Box::new(
            bad_request("date range must not exceed 5 years").into_response(),
        ));
    }

    // Convert NaiveDate to DateTime<Utc> at midnight UTC
    let start = Utc.from_utc_datetime(&start.and_hms_opt(0, 0, 0).unwrap());
    let end = Utc.from_utc_datetime(&end.and_hms_opt(0, 0, 0).unwrap());

    Ok((start, end))
}

/// Validates that no single predicate list field, nor the total `tags`
/// vec, on `fields` exceeds [`MAX_PREDICATE_VALUES`]. `label` identifies
/// which side of the request `fields` came from (e.g. `""` for the flat
/// `/cost/summary`-style requests, or `"current."`/`"previous."` for
/// `compare`'s nested fields) so the 400 body names the offending field
/// precisely.
fn validate_filter_fields(fields: &FilterFields, label: &str) -> Result<(), Box<Response>> {
    let checks: [(&str, usize); 6] = [
        ("accounts", fields.accounts.len()),
        ("services", fields.services.len()),
        ("regions", fields.regions.len()),
        ("charge_categories", fields.charge_categories.len()),
        ("resource_ids", fields.resource_ids.len()),
        ("tags", fields.tags.len()),
    ];
    for (name, len) in checks {
        if len > MAX_PREDICATE_VALUES {
            return Err(Box::new(
                bad_request(format!(
                    "{label}{name} must not contain more than {MAX_PREDICATE_VALUES} values"
                ))
                .into_response(),
            ));
        }
    }
    Ok(())
}

/// Parses the optional `metric` request field, defaulting to
/// [`CostMetric::default`] when absent.
fn resolve_metric(metric: Option<&str>) -> Result<CostMetric, Box<Response>> {
    match metric {
        Some(m) => parse_metric(m)
            .ok_or_else(|| Box::new(bad_request(format!("unknown metric '{}'", m)).into_response())),
        None => Ok(CostMetric::default()),
    }
}

/// Resolves `source_id` against `state.repos`, defaulting to the first
/// configured source when absent.
fn resolve_source(
    state: &AppState,
    source_id: Option<&str>,
) -> Result<Arc<dyn CostRepository>, Box<Response>> {
    let source_id = if let Some(id) = source_id {
        id.to_string()
    } else {
        match state.config.sources.first() {
            Some(src) => src.id.clone(),
            None => return Err(Box::new(bad_request("no sources configured").into_response())),
        }
    };

    match state.repos.get(&source_id) {
        Some(r) => Ok(r.clone()),
        None => Err(Box::new(
            bad_request(format!("unknown source_id '{}'", source_id)).into_response(),
        )),
    }
}

/// Shared request preamble for the `/cost/*` handlers: validates the date
/// range, parses the metric, resolves `source_id` against `state.repos`
/// (defaulting to the first configured source), converts the `NaiveDate`
/// bounds to UTC `DateTime`s, and builds the resulting `CostFilter`.
///
/// Request-shape-specific parsing (granularity, group_by, dimension, limit)
/// stays in each handler — only the genuinely shared logic lives here.
fn resolve_common(
    state: &AppState,
    source_id: Option<&str>,
    start: NaiveDate,
    end: NaiveDate,
    metric: Option<&str>,
    filters: &FilterFields,
) -> Result<ResolvedRequest, Box<Response>> {
    let (start, end) = validate_date_range(start, end)?;
    let metric = resolve_metric(metric)?;
    let repo = resolve_source(state, source_id)?;
    validate_filter_fields(filters, "")?;

    let mut filter = CostFilter::date_range(start, end);
    filter.metric = metric;
    filters.apply(&mut filter);

    Ok(ResolvedRequest { repo, filter })
}

pub async fn cost_summary(
    State(state): State<AppState>,
    Json(body): Json<SummaryRequest>,
) -> impl IntoResponse {
    let ResolvedRequest { repo, filter } = match resolve_common(
        &state,
        body.source_id.as_deref(),
        body.start,
        body.end,
        body.metric.as_deref(),
        &body.filters,
    ) {
        Ok(resolved) => resolved,
        Err(resp) => return *resp,
    };

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
    #[serde(flatten)]
    pub filters: FilterFields,
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
    let ResolvedRequest { repo, mut filter } = match resolve_common(
        &state,
        body.source_id.as_deref(),
        body.start,
        body.end,
        body.metric.as_deref(),
        &body.filters,
    ) {
        Ok(resolved) => resolved,
        Err(resp) => return *resp,
    };
    let metric = filter.metric;

    // Parse granularity
    let granularity = if let Some(ref g) = body.granularity {
        match parse_granularity(g) {
            Some(g) => g,
            None => return bad_request(format!("unknown granularity '{}'", g)).into_response(),
        }
    } else {
        TimeGranularity::default()
    };
    filter.granularity = granularity;

    // Parse group_by (optional)
    let group_by = if let Some(ref d) = body.group_by {
        match parse_dimension(d) {
            Some(dim) => Some(dim),
            None => return bad_request(format!("unknown group_by '{}'", d)).into_response(),
        }
    } else {
        None
    };

    // Call repo — this is a blocking DuckDB call; run it on the blocking thread pool
    let result = tokio::task::spawn_blocking(move || repo.timeseries(&filter, group_by)).await;

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
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(TimeseriesResponse {
                metric,
                currency: result.currency,
                granularity,
                series: result.points,
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
    #[serde(flatten)]
    pub filters: FilterFields,
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
    let ResolvedRequest { repo, filter } = match resolve_common(
        &state,
        body.source_id.as_deref(),
        body.start,
        body.end,
        body.metric.as_deref(),
        &body.filters,
    ) {
        Ok(resolved) => resolved,
        Err(resp) => return *resp,
    };
    let metric = filter.metric;

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
        return bad_request(format!("limit must not exceed {}", MAX_BREAKDOWN_LIMIT))
            .into_response();
    }

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
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(BreakdownResponse {
                metric,
                currency: result.currency,
                dimension,
                rows: result.rows,
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/compare
// ---------------------------------------------------------------------------

/// `current`/`previous` are separate, independently-deserialized
/// `FilterFields` (rather than a single flat set of predicate fields shared
/// with the other `/cost/*` requests) *specifically* because the two
/// periods being compared can legitimately be scoped differently — this is
/// a deliberate shape asymmetry, not an oversight. See `CompareResponse`'s
/// `current_filters`/`previous_filters` for how the response makes the
/// effective filters on each side visible, so an accidentally-omitted
/// `previous` (which silently defaults to "no filter") is easy to spot
/// rather than producing a silently-wrong comparison.
#[derive(Deserialize)]
pub struct CompareRequest {
    pub source_id: Option<String>,
    pub current_start: NaiveDate,
    pub current_end: NaiveDate,
    pub previous_start: NaiveDate,
    pub previous_end: NaiveDate,
    pub metric: Option<String>,
    pub dimension: Option<String>,
    /// Predicate fields for the *current* period only — `compare()` applies
    /// each period's own filter to its own aggregate, so `current` and
    /// `previous` are independent and may differ (see `CostRepository::compare`'s
    /// doc comment). Defaults to no filter (all fields empty) when absent.
    #[serde(default)]
    pub current: FilterFields,
    /// Predicate fields for the *previous* period only. See `current`.
    #[serde(default)]
    pub previous: FilterFields,
}

#[derive(Serialize)]
pub struct CompareResponse {
    pub metric: CostMetric,
    pub currency: String,
    pub dimension: Option<Dimension>,
    pub rows: Vec<domain::cost::CompareRow>,
    /// The predicate fields actually applied to the *current* period,
    /// echoed back exactly as parsed from the request (including
    /// `#[serde(default)]`-filled-in empty defaults when the client omitted
    /// `current` entirely). Makes it immediately visible in the response
    /// when `current`/`previous` were scoped asymmetrically — including the
    /// common "forgot to set previous" mistake, which would otherwise
    /// silently compare a filtered current period against an unfiltered
    /// previous one.
    pub current_filters: FilterFields,
    /// The predicate fields actually applied to the *previous* period. See
    /// `current_filters`.
    pub previous_filters: FilterFields,
}

pub async fn cost_compare(
    State(state): State<AppState>,
    Json(body): Json<CompareRequest>,
) -> impl IntoResponse {
    // Validate both date-range pairs through the same path `resolve_common`
    // uses for its single pair — no hand-copied start/end or max-range checks.
    let (current_start, current_end) =
        match validate_date_range(body.current_start, body.current_end) {
            Ok(range) => range,
            Err(resp) => return *resp,
        };
    let (previous_start, previous_end) =
        match validate_date_range(body.previous_start, body.previous_end) {
            Ok(range) => range,
            Err(resp) => return *resp,
        };

    let metric = match resolve_metric(body.metric.as_deref()) {
        Ok(m) => m,
        Err(resp) => return *resp,
    };

    let repo = match resolve_source(&state, body.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };

    if let Err(resp) = validate_filter_fields(&body.current, "current.") {
        return *resp;
    }
    if let Err(resp) = validate_filter_fields(&body.previous, "previous.") {
        return *resp;
    }

    // Parse dimension (optional — unlike breakdown, compare without one is a
    // valid, meaningful single aggregate row)
    let dimension = match body.dimension {
        Some(ref d) => match parse_dimension(d) {
            Some(dim) => Some(dim),
            None => return bad_request(format!("unknown dimension '{}'", d)).into_response(),
        },
        None => None,
    };

    // Cloned before the filters are consumed below, so the effective
    // filters can be echoed back on the response (see `CompareResponse`'s
    // `current_filters`/`previous_filters` doc comment).
    let current_filters = body.current.clone();
    let previous_filters = body.previous.clone();

    let mut current_filter = CostFilter::date_range(current_start, current_end);
    current_filter.metric = metric;
    body.current.apply(&mut current_filter);
    let mut previous_filter = CostFilter::date_range(previous_start, previous_end);
    previous_filter.metric = metric;
    body.previous.apply(&mut previous_filter);

    // Call repo — this is a blocking DuckDB call; run it on the blocking thread pool
    let result = tokio::task::spawn_blocking(move || {
        repo.compare(&current_filter, &previous_filter, dimension)
    })
    .await;

    match result {
        Err(join_err) => {
            tracing::error!(error = %join_err, "cost_compare task panicked");
            internal_error("internal server error").into_response()
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        ))
        .into_response(),
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, "cost_compare query failed");
            internal_error("internal server error").into_response()
        }
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(CompareResponse {
                metric,
                currency: result.currency,
                dimension,
                rows: result.rows,
                current_filters,
                previous_filters,
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/v1/filter-values/{services,accounts,regions,tag-keys,tag-values}
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FilterValuesQuery {
    pub source_id: Option<String>,
}

#[derive(Deserialize)]
pub struct TagValuesQuery {
    pub source_id: Option<String>,
    pub key: Option<String>,
}

#[derive(Serialize)]
pub struct FilterValuesResponse {
    pub values: Vec<String>,
}

/// Shared response handling for the filter-value lookup endpoints: runs the
/// blocking DuckDB call and maps errors consistently with the `/cost/*`
/// handlers (including the 409 for `MultipleCurrencies`, even though none of
/// these queries are expected to hit it — keeps the mapping uniform).
async fn respond_with_values(
    result: Result<Result<Vec<String>, QueryError>, tokio::task::JoinError>,
    op: &str,
) -> Response {
    match result {
        Err(join_err) => {
            tracing::error!(error = %join_err, op, "filter-values task panicked");
            internal_error("internal server error").into_response()
        }
        Ok(Err(QueryError::MultipleCurrencies(currencies))) => conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        ))
        .into_response(),
        Ok(Err(query_err)) => {
            tracing::error!(error = %query_err, op, "filter-values query failed");
            internal_error("internal server error").into_response()
        }
        Ok(Ok(values)) => (StatusCode::OK, Json(FilterValuesResponse { values })).into_response(),
    }
}

pub async fn filter_values_services(
    State(state): State<AppState>,
    Query(query): Query<FilterValuesQuery>,
) -> impl IntoResponse {
    let repo = match resolve_source(&state, query.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let result = tokio::task::spawn_blocking(move || repo.distinct_services()).await;
    respond_with_values(result, "distinct_services").await
}

pub async fn filter_values_accounts(
    State(state): State<AppState>,
    Query(query): Query<FilterValuesQuery>,
) -> impl IntoResponse {
    let repo = match resolve_source(&state, query.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let result = tokio::task::spawn_blocking(move || repo.distinct_accounts()).await;
    respond_with_values(result, "distinct_accounts").await
}

pub async fn filter_values_regions(
    State(state): State<AppState>,
    Query(query): Query<FilterValuesQuery>,
) -> impl IntoResponse {
    let repo = match resolve_source(&state, query.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let result = tokio::task::spawn_blocking(move || repo.distinct_regions()).await;
    respond_with_values(result, "distinct_regions").await
}

pub async fn filter_values_tag_keys(
    State(state): State<AppState>,
    Query(query): Query<FilterValuesQuery>,
) -> impl IntoResponse {
    let repo = match resolve_source(&state, query.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let result = tokio::task::spawn_blocking(move || repo.distinct_tag_keys()).await;
    respond_with_values(result, "distinct_tag_keys").await
}

pub async fn filter_values_tag_values(
    State(state): State<AppState>,
    Query(query): Query<TagValuesQuery>,
) -> impl IntoResponse {
    let key = match query.key {
        Some(k) if !k.is_empty() => k,
        _ => return bad_request("key is required").into_response(),
    };
    let repo = match resolve_source(&state, query.source_id.as_deref()) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let result = tokio::task::spawn_blocking(move || repo.distinct_tag_values(&key)).await;
    respond_with_values(result, "distinct_tag_values").await
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
        let services: Vec<String> = (0..(MAX_PREDICATE_VALUES + 1))
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
        let accounts: Vec<String> = (0..(MAX_PREDICATE_VALUES + 1))
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
}
