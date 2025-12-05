mod config;
mod job_engine;
mod routes;
mod store;

use crate::config::{AppConfig, build_kube_client};
use crate::job_engine::{JobEngine, KubernetesJobEngine};
use crate::store::{Store, MemoryStore};
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
    pub job_engine: Arc<dyn JobEngine>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config_path();
    let app_config = AppConfig::load(&config_path)?;
    
    // Resolve OpenAI API key
    let openai_api_key = env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| app_config.metis.openai_api_key.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "OPENAI_API_KEY is not set. Provide it via the environment or config.toml."
        ))?;
    
    // Build Kubernetes client
    let kube_client = build_kube_client(&app_config.kubernetes).await?;
    
    // Create job engine
    let job_engine = KubernetesJobEngine {
        namespace: app_config.metis.namespace.clone(),
        worker_image: app_config.metis.worker_image.clone(),
        openai_api_key,
        server_hostname: app_config.metis.server_hostname.clone(),
        client: kube_client,
    };
    
    let store: Arc<RwLock<Box<dyn Store>>> = Arc::new(RwLock::new(Box::new(MemoryStore::new())));
    
    let state = AppState {
        config: Arc::new(app_config),
        store,
        job_engine: Arc::new(job_engine),
    };

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route("/v1/jobs/:job_id/logs", get(routes::jobs::logs::get_job_logs))
        .route(
            "/v1/jobs/:job_id/output",
            get(routes::jobs::output::get_job_output).post(routes::jobs::output::set_job_output),
        )
        .route(
            "/v1/jobs/:job_id/context",
            get(routes::jobs::context::get_job_context),
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
