mod config;
mod routes;

use crate::config::AppConfig;
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config_path = config_path();
    let app_config = AppConfig::load(&config_path)?;
    let state = AppState {
        config: Arc::new(app_config),
    };

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route("/v1/jobs/:job_id/logs", get(routes::logs::get_job_logs))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    println!(
        "metis-server listening on http://{}",
        listener.local_addr()?
    );

    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

fn config_path() -> PathBuf {
    std::env::var("METIS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}
