mod config;
mod routes;

use crate::config::AppConfig;
use axum::{
    Json, Router,
    routing::{get, post},
};
use metis_common::job_outputs::JobOutputPayload;
use metis_common::jobs::CreateJobRequestContext;
use serde_json::json;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub job_outputs: Arc<RwLock<HashMap<String, JobOutputPayload>>>,
    pub job_contexts: Arc<RwLock<HashMap<String, CreateJobRequestContext>>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config_path();
    let app_config = AppConfig::load(&config_path)?;
    let state = AppState {
        config: Arc::new(app_config),
        job_outputs: Arc::new(RwLock::new(HashMap::new())),
        job_contexts: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route("/v1/jobs/:job_id/logs", get(routes::logs::get_job_logs))
        .route(
            "/v1/jobs/:job_id/output",
            get(routes::output::get_job_output).post(routes::output::set_job_output),
        )
        .route(
            "/v1/jobs/:job_id/context",
            get(routes::context::get_job_context),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    let addr = listener.local_addr()?;

    info!("metis-server listening on http://{}", addr);
    println!("metis-server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    info!("health_check invoked");
    Json(json!({ "status": "ok" }))
}

fn config_path() -> PathBuf {
    std::env::var("METIS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}
