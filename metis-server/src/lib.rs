#![allow(clippy::too_many_arguments)]

pub mod app;
pub mod background;
pub mod config;
pub mod domain;
pub mod ee;
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
use crate::background::start_background_scheduler;
#[cfg(feature = "kubernetes")]
use crate::config::build_kube_client;
use crate::config::{AppConfig, GithubAppSection, JobEngineConfig, StorageConfig};
use crate::domain::actors::{Actor, ActorRef};
use crate::domain::secrets::SecretManager;
use crate::domain::users::{User, Username};
#[cfg(feature = "kubernetes")]
use crate::job_engine::KubernetesJobEngine;
use crate::job_engine::{LocalDockerJobEngine, LocalJobEngine};
use crate::store::{MemoryStore, Store, StoreError, sqlite_store::SqliteStore};
#[cfg(feature = "postgres")]
use crate::store::{
    migration,
    postgres_v2::{self, PostgresStoreV2},
};
use anyhow::Context;
use axum::{
    Json, Router, middleware,
    routing::{get, post, put},
};
use jsonwebtoken::EncodingKey;
use metis_common::constants::ENV_METIS_CONFIG;
use octocrab::Octocrab;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use tracing::info;

/// Build an `AppState` from an `AppConfig`.
///
/// This initializes the store, job engine, secret manager, and constructs the
/// shared application state. In local auth mode it also creates the default
/// user and writes the auth token file.
pub async fn build_app_state(app_config: AppConfig) -> anyhow::Result<AppState> {
    let service_state = ServiceState::default();

    info!(
        auth_mode = %app_config.auth,
        storage = %app_config.storage,
        job_engine = %app_config.job_engine,
        "starting server"
    );

    let github_app = match app_config.auth.github_app() {
        Some(gh) => build_github_app_client(gh)?,
        None => None,
    };

    // Initialize store based on configured storage backend
    let store: Arc<dyn Store> = match &app_config.storage {
        StorageConfig::Sqlite { sqlite_path } => {
            let db_url = format!("sqlite:{sqlite_path}?mode=rwc");
            let pool = SqliteStore::init_pool(&db_url)
                .await
                .context("failed to initialize SQLite pool")?;
            SqliteStore::run_migrations(&pool)
                .await
                .context("failed to run SQLite migrations")?;
            info!(path = %sqlite_path, "connected to SQLite and applied migrations");
            Arc::new(SqliteStore::new(pool))
        }
        StorageConfig::Postgres { database } => {
            #[cfg(feature = "postgres")]
            {
                let postgres_pool = postgres_v2::init_pool(database)
                    .await?
                    .context("database.url is required for postgres storage backend")?;
                postgres_v2::run_migrations(&postgres_pool).await?;
                info!("connected to Postgres and applied migrations");

                // Run migration from v1 to v2 in case there is unmigrated data
                let migration_result = migration::migrate_v1_to_v2(&postgres_pool).await?;
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
                Arc::new(PostgresStoreV2::new(postgres_pool))
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = database;
                anyhow::bail!(
                    "PostgreSQL storage backend requires the 'postgres' Cargo feature. \
                     Rebuild with `--features postgres`."
                );
            }
        }
        StorageConfig::Memory => {
            info!("using in-memory store");
            Arc::new(MemoryStore::new())
        }
    };

    // Initialize SecretManager from config (mandatory) — needed before
    // setup_local_auth so we can encrypt the GitHub PAT.
    let secret_manager = Arc::new(
        SecretManager::from_base64(&app_config.metis.secret_encryption_key)
            .context("invalid metis.METIS_SECRET_ENCRYPTION_KEY")?,
    );
    info!("secret encryption enabled");

    // In local auth mode, create a default user actor.
    if app_config.auth.is_local() {
        setup_local_auth(&app_config, store.as_ref(), &secret_manager).await?;
    }

    // Create job engine based on configured backend
    let job_engine: Arc<dyn crate::job_engine::JobEngine> = match &app_config.job_engine {
        JobEngineConfig::Docker => {
            let hostname = app_config.metis.server_hostname.trim();
            let server_url = if hostname.is_empty() {
                "http://host.docker.internal:8080".to_string()
            } else {
                format!("http://{hostname}")
            };
            match LocalDockerJobEngine::new(server_url).await {
                Ok(engine) => {
                    info!("using local Docker job engine");
                    Arc::new(engine)
                }
                Err(err) => {
                    anyhow::bail!(
                        "Docker is not available ({err}). Install Docker or use \
                         job_engine: \"local\" in your config.",
                    );
                }
            }
        }
        #[cfg(feature = "kubernetes")]
        JobEngineConfig::Kubernetes { kubernetes } => {
            let kube_client = build_kube_client(kubernetes).await?;
            let engine = KubernetesJobEngine {
                namespace: app_config.metis.namespace.clone(),
                server_hostname: app_config.metis.server_hostname.clone(),
                client: kube_client,
                image_pull_secrets: kubernetes.image_pull_secrets.clone(),
            };
            info!("using Kubernetes job engine");
            Arc::new(engine)
        }
        #[cfg(not(feature = "kubernetes"))]
        JobEngineConfig::Kubernetes { .. } => {
            anyhow::bail!(
                "Kubernetes job engine requires the 'kubernetes' Cargo feature. Rebuild with --features kubernetes"
            );
        }
        JobEngineConfig::Local { log_dir } => {
            let local_hostname = app_config.metis.server_hostname.trim();
            if local_hostname.is_empty() {
                anyhow::bail!(
                    "metis.server_hostname must be configured when using \
                     job_engine: \"local\""
                );
            }
            let local_server_url = format!("http://{local_hostname}");
            let log_dir_path = log_dir
                .as_ref()
                .map(crate::config::expand_path)
                .unwrap_or_else(|| std::env::temp_dir().join("metis-local-jobs"));
            info!(?log_dir_path, "using local process job engine");
            let engine = Arc::new(LocalJobEngine::new(local_server_url, log_dir_path, None));
            engine.start_reaper();
            engine
        }
    };

    let state = AppState::new(
        Arc::new(app_config),
        github_app,
        Arc::new(service_state),
        store,
        job_engine,
        secret_manager,
    );

    // Ensure the 'inbox' label exists (recurse=false, hidden=true).
    state.ensure_inbox_label().await;

    Ok(state)
}

/// Build the base Axum router with all metis API routes.
///
/// The returned router has public routes (health check, login) and
/// authenticated routes (issues, patches, sessions, etc.) but does **not** have
/// state applied. Call `.with_state(state)` after merging any additional
/// routes your application needs.
pub fn build_router(state: &AppState) -> Router<AppState> {
    let public_routes = Router::new()
        .route("/health", get(health_check))
        .route("/v1/version", get(routes::version::get_version))
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
            "/v1/labels",
            get(routes::labels::list_labels).post(routes::labels::create_label),
        )
        .route(
            "/v1/labels/:label_id",
            get(routes::labels::get_label)
                .put(routes::labels::update_label)
                .delete(routes::labels::delete_label),
        )
        .route(
            "/v1/labels/:label_id/objects/:object_id",
            put(routes::labels::add_label_association)
                .delete(routes::labels::remove_label_association),
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
        .route(
            "/v1/sessions",
            get(routes::sessions::list_sessions).post(routes::sessions::create_session),
        )
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
            "/v1/sessions/:session_id",
            get(routes::sessions::get_session).delete(routes::sessions::kill::kill_session),
        )
        .route(
            "/v1/sessions/:session_id/versions",
            get(routes::sessions::list_session_versions),
        )
        .route(
            "/v1/sessions/:session_id/versions/:version_number",
            get(routes::sessions::get_session_version),
        )
        .route(
            "/v1/sessions/:session_id/logs",
            get(routes::sessions::logs::get_session_logs),
        )
        .route(
            "/v1/sessions/:session_id/status",
            post(routes::sessions::status::set_session_status),
        )
        .route(
            "/v1/sessions/:session_id/context",
            get(routes::sessions::context::get_session_context),
        )
        .route(
            "/v1/messages",
            get(routes::messages::list_messages).post(routes::messages::send_message),
        )
        .route(
            "/v1/messages/receive",
            get(routes::messages::receive_messages),
        )
        .route("/v1/whoami", get(routes::whoami::whoami))
        .route("/v1/users/:username", get(routes::users::get_user))
        .route(
            "/v1/users/:username/secrets",
            get(routes::secrets::list_secrets),
        )
        .route(
            "/v1/users/:username/secrets/:name",
            put(routes::secrets::set_secret).delete(routes::secrets::delete_secret),
        )
        .route("/v1/events", get(routes::events::get_events))
        .route(
            "/v1/notifications",
            get(routes::notifications::list_notifications),
        )
        .route(
            "/v1/notifications/unread-count",
            get(routes::notifications::unread_count),
        )
        .route(
            "/v1/notifications/:notification_id/read",
            post(routes::notifications::mark_read),
        )
        .route(
            "/v1/notifications/read-all",
            post(routes::notifications::mark_all_read),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            routes::auth::require_auth,
        ));

    Router::new().merge(public_routes).merge(protected_routes)
}

pub async fn run_with_state(
    state: AppState,
    listener: tokio::net::TcpListener,
    app: Router,
) -> anyhow::Result<()> {
    // Run scheduler-backed workers for background processing (jobs, agents, GitHub poller)
    let scheduler = start_background_scheduler(state.clone());

    // Start automation runner (event-driven side effects from the policy engine)
    let (automation_shutdown_tx, automation_shutdown_rx) = tokio::sync::watch::channel(false);
    let automation_handle =
        crate::policy::runner::spawn_automation_runner(state, automation_shutdown_rx);

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

    let state = build_app_state(app_config).await?;

    let app = build_router(&state).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    run_with_state(state, listener, app).await
}

/// Create a default user actor for local auth mode.
///
/// When `auth_token_file` is set in the config, the generated auth token is
/// written to that path so the CLI can pick it up.
pub async fn setup_local_auth(
    config: &AppConfig,
    store: &dyn Store,
    secret_manager: &SecretManager,
) -> anyhow::Result<()> {
    let username = Username::from(
        config
            .auth
            .local_username()
            .context("setup_local_auth called without local auth config")?,
    );

    let actor_name = format!("u-{username}");

    let system_actor = ActorRef::System {
        worker_name: "local-auth-setup".into(),
        on_behalf_of: None,
    };

    // Create the actor and user if they don't already exist.
    match store.get_actor(&actor_name).await {
        Ok(_) => {
            info!("local auth actor already exists in store, skipping actor creation");
        }
        Err(StoreError::ActorNotFound(_)) => {
            let (actor, auth_token) = Actor::new_for_user(username.clone());
            store.add_actor(actor.clone(), &system_actor).await?;

            let user = User::new(
                username.clone(),
                None, // no GitHub user ID for PAT-based local mode
                false,
            );
            match store.add_user(user.clone(), &system_actor).await {
                Ok(()) => {}
                Err(StoreError::UserAlreadyExists(_)) => {}
                Err(err) => return Err(err.into()),
            }

            // Write the auth token to a file if auth_token_file is configured.
            if let Some(path) = config.auth.auth_token_file() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "failed to create directory for auth token at {}",
                            parent.display()
                        )
                    })?;
                }
                std::fs::write(path, &auth_token)
                    .with_context(|| format!("failed to write auth token to {}", path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                        .with_context(|| {
                            format!("failed to set permissions on {}", path.display())
                        })?;
                }
                info!("auth token written to {}", path.display());
            }
        }
        Err(err) => return Err(err.into()),
    }

    // Always store the GitHub PAT from config into the encrypted secret store.
    // This ensures downstream code (get_github_token_for_user) can find it,
    // and handles token updates between server restarts.
    if let Some(github_token) = config.auth.github_token() {
        let encrypted = secret_manager
            .encrypt(github_token)
            .context("failed to encrypt GitHub token")?;
        store
            .set_user_secret(
                &username,
                crate::domain::secrets::SECRET_GITHUB_TOKEN,
                &encrypted,
            )
            .await
            .context("failed to store GitHub token in secret store")?;
        info!("GitHub PAT stored in user_secrets for {username}");
    }

    info!("local auth configured");
    Ok(())
}

async fn health_check() -> Json<serde_json::Value> {
    info!("health_check invoked");
    Json(json!({ "status": "ok" }))
}

pub fn config_path() -> PathBuf {
    std::env::var(ENV_METIS_CONFIG)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.yaml"))
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
