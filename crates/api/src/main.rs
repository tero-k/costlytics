use std::net::SocketAddr;

use axum::serve;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Harness settings file; Playwright points this at a scratch copy so
    // e2e Settings tests never modify the checked-in example.
    let settings_path = std::env::var("COSTLYTICS_CONFIG").unwrap_or_else(|_| "config/example.toml".into());
    let config = data::config::AppConfig::load(&settings_path)?;

    let host = config.server.host.clone();
    let port = config.server.port;

    let app = api::build_app_persistent(config, settings_path.into())?;

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve(listener, app).await?;
    Ok(())
}
