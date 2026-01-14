#![allow(clippy::too_many_arguments)]

mod background;
mod config;
mod job_engine;
mod routes;
mod state;
mod store;
#[cfg(test)]
mod test;

use crate::background::{
    AgentQueue, Spawner, monitor_running_jobs, process_pending_jobs, run_spawners,
};
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
    pub spawners: Vec<Arc<dyn Spawner>>,
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

    // Spawn background task to run configured spawners
    let spawner_state = state.clone();
    tokio::spawn(async move {
        run_spawners(spawner_state).await;
    });

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
            "/v1/patches",
            get(routes::patches::list_patches).post(routes::patches::create_patch),
        )
        .route(
            "/v1/patches/:patch_id",
            get(routes::patches::get_patch).put(routes::patches::update_patch),
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

fn config_path() -> PathBuf {
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

#[cfg(test)]
mod tests {
    use crate::{
        job_engine::{JobStatus, MockJobEngine},
        state::{GitRepository, ServiceState},
        store::{Status, Task, TaskError, TaskExt},
        test::{
            spawn_test_server, spawn_test_server_with_state, test_client, test_state,
            test_state_with_engine,
        },
    };
    use chrono::{Duration, Utc};
    use metis_common::{
        TaskId,
        constants::ENV_GH_TOKEN,
        issues::{
            Issue, IssueRecord, IssueStatus, IssueType, ListIssuesResponse, SearchIssuesQuery,
            UpsertIssueRequest, UpsertIssueResponse,
        },
        job_status::GetJobStatusResponse,
        jobs::{Bundle, BundleSpec, CreateJobResponse, JobRecord, ListJobsResponse, WorkerContext},
        patches::{
            ListPatchesResponse, Patch, PatchRecord, PatchStatus, SearchPatchesQuery,
            UpsertPatchRequest, UpsertPatchResponse,
        },
        task_status::Event,
    };
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    fn default_image() -> String {
        crate::config::MetisSection::default().worker_image
    }

    fn task_id(value: &str) -> TaskId {
        value.parse().expect("task id should be valid")
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
        let service_state = state.service_state.clone();
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
        assert!(!body.job_id.as_ref().trim().is_empty());

        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        let resolved = task.resolve(service_state.as_ref(), &default_image)?;
        let Task {
            context,
            program,
            params,
            ..
        } = task;

        assert_eq!(program, "0");
        assert!(params.is_empty());
        assert_eq!(context, BundleSpec::None);
        assert_eq!(resolved.context.bundle, Bundle::None);
        assert_eq!(resolved.image, default_image);

        let status = store_read.get_status(&body.job_id).await?;
        assert_eq!(status, Status::Pending);
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
        let service_state = state.service_state.clone();
        let fallback_image = state.config.metis.worker_image.clone();
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
        let resolved = task.resolve(service_state.as_ref(), &fallback_image)?;
        let Task { context, .. } = task;
        assert_eq!(
            context,
            BundleSpec::ServiceRepository {
                name: "private-repo".to_string(),
                rev: None
            }
        );
        assert_eq!(
            resolved.context.bundle,
            Bundle::GitRepository {
                url: repo.remote_url.clone(),
                rev: "develop".to_string()
            }
        );
        assert_eq!(
            resolved.env_vars.get(ENV_GH_TOKEN),
            Some(&"token-123".to_string())
        );
        assert_eq!(resolved.image, "ghcr.io/example/repo:main");

        Ok(())
    }

    #[tokio::test]
    async fn create_job_respects_image_override() -> anyhow::Result<()> {
        let state = test_state();
        let service_state = state.service_state.clone();
        let fallback_image = state.config.metis.worker_image.clone();
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
        let resolved = task.resolve(service_state.as_ref(), &fallback_image)?;
        assert_eq!(task.image, Some("ghcr.io/example/custom:dev".to_string()));
        assert_eq!(resolved.image, "ghcr.io/example/custom:dev");

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
        let service_state = state.service_state.clone();
        let fallback_image = state.config.metis.worker_image.clone();
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
        let resolved = task.resolve(service_state.as_ref(), &fallback_image)?;
        assert_eq!(
            resolved.env_vars.get(ENV_GH_TOKEN),
            Some(&"token-123".to_string())
        );
        assert_eq!(resolved.image, "ghcr.io/example/override:main");

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
        let Task { env_vars, .. } = task;
        assert_eq!(env_vars.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(env_vars.get("PROMPT"), Some(&"custom prompt".to_string()));

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
        let service_state = state.service_state.clone();
        let fallback_image = state.config.metis.worker_image.clone();
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
        let resolved = task.resolve(service_state.as_ref(), &fallback_image)?;
        assert_eq!(
            resolved.env_vars.get(ENV_GH_TOKEN),
            Some(&"user-supplied".to_string())
        );
        assert_eq!(
            resolved.env_vars.get("PROMPT"),
            None,
            "server should not inject prompt automatically"
        );
        assert_eq!(resolved.image, "ghcr.io/example/repo:main");

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

        let oldest_id = task_id("t-oldest");
        let middle_id = task_id("t-middle");
        let newest_id = task_id("t-newest");
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    oldest_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    now - Duration::seconds(30),
                )
                .await?;
            store_write
                .add_task_with_id(
                    middle_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    now - Duration::seconds(20),
                )
                .await?;
            store_write
                .add_task_with_id(
                    newest_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
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
        let ids: Vec<TaskId> = body.jobs.into_iter().map(|job| job.id).collect();
        assert_eq!(ids, vec![newest_id, middle_id, oldest_id]);
        Ok(())
    }

    #[tokio::test]
    async fn get_job_returns_summary_for_existing_job() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;
        let job_id = task_id("t-jobab");
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
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
        let summary: JobRecord = response.json().await?;
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
        assert_eq!(
            body,
            json!({ "error": "id ' ' is missing a supported prefix" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_rejects_job_id_with_whitespace_padding() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;
        let job_id = task_id("t-trim");
        let now = Utc::now();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
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

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("id ' {job_id} ' is missing a supported prefix") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let missing_id = task_id("t-missing");
        let response = client
            .get(format!("{}/v1/jobs/{missing_id}", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("job '{missing_id}' not found") })
        );
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
        assert_eq!(
            body,
            json!({ "error": "id ' ' is missing a supported prefix" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_returns_bad_request_when_multiple_jobs_found() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        let job_id = task_id("t-jobaa");
        engine.insert_job(&job_id, JobStatus::Running).await;
        engine.insert_job(&job_id, JobStatus::Failed).await;
        let state = test_state_with_engine(engine);
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/{job_id}/logs", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("Multiple jobs found for metis-id '{job_id}'") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let missing_id = task_id("t-missing");
        let response = client
            .get(format!("{}/v1/jobs/{missing_id}/logs", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("Job '{missing_id}' not found") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_logs_streams_when_watching_running_job() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        let job_id = task_id("t-stream");
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
        assert_eq!(
            body,
            json!({ "error": "id ' ' is missing a supported prefix" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn kill_job_returns_not_found_for_unknown_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let missing_id = task_id("t-missing");
        let response = client
            .delete(format!("{}/v1/jobs/{missing_id}", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("Job '{missing_id}' not found") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn kill_job_handles_multiple_matches_conflict() -> anyhow::Result<()> {
        let engine = Arc::new(MockJobEngine::new());
        let job_id = task_id("t-dupe");
        engine.insert_job(&job_id, JobStatus::Running).await;
        engine.insert_job(&job_id, JobStatus::Running).await;
        let state = test_state_with_engine(engine);
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .delete(format!("{}/v1/jobs/{job_id}", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": format!("Multiple jobs found for metis-id '{job_id}'") })
        );
        Ok(())
    }

    #[tokio::test]
    async fn set_job_status_rejects_empty_job_id() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs/ /status", server.base_url()))
            .json(&json!({ "status": "complete" }))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "error": "id ' ' is missing a supported prefix" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn set_job_status_returns_not_found_for_missing_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let missing_id = task_id("t-missing");
        let response = client
            .post(format!("{}/v1/jobs/{missing_id}/status", server.base_url()))
            .json(&json!({ "status": "complete" }))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await?;
        assert!(body["error"].as_str().unwrap_or("").contains("not found"));
        Ok(())
    }

    #[tokio::test]
    async fn set_job_status_persists_result_for_spawn_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let patch_id;
        let job_id = task_id("t-spawn");
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            patch_id = store_write
                .add_patch(Patch {
                    title: "done".to_string(),
                    diff: "diff".to_string(),
                    description: "done".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                })
                .await?;
            store_write
                .emit_task_artifacts(&job_id, vec![patch_id.clone().into()], Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs/{job_id}/status", server.base_url()))
            .json(&json!({ "status": "complete" }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "job_id": job_id.as_ref(), "status": "complete" })
        );

        let store_read = store.read().await;
        let status = store_read.get_status(&job_id).await?;
        assert_eq!(status, Status::Complete);
        let result = store_read.get_result(&job_id);
        assert!(matches!(result, Some(Ok(()))));
        let status_log = store_read.get_status_log(&job_id).await?;
        assert_eq!(
            status_log.emitted_artifacts(),
            Some(vec![patch_id.clone().into()])
        );

        Ok(())
    }

    #[tokio::test]
    async fn set_job_status_records_last_message() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let job_id = task_id("t-lastmsg");
        {
            let mut store_write = state.store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
        }
        let server = spawn_test_server_with_state(state.clone()).await?;
        let client = test_client();

        let response = client
            .post(format!(
                "{}/v1/jobs/{}/status",
                server.base_url(),
                job_id.as_ref()
            ))
            .json(&json!({
                "status": "complete",
                "last_message": "all done"
            }))
            .send()
            .await?;

        assert!(response.status().is_success());

        let store_read = state.store.read().await;
        let status_log = store_read.get_status_log(&job_id).await?;
        match status_log.events.last() {
            Some(Event::Completed { last_message, .. }) => {
                assert_eq!(last_message.as_deref(), Some("all done"))
            }
            other => panic!("expected completed event, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn set_job_status_can_mark_failed() -> anyhow::Result<()> {
        let state = test_state();
        let job_id = task_id("t-fail");
        {
            let mut store_write = state.store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
        }
        let server = spawn_test_server_with_state(state.clone()).await?;
        let client = test_client();

        let response = client
            .post(format!("{}/v1/jobs/{job_id}/status", server.base_url()))
            .json(&json!({ "status": "failed", "reason": "boom" }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await?;
        assert_eq!(
            body,
            json!({ "job_id": job_id.as_ref(), "status": "failed" })
        );

        let store_read = state.store.read().await;
        let status = store_read.get_status(&job_id).await?;
        assert_eq!(status, Status::Failed);
        let result = store_read.get_result(&job_id);
        assert!(matches!(
            result,
            Some(Err(TaskError::JobEngineError { reason })) if reason == "boom"
        ));
        Ok(())
    }

    #[tokio::test]
    async fn get_job_status_returns_status_log() -> anyhow::Result<()> {
        let state = test_state();
        let job_id = task_id("t-status");
        {
            let mut store_write = state.store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            store_write
                .mark_task_complete(&job_id, Ok(()), None, Utc::now())
                .await?;
        }

        let server = spawn_test_server_with_state(state).await?;
        let client = test_client();

        let response = client
            .get(format!("{}/v1/jobs/{job_id}/status", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: GetJobStatusResponse = response.json().await?;
        assert_eq!(body.job_id, job_id);
        assert_eq!(body.status_log.current_status(), Status::Complete);
        assert!(matches!(
            body.status_log.events.last(),
            Some(Event::Completed { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn job_output_can_be_retrieved_via_events_and_patches() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let job_id = task_id("t-output");
        let patch_id;
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            patch_id = store_write
                .add_patch(Patch {
                    title: "all good".to_string(),
                    diff: "diff".to_string(),
                    description: "all good".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                })
                .await?;
            store_write
                .emit_task_artifacts(&job_id, vec![patch_id.clone().into()], Utc::now())
                .await?;
            store_write
                .mark_task_complete(&job_id, Ok(()), None, Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/{job_id}", server.base_url()))
            .send()
            .await?;

        assert!(response.status().is_success());
        let summary: JobRecord = response.json().await?;
        let emitted_ids = summary
            .status_log
            .events
            .iter()
            .find_map(|event| match event {
                Event::Emitted { artifact_ids, .. } => Some(artifact_ids.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(emitted_ids, vec![patch_id.clone().into()]);

        let patch_response = client
            .get(format!("{}/v1/patches/{patch_id}", server.base_url()))
            .send()
            .await?;
        assert!(patch_response.status().is_success());
        let patch_record: PatchRecord = patch_response.json().await?;
        assert_eq!(patch_record.id, patch_id);
        let Patch {
            title,
            diff,
            description,
            ..
        } = patch_record.patch;
        assert_eq!(title, "all good");
        assert_eq!(diff, "diff");
        assert_eq!(description, "all good");
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
        assert_eq!(
            body,
            json!({ "error": "id ' ' is missing a supported prefix" })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_returns_not_found_for_unknown_job() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let missing_id = task_id("t-missing");
        let response = client
            .get(format!(
                "{}/v1/jobs/{missing_id}/context",
                server.base_url()
            ))
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
        let context_spec = BundleSpec::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        };
        let parent_job_id = task_id("t-parentjob");
        let ctx_job_id = task_id("t-ctxjob");
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    parent_job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write
                .mark_task_running(&parent_job_id, Utc::now())
                .await?;
            let parent_patch_id = store_write
                .add_patch(Patch {
                    title: "done".to_string(),
                    diff: "patch-content".to_string(),
                    description: "done".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                })
                .await?;
            store_write
                .emit_task_artifacts(&parent_job_id, vec![parent_patch_id.into()], Utc::now())
                .await?;
            store_write
                .mark_task_complete(&parent_job_id, Ok(()), None, Utc::now())
                .await?;
            store_write
                .add_task_with_id(
                    ctx_job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: context_spec.clone(),
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!(
                "{}/v1/jobs/{ctx_job_id}/context",
                server.base_url()
            ))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: WorkerContext = response.json().await?;
        assert_eq!(
            body.request_context,
            Bundle::GitRepository {
                url: "https://example.com/repo.git".to_string(),
                rev: "main".to_string(),
            }
        );
        assert!(body.params.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_includes_task_variables() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let store = state.store.clone();
        let job_id = task_id("t-envjob");
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image.clone()),
                        env_vars: HashMap::from([(
                            "SECRET_VALUE".to_string(),
                            "keep-me-safe".to_string(),
                        )]),
                    },
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/{job_id}/context", server.base_url()))
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
    async fn patches_can_be_created_and_retrieved() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let patch = Patch {
            title: "Initial patch".to_string(),
            diff: "diff --git a/file b/file".to_string(),
            description: "initial patch".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
        };

        let response = client
            .post(format!("{}/v1/patches", server.base_url()))
            .json(&UpsertPatchRequest {
                patch: patch.clone(),
                job_id: None,
            })
            .send()
            .await?;

        assert!(response.status().is_success());
        let created: UpsertPatchResponse = response.json().await?;
        assert!(!created.patch_id.as_ref().is_empty());

        let fetched: PatchRecord = client
            .get(format!(
                "{}/v1/patches/{}",
                server.base_url(),
                created.patch_id
            ))
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(fetched.id, created.patch_id);
        assert_eq!(fetched.patch, patch);
        Ok(())
    }

    #[tokio::test]
    async fn creating_patch_with_job_id_emits_event() -> anyhow::Result<()> {
        let state = test_state();
        let default_image = default_image();
        let job_id = task_id("t-emit");
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    job_id.clone(),
                    Task {
                        program: "0".to_string(),
                        params: vec![],
                        context: BundleSpec::None,
                        spawned_from: None,
                        image: Some(default_image),
                        env_vars: HashMap::new(),
                    },
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
        }

        let server = spawn_test_server_with_state(state).await?;
        let client = test_client();
        let response = client
            .post(format!("{}/v1/patches", server.base_url()))
            .json(&UpsertPatchRequest {
                patch: Patch {
                    title: "artifact for emit".to_string(),
                    diff: "diff --git a/file b/file".to_string(),
                    description: "artifact for emit".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                },
                job_id: Some(job_id.clone()),
            })
            .send()
            .await?;

        assert!(response.status().is_success());
        let created: UpsertPatchResponse = response.json().await?;

        let emitted = {
            let store_read = store.read().await;
            store_read
                .get_status_log(&job_id)
                .await?
                .emitted_artifacts()
        };
        assert_eq!(emitted, Some(vec![created.patch_id.into()]));
        Ok(())
    }

    #[tokio::test]
    async fn list_endpoints_support_filters() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();

        let patch = Patch {
            title: "refactor logging".to_string(),
            diff: "refactor logging".to_string(),
            description: "refactor logging".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
        };
        let filtered_patch = Patch {
            title: "login retry patch".to_string(),
            diff: "add login retry handling".to_string(),
            description: "login retry patch".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
        };

        for patch in [patch.clone(), filtered_patch.clone()] {
            let response = client
                .post(format!("{}/v1/patches", server.base_url()))
                .json(&UpsertPatchRequest {
                    patch,
                    job_id: None,
                })
                .send()
                .await?;
            assert!(response.status().is_success());
        }

        let patch_results: ListPatchesResponse = client
            .get(format!("{}/v1/patches", server.base_url()))
            .query(&SearchPatchesQuery {
                q: Some("login".to_string()),
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(patch_results.patches.len(), 1);
        assert_eq!(patch_results.patches[0].patch, filtered_patch);

        let issue = Issue {
            issue_type: IssueType::Bug,
            description: "login fails for guests".to_string(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies: vec![],
            patches: Vec::new(),
        };
        let assigned_issue = Issue {
            issue_type: IssueType::Task,
            description: "assigned issue".to_string(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: Some("owner-1".to_string()),
            dependencies: vec![],
            patches: Vec::new(),
        };
        let closed_issue = Issue {
            issue_type: IssueType::Task,
            description: "retire old endpoint".to_string(),
            progress: String::new(),
            status: IssueStatus::Closed,
            assignee: None,
            dependencies: vec![],
            patches: Vec::new(),
        };

        for issue in [issue.clone(), assigned_issue.clone(), closed_issue.clone()] {
            let response = client
                .post(format!("{}/v1/issues", server.base_url()))
                .json(&UpsertIssueRequest {
                    issue,
                    job_id: None,
                })
                .send()
                .await?;
            assert!(response.status().is_success());
        }

        let filtered_issues: ListIssuesResponse = client
            .get(format!("{}/v1/issues", server.base_url()))
            .query(&SearchIssuesQuery {
                issue_type: Some(IssueType::Bug),
                status: None,
                assignee: None,
                q: None,
                graph_filters: Vec::new(),
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(filtered_issues.issues.len(), 1);
        assert_eq!(filtered_issues.issues[0].issue, issue);

        let filtered_by_assignee: ListIssuesResponse = client
            .get(format!("{}/v1/issues", server.base_url()))
            .query(&SearchIssuesQuery {
                issue_type: None,
                status: None,
                assignee: Some("OWNER-1".to_string()),
                q: None,
                graph_filters: Vec::new(),
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(filtered_by_assignee.issues.len(), 1);
        assert_eq!(filtered_by_assignee.issues[0].issue, assigned_issue);

        let filtered_by_status: ListIssuesResponse = client
            .get(format!("{}/v1/issues", server.base_url()))
            .query(&SearchIssuesQuery {
                issue_type: None,
                status: Some(IssueStatus::Closed),
                assignee: None,
                q: None,
                graph_filters: Vec::new(),
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(filtered_by_status.issues.len(), 1);
        assert_eq!(filtered_by_status.issues[0].issue, closed_issue);
        Ok(())
    }

    #[tokio::test]
    async fn update_issue_replaces_existing_value() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();

        let created: UpsertIssueResponse = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&UpsertIssueRequest {
                issue: Issue {
                    issue_type: IssueType::Task,
                    description: "original details".to_string(),
                    progress: "Initial progress".to_string(),
                    status: IssueStatus::Open,
                    assignee: None,
                    dependencies: vec![],
                    patches: Vec::new(),
                },
                job_id: None,
            })
            .send()
            .await?
            .json()
            .await?;

        let updated: UpsertIssueResponse = client
            .put(format!(
                "{}/v1/issues/{}",
                server.base_url(),
                created.issue_id
            ))
            .json(&UpsertIssueRequest {
                issue: Issue {
                    issue_type: IssueType::Task,
                    description: "updated details".to_string(),
                    progress: "Updated progress".to_string(),
                    status: IssueStatus::InProgress,
                    assignee: None,
                    dependencies: vec![],
                    patches: Vec::new(),
                },
                job_id: None,
            })
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(updated.issue_id, created.issue_id);

        let fetched: IssueRecord = client
            .get(format!(
                "{}/v1/issues/{}",
                server.base_url(),
                created.issue_id
            ))
            .send()
            .await?
            .json()
            .await?;

        assert_eq!(
            fetched.issue,
            Issue {
                issue_type: IssueType::Task,
                description: "updated details".to_string(),
                progress: "Updated progress".to_string(),
                status: IssueStatus::InProgress,
                assignee: None,
                dependencies: vec![],
                patches: Vec::new(),
            }
        );
        Ok(())
    }
}
