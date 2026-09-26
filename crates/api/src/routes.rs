use axum::{
    routing::{get, post},
    Router,
};
use axum::http::{HeaderName, HeaderValue};
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::{handlers, state::AppState};

/// Build the application router with all routes and middleware attached.
pub fn build_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/api/v1/health", get(handlers::health))
        .route("/api/v1/sources", get(handlers::sources_list))
        .route("/api/v1/cost/summary", post(handlers::cost_summary))
        .route("/api/v1/cost/timeseries", post(handlers::cost_timeseries))
        .route("/api/v1/cost/breakdown", post(handlers::cost_breakdown))
        .route("/api/v1/cost/compare", post(handlers::cost_compare))
        .route("/api/v1/cost/estimate", post(handlers::cost_estimate))
        .route("/api/v1/cost/resource-search", post(handlers::cost_resource_search))
        .route(
            "/api/v1/filter-values/services",
            get(handlers::filter_values_services),
        )
        .route(
            "/api/v1/filter-values/accounts",
            get(handlers::filter_values_accounts),
        )
        .route(
            "/api/v1/filter-values/regions",
            get(handlers::filter_values_regions),
        )
        .route(
            "/api/v1/filter-values/tag-keys",
            get(handlers::filter_values_tag_keys),
        )
        .route(
            "/api/v1/filter-values/tag-values",
            get(handlers::filter_values_tag_values),
        )
        .route("/api/v1/settings", get(handlers::settings_get))
        .route("/api/v1/settings/source-save", post(handlers::settings_source_save))
        .route("/api/v1/settings/source-delete", post(handlers::settings_source_delete))
        .route("/api/v1/settings/source-test", post(handlers::settings_source_test))
        .route("/api/v1/settings/source-reload", post(handlers::settings_source_reload))
        .route("/api/v1/settings/cost-guard-save", post(handlers::settings_cost_guard_save))
        .with_state(state);

    // Security headers via tower-http
    router
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        // Per-request tracing so `tracing_subscriber`'s init in main.rs is actually useful.
        .layer(TraceLayer::new_for_http())
        // Permissive CORS: acceptable for a single-operator, self-hosted MVP dashboard.
        // Tighten this (explicit allowed origins) before any multi-origin deployment.
        .layer(CorsLayer::permissive())
}
