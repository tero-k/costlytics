use std::net::SocketAddr;

use axum::serve;
use tracing_subscriber::EnvFilter;

use api::build_app;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = data::config::AppConfig::load("config/example.toml")?;

    let host = config.server.host.clone();
    let port = config.server.port;

    let app = build_app(config)?;

    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve(listener, app).await?;
    Ok(())
}
