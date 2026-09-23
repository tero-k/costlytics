use std::sync::Arc;

/// Shared state cloned into every Axum handler: the transport-agnostic
/// backend (`crates/service`) this dev/test harness wraps.
pub type AppState = Arc<service::CostlyticsService>;
