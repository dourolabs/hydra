#![allow(clippy::too_many_arguments)]

pub mod app;
pub mod background;
pub mod config;
pub mod job_engine;
pub mod merge_queue;
pub mod routes;
pub mod store;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

#[cfg(test)]
mod test;

use crate::app::{AppState, ServiceState};
use crate::background::{AgentQueue, Spawner, start_background_scheduler};
use crate::config::{AppConfig, build_kube_client};
use crate::job_engine::KubernetesJobEngine;
use crate::store::{
    MemoryStore, Store,
    postgres::{self, PostgresStore},
};
use axum::{
    Json, Router,
    routing::{delete, get, post, put},
};
use metis_common::constants::{ENV_METIS_CONFIG, ENV_OPENAI_API_KEY};
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

pub async fn run_with_state(
    state: AppState,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<()> {
    // Run scheduler-backed workers for background processing (jobs, spawners, GitHub poller)
    let scheduler = start_background_scheduler(state.clone());

    let app = Router::new()
        .route("/health", get(health_check))
        .route(
            "/v1/issues",
            get(routes::issues::list_issues).post(routes::issues::create_issue),
        )
        .route(
            "/v1/issues/:issue_id",
            get(routes::issues::get_issue).put(routes::issues::update_issue),
        )
        .route(
            "/v1/issues/:issue_id/todo-items",
            post(routes::issues::add_todo_item).put(routes::issues::replace_todo_list),
        )
        .route(
            "/v1/issues/:issue_id/todo-items/:item_number",
            post(routes::issues::set_todo_item_status),
        )
        .route(
            "/v1/issues/:issue_id/todo-items/:item_number/",
            post(routes::issues::set_todo_item_status),
        )
        .route(
            "/v1/patches",
            get(routes::patches::list_patches).post(routes::patches::create_patch),
        )
        .route(
            "/v1/patches/:patch_id",
            get(routes::patches::get_patch).put(routes::patches::update_patch),
        )
        .route(
            "/v1/repositories",
            get(routes::repositories::list_repositories)
                .post(routes::repositories::create_repository),
        )
        .route(
            "/v1/users",
            get(routes::users::list_users).post(routes::users::create_user),
        )
        .route("/v1/users/:username", delete(routes::users::delete_user))
        .route(
            "/v1/users/:username/github-token",
            put(routes::users::set_github_token),
        )
        .route(
            "/v1/repositories/:organization/:repo",
            put(routes::repositories::update_repository),
        )
        .route(
            "/v1/merge-queues/:organization/:repo/:branch/patches",
            get(routes::merge_queues::get_merge_queue).post(routes::merge_queues::enqueue_patch),
        )
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route("/v1/agents", get(routes::agents::list_agents))
        .route(
            "/v1/jobs/:job_id",
            get(routes::jobs::get_job).delete(routes::jobs::kill::kill_job),
        )
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route(
            "/v1/jobs/:job_id/logs",
            get(routes::jobs::logs::get_job_logs),
        )
        .route(
            "/v1/jobs/:job_id/status",
            get(routes::jobs::status::get_job_status).post(routes::jobs::status::set_job_status),
        )
        .route(
            "/v1/jobs/:job_id/context",
            get(routes::jobs::context::get_job_context),
        )
        .with_state(state);

    let addr = listener.local_addr()?;

    info!("metis-server listening on http://{}", addr);
    println!("metis-server listening on http://{addr}");

    let serve_result = axum::serve(listener, app).await;
    scheduler.shutdown().await;
    serve_result?;

    Ok(())
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config_path();
    let app_config = AppConfig::load(&config_path)?;
    let service_state = ServiceState::from_config(&app_config.service);

    // Resolve OpenAI API key
    let openai_api_key = env::var(ENV_OPENAI_API_KEY)
        .ok()
        .or_else(|| app_config.metis.openai_api_key.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ENV_OPENAI_API_KEY} is not set. Provide it via the environment or config.toml."
            )
        })?;

    // Build Kubernetes client
    let kube_client = build_kube_client(&app_config.kubernetes).await?;

    // Create job engine
    let job_engine = KubernetesJobEngine {
        namespace: app_config.metis.namespace.clone(),
        openai_api_key,
        server_hostname: app_config.metis.server_hostname.clone(),
        client: kube_client,
    };

    let postgres_pool = postgres::init_pool(&app_config.database).await?;
    if let Some(pool) = &postgres_pool {
        postgres::run_migrations(pool).await?;
        info!("connected to Postgres and applied migrations");
    } else {
        info!("no Postgres database configured; using in-memory store");
    }

    let store: Arc<RwLock<Box<dyn Store>>> = match postgres_pool.clone() {
        Some(pool) => Arc::new(RwLock::new(Box::new(PostgresStore::new(pool)))),
        None => Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
    };

    let spawners = build_spawners(&app_config);

    let state = AppState {
        config: Arc::new(app_config),
        service_state: Arc::new(service_state),
        store,
        job_engine: Arc::new(job_engine),
        spawners,
    };

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    run_with_state(state, listener).await
}

async fn health_check() -> Json<serde_json::Value> {
    info!("health_check invoked");
    Json(json!({ "status": "ok" }))
}

pub fn config_path() -> PathBuf {
    std::env::var(ENV_METIS_CONFIG)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}

fn build_spawners(config: &AppConfig) -> Vec<Arc<dyn Spawner>> {
    config
        .background
        .agent_queues
        .iter()
        .map(|queue| Arc::new(AgentQueue::from_config(queue)) as Arc<dyn Spawner>)
        .collect()
}
