#![allow(clippy::too_many_arguments)]

pub mod app;
pub mod background;
pub mod config;
pub mod domain;
pub mod job_engine;
pub mod merge_queue;
pub mod policy;
pub mod routes;
pub mod store;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

#[cfg(test)]
mod test;

use crate::app::{AppState, ServiceState};
use crate::background::{AgentQueue, start_background_scheduler};
use crate::config::StoreVersion;
use crate::config::{AppConfig, GithubAppSection, build_kube_client};
use crate::job_engine::KubernetesJobEngine;
use crate::store::{
    MemoryStore, Store, migration,
    postgres::{self, PostgresStore},
    postgres_v2::PostgresStoreV2,
};
use anyhow::Context;
use axum::{
    Json, Router, middleware,
    routing::{get, post, put},
};
use jsonwebtoken::EncodingKey;
use metis_common::constants::{
    ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_METIS_CONFIG, ENV_METIS_STORE_VERSION,
    ENV_OPENAI_API_KEY,
};
use octocrab::Octocrab;
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

pub async fn run_with_state(
    state: AppState,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<()> {
    // Run scheduler-backed workers for background processing (jobs, agents, GitHub poller)
    let scheduler = start_background_scheduler(state.clone());

    // Start automation runner (event-driven side effects from the policy engine)
    let (automation_shutdown_tx, automation_shutdown_rx) = tokio::sync::watch::channel(false);
    let automation_handle =
        crate::policy::runner::spawn_automation_runner(state.clone(), automation_shutdown_rx);

    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route("/v1/login", post(routes::login::login))
        .route(
            "/v1/github/app/client-id",
            get(routes::github::get_github_app_client_id),
        );

    let protected_routes = Router::new()
        .route(
            "/v1/issues",
            get(routes::issues::list_issues).post(routes::issues::create_issue),
        )
        .route(
            "/v1/issues/:issue_id",
            get(routes::issues::get_issue)
                .put(routes::issues::update_issue)
                .delete(routes::issues::delete_issue),
        )
        .route(
            "/v1/issues/:issue_id/versions",
            get(routes::issues::list_issue_versions),
        )
        .route(
            "/v1/issues/:issue_id/versions/:version_number",
            get(routes::issues::get_issue_version),
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
            get(routes::patches::get_patch)
                .put(routes::patches::update_patch)
                .delete(routes::patches::delete_patch),
        )
        .route(
            "/v1/patches/:patch_id/versions",
            get(routes::patches::list_patch_versions),
        )
        .route(
            "/v1/patches/:patch_id/versions/:version_number",
            get(routes::patches::get_patch_version),
        )
        .route(
            "/v1/patches/:patch_id/assets",
            post(routes::patches::create_patch_asset),
        )
        .route(
            "/v1/documents",
            get(routes::documents::list_documents).post(routes::documents::create_document),
        )
        .route(
            "/v1/documents/:document_id",
            get(routes::documents::get_document)
                .put(routes::documents::update_document)
                .delete(routes::documents::delete_document),
        )
        .route(
            "/v1/documents/:document_id/versions",
            get(routes::documents::list_document_versions),
        )
        .route(
            "/v1/documents/:document_id/versions/:version_number",
            get(routes::documents::get_document_version),
        )
        .route(
            "/v1/repositories",
            get(routes::repositories::list_repositories)
                .post(routes::repositories::create_repository),
        )
        .route(
            "/v1/repositories/:organization/:repo",
            put(routes::repositories::update_repository)
                .delete(routes::repositories::delete_repository),
        )
        .route(
            "/v1/merge-queues/:organization/:repo/:branch/patches",
            get(routes::merge_queues::get_merge_queue).post(routes::merge_queues::enqueue_patch),
        )
        .route("/v1/github/token", get(routes::github::get_github_token))
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
        .route(
            "/v1/agents",
            get(routes::agents::list_agents).post(routes::agents::create_agent),
        )
        .route(
            "/v1/agents/:agent_name",
            get(routes::agents::get_agent)
                .put(routes::agents::update_agent)
                .delete(routes::agents::delete_agent),
        )
        .route(
            "/v1/jobs/:job_id",
            get(routes::jobs::get_job).delete(routes::jobs::kill::kill_job),
        )
        .route(
            "/v1/jobs/:job_id/versions",
            get(routes::jobs::list_job_versions),
        )
        .route(
            "/v1/jobs/:job_id/versions/:version_number",
            get(routes::jobs::get_job_version),
        )
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route(
            "/v1/jobs/:job_id/logs",
            get(routes::jobs::logs::get_job_logs),
        )
        .route(
            "/v1/jobs/:job_id/status",
            post(routes::jobs::status::set_job_status),
        )
        .route(
            "/v1/jobs/:job_id/context",
            get(routes::jobs::context::get_job_context),
        )
        .route("/v1/whoami", get(routes::whoami::whoami))
        .route("/v1/users/:username", get(routes::users::get_user))
        .route("/v1/events", get(routes::events::get_events))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::auth::require_auth,
        ));

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state);

    let addr = listener.local_addr()?;

    info!("metis-server listening on http://{}", addr);
    println!("metis-server listening on http://{addr}");

    let serve_result = axum::serve(listener, app).await;
    scheduler.shutdown().await;
    let _ = automation_shutdown_tx.send(true);
    let _ = automation_handle.await;
    serve_result?;

    Ok(())
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = config_path();
    let app_config = AppConfig::load(&config_path)?;
    let service_state = ServiceState::default();
    let github_app = build_github_app_client(&app_config.github_app)?;

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
    let anthropic_api_key = env::var(ENV_ANTHROPIC_API_KEY)
        .ok()
        .or_else(|| app_config.metis.anthropic_api_key.clone())
        .filter(|value| !value.trim().is_empty());
    let claude_code_oauth_token = env::var(ENV_CLAUDE_CODE_OAUTH_TOKEN)
        .ok()
        .or_else(|| app_config.metis.claude_code_oauth_token.clone())
        .filter(|value| !value.trim().is_empty());

    // Build Kubernetes client
    let kube_client = build_kube_client(&app_config.kubernetes).await?;

    // Resolve store version from environment or config (env takes precedence)
    let store_version = env::var(ENV_METIS_STORE_VERSION)
        .ok()
        .and_then(|v| v.parse::<StoreVersion>().ok())
        .unwrap_or(app_config.database.store_version);

    let postgres_pool = postgres::init_pool(&app_config.database).await?;
    if let Some(pool) = &postgres_pool {
        postgres::run_migrations(pool).await?;
        postgres::migrate_payloads(pool).await?;
        info!("connected to Postgres, applied migrations, and migrated payloads");
    } else {
        info!("no Postgres database configured; using in-memory store");
    }

    let store: Arc<dyn Store> = match postgres_pool.clone() {
        Some(pool) => {
            match store_version {
                StoreVersion::V1 => {
                    info!(store_version = %store_version, "using v1 (JSONB) store");
                    Arc::new(PostgresStore::new(pool))
                }
                StoreVersion::V2 => {
                    info!(store_version = %store_version, "using v2 (column-based) store");
                    // Run migration from v1 to v2 before starting with v2 store
                    let migration_result = migration::migrate_v1_to_v2(&pool).await?;
                    if migration_result.total() > 0 {
                        info!(
                            total = migration_result.total(),
                            issues = migration_result.issues_migrated,
                            patches = migration_result.patches_migrated,
                            tasks = migration_result.tasks_migrated,
                            users = migration_result.users_migrated,
                            actors = migration_result.actors_migrated,
                            repositories = migration_result.repositories_migrated,
                            documents = migration_result.documents_migrated,
                            "migrated data from v1 to v2 tables"
                        );
                    }
                    Arc::new(PostgresStoreV2::new(pool))
                }
            }
        }
        None => Arc::new(MemoryStore::new()),
    };

    // Create job engine
    let job_engine = KubernetesJobEngine {
        namespace: app_config.metis.namespace.clone(),
        openai_api_key,
        anthropic_api_key,
        claude_code_oauth_token: claude_code_oauth_token.clone(),
        server_hostname: app_config.metis.server_hostname.clone(),
        client: kube_client,
        image_pull_secrets: app_config.kubernetes.image_pull_secrets.clone(),
    };

    let agents = build_agents(&app_config);

    let state = AppState::new(
        Arc::new(app_config),
        github_app,
        Arc::new(service_state),
        store,
        Arc::new(job_engine),
        agents,
    );

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

fn build_agents(config: &AppConfig) -> Arc<RwLock<Vec<Arc<AgentQueue>>>> {
    Arc::new(RwLock::new(
        config
            .background
            .agent_queues
            .iter()
            .map(|queue| Arc::new(AgentQueue::from_config(queue)))
            .collect(),
    ))
}

fn build_github_app_client(config: &GithubAppSection) -> anyhow::Result<Option<Octocrab>> {
    let key = EncodingKey::from_rsa_pem(config.private_key().as_bytes())
        .context("invalid GitHub App private key")?;
    Octocrab::builder()
        .app(config.app_id(), key)
        .build()
        .map(Some)
        .context("building GitHub App client")
}
