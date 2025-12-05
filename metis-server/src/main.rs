mod config;
mod job_engine;
mod routes;
mod store;

use crate::config::{AppConfig, build_kube_client};
use crate::job_engine::{JobEngine, KubernetesJobEngine};
use crate::store::{Store, MemoryStore, Status, Task};
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

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

    // Spawn background task to process pending jobs
    let background_state = state.clone();
    tokio::spawn(async move {
        process_pending_jobs(background_state).await;
    });

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route("/v1/jobs/:job_id/logs", get(routes::jobs::logs::get_job_logs))
        .route("/v1/jobs/:job_id/kill", post(routes::jobs::kill::kill_job))
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

/// Background task that periodically processes pending jobs.
/// 
/// This function runs in a loop, checking for pending tasks every few seconds
/// and starting them by:
/// 1. Setting their status to Running
/// 2. Creating the Kubernetes job via the job engine
async fn process_pending_jobs(state: AppState) {
    loop {
        // Check every 2 seconds
        sleep(Duration::from_secs(2)).await;

        // Get pending tasks
        let pending_ids = {
            let store = state.store.read().await;
            match store.list_pending_tasks().await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list pending tasks");
                    continue;
                }
            }
        };

        if pending_ids.is_empty() {
            continue;
        }

        info!(count = pending_ids.len(), "found pending tasks to process");

        // Process each pending task
        for metis_id in pending_ids {
            // Get the task to extract the prompt
            let prompt = {
                let store = state.store.read().await;
                match store.get_task(&metis_id).await {
                    Ok(Task::Spawn { prompt, .. }) => prompt,
                    Ok(Task::Ask) => {
                        warn!(metis_id = %metis_id, "task is Ask type, skipping job creation");
                        continue;
                    }
                    Err(err) => {
                        error!(metis_id = %metis_id, error = %err, "failed to get task");
                        continue;
                    }
                }
            };

            // Create the Kubernetes job
            match state.job_engine.create_job(&metis_id, &prompt).await {
                Ok(()) => {
                    info!(metis_id = %metis_id, "successfully created Kubernetes job");
                    // Set status to Running after successful job creation
                    let mut store = state.store.write().await;
                    match store.update_task_status(&metis_id, Status::Running).await {
                        Ok(()) => {
                            info!(metis_id = %metis_id, "set task status to Running");
                        }
                        Err(err) => {
                            warn!(metis_id = %metis_id, error = %err, "failed to set task to Running");
                        }
                    }
                }
                Err(err) => {
                    error!(metis_id = %metis_id, error = %err, "failed to create Kubernetes job");
                    // Set status to Failed
                    let mut store = state.store.write().await;
                    if let Err(update_err) = store.update_task_status(&metis_id, Status::Failed).await {
                        error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed");
                    } else {
                        info!(metis_id = %metis_id, "set task status to Failed");
                    }
                }
            }
        }
    }
}
