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
    let openai_api_key = env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| app_config.metis.openai_api_key.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "OPENAI_API_KEY is not set. Provide it via the environment or config.toml."
            )
        })?;

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
    std::env::var("METIS_CONFIG")
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
        job_outputs::JobOutputPayload,
        jobs::{
            Bundle, CreateJobResponse, JobSummary, ListJobsResponse, ParentContext, WorkerContext,
        },
    };
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

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
                env_vars: _,
            } => {
                assert_eq!(program, "0");
                assert!(params.is_empty());
                assert_eq!(context, Bundle::None);
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
            .json(&json!({ "parent_ids": ["parent-1"] }))
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
                context, env_vars, ..
            } => {
                assert_eq!(
                    context,
                    Bundle::GitRepository {
                        url: repo.remote_url.clone(),
                        rev: "develop".to_string()
                    }
                );
                assert_eq!(env_vars.get("GH_TOKEN"), Some(&"token-123".to_string()));
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
                "prompt": "set variables",
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
                "context": { "type": "service_repository", "name": "private-repo" },
                "variables": { "GH_TOKEN": "user-supplied" }
            }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn { env_vars, .. } => {
                assert_eq!(env_vars.get("GH_TOKEN"), Some(&"user-supplied".to_string()));
                assert_eq!(
                    env_vars.get("PROMPT"),
                    None,
                    "server should not inject prompt automatically"
                );
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
        assert_eq!(summary.status_log.current_status, Status::Running);
        assert_eq!(
            summary.status_log.start_time,
            Some(now - Duration::seconds(10))
        );
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
        assert_eq!(body, json!({ "error": "job-1" }));
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
        assert_eq!(body, json!({ "error": "missing" }));
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
        assert_eq!(body, json!({ "error": "job_id is required" }));
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
        assert_eq!(body, json!({ "error": "unknown" }));
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
        assert_eq!(body, json!({ "error": "dupe" }));
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
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
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
        let store = state.store.clone();
        let payload = JobOutputPayload {
            last_message: "all good".to_string(),
            patch: "diff".to_string(),
            bundle: Bundle::None,
        };
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
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write.mark_task_running(&job_id, Utc::now()).await?;
            store_write
                .mark_task_complete(&job_id, Ok(payload.clone()), Utc::now())
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
        let store = state.store.clone();
        let context = Bundle::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        };
        {
            let mut store_write = store.write().await;
            let parent_output = JobOutputPayload {
                last_message: "done".to_string(),
                patch: "patch-content".to_string(),
                bundle: Bundle::None,
            };
            store_write
                .add_task_with_id(
                    "parent-job".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: Bundle::None,
                        env_vars: HashMap::new(),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write
                .mark_task_running(&"parent-job".to_string(), Utc::now())
                .await?;
            store_write
                .mark_task_complete(
                    &"parent-job".to_string(),
                    Ok(parent_output.clone()),
                    Utc::now(),
                )
                .await?;
            store_write
                .add_task_with_id(
                    "ctx-job".to_string(),
                    Task::Spawn {
                        program: "0".to_string(),
                        params: vec![],
                        context: context.clone(),
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
}
