use std::collections::HashMap;
use std::sync::Arc;

use data::queries::summary::CostRepository;

/// Shared application state cloned into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub config: data::config::AppConfig,
    /// One DuckDB connection pool per `source_id`.
    pub pools: HashMap<String, data::duckdb_pool::DbPool>,
    /// One `CostRepository` per `source_id`.
    pub repos: HashMap<String, Arc<dyn CostRepository>>,
}
