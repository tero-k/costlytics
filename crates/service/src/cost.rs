use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use data::queries::summary::{CostRepository, QueryError};
use domain::{
    cost::{BreakdownRow, CostMetric, CostSummary, TimeSeriesPoint},
    dimensions::Dimension,
    filters::{CostFilter, TagFilter, TimeGranularity},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{error::ServiceError, registry::SourceRegistry};
use data::config::CostGuardConfig;

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
    pub(crate) fn apply(&self, filter: &mut CostFilter) {
        filter.accounts = self.accounts.clone();
        filter.services = self.services.clone();
        filter.regions = self.regions.clone();
        filter.charge_categories = self.charge_categories.clone();
        filter.resource_ids = self.resource_ids.clone();
        filter.tags = self.tags.clone();
    }
}

#[derive(Debug, Deserialize)]
pub struct SummaryRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
    #[serde(flatten)]
    pub filters: FilterFields,
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
) -> Result<(DateTime<Utc>, DateTime<Utc>), ServiceError> {
    // Validate: start must be before end
    if start >= end {
        return Err(ServiceError::bad_request("start must be before end"));
    }

    // Validate: date range must not exceed 5 years (~1827 days)
    let days = (end - start).num_days();
    if days > MAX_DATE_RANGE_DAYS {
        return Err(ServiceError::bad_request("date range must not exceed 5 years"));
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
fn validate_filter_fields(fields: &FilterFields, label: &str) -> Result<(), ServiceError> {
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
            return Err(ServiceError::bad_request(format!(
                "{label}{name} must not contain more than {MAX_PREDICATE_VALUES} values"
            )));
        }
    }
    Ok(())
}

/// Parses the optional `metric` request field, defaulting to
/// [`CostMetric::default`] when absent.
fn resolve_metric(metric: Option<&str>) -> Result<CostMetric, ServiceError> {
    match metric {
        Some(m) => parse_metric(m)
            .ok_or_else(|| ServiceError::bad_request(format!("unknown metric '{}'", m))),
        None => Ok(CostMetric::default()),
    }
}

/// Shared request preamble for the `/cost/*` handlers: validates the date
/// range, parses the metric, resolves `source_id` against `registry`
/// (defaulting to the first configured source), converts the `NaiveDate`
/// bounds to UTC `DateTime`s, and builds the resulting `CostFilter`.
///
/// Request-shape-specific parsing (granularity, group_by, dimension, limit)
/// stays in each handler — only the genuinely shared logic lives here.
fn resolve_common(
    registry: &SourceRegistry,
    source_id: Option<&str>,
    start: NaiveDate,
    end: NaiveDate,
    metric: Option<&str>,
    filters: &FilterFields,
) -> Result<(Arc<dyn CostRepository>, CostFilter), ServiceError> {
    let (start, end) = validate_date_range(start, end)?;
    let metric = resolve_metric(metric)?;
    let repo = registry.repo(source_id)?;
    validate_filter_fields(filters, "")?;

    let mut filter = CostFilter::date_range(start, end);
    filter.metric = metric;
    filters.apply(&mut filter);
    Ok((repo, filter))
}

/// Maps repository errors exactly as the old handlers did: 409 for mixed
/// currencies, an opaque 500 (details logged) for everything else.
fn query_error(op: &str, err: QueryError) -> ServiceError {
    match err {
        QueryError::MultipleCurrencies(currencies) => ServiceError::conflict(format!(
            "multiple currencies present: [{}]; currency filtering is not yet supported",
            currencies.join(", ")
        )),
        other => {
            tracing::error!(error = %other, op, "query failed");
            ServiceError::internal("internal server error")
        }
    }
}

pub fn health() -> HealthResponse {
    HealthResponse { status: "ok", version: env!("CARGO_PKG_VERSION") }
}

pub fn cost_summary(registry: &SourceRegistry, req: SummaryRequest) -> Result<CostSummary, ServiceError> {
    let (repo, filter) = resolve_common(
        registry, req.source_id.as_deref(), req.start, req.end, req.metric.as_deref(), &req.filters,
    )?;
    repo.summary(&filter).map_err(|e| query_error("cost_summary", e))
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

#[derive(Debug, Deserialize)]
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

pub fn cost_timeseries(
    registry: &SourceRegistry,
    req: TimeseriesRequest,
) -> Result<TimeseriesResponse, ServiceError> {
    let (repo, mut filter) = resolve_common(
        registry, req.source_id.as_deref(), req.start, req.end, req.metric.as_deref(), &req.filters,
    )?;
    let metric = filter.metric;

    // Parse granularity
    let granularity = if let Some(ref g) = req.granularity {
        parse_granularity(g).ok_or_else(|| ServiceError::bad_request(format!("unknown granularity '{}'", g)))?
    } else {
        TimeGranularity::default()
    };
    filter.granularity = granularity;

    // Parse group_by (optional)
    let group_by = if let Some(ref d) = req.group_by {
        Some(parse_dimension(d).ok_or_else(|| ServiceError::bad_request(format!("unknown group_by '{}'", d)))?)
    } else {
        None
    };

    let result = repo.timeseries(&filter, group_by).map_err(|e| query_error("cost_timeseries", e))?;

    Ok(TimeseriesResponse { metric, currency: result.currency, granularity, series: result.points })
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/breakdown
// ---------------------------------------------------------------------------

/// Default and maximum Top-N result size for breakdown queries.
const DEFAULT_BREAKDOWN_LIMIT: usize = 50;
const MAX_BREAKDOWN_LIMIT: usize = 10_000;

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Serialize)]
pub struct BreakdownResponse {
    pub metric: CostMetric,
    pub currency: String,
    pub dimension: Dimension,
    pub rows: Vec<BreakdownRow>,
}

pub fn cost_breakdown(
    registry: &SourceRegistry,
    req: BreakdownRequest,
) -> Result<BreakdownResponse, ServiceError> {
    let (repo, filter) = resolve_common(
        registry, req.source_id.as_deref(), req.start, req.end, req.metric.as_deref(), &req.filters,
    )?;
    let metric = filter.metric;

    // Parse dimension (required)
    let dimension = match req.dimension {
        Some(ref d) => parse_dimension(d).ok_or_else(|| ServiceError::bad_request(format!("unknown dimension '{}'", d)))?,
        None => return Err(ServiceError::bad_request("dimension is required")),
    };

    // Validate limit
    let limit = req.limit.unwrap_or(DEFAULT_BREAKDOWN_LIMIT);
    if limit == 0 {
        return Err(ServiceError::bad_request("limit must be greater than 0"));
    }
    if limit > MAX_BREAKDOWN_LIMIT {
        return Err(ServiceError::bad_request(format!("limit must not exceed {}", MAX_BREAKDOWN_LIMIT)));
    }

    let result = repo.breakdown(&filter, dimension, limit).map_err(|e| query_error("cost_breakdown", e))?;

    Ok(BreakdownResponse { metric, currency: result.currency, dimension, rows: result.rows })
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
#[derive(Debug, Deserialize)]
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

pub fn cost_compare(registry: &SourceRegistry, req: CompareRequest) -> Result<CompareResponse, ServiceError> {
    // Validate both date-range pairs through the same path `resolve_common`
    // uses for its single pair — no hand-copied start/end or max-range checks.
    let (current_start, current_end) = validate_date_range(req.current_start, req.current_end)?;
    let (previous_start, previous_end) = validate_date_range(req.previous_start, req.previous_end)?;

    let metric = resolve_metric(req.metric.as_deref())?;

    let repo = registry.repo(req.source_id.as_deref())?;

    validate_filter_fields(&req.current, "current.")?;
    validate_filter_fields(&req.previous, "previous.")?;

    // Parse dimension (optional — unlike breakdown, compare without one is a
    // valid, meaningful single aggregate row)
    let dimension = match req.dimension {
        Some(ref d) => Some(parse_dimension(d).ok_or_else(|| ServiceError::bad_request(format!("unknown dimension '{}'", d)))?),
        None => None,
    };

    // Cloned before the filters are consumed below, so the effective
    // filters can be echoed back on the response (see `CompareResponse`'s
    // `current_filters`/`previous_filters` doc comment).
    let current_filters = req.current.clone();
    let previous_filters = req.previous.clone();

    let mut current_filter = CostFilter::date_range(current_start, current_end);
    current_filter.metric = metric;
    req.current.apply(&mut current_filter);
    let mut previous_filter = CostFilter::date_range(previous_start, previous_end);
    previous_filter.metric = metric;
    req.previous.apply(&mut previous_filter);

    let result = repo
        .compare(&current_filter, &previous_filter, dimension)
        .map_err(|e| query_error("cost_compare", e))?;

    Ok(CompareResponse {
        metric,
        currency: result.currency,
        dimension,
        rows: result.rows,
        current_filters,
        previous_filters,
    })
}

// ---------------------------------------------------------------------------
// filter-values / tag-values lookups
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct FilterValuesQuery {
    pub source_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TagValuesQuery {
    pub source_id: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FilterValuesResponse {
    pub values: Vec<String>,
}

/// Which distinct-value lookup `filter_values` performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDimension {
    Services,
    Accounts,
    Regions,
    TagKeys,
}

pub fn filter_values(
    registry: &SourceRegistry,
    dimension: FilterDimension,
    query: FilterValuesQuery,
) -> Result<FilterValuesResponse, ServiceError> {
    let repo = registry.repo(query.source_id.as_deref())?;
    let (op, result) = match dimension {
        FilterDimension::Services => ("distinct_services", repo.distinct_services()),
        FilterDimension::Accounts => ("distinct_accounts", repo.distinct_accounts()),
        FilterDimension::Regions => ("distinct_regions", repo.distinct_regions()),
        FilterDimension::TagKeys => ("distinct_tag_keys", repo.distinct_tag_keys()),
    };
    result.map(|values| FilterValuesResponse { values }).map_err(|e| query_error(op, e))
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/resource-search
// ---------------------------------------------------------------------------

const DEFAULT_RESOURCE_SEARCH_LIMIT: usize = 50;
const MAX_RESOURCE_SEARCH_LIMIT: usize = 200;

/// A resource-ID substring search, scoped like any `/cost/*` query (date
/// range, metric for ranking, predicate fields) because resource lists are
/// far too large for the whole-dataset `filter-values` lookups.
#[derive(Debug, Deserialize)]
pub struct ResourceSearchRequest {
    pub source_id: Option<String>,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub metric: Option<String>,
    #[serde(default)]
    pub q: String,
    pub limit: Option<usize>,
    #[serde(flatten)]
    pub filters: FilterFields,
}

pub fn resource_search(
    registry: &SourceRegistry,
    req: ResourceSearchRequest,
) -> Result<FilterValuesResponse, ServiceError> {
    let needle = req.q.trim();
    if needle.is_empty() {
        return Err(ServiceError::bad_request("q is required"));
    }
    let (repo, filter) = resolve_common(
        registry, req.source_id.as_deref(), req.start, req.end, req.metric.as_deref(), &req.filters,
    )?;
    let limit = req.limit.unwrap_or(DEFAULT_RESOURCE_SEARCH_LIMIT).clamp(1, MAX_RESOURCE_SEARCH_LIMIT);
    repo.search_resources(&filter, needle, limit)
        .map(|values| FilterValuesResponse { values })
        .map_err(|e| query_error("resource_search", e))
}

pub fn tag_values(registry: &SourceRegistry, query: TagValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    let key = match query.key {
        Some(k) if !k.is_empty() => k,
        _ => return Err(ServiceError::bad_request("key is required")),
    };
    let repo = registry.repo(query.source_id.as_deref())?;
    repo.distinct_tag_values(&key)
        .map(|values| FilterValuesResponse { values })
        .map_err(|e| query_error("distinct_tag_values", e))
}

// ---------------------------------------------------------------------------
// POST /api/v1/cost/estimate
// ---------------------------------------------------------------------------

/// Upper bound on `scans` per estimate request (a page fires a handful).
const MAX_ESTIMATE_SCANS: usize = 32;
/// Upper bound on ranges within one scan (`compare` reads two).
const MAX_RANGES_PER_SCAN: usize = 4;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EstimateRange {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// The queries a page is about to run: one entry per query, each listing
/// the date ranges that query reads (two for a current-vs-previous compare).
#[derive(Debug, Deserialize)]
pub struct EstimateRequest {
    pub source_id: Option<String>,
    pub scans: Vec<Vec<EstimateRange>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostTier {
    None,
    Soft,
    Hard,
}

#[derive(Debug, Serialize)]
pub struct EstimateResponse {
    /// Queries read over the network; local sources never cost anything.
    pub remote: bool,
    /// False when the source could not be indexed; bytes/usd are then 0.
    pub known: bool,
    pub bytes: u64,
    pub requests: u64,
    pub usd: f64,
    pub tier: CostTier,
    /// Some row groups lacked date statistics; the estimate is an upper bound.
    pub stats_missing: bool,
    pub soft_limit_usd: f64,
    pub hard_limit_usd: f64,
}

const BYTES_PER_GB: f64 = 1_000_000_000.0;

/// Estimates what running `req.scans` against the source would read from
/// S3 and what that costs under `guard`'s rates, and classifies it against
/// `guard`'s limits. `treat_local_as_remote` lets the dev/test harness
/// exercise the warning flow against local fixtures.
pub fn cost_estimate(
    registry: &SourceRegistry,
    guard: &CostGuardConfig,
    treat_local_as_remote: bool,
    req: EstimateRequest,
) -> Result<EstimateResponse, ServiceError> {
    if req.scans.is_empty() || req.scans.len() > MAX_ESTIMATE_SCANS {
        return Err(ServiceError::bad_request(format!(
            "scans must contain between 1 and {MAX_ESTIMATE_SCANS} entries"
        )));
    }
    let mut scans = Vec::with_capacity(req.scans.len());
    for ranges in &req.scans {
        if ranges.is_empty() || ranges.len() > MAX_RANGES_PER_SCAN {
            return Err(ServiceError::bad_request(format!(
                "each scan must contain between 1 and {MAX_RANGES_PER_SCAN} ranges"
            )));
        }
        for r in ranges {
            validate_date_range(r.start, r.end)?;
        }
        scans.push(ranges.iter().map(|r| (r.start, r.end)).collect::<Vec<_>>());
    }

    let scan = registry.scan(req.source_id.as_deref())?;
    let remote = scan.remote || treat_local_as_remote;
    let mut out = EstimateResponse {
        remote,
        known: scan.index.is_some(),
        bytes: 0,
        requests: 0,
        usd: 0.0,
        tier: CostTier::None,
        stats_missing: false,
        soft_limit_usd: guard.soft_limit_usd,
        hard_limit_usd: guard.hard_limit_usd,
    };
    let Some(index) = scan.index else {
        return Ok(out);
    };
    // No result or object cache: every query reads its row groups again.
    for ranges in &scans {
        let e = index.estimate(ranges);
        out.bytes += e.bytes;
        out.requests += e.requests;
        out.stats_missing |= e.stats_missing;
    }
    out.usd = out.bytes as f64 / BYTES_PER_GB * guard.egress_usd_per_gb
        + out.requests as f64 / 1000.0 * guard.get_usd_per_1000;
    if guard.enabled && remote {
        out.tier = if out.usd >= guard.hard_limit_usd {
            CostTier::Hard
        } else if out.usd >= guard.soft_limit_usd {
            CostTier::Soft
        } else {
            CostTier::None
        };
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::register::Registered;
    use crate::ErrorKind;
    use data::config::DataSource;
    use data::fixtures::{generate_focus12_fixture, FIXTURE_AMORTIZED_TOTAL_AUG};

    fn registry_with_fixture() -> (tempfile::TempDir, SourceRegistry) {
        let dir = tempfile::tempdir().unwrap();
        generate_focus12_fixture(dir.path()).unwrap();
        let source = DataSource {
            id: "f".into(),
            name: "F".into(),
            s3_uri: dir.path().to_str().unwrap().into(),
            ..Default::default()
        };
        let reg = SourceRegistry::new();
        reg.mark_pending(&source);
        let registered: Registered = crate::register::register_source(&source, None).unwrap();
        reg.set_result("f", Ok(registered));
        (dir, reg)
    }

    fn summary_req(start: &str, end: &str) -> SummaryRequest {
        serde_json::from_value(serde_json::json!({ "start": start, "end": end })).unwrap()
    }

    #[test]
    fn summary_matches_fixture_total() {
        let (_dir, reg) = registry_with_fixture();
        let s = cost_summary(&reg, summary_req("2026-08-01", "2026-09-01")).unwrap();
        assert!((s.total - FIXTURE_AMORTIZED_TOTAL_AUG).abs() < 0.01);
    }

    #[test]
    fn summary_rejects_inverted_range() {
        let (_dir, reg) = registry_with_fixture();
        let err = cost_summary(&reg, summary_req("2026-09-01", "2026-08-01")).unwrap_err();
        assert_eq!(err, ServiceError::bad_request("start must be before end"));
    }

    #[test]
    fn breakdown_requires_dimension() {
        let (_dir, reg) = registry_with_fixture();
        let req: BreakdownRequest =
            serde_json::from_value(serde_json::json!({ "start": "2026-08-01", "end": "2026-09-01" })).unwrap();
        assert_eq!(cost_breakdown(&reg, req).unwrap_err().message, "dimension is required");
    }

    #[test]
    fn tag_values_requires_key() {
        let (_dir, reg) = registry_with_fixture();
        let err = tag_values(&reg, TagValuesQuery::default()).unwrap_err();
        assert_eq!(err.kind, ErrorKind::BadRequest);
        assert_eq!(err.message, "key is required");
    }

    #[test]
    fn filter_values_lists_services() {
        let (_dir, reg) = registry_with_fixture();
        let v = filter_values(&reg, FilterDimension::Services, FilterValuesQuery::default()).unwrap();
        assert!(!v.values.is_empty());
    }

    fn resource_search_req(body: serde_json::Value) -> ResourceSearchRequest {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn resource_search_requires_query_and_clamps_limit() {
        let (_dir, reg) = registry_with_fixture();
        let err = resource_search(
            &reg,
            resource_search_req(serde_json::json!({ "start": "2026-08-01", "end": "2026-09-01", "q": "  " })),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::BadRequest);

        let res = resource_search(
            &reg,
            resource_search_req(serde_json::json!({ "start": "2026-08-01", "end": "2026-09-01", "q": "-", "limit": 100_000 })),
        )
        .unwrap();
        assert!(res.values.len() <= MAX_RESOURCE_SEARCH_LIMIT);
    }

    fn estimate_req(scans: serde_json::Value) -> EstimateRequest {
        serde_json::from_value(serde_json::json!({ "scans": scans })).unwrap()
    }

    fn aug() -> serde_json::Value {
        serde_json::json!([{ "start": "2026-08-01", "end": "2026-09-01" }])
    }

    #[test]
    fn estimate_for_local_source_never_alerts() {
        let (_dir, reg) = registry_with_fixture();
        let guard = CostGuardConfig { soft_limit_usd: 0.0, hard_limit_usd: 0.0, ..Default::default() };
        let e = cost_estimate(&reg, &guard, false, estimate_req(serde_json::json!([aug()]))).unwrap();
        assert!(!e.remote && e.known);
        assert!(e.bytes > 0);
        assert_eq!(e.tier, CostTier::None);
    }

    #[test]
    fn estimate_tiers_follow_limits_and_query_count() {
        let (_dir, reg) = registry_with_fixture();
        let one = cost_estimate(&reg, &CostGuardConfig::default(), true, estimate_req(serde_json::json!([aug()]))).unwrap();
        assert!(one.remote && one.usd > 0.0);
        assert_eq!(one.tier, CostTier::None, "a tiny fixture is far below the default limits");

        let three = cost_estimate(
            &reg, &CostGuardConfig::default(), true, estimate_req(serde_json::json!([aug(), aug(), aug()])),
        )
        .unwrap();
        assert_eq!(three.bytes, 3 * one.bytes);

        let soft = CostGuardConfig { soft_limit_usd: one.usd, hard_limit_usd: one.usd * 2.0, ..Default::default() };
        assert_eq!(cost_estimate(&reg, &soft, true, estimate_req(serde_json::json!([aug()]))).unwrap().tier, CostTier::Soft);
        assert_eq!(cost_estimate(&reg, &soft, true, estimate_req(serde_json::json!([aug(), aug()]))).unwrap().tier, CostTier::Hard);

        let disabled = CostGuardConfig { enabled: false, ..soft };
        assert_eq!(cost_estimate(&reg, &disabled, true, estimate_req(serde_json::json!([aug(), aug()]))).unwrap().tier, CostTier::None);
    }

    #[test]
    fn estimate_rejects_bad_ranges_and_shapes() {
        let (_dir, reg) = registry_with_fixture();
        let g = CostGuardConfig::default();
        let backwards = serde_json::json!([[{ "start": "2026-09-01", "end": "2026-08-01" }]]);
        for scans in [serde_json::json!([]), serde_json::json!([[]]), backwards] {
            let err = cost_estimate(&reg, &g, false, estimate_req(scans)).unwrap_err();
            assert_eq!(err.kind, ErrorKind::BadRequest);
        }
    }
}
