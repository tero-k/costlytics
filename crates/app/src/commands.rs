//! One Tauri command per `/api/v1/...` path; names must match
//! `commandName()` in `web/src/transport.ts`. Every command runs its
//! blocking DuckDB / S3 work off the UI thread.

use std::sync::Arc;

use serde::Serialize;
use service::app::{
    SaveSourceRequest, SettingsResponse, SourceIdRequest, TestSourceRequest, TestSourceResponse,
};
use data::config::CostGuardConfig;
use service::cost::{
    self, BreakdownRequest, BreakdownResponse, CompareRequest, CompareResponse, EstimateRequest,
    EstimateResponse, FilterDimension,
    FilterValuesQuery, FilterValuesResponse, ResourceSearchRequest, SummaryRequest, TagValuesQuery, TimeseriesRequest,
    TimeseriesResponse,
};
use service::sources::{SourceEntry, SourcesResponse};
use service::{CostlyticsService, ServiceError};

type Svc<'a> = tauri::State<'a, Arc<CostlyticsService>>;

async fn blocking<T, F>(svc: &Svc<'_>, f: F) -> Result<T, ServiceError>
where
    T: Serialize + Send + 'static,
    F: FnOnce(&CostlyticsService) -> Result<T, ServiceError> + Send + 'static,
{
    let svc = Arc::clone(svc.inner());
    tauri::async_runtime::spawn_blocking(move || f(&svc))
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "command task panicked");
            Err(ServiceError::internal("internal error"))
        })
}

#[tauri::command]
pub async fn cost_summary(svc: Svc<'_>, req: SummaryRequest) -> Result<domain::cost::CostSummary, ServiceError> {
    blocking(&svc, move |s| cost::cost_summary(&s.registry, req)).await
}

#[tauri::command]
pub async fn cost_timeseries(svc: Svc<'_>, req: TimeseriesRequest) -> Result<TimeseriesResponse, ServiceError> {
    blocking(&svc, move |s| cost::cost_timeseries(&s.registry, req)).await
}

#[tauri::command]
pub async fn cost_breakdown(svc: Svc<'_>, req: BreakdownRequest) -> Result<BreakdownResponse, ServiceError> {
    blocking(&svc, move |s| cost::cost_breakdown(&s.registry, req)).await
}

#[tauri::command]
pub async fn cost_compare(svc: Svc<'_>, req: CompareRequest) -> Result<CompareResponse, ServiceError> {
    blocking(&svc, move |s| cost::cost_compare(&s.registry, req)).await
}

#[tauri::command]
pub async fn cost_estimate(svc: Svc<'_>, req: EstimateRequest) -> Result<EstimateResponse, ServiceError> {
    blocking(&svc, move |s| s.cost_estimate(req)).await
}

#[tauri::command]
pub async fn cost_resource_search(svc: Svc<'_>, req: ResourceSearchRequest) -> Result<FilterValuesResponse, ServiceError> {
    blocking(&svc, move |s| cost::resource_search(&s.registry, req)).await
}

async fn values(svc: Svc<'_>, dim: FilterDimension, req: FilterValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    blocking(&svc, move |s| cost::filter_values(&s.registry, dim, req)).await
}

#[tauri::command]
pub async fn filter_values_services(svc: Svc<'_>, req: FilterValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    values(svc, FilterDimension::Services, req).await
}

#[tauri::command]
pub async fn filter_values_accounts(svc: Svc<'_>, req: FilterValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    values(svc, FilterDimension::Accounts, req).await
}

#[tauri::command]
pub async fn filter_values_regions(svc: Svc<'_>, req: FilterValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    values(svc, FilterDimension::Regions, req).await
}

#[tauri::command]
pub async fn filter_values_tag_keys(svc: Svc<'_>, req: FilterValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    values(svc, FilterDimension::TagKeys, req).await
}

#[tauri::command]
pub async fn filter_values_tag_values(svc: Svc<'_>, req: TagValuesQuery) -> Result<FilterValuesResponse, ServiceError> {
    blocking(&svc, move |s| cost::tag_values(&s.registry, req)).await
}

#[tauri::command]
pub async fn sources(svc: Svc<'_>) -> Result<SourcesResponse, ServiceError> {
    blocking(&svc, |s| Ok(s.sources())).await
}

#[tauri::command]
pub async fn settings(svc: Svc<'_>) -> Result<SettingsResponse, ServiceError> {
    blocking(&svc, |s| Ok(s.settings())).await
}

#[tauri::command]
pub async fn settings_source_save(svc: Svc<'_>, req: SaveSourceRequest) -> Result<SourceEntry, ServiceError> {
    blocking(&svc, move |s| s.save_source(req)).await
}

#[tauri::command]
pub async fn settings_source_delete(svc: Svc<'_>, req: SourceIdRequest) -> Result<SourcesResponse, ServiceError> {
    blocking(&svc, move |s| s.delete_source(req)).await
}

#[tauri::command]
pub async fn settings_source_test(svc: Svc<'_>, req: TestSourceRequest) -> Result<TestSourceResponse, ServiceError> {
    blocking(&svc, move |s| s.test_source(req)).await
}

#[tauri::command]
pub async fn settings_source_reload(svc: Svc<'_>, req: SourceIdRequest) -> Result<SourceEntry, ServiceError> {
    blocking(&svc, move |s| s.reload_source(req)).await
}

#[tauri::command]
pub async fn settings_cost_guard_save(svc: Svc<'_>, req: CostGuardConfig) -> Result<CostGuardConfig, ServiceError> {
    blocking(&svc, move |s| s.save_cost_guard(req)).await
}
