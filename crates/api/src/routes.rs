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
        .route("/api/v1/cost/summary", post(handlers::cost_summary))
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
