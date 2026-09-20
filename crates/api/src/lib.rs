pub mod handlers;
pub mod routes;
pub mod state;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};

use data::{
    adapters::{cur2, focus10, focus12},
    duckdb_pool,
    object_store::{LocalObjectStore, YearMonth},
    queries::summary::{CostRepository, DuckDbCostRepository},
    schema_detection::{self, DetectedSchema},
};

use crate::state::{AppState, SourceDiagnostic, SourceStatus};

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
/// files are schema-detected; sources whose first file is confirmed FOCUS 1.0,
/// FOCUS 1.2, or CUR 2.0 get their `normalized_cost` VIEW registered. For S3 sources,
/// startup discovery is skipped entirely; a warning is logged and the source
/// will be unavailable this session (S3 `ObjectStore` support is out of scope).
pub fn build_app(config: data::config::AppConfig) -> anyhow::Result<axum::Router> {
    let mut pools: HashMap<String, duckdb_pool::DbPool> = HashMap::new();
    let mut repos: HashMap<String, Arc<dyn CostRepository>> = HashMap::new();
    // Per-source diagnostic outcome, captured alongside (not instead of) the
    // existing `tracing::warn!`/`tracing::error!` calls below — every
    // `continue` point pushes a `Skipped` entry with the same reason that's
    // already being logged, and the loop's successful-registration exit
    // pushes a `Registered` entry. See `SourceDiagnostic`/`SourceStatus` in
    // `state.rs`. This never changes which sources get registered/skipped or
    // in what order — it only records what already happens.
    let mut source_statuses: Vec<SourceDiagnostic> = Vec::new();

    let (range_start, range_end) = default_discovery_range();
    let store = LocalObjectStore;

    for source in &config.sources {
        if source.is_s3() {
            let reason =
                "S3 source; S3 discovery not implemented this session".to_string();
            tracing::warn!(
                source_id = %source.id,
                "S3 source configured; S3 discovery is not implemented this session, skipping."
            );
            source_statuses.push(SourceDiagnostic {
                id: source.id.clone(),
                name: source.name.clone(),
                configured_type: source.source_type.clone(),
                status: SourceStatus::Skipped { reason },
            });
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
                source_statuses.push(SourceDiagnostic {
                    id: source.id.clone(),
                    name: source.name.clone(),
                    configured_type: source.source_type.clone(),
                    status: SourceStatus::Skipped {
                        reason: format!("partition discovery failed: {e}"),
                    },
                });
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
                source_statuses.push(SourceDiagnostic {
                    id: source.id.clone(),
                    name: source.name.clone(),
                    configured_type: source.source_type.clone(),
                    status: SourceStatus::Skipped {
                        reason: format!("failed to build DuckDB pool: {e}"),
                    },
                });
                continue;
            }
        };

        // Confirm schema and register view (only if we have files to inspect).
        // `detected_format` stays "none" when `files` is empty: no schema
        // detection runs in that case (see the `if !files.is_empty()` guard
        // below), yet the source still falls through to the unconditional
        // `pools`/`repos` insertion at the end of this iteration (existing
        // behavior, unchanged by this task) and is therefore queryable via
        // `source_id` — just with zero rows. Judgment call (documented per
        // the task brief): an empty-but-otherwise-valid local source is
        // reported `Registered { file_count: 0, .. }`, not `Skipped`, since
        // that's what it actually is from the API's point of view — it has a
        // pool/repo and answers queries (with empty results), which is a
        // materially different situation from a source that was never
        // registered at all (e.g. schema detection failure). A reader of
        // `GET /api/v1/sources` sees `file_count: 0` and a `detected_format`
        // of "none", which already makes the "no data" situation visible
        // without conflating it with a hard registration failure.
        let mut detected_format = "none".to_string();
        if !files.is_empty() {
            let conn = match pool.get() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        source_id = %source.id,
                        error = %e,
                        "failed to get pooled connection for schema detection/view registration"
                    );
                    source_statuses.push(SourceDiagnostic {
                        id: source.id.clone(),
                        name: source.name.clone(),
                        configured_type: source.source_type.clone(),
                        status: SourceStatus::Skipped {
                            reason: format!(
                                "failed to get pooled connection for schema detection/view registration: {e}"
                            ),
                        },
                    });
                    continue;
                }
            };

            if let Err(e) = conn.execute_batch("LOAD parquet;") {
                tracing::error!(source_id = %source.id, error = %e, "failed to load parquet extension");
                source_statuses.push(SourceDiagnostic {
                    id: source.id.clone(),
                    name: source.name.clone(),
                    configured_type: source.source_type.clone(),
                    status: SourceStatus::Skipped {
                        reason: format!("failed to load parquet extension: {e}"),
                    },
                });
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
                        source_statuses.push(SourceDiagnostic {
                            id: source.id.clone(),
                            name: source.name.clone(),
                            configured_type: source.source_type.clone(),
                            status: SourceStatus::Skipped {
                                reason: format!("failed to register view: {e}"),
                            },
                        });
                        continue;
                    }
                    detected_format = "focus12".to_string();
                }
                Ok(DetectedSchema::Cur2) => {
                    if let Err(e) = cur2::register_view(&conn, &files) {
                        tracing::error!(
                            source_id = %source.id,
                            error = %e,
                            "failed to register normalized_cost view; skipping source"
                        );
                        source_statuses.push(SourceDiagnostic {
                            id: source.id.clone(),
                            name: source.name.clone(),
                            configured_type: source.source_type.clone(),
                            status: SourceStatus::Skipped {
                                reason: format!("failed to register view: {e}"),
                            },
                        });
                        continue;
                    }
                    detected_format = "cur2".to_string();
                }
                Ok(DetectedSchema::Focus10) => {
                    if let Err(e) = focus10::register_view(&conn, &files) {
                        tracing::error!(
                            source_id = %source.id,
                            error = %e,
                            "failed to register normalized_cost view; skipping source"
                        );
                        source_statuses.push(SourceDiagnostic {
                            id: source.id.clone(),
                            name: source.name.clone(),
                            configured_type: source.source_type.clone(),
                            status: SourceStatus::Skipped {
                                reason: format!("failed to register view: {e}"),
                            },
                        });
                        continue;
                    }
                    detected_format = "focus10".to_string();
                }
                Ok(other) => {
                    tracing::warn!(
                        source_id = %source.id,
                        detected = ?other,
                        "detected schema is not FOCUS 1.0, FOCUS 1.2, or CUR 2.0; only these formats are supported this session, skipping source"
                    );
                    source_statuses.push(SourceDiagnostic {
                        id: source.id.clone(),
                        name: source.name.clone(),
                        configured_type: source.source_type.clone(),
                        status: SourceStatus::Skipped {
                            reason: "detected schema is not FOCUS 1.0/1.2 or CUR 2.0"
                                .to_string(),
                        },
                    });
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        source_id = %source.id,
                        error = %e,
                        "schema detection failed; skipping source"
                    );
                    source_statuses.push(SourceDiagnostic {
                        id: source.id.clone(),
                        name: source.name.clone(),
                        configured_type: source.source_type.clone(),
                        status: SourceStatus::Skipped {
                            reason: format!("schema detection failed: {e}"),
                        },
                    });
                    continue;
                }
            }
        }

        let repo: Arc<dyn CostRepository> = Arc::new(DuckDbCostRepository::new(pool.clone()));

        pools.insert(source.id.clone(), pool);
        repos.insert(source.id.clone(), repo);
        source_statuses.push(SourceDiagnostic {
            id: source.id.clone(),
            name: source.name.clone(),
            configured_type: source.source_type.clone(),
            status: SourceStatus::Registered {
                detected_format,
                file_count: files.len(),
            },
        });
    }

    let state = AppState {
        config,
        pools,
        repos,
        source_statuses,
    };

    Ok(routes::build_router(state))
}
