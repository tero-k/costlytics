//! Dev/test-only HTTP harness over `crates/service` — NOT part of the
//! desktop product. It exists so the Vite dev server, Playwright e2e suite
//! and `tests/http_integration.rs` exercise the real backend over HTTP.

pub mod handlers;
pub mod routes;
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;

use service::secrets::MemorySecretStore;
use service::CostlyticsService;

/// Harness with an in-memory secret store and no settings persistence;
/// sources are registered synchronously before this returns.
pub fn build_app(config: data::config::AppConfig) -> anyhow::Result<axum::Router> {
    build(config, None)
}

/// Like [`build_app`], but Settings changes are saved to `settings_path`.
pub fn build_app_persistent(
    config: data::config::AppConfig,
    settings_path: PathBuf,
) -> anyhow::Result<axum::Router> {
    build(config, Some(settings_path))
}

fn build(config: data::config::AppConfig, settings_path: Option<PathBuf>) -> anyhow::Result<axum::Router> {
    let svc = Arc::new(CostlyticsService::new(
        config,
        settings_path,
        Arc::new(MemorySecretStore::default()),
    ));
    svc.register_all();
    Ok(routes::build_router(svc))
}
