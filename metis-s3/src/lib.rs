pub mod config;
pub mod s3;

use anyhow::Result;
use axum::{Json, Router, routing::get};
use serde_json::json;
use std::path::PathBuf;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::info;

pub fn build_router(storage_root: PathBuf, request_body_limit_bytes: usize) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(RequestBodyLimitLayer::new(request_body_limit_bytes))
        .layer(TraceLayer::new_for_http());

    Router::new()
        .route("/healthz", get(healthz))
        .merge(s3::router(storage_root))
        .layer(middleware)
}

pub async fn serve(
    listener: TcpListener,
    storage_root: PathBuf,
    request_body_limit_bytes: usize,
) -> Result<()> {
    axum::serve(
        listener,
        build_router(storage_root, request_body_limit_bytes),
    )
    .await?;
    Ok(())
}

async fn healthz() -> Json<serde_json::Value> {
    info!("healthz invoked");
    Json(json!({ "status": "ok" }))
}
