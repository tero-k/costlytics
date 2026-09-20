use std::collections::HashMap;
use std::sync::Arc;

use data::queries::summary::CostRepository;

/// Outcome of `build_app`'s startup registration attempt for a single
/// configured source, captured alongside (not instead of) the existing
/// `tracing::warn!`/`tracing::error!` calls in that loop. See
/// `crate::build_app` for the single place these are constructed.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceStatus {
    /// The source's `normalized_cost` view was registered and it is
    /// queryable via `source_id`.
    Registered {
        detected_format: String,
        file_count: usize,
    },
    /// The source was not registered; `reason` is a human-readable
    /// explanation reusing the wording of the log message that already
    /// fires at the corresponding `continue` point in `build_app`.
    Skipped { reason: String },
}

/// Per-source diagnostic entry: the source's static config plus the outcome
/// of startup registration. One of these exists per entry in
/// `config.sources`, in the same order.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceDiagnostic {
    pub id: String,
    pub name: String,
    pub configured_type: data::config::SourceType,
    pub status: SourceStatus,
}

/// Shared application state cloned into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub config: data::config::AppConfig,
    /// One DuckDB connection pool per `source_id`.
    pub pools: HashMap<String, data::duckdb_pool::DbPool>,
    /// One `CostRepository` per `source_id`.
    pub repos: HashMap<String, Arc<dyn CostRepository>>,
    /// Per-configured-source registration outcome, in `config.sources`
    /// order. Populated once at startup by `build_app`; see
    /// `SourceDiagnostic`/`SourceStatus`.
    pub source_statuses: Vec<SourceDiagnostic>,
}
