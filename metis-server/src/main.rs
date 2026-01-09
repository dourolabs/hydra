#![allow(clippy::too_many_arguments)]

mod background;
mod config;
mod job_engine;
mod routes;
mod state;
mod store;
#[cfg(test)]
mod test;

use crate::background::{monitor_running_jobs, process_pending_jobs};
use crate::config::{AppConfig, build_kube_client};
use crate::job_engine::{JobEngine, KubernetesJobEngine};
use crate::state::ServiceState;
use crate::store::{MemoryStore, Store};
use axum::{
    Json, Router,
    routing::{get, post},
};
use metis_common::constants::{ENV_METIS_CONFIG, ENV_OPENAI_API_KEY};
use serde_json::json;
use std::{env, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub service_state: Arc<ServiceState>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
    pub job_engine: Arc<dyn JobEngine>,
}

async fn run_with_state(state: AppState, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    // Spawn background task to process pending jobs
    let background_state = state.clone();
    tokio::spawn(async move {
        process_pending_jobs(background_state).await;
    });

    // Spawn background task to monitor running jobs
    let monitor_state = state.clone();
    tokio::spawn(async move {
        monitor_running_jobs(monitor_state).await;
    });

    let app = Router::new()
        .route("/health", get(health_check))
        .route(
            "/v1/artifacts",
            get(routes::artifacts::list_artifacts).post(routes::artifacts::create_artifact),
        )
        .route(
            "/v1/artifacts/:artifact_id",
            get(routes::artifacts::get_artifact).put(routes::artifacts::update_artifact),
        )
        .route("/v1/jobs/", get(routes::jobs::list_jobs))
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
            "/v1/jobs/:job_id/output",
            get(routes::jobs::output::get_job_output).post(routes::jobs::output::set_job_output),
        )
        .route(
            "/v1/jobs/:job_id/events/emitted",
            post(routes::jobs::events::emit_artifacts),
        )
        .route(
            "/v1/jobs/:job_id/context",
            get(routes::jobs::context::get_job_context),
        )
        .with_state(state);

    let addr = listener.local_addr()?;

    info!("metis-server listening on http://{}", addr);
    println!("metis-server listening on http://{addr}");

    axum::serve(listener, app).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    let store: Arc<RwLock<Box<dyn Store>>> = Arc::new(RwLock::new(Box::new(MemoryStore::new())));

    let state = AppState {
        config: Arc::new(app_config),
        service_state: Arc::new(service_state),
        store,
        job_engine: Arc::new(job_engine),
    };

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    run_with_state(state, listener).await
}

async fn health_check() -> Json<serde_json::Value> {
    info!("health_check invoked");
    Json(json!({ "status": "ok" }))
}

fn config_path() -> PathBuf {
    std::env::var(ENV_METIS_CONFIG)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}

#[cfg(test)]
mod tests {
    use crate::{
        job_engine::{JobStatus, MockJobEngine},
        state::{GitRepository, ServiceState},
        store::{Edge, Status, Task},
        test::{
            spawn_test_server, spawn_test_server_with_state, test_client, test_state,
            test_state_with_engine,
        },
    };
    use chrono::{Duration, Utc};
    use metis_common::{
        artifacts::{
            Artifact, ArtifactKind, ArtifactRecord, ListArtifactsResponse, SearchArtifactsQuery,
            UpsertArtifactRequest, UpsertArtifactResponse,
        },
        constants::ENV_GH_TOKEN,
        job_outputs::JobOutputPayload,
        jobs::{
            Bundle, CreateJobResponse, JobSummary, ListJobsResponse, ParentContext, WorkerContext,
        },
    };
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    fn default_image() -> String {
        crate::config::MetisSection::default().worker_image
    }

    #[tokio::test]
    async fn health_route_runs_with_injected_dependencies() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/health", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "status": "ok" }));

        Ok(())
    }

    #[tokio::test]
    async fn create_job_enqueues_task() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = state.config.metis.worker_image.clone();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({ "program": "0" }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        assert!(!body.job_id.trim().is_empty());

        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn {
                context,
                program,
                params,
                image,
                env_vars: _,
            } => {
                assert_eq!(program, "0");
                assert!(params.is_empty());
                assert_eq!(context, Bundle::None);
                assert_eq!(image, default_image);
            }
        }

        let status = store_read.get_status(&body.job_id).await?;
        assert_eq!(status, Status::Pending);
        let parents = store_read.get_parents(&body.job_id).await?;
        assert!(parents.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn create_job_respects_parent_dependencies() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = state.config.metis.worker_image.clone();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        // Seed a parent task that is still pending.
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "parent-1".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({ "program": "0", "parent_ids": ["parent-1"] }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        assert!(!body.job_id.trim().is_empty());

        let store_read = store.read().await;
        let parents = store_read.get_parents(&body.job_id).await?;
        assert_eq!(
            parents,
            vec![Edge {
                id: "parent-1".to_string(),
                name: None
            }]
        );
        let status = store_read.get_status(&body.job_id).await?;
        assert_eq!(status, Status::Blocked);
        Ok(())
    }

    #[tokio::test]
    async fn create_job_allows_service_repository_bundle() -> anyhow::Result<()> {
        let mut state = test_state();
        let repo = GitRepository {
            name: "private-repo".to_string(),
            remote_url: "https://example.com/private.git".to_string(),
            default_branch: Some("develop".to_string()),
            github_token: Some("token-123".to_string()),
            default_image: Some("ghcr.io/example/repo:main".to_string()),
        };
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([("private-repo".to_string(), repo.clone())]),
        });
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "context": { "type": "service_repository", "name": "private-repo" }
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn {
                context,
                env_vars,
                image,
                ..
            } => {
                assert_eq!(
                    context,
                    Bundle::GitRepository {
                        url: repo.remote_url.clone(),
                        rev: "develop".to_string()
                    }
                );
                assert_eq!(env_vars.get(ENV_GH_TOKEN), Some(&"token-123".to_string()));
                assert_eq!(image, "ghcr.io/example/repo:main");
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_job_respects_image_override() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "image": "ghcr.io/example/custom:dev"
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn { image, .. } => {
                assert_eq!(image, "ghcr.io/example/custom:dev");
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_job_image_override_beats_repo_default() -> anyhow::Result<()> {
        let mut state = test_state();
        let repo = GitRepository {
            name: "private-repo".to_string(),
            remote_url: "https://example.com/private.git".to_string(),
            default_branch: Some("develop".to_string()),
            github_token: Some("token-123".to_string()),
            default_image: Some("ghcr.io/example/repo:main".to_string()),
        };
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([("private-repo".to_string(), repo.clone())]),
        });
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "context": { "type": "service_repository", "name": "private-repo" },
                "image": "ghcr.io/example/override:main"
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn {
                image, env_vars, ..
            } => {
                assert_eq!(image, "ghcr.io/example/override:main");
                assert_eq!(env_vars.get(ENV_GH_TOKEN), Some(&"token-123".to_string()));
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_job_stores_provided_variables() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "variables": { "FOO": "bar", "PROMPT": "custom prompt" }
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn { env_vars, .. } => {
                assert_eq!(env_vars.get("FOO"), Some(&"bar".to_string()));
                assert_eq!(env_vars.get("PROMPT"), Some(&"custom prompt".to_string()));
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_job_respects_user_supplied_github_token_variable() -> anyhow::Result<()> {
        let mut state = test_state();
        let repo = GitRepository {
            name: "private-repo".to_string(),
            remote_url: "https://example.com/private.git".to_string(),
            default_branch: Some("develop".to_string()),
            github_token: Some("token-123".to_string()),
            default_image: Some("ghcr.io/example/repo:main".to_string()),
        };
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([("private-repo".to_string(), repo.clone())]),
        });
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "context": { "type": "service_repository", "name": "private-repo" },
                "variables": { ENV_GH_TOKEN: "user-supplied" }
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn {
                env_vars, image, ..
            } => {
                assert_eq!(
                    env_vars.get(ENV_GH_TOKEN),
                    Some(&"user-supplied".to_string())
                );
                assert_eq!(
                    env_vars.get("PROMPT"),
                    None,
                    "server should not inject prompt automatically"
                );
                assert_eq!(image, "ghcr.io/example/repo:main");
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_job_rejects_unknown_service_repository() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({
                "program": "0",
                "context": { "type": "service_repository", "name": "missing" }
            }))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "unknown repository 'missing'" }));
        Ok(())
    }

    #[tokio::test]
    async fn list_jobs_returns_empty_list_when_store_is_empty() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: ListJobsResponse = response.json().await?;
        assert!(body.jobs.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn list_jobs_sorts_summaries_by_most_recent_time() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(engine);
        let default_image = default_image();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let oldest_id = "oldest".to_string();
        let middle_id = "middle".to_string();
        let newest_id = "newest".to_string();
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    oldest_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    now - Duration::seconds(30),
                )
                .await?;
            store_write
                .add_task_with_id(
                    middle_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    now - Duration::seconds(20),
                )
                .await?;
            store_write
                .add_task_with_id(
                    newest_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    now - Duration::seconds(10),
                )
                .await?;
            store_write
                .mark_task_running(&middle_id, now - Duration::seconds(15))
                .await?;
            store_write
                .mark_task_running(&newest_id, now - Duration::seconds(5))
                .await?;
        }

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: ListJobsResponse = response.json().await?;
        let ids: Vec<String> = body.jobs.into_iter().map(|job| job.id).collect();
        assert_eq!(ids, vec![newest_id, middle_id, oldest_id]);
        Ok(())
    }

    #[tokio::test]
    async fn get_job_returns_summary_for_existing_job() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;
        let job_id = "job-123".to_string();
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    now - Duration::seconds(20),
                )
                .await?;
            store_write
                .mark_task_running(&job_id, now - Duration::seconds(10))
                .await?;
        }

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/{job_id}", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let summary: JobSummary = response.json().await?;
        assert_eq!(summary.id, job_id);
        assert_eq!(summary.status_log.current_status(), Status::Running);
        assert_eq!(
            summary.status_log.start_time(),
            Some(now - Duration::seconds(10))
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/%20", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_trims_job_id_path() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;
        let job_id = "trim-job".to_string();
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    now - Duration::seconds(30),
                )
                .await?;
            store_write
                .mark_task_running(&job_id, now - Duration::seconds(10))
                .await?;
        }

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/%20{}%20", server.base_url(), job_id))
            .send()
            .await?;

        assert!(response.status().is_success());
        let summary: JobSummary = response.json().await?;
        assert_eq!(summary.id, job_id);
        Ok(())
    }

    #[tokio::test]
    async fn get_job_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/missing", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job 'missing' not found" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/ /logs", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_returns_bad_request_when_multiple_jobs_found() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        engine
            .insert_job(&"job-1".to_string(), JobStatus::Running)
            .await;
        engine
            .insert_job(&"job-1".to_string(), JobStatus::Failed)
            .await;
        let state = test_state_with_engine(engine);
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/job-1/logs", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": "Multiple jobs found for metis-id 'job-1'" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/missing/logs", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "Job 'missing' not found" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_streams_when_watching_running_job() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        let job_id = "job-stream".to_string();
        engine.insert_job(&job_id, JobStatus::Running).await;
        engine
            .set_logs(
                &job_id,
                vec!["first chunk".to_string(), "second chunk".to_string()],
            )
            .await;
        let state = test_state_with_engine(engine);
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!(
                "{}/v1/jobs/{job_id}/logs?watch=true",
                server.base_url()
            ))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body = response.text().await?;
        assert!(!body.is_empty(), "expected SSE body, got empty string");
        assert!(body.contains("first chunk"));
        assert!(body.contains("second chunk"));
        Ok(())
    }

    #[tokio::test]
    async fn kill_job_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .delete(format!("{}/v1/jobs/%20", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn kill_job_returns_not_found_for_unknown_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .delete(format!("{}/v1/jobs/unknown", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "Job 'unknown' not found" }));
        Ok(())
    }

    #[tokio::test]
    async fn kill_job_handles_multiple_matches_conflict() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        engine
            .insert_job(&"dupe".to_string(), JobStatus::Running)
            .await;
        engine
            .insert_job(&"dupe".to_string(), JobStatus::Running)
            .await;
        let state = test_state_with_engine(engine);
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .delete(format!("{}/v1/jobs/dupe", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": "Multiple jobs found for metis-id 'dupe'" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn set_job_output_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let payload = JobOutputPayload {
            last_message: "msg".to_string(),
            patch: "diff".to_string(),
            bundle: Bundle::None,
        };
        let response = client
            .post(format!("{}/v1/jobs/ /output", server.base_url()))
            .json(&payload)
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn set_job_output_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let payload = JobOutputPayload {
            last_message: "msg".to_string(),
            patch: "diff".to_string(),
            bundle: Bundle::None,
        };
        let response = client
            .post(format!("{}/v1/jobs/missing/output", server.base_url()))
            .json(&payload)
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert!(body["error"].as_str().unwrap_or("").contains("not found"));
        Ok(())
    }

    #[tokio::test]
    async fn set_job_output_persists_result_for_spawn_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            let job_id = "spawn-job".to_string();
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            let artifact_id = store_write
                .add_artifact(Artifact::Patch {
                    diff: "diff".to_string(),
                    description: "done".to_string(),
                })
                .await?;
            store_write
                .emit_task_artifacts(&job_id, vec![artifact_id], Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let payload = JobOutputPayload {
            last_message: "done".to_string(),
            patch: "diff".to_string(),
            bundle: Bundle::None,
        };
        let response = client
            .post(format!("{}/v1/jobs/spawn-job/output", server.base_url()))
            .json(&payload)
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({
                "job_id": "spawn-job",
                "output": { "last_message": "done", "patch": "diff", "bundle": { "type": "none" } }
            })
        );

        Ok(())
    }

    #[tokio::test]
    async fn get_job_output_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/ /output", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_output_returns_not_found_for_unknown_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/missing/output", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert!(body["error"].as_str().unwrap_or("").contains("not found"));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_output_returns_stored_output() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            let job_id = "with-output".to_string();
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            let artifact_id = store_write
                .add_artifact(Artifact::Patch {
                    diff: "diff".to_string(),
                    description: "all good".to_string(),
                })
                .await?;
            store_write
                .emit_task_artifacts(&job_id, vec![artifact_id], Utc::now())
                .await?;
            store_write
                .mark_task_complete(&job_id, Ok(()), Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/with-output/output", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({
                "job_id": "with-output",
                "output": { "last_message": "all good", "patch": "diff", "bundle": {"type": "none"}}
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_output_errors_when_result_is_missing() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            let job_id = "no-output".to_string();
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/no-output/output", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert!(
            body["error"]
                .as_str()
                .unwrap_or("")
                .contains("has not completed")
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/ /context", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "job_id must not be empty" }));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_returns_not_found_for_unknown_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/missing/context", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert!(body["error"].as_str().unwrap_or("").contains("not found"));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_returns_context_for_spawn_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let context = Bundle::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        };
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "parent-job".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write
                .mark_task_running(&"parent-job".to_string(), Utc::now())
                .await?;
            let parent_artifact_id = store_write
                .add_artifact(Artifact::Patch {
                    diff: "patch-content".to_string(),
                    description: "done".to_string(),
                })
                .await?;
            store_write
                .emit_task_artifacts(
                    &"parent-job".to_string(),
                    vec![parent_artifact_id],
                    Utc::now(),
                )
                .await?;
            store_write
                .mark_task_complete(&"parent-job".to_string(), Ok(()), Utc::now())
                .await?;
            store_write
                .add_task_with_id(
                    "ctx-job".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: context.clone(),
                        image: default_image.clone(),
                        env_vars: HashMap::new(),
                    },
                    vec![Edge {
                        id: "parent-job".to_string(),
                        name: None,
                    }],
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/ctx-job/context", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: WorkerContext = response.json().await?;
        assert_eq!(body.request_context, context);
        assert!(body.params.is_empty());
        assert_eq!(body.parents.len(), 1);
        assert_eq!(
            body.parents.get("parent-job"),
            Some(&ParentContext {
                name: None,
                output: JobOutputPayload {
                    last_message: "done".to_string(),
                    patch: "patch-content".to_string(),
                    bundle: Bundle::None,
                }
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_includes_task_variables() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "env-job".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        image: default_image.clone(),
                        env_vars: HashMap::from([(
                            "SECRET_VALUE".to_string(),
                            "keep-me-safe".to_string(),
                        )]),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/env-job/context", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: WorkerContext = response.json().await?;
        assert_eq!(
            body.variables,
            HashMap::from([("SECRET_VALUE".to_string(), "keep-me-safe".to_string())])
        );
        Ok(())
    }

    #[tokio::test]
    async fn artifacts_can_be_created_and_retrieved() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let artifact = Artifact::Patch {
            diff: "diff --git a/file b/file".to_string(),
            description: "initial patch".to_string(),
        };

        let response = client
            .post(format!("{}/v1/artifacts", server.base_url()))
            .json(&UpsertArtifactRequest {
                artifact: artifact.clone(),
            })
            .send()
            .await?;

        assert!(response.status().is_success());
        let created: UpsertArtifactResponse = response.json().await?;
        assert!(!created.artifact_id.is_empty());

        let fetched: ArtifactRecord = client
            .get(format!(
                "{}/v1/artifacts/{}",
                server.base_url(),
                created.artifact_id
            ))
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(fetched.id, created.artifact_id);
        assert_eq!(fetched.artifact, artifact);
        Ok(())
    }

    #[tokio::test]
    async fn list_artifacts_supports_filters() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let patch = Artifact::Patch {
            diff: "refactor logging".to_string(),
            description: "refactor logging".to_string(),
        };
        let issue = Artifact::Issue {
            description: "login fails for guests".to_string(),
        };
        let filtered_patch = Artifact::Patch {
            diff: "add login retry handling".to_string(),
            description: "login retry patch".to_string(),
        };

        for artifact in [patch.clone(), issue.clone(), filtered_patch.clone()] {
            let response = client
                .post(format!("{}/v1/artifacts", server.base_url()))
                .json(&UpsertArtifactRequest { artifact })
                .send()
                .await?;
            assert!(response.status().is_success());
        }

        let all: ListArtifactsResponse = client
            .get(format!("{}/v1/artifacts", server.base_url()))
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(all.artifacts.len(), 3);

        let filtered: ListArtifactsResponse = client
            .get(format!("{}/v1/artifacts", server.base_url()))
            .query(&SearchArtifactsQuery {
                artifact_type: Some(ArtifactKind::Patch),
                q: Some("login".to_string()),
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(filtered.artifacts.len(), 1);
        assert_eq!(filtered.artifacts[0].artifact, filtered_patch);
        Ok(())
    }

    #[tokio::test]
    async fn update_artifact_replaces_existing_value() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();

        let created: UpsertArtifactResponse = client
            .post(format!("{}/v1/artifacts", server.base_url()))
            .json(&UpsertArtifactRequest {
                artifact: Artifact::Patch {
                    diff: "old diff".to_string(),
                    description: "old patch".to_string(),
                },
            })
            .send()
            .await?
            .json()
            .await?;

        let updated: UpsertArtifactResponse = client
            .put(format!(
                "{}/v1/artifacts/{}",
                server.base_url(),
                created.artifact_id
            ))
            .json(&UpsertArtifactRequest {
                artifact: Artifact::Issue {
                    description: "updated details".to_string(),
                },
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(updated.artifact_id, created.artifact_id);

        let fetched: ArtifactRecord = client
            .get(format!(
                "{}/v1/artifacts/{}",
                server.base_url(),
                created.artifact_id
            ))
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(
            fetched.artifact,
            Artifact::Issue {
                description: "updated details".to_string()
            }
        );
        Ok(())
    }
}
