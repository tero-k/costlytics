mod handlers;
mod routes;
mod state;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::serve;
use tracing_subscriber::EnvFilter;

use data::{
    adapters::focus12,
    duckdb_pool,
    queries::summary::{CostRepository, DuckDbCostRepository},
};

use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = data::config::AppConfig::load("config/example.toml")?;

    let app = build_app(config)?;

    let addr: SocketAddr = "127.0.0.1:3000".parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve(listener, app).await?;
    Ok(())
}

/// Build the Axum `Router` for the given config.
///
/// For local (non-S3) sources, the DuckDB pool is built and the `normalized_cost`
/// VIEW is registered eagerly.  For S3 sources, startup is skipped; a warning is
/// logged and the source will be unavailable until Task 8 adds discovery.
pub fn build_app(config: data::config::AppConfig) -> anyhow::Result<axum::Router> {
    let mut pools: HashMap<String, duckdb_pool::DbPool> = HashMap::new();
    let mut repos: HashMap<String, Arc<dyn CostRepository>> = HashMap::new();

    for source in &config.sources {
        if source.is_s3() {
            tracing::warn!(
                source_id = %source.id,
                "S3 source configured; discovery will run on first request."
            );
            continue;
        }

        // Local source: scan directory for .parquet files
        let dir_path = &source.s3_uri;
        let files = collect_parquet_files(dir_path);

        if files.is_empty() {
            tracing::warn!(
                source_id = %source.id,
                path = %dir_path,
                "no Parquet files found; source will return empty results"
            );
        } else {
            tracing::info!(
                source_id = %source.id,
                file_count = files.len(),
                "found Parquet files for source"
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

        // Register view (if we have files)
        if !files.is_empty() {
            let conn = pool.get()?;
            if let Err(e) = focus12::register_view(&conn, &files) {
                tracing::error!(
                    source_id = %source.id,
                    error = %e,
                    "failed to register normalized_cost view"
                );
            }
        }

        let repo: Arc<dyn CostRepository> =
            Arc::new(DuckDbCostRepository::new(pool.clone()));

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

/// Scan `dir_path` for files with a `.parquet` extension.
/// Returns a sorted list of absolute path strings with forward slashes,
/// suitable for embedding in DuckDB SQL.
fn collect_parquet_files(dir_path: &str) -> Vec<String> {
    let path = std::path::Path::new(dir_path);
    if !path.is_dir() {
        return Vec::new();
    }
    let mut files: Vec<String> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let p = entry.path();
            if p.extension()?.to_ascii_lowercase() == "parquet" {
                Some(
                    p.canonicalize()
                        .unwrap_or(p)
                        .to_string_lossy()
                        .replace('\\', "/"),
                )
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
}
