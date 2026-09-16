pub mod handlers;
pub mod routes;
pub mod state;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};

use data::{
    adapters::focus12,
    duckdb_pool,
    object_store::{LocalObjectStore, YearMonth},
    queries::summary::{CostRepository, DuckDbCostRepository},
    schema_detection::{self, DetectedSchema},
};

use crate::state::AppState;

/// Default date range used for startup partition discovery: the last 5 years
/// through the end of next month. This is broad enough to pick up any
/// generated fixture data or real historical exports without requiring the
/// operator to configure a range just to get the server to boot.
pub fn default_discovery_range() -> (NaiveDate, NaiveDate) {
    let today = Utc::now().date_naive();
    let start = NaiveDate::from_ymd_opt(today.year() - 5, today.month(), 1).unwrap_or(today);

    let current_ym = YearMonth::new(today.year(), today.month());
    let next_ym = if current_ym.month == 12 {
        YearMonth::new(current_ym.year + 1, 1)
    } else {
        YearMonth::new(current_ym.year, current_ym.month + 1)
    };
    let end = next_ym.end_date_exclusive();

    (start, end)
}

/// Build the Axum `Router` for the given config.
///
/// For local (non-S3) sources, partition discovery runs eagerly over a broad
/// default date range (see `default_discovery_range`), using
/// `data::discovery::discover_partitions` with a `LocalObjectStore`. Discovered
/// files are schema-detected; only sources whose first file is confirmed
/// FOCUS 1.2 get their `normalized_cost` VIEW registered. For S3 sources,
/// startup discovery is skipped entirely; a warning is logged and the source
/// will be unavailable this session (S3 `ObjectStore` support is out of scope).
pub fn build_app(config: data::config::AppConfig) -> anyhow::Result<axum::Router> {
    let mut pools: HashMap<String, duckdb_pool::DbPool> = HashMap::new();
    let mut repos: HashMap<String, Arc<dyn CostRepository>> = HashMap::new();

    let (range_start, range_end) = default_discovery_range();
    let store = LocalObjectStore;

    for source in &config.sources {
        if source.is_s3() {
            tracing::warn!(
                source_id = %source.id,
                "S3 source configured; S3 discovery is not implemented this session, skipping."
            );
            continue;
        }

        // Local source: discover BILLING_PERIOD=YYYY-MM partitions via Manifest.json.
        let partitions = match data::discovery::discover_partitions(
            &store,
            &source.s3_uri,
            range_start,
            range_end,
        ) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    source_id = %source.id,
                    error = %e,
                    "partition discovery failed; skipping source"
                );
                continue;
            }
        };

        let files: Vec<String> = partitions.into_iter().flat_map(|p| p.files).collect();

        if files.is_empty() {
            tracing::warn!(
                source_id = %source.id,
                path = %source.s3_uri,
                "no Parquet files found via discovery; source will return empty results"
            );
        } else {
            tracing::info!(
                source_id = %source.id,
                file_count = files.len(),
                "found Parquet files for source via discovery"
            );
        }

        // Build pool
        let pool = match duckdb_pool::build_pool() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(source_id = %source.id, error = %e, "failed to build DuckDB pool");
                continue;
            }
        };

        // Confirm schema and register view (only if we have files to inspect).
        if !files.is_empty() {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        source_id = %source.id,
                        error = %e,
                        "failed to get pooled connection for schema detection/view registration"
                    );
                    continue;
                }
            };

            if let Err(e) = conn.execute_batch("LOAD parquet;") {
                tracing::error!(source_id = %source.id, error = %e, "failed to load parquet extension");
                continue;
            }

            match schema_detection::detect_schema(&conn, &files[0]) {
                Ok(DetectedSchema::Focus12) => {
                    if let Err(e) = focus12::register_view(&conn, &files) {
                        tracing::error!(
                            source_id = %source.id,
                            error = %e,
                            "failed to register normalized_cost view; skipping source"
                        );
                        continue;
                    }
                }
                Ok(other) => {
                    tracing::warn!(
                        source_id = %source.id,
                        detected = ?other,
                        "detected schema is not FOCUS 1.2; only FOCUS 1.2 is supported this session, skipping source"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        source_id = %source.id,
                        error = %e,
                        "schema detection failed; skipping source"
                    );
                    continue;
                }
            }
        }

        let repo: Arc<dyn CostRepository> = Arc::new(DuckDbCostRepository::new(pool.clone()));

        pools.insert(source.id.clone(), pool);
        repos.insert(source.id.clone(), repo);
    }

    let state = AppState {
        config,
        pools,
        repos,
    };

    Ok(routes::build_router(state))
}
