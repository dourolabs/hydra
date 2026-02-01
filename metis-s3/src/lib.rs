pub mod config;
pub mod s3;

use anyhow::Result;
use axum::{Json, Router, routing::get};
use serde_json::json;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

pub fn build_router(storage_root: PathBuf) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .merge(s3::router(storage_root))
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(listener: TcpListener, storage_root: PathBuf) -> Result<()> {
    axum::serve(listener, build_router(storage_root)).await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    info!("healthz invoked");
    Json(json!({ "status": "ok" }))
}
