#![allow(clippy::too_many_arguments)]

mod config;
mod job_engine;
mod routes;
mod store;
#[cfg(test)]
mod test;

use crate::config::{AppConfig, build_kube_client};
use crate::job_engine::{JobEngine, KubernetesJobEngine};
use crate::store::{MemoryStore, Status, Store, Task};
use axum::{
    Json, Router,
    routing::{get, post},
};
use chrono::Utc;
use serde_json::json;
use std::{collections::HashSet, env, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
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
        .route("/v1/jobs", post(routes::jobs::create_job))
        .route(
            "/v1/jobs/:job_id/logs",
            get(routes::jobs::logs::get_job_logs),
        )
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
            match store.list_tasks_with_status(Status::Pending).await {
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
                    match store.mark_task_running(&metis_id, Utc::now()).await {
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
                    let failure_reason = format!("Failed to create Kubernetes job: {err}");
                    if let Err(update_err) = store
                        .mark_task_failed(&metis_id, failure_reason, Utc::now())
                        .await
                    {
                        error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed");
                    } else {
                        info!(metis_id = %metis_id, "set task status to Failed");
                    }
                }
            }
        }
    }
}

/// Background task that periodically monitors running jobs.
///
/// This function runs in a loop, checking for running tasks every few seconds
/// and updating their status based on the job engine state:
/// 1. Gets all running tasks from the store
/// 2. Checks each job's status in the job engine
/// 3. Updates the store status to Complete or Failed if the job has finished
async fn monitor_running_jobs(state: AppState) {
    loop {
        // Check every 5 seconds
        sleep(Duration::from_secs(5)).await;

        // Kill any jobs that are running in the engine but missing from the store
        let job_engine_jobs = match state.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                Vec::new()
            }
        };

        if !job_engine_jobs.is_empty() {
            let store_task_ids = {
                let store = state.store.read().await;
                match store.list_tasks().await {
                    Ok(ids) => Some(ids),
                    Err(err) => {
                        error!(error = %err, "failed to list tasks from store for job reconciliation");
                        None
                    }
                }
            };

            if let Some(store_task_ids) = store_task_ids {
                let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
                let orphaned_jobs: Vec<_> = job_engine_jobs
                    .into_iter()
                    .filter(|job| !store_task_set.contains(&job.id))
                    .collect();

                if !orphaned_jobs.is_empty() {
                    info!(
                        count = orphaned_jobs.len(),
                        "killing jobs present in engine but missing from store"
                    );
                }

                for job in orphaned_jobs {
                    match state.job_engine.kill_job(&job.id).await {
                        Ok(()) => {
                            info!(metis_id = %job.id, "killed job not present in store");
                        }
                        Err(err) => {
                            warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
                        }
                    }
                }
            }
        }

        // Get running tasks
        let running_ids = {
            let store = state.store.read().await;
            match store.list_tasks_with_status(Status::Running).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list running tasks");
                    continue;
                }
            }
        };

        if running_ids.is_empty() {
            continue;
        }

        info!(count = running_ids.len(), "found running tasks to monitor");

        // Check each running job's status
        for metis_id in running_ids {
            match state.job_engine.find_job_by_metis_id(&metis_id).await {
                Ok(job) => {
                    match job.status {
                        crate::job_engine::JobStatus::Complete => {
                            // Update status in store
                            let mut store = state.store.write().await;
                            let end_time = job.completion_time.unwrap_or_else(|| Utc::now());
                            match store.mark_task_complete(&metis_id, end_time).await {
                                Ok(()) => {
                                    info!(metis_id = %metis_id, "updated task status to Complete from job engine");
                                }
                                Err(err) => {
                                    warn!(metis_id = %metis_id, error = %err, "failed to update task status to Complete");
                                }
                            }
                        }
                        crate::job_engine::JobStatus::Failed => {
                            // Update status in store
                            let mut store = state.store.write().await;
                            let end_time = job.completion_time.unwrap_or_else(|| Utc::now());
                            let failure_reason = job.failure_message.unwrap_or_else(|| {
                                "Job failed for an undetermined reason".to_string()
                            });
                            match store
                                .mark_task_failed(&metis_id, failure_reason, end_time)
                                .await
                            {
                                Ok(()) => {
                                    info!(metis_id = %metis_id, "updated task status to Failed from job engine");
                                }
                                Err(err) => {
                                    warn!(metis_id = %metis_id, error = %err, "failed to update task status to Failed");
                                }
                            }
                        }
                        crate::job_engine::JobStatus::Running => {
                            // Still running, skip
                            continue;
                        }
                    }
                }
                Err(crate::job_engine::JobEngineError::NotFound(_)) => {
                    // Job not found in Kubernetes - might have been deleted or never created
                    // This could happen if the job was cleaned up externally
                    warn!(metis_id = %metis_id, "job not found in job engine, marking as failed");
                    let mut store = state.store.write().await;
                    let failure_reason = "Job not found in job engine".to_string();
                    if let Err(update_err) = store
                        .mark_task_failed(&metis_id, failure_reason, Utc::now())
                        .await
                    {
                        error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed");
                    }
                }
                Err(err) => {
                    error!(metis_id = %metis_id, error = %err, "failed to check job status in job engine");
                    // Don't update status on transient errors
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        job_engine::{JobStatus, MockJobEngine},
        store::{Status, Task},
        test::{
            spawn_test_server, spawn_test_server_with_state, test_client, test_state,
            test_state_with_engine,
        },
    };
    use chrono::{Duration, Utc};
    use metis_common::{
        job_outputs::{JobOutputPayload, JobOutputType},
        jobs::{CreateJobRequestContext, CreateJobResponse, ListJobsResponse, WorkerContext},
    };
    use serde_json::json;
    use std::sync::Arc;

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
    async fn create_job_rejects_empty_prompt() -> anyhow::Result<()> {
        let server = spawn_test_server().await?;
        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({ "prompt": "   " }))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "prompt is required" }));
        Ok(())
    }

    #[tokio::test]
    async fn create_job_trims_prompt_and_enqueues_task() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({ "prompt": "  run tests  " }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        assert!(!body.job_id.trim().is_empty());

        let store_read = store.read().await;
        let task = store_read.get_task(&body.job_id).await?;
        match task {
            Task::Spawn {
                prompt,
                context,
                output_type,
                result,
            } => {
                assert_eq!(prompt, "run tests");
                assert_eq!(context, CreateJobRequestContext::None);
                assert_eq!(output_type, JobOutputType::Patch);
                assert!(result.is_none());
            }
            Task::Ask => panic!("expected spawn task"),
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
                        prompt: "parent task".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }

        let client = test_client();
        let response = client
            .post(format!("{}/v1/jobs", server.base_url()))
            .json(&json!({ "prompt": "child task", "parent_ids": ["parent-1"] }))
            .send()
            .await?;

        assert!(response.status().is_success());
        let body: CreateJobResponse = response.json().await?;
        assert!(!body.job_id.trim().is_empty());

        let store_read = store.read().await;
        let parents = store_read.get_parents(&body.job_id).await?;
        assert_eq!(parents, vec!["parent-1".to_string()]);
        let status = store_read.get_status(&body.job_id).await?;
        assert_eq!(status, Status::Blocked);
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
                        prompt: "old".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
                    },
                    vec![],
                    now - Duration::seconds(30),
                )
                .await?;
            store_write
                .add_task_with_id(
                    middle_id.clone(),
                    Task::Spawn {
                        prompt: "mid".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
                    },
                    vec![],
                    now - Duration::seconds(20),
                )
                .await?;
            store_write
                .add_task_with_id(
                    newest_id.clone(),
                    Task::Spawn {
                        prompt: "new".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
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
            .post(format!("{}/v1/jobs/ /kill", server.base_url()))
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
            .post(format!("{}/v1/jobs/unknown/kill", server.base_url()))
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
            .post(format!("{}/v1/jobs/dupe/kill", server.base_url()))
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
    async fn set_job_output_rejects_ask_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id("ask-job".to_string(), Task::Ask, vec![], Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let payload = JobOutputPayload {
            last_message: "msg".to_string(),
            patch: "diff".to_string(),
        };
        let response = client
            .post(format!("{}/v1/jobs/ask-job/output", server.base_url()))
            .json(&payload)
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "Cannot set output on Ask task" }));
        Ok(())
    }

    #[tokio::test]
    async fn set_job_output_persists_result_for_spawn_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "spawn-job".to_string(),
                    Task::Spawn {
                        prompt: "do work".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let payload = JobOutputPayload {
            last_message: "done".to_string(),
            patch: "diff".to_string(),
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
                "output_type": "patch",
                "output": { "last_message": "done", "patch": "diff" }
            })
        );

        let store_read = store.read().await;
        let task = store_read.get_task(&"spawn-job".to_string()).await?;
        match task {
            Task::Spawn { result, .. } => assert_eq!(
                result,
                Some(JobOutputPayload {
                    last_message: "done".to_string(),
                    patch: "diff".to_string()
                })
            ),
            Task::Ask => panic!("expected spawn task"),
        }
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
        };
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "with-output".to_string(),
                    Task::Spawn {
                        prompt: "do work".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: Some(payload.clone()),
                    },
                    vec![],
                    Utc::now(),
                )
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
                "output_type": "patch",
                "output": { "last_message": "all good", "patch": "diff" }
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
            store_write
                .add_task_with_id(
                    "no-output".to_string(),
                    Task::Spawn {
                        prompt: "do work".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: None,
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
        let context = CreateJobRequestContext::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        };
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id(
                    "parent-job".to_string(),
                    Task::Spawn {
                        prompt: "prepare".to_string(),
                        context: CreateJobRequestContext::None,
                        output_type: JobOutputType::Patch,
                        result: Some(JobOutputPayload {
                            last_message: "done".to_string(),
                            patch: "patch-content".to_string(),
                        }),
                    },
                    vec![],
                    Utc::now(),
                )
                .await?;
            store_write
                .add_task_with_id(
                    "ctx-job".to_string(),
                    Task::Spawn {
                        prompt: "do work".to_string(),
                        context: context.clone(),
                        output_type: JobOutputType::Patch,
                        result: None,
                    },
                    vec!["parent-job".to_string()],
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
        assert_eq!(body.parents.len(), 1);
        assert_eq!(body.output_type, JobOutputType::Patch);
        assert_eq!(
            body.parents.get("parent-job"),
            Some(&JobOutputPayload {
                last_message: "done".to_string(),
                patch: "patch-content".to_string()
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn get_job_context_rejects_ask_tasks() -> anyhow::Result<()> {
        let state = test_state();
        let store = state.store.clone();
        {
            let mut store_write = store.write().await;
            store_write
                .add_task_with_id("ask-context".to_string(), Task::Ask, vec![], Utc::now())
                .await?;
        }
        let server = spawn_test_server_with_state(state).await?;

        let client = test_client();
        let response = client
            .get(format!("{}/v1/jobs/ask-context/context", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await?;
        assert_eq!(body, json!({ "error": "Ask tasks do not have context" }));
        Ok(())
    }
}
