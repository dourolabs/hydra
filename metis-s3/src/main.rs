mod config;
mod s3;

use anyhow::Result;
use axum::{Json, Router, routing::get};
use config::AppConfig;
use serde_json::json;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config::config_path();
    let app_config = AppConfig::load(&config_path)?;
    let bind_addr = app_config.bind_addr();
    let storage_root = app_config.storage_root();

    info!(
        bind_addr = %bind_addr,
        storage_root = %storage_root.display(),
        "metis-s3 configuration loaded"
    );

    let app = build_router(&app_config);
    let listener = TcpListener::bind(&bind_addr).await?;
    let addr = listener.local_addr()?;

    info!("metis-s3 listening on http://{}", addr);
    println!("metis-s3 listening on http://{addr}");

    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(config: &AppConfig) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(RequestBodyLimitLayer::new(
            config.server.request_body_limit_bytes,
        ))
        .layer(TraceLayer::new_for_http());

    Router::new()
        .route("/healthz", get(healthz))
        .merge(s3::router(config.storage_root()))
        .layer(middleware)
}

async fn healthz() -> Json<serde_json::Value> {
    info!("healthz invoked");
    Json(json!({ "status": "ok" }))
}
