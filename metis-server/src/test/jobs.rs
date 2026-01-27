use super::common::{default_image, patch_diff, service_repo_name, service_repository, task_id};
use crate::domain::{
    issues::{Issue, IssueStatus, IssueType, JobSettings},
    jobs::{Bundle, BundleSpec, CreateJobResponse, JobRecord, ListJobsResponse, WorkerContext},
    patches::{Patch, PatchStatus},
    task_status::Event,
    users::Username,
};
use crate::{
    job_engine::JobStatus,
    store::{Status, Task, TaskError},
    test_utils::{
        MockJobEngine, add_repository, spawn_test_server, spawn_test_server_with_state,
        test_client, test_state_handles, test_state_with_engine_handles,
    },
};
use chrono::{Duration, Utc};
use metis_common::{TaskId, job_status::GetJobStatusResponse};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn create_job_enqueues_task() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    assert!(!body.job_id.as_ref().trim().is_empty());

    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    let Task {
        context, prompt, ..
    } = task;

    assert_eq!(prompt, "0");
    assert_eq!(context, BundleSpec::None);
    assert_eq!(resolved.context.bundle, Bundle::None);
    assert_eq!(resolved.image, resolver_state.config.job.default_image);

    let status = check_state.get_task_status(&body.job_id).await?;
    assert_eq!(status, Status::Pending);
    Ok(())
}

#[tokio::test]
async fn create_job_allows_service_repository_bundle() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "service_repository", "name": repo_name.to_string() }
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    let Task { context, .. } = task;
    assert_eq!(
        context,
        BundleSpec::ServiceRepository {
            name: repo_name.clone(),
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
    assert_eq!(resolved.image, "ghcr.io/example/repo:main");

    Ok(())
}

#[tokio::test]
async fn create_job_respects_image_override() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "image": "ghcr.io/example/custom:dev"
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(task.image, Some("ghcr.io/example/custom:dev".to_string()));
    assert_eq!(resolved.image, "ghcr.io/example/custom:dev");

    Ok(())
}

#[tokio::test]
async fn create_job_image_override_beats_repo_default() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "service_repository", "name": repo_name.to_string() },
            "image": "ghcr.io/example/override:main"
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(resolved.image, "ghcr.io/example/override:main");

    Ok(())
}

#[tokio::test]
async fn create_job_stores_provided_variables() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "variables": { "FOO": "bar", "PROMPT": "custom prompt" }
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    let task = check_state.get_task(&body.job_id).await?;
    let Task { env_vars, .. } = task;
    assert_eq!(env_vars.get("FOO"), Some(&"bar".to_string()));
    assert_eq!(env_vars.get("PROMPT"), Some(&"custom prompt".to_string()));

    Ok(())
}

#[tokio::test]
async fn job_settings_override_request_with_remote_url_priority() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();

    let job_settings = JobSettings {
        repo_name: Some(repo_name.clone()),
        remote_url: Some("https://override.example.com/repo.git".to_string()),
        image: Some("ghcr.io/example/issue:latest".to_string()),
        branch: Some("issue-branch".to_string()),
        max_retries: None,
        cpu_limit: Some("600m".to_string()),
        memory_limit: Some("512Mi".to_string()),
    };

    let issue_id = handles
        .store
        .add_issue(Issue {
            issue_type: IssueType::Task,
            description: "use overrides".to_string(),
            creator: Username::from("tester"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            job_settings: job_settings.clone(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
        })
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "git_repository", "url": "https://task.example.com/base.git", "rev": "task-branch" },
            "issue_id": issue_id
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;

    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(
        resolved.context.bundle,
        Bundle::GitRepository {
            url: "https://override.example.com/repo.git".to_string(),
            rev: "issue-branch".to_string(),
        }
    );
    assert_eq!(resolved.image, "ghcr.io/example/issue:latest");

    let context_response = client
        .get(format!(
            "{}/v1/jobs/{}/context",
            server.base_url(),
            body.job_id.as_ref()
        ))
        .send()
        .await?;
    assert!(context_response.status().is_success());
    let worker_context: WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        Bundle::GitRepository {
            url: "https://override.example.com/repo.git".to_string(),
            rev: "issue-branch".to_string(),
        }
    );
    Ok(())
}

#[tokio::test]
async fn job_settings_use_repo_name_and_branch_overrides() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();

    let job_settings = JobSettings {
        repo_name: Some(repo_name.clone()),
        remote_url: None,
        image: None,
        branch: Some("issue-branch".to_string()),
        max_retries: None,
        cpu_limit: None,
        memory_limit: None,
    };

    let issue_id = handles
        .store
        .add_issue(Issue {
            issue_type: IssueType::Task,
            description: "use repo override".to_string(),
            creator: Username::from("tester"),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            job_settings: job_settings.clone(),
            todo_list: Vec::new(),
            dependencies: Vec::new(),
            patches: Vec::new(),
        })
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "git_repository", "url": "https://task.example.com/base.git", "rev": "task-branch" },
            "issue_id": issue_id
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;

    let task = check_state.get_task(&body.job_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(
        resolved.context.bundle,
        Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "issue-branch".to_string(),
        }
    );
    assert_eq!(resolved.image, "ghcr.io/example/repo:main");

    let context_response = client
        .get(format!(
            "{}/v1/jobs/{}/context",
            server.base_url(),
            body.job_id.as_ref()
        ))
        .send()
        .await?;
    assert!(context_response.status().is_success());
    let worker_context: WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "issue-branch".to_string(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn create_job_rejects_unknown_service_repository() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "service_repository", "name": "missing/repo" }
        }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": "unknown repository 'missing/repo'" })
    );
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
    let handles = test_state_with_engine_handles(engine);
    let default_image = default_image();
    let server = spawn_test_server_with_state(handles.state, handles.store.clone()).await?;

    let oldest_id = task_id("t-oldest");
    let middle_id = task_id("t-middle");
    let newest_id = task_id("t-newest");
    let now = Utc::now();
    handles
        .store
        .add_task_with_id(
            oldest_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            now - Duration::seconds(30),
        )
        .await?;
    handles
        .store
        .add_task_with_id(
            middle_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            now - Duration::seconds(20),
        )
        .await?;
    handles
        .store
        .add_task_with_id(
            newest_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            now - Duration::seconds(10),
        )
        .await?;
    handles
        .store
        .mark_task_running(&middle_id, now - Duration::seconds(15))
        .await?;
    handles
        .store
        .mark_task_running(&newest_id, now - Duration::seconds(5))
        .await?;

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
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let job_id = task_id("t-jobab");
    let now = Utc::now();
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            now - Duration::seconds(20),
        )
        .await?;
    handles
        .store
        .mark_task_running(&job_id, now - Duration::seconds(10))
        .await?;

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
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let job_id = task_id("t-trim");
    let now = Utc::now();
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            now - Duration::seconds(30),
        )
        .await?;
    handles
        .store
        .mark_task_running(&job_id, now - Duration::seconds(10))
        .await?;

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
    let handles = test_state_with_engine_handles(engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;

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
    let handles = test_state_with_engine_handles(engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;

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
    let handles = test_state_with_engine_handles(engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;

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
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let check_state = state.clone();
    let job_id = task_id("t-spawn");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles.store.mark_task_running(&job_id, Utc::now()).await?;
    let patch_id = handles
        .store
        .add_patch(Patch {
            title: "done".to_string(),
            description: "done".to_string(),
            diff: patch_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: Some(job_id.clone()),
            reviews: Vec::new(),
            service_repo_name: service_repo_name(),
            github: None,
        })
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

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

    let status = check_state.get_task_status(&job_id).await?;
    assert_eq!(status, Status::Complete);
    let status_log = check_state.get_status_log(&job_id).await?;
    assert!(matches!(status_log.result(), Some(Ok(()))));
    let patch = check_state.get_patch(&patch_id).await?;
    assert_eq!(patch.item.created_by, Some(job_id));

    Ok(())
}

#[tokio::test]
async fn set_job_status_records_last_message() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let job_id = task_id("t-lastmsg");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles.store.mark_task_running(&job_id, Utc::now()).await?;
    let server = spawn_test_server_with_state(state.clone(), handles.store.clone()).await?;
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

    let status_log = state.get_status_log(&job_id).await?;
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
    let handles = test_state_handles();
    let state = handles.state;
    let job_id = task_id("t-fail");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles.store.mark_task_running(&job_id, Utc::now()).await?;
    let server = spawn_test_server_with_state(state.clone(), handles.store.clone()).await?;
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

    let status = state.get_task_status(&job_id).await?;
    assert_eq!(status, Status::Failed);
    let status_log = state.get_status_log(&job_id).await?;
    assert!(matches!(
        status_log.result(),
        Some(Err(TaskError::JobEngineError { reason })) if reason == "boom"
    ));
    Ok(())
}

#[tokio::test]
async fn get_job_status_returns_status_log() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let job_id = task_id("t-status");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles.store.mark_task_running(&job_id, Utc::now()).await?;
    handles
        .store
        .mark_task_complete(&job_id, Ok(()), None, Utc::now())
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/jobs/{job_id}/status", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: GetJobStatusResponse = response.json().await?;
    assert_eq!(body.job_id, job_id);
    let status_log: crate::domain::task_status::TaskStatusLog = body.status_log.into();
    assert_eq!(status_log.current_status(), Status::Complete);
    assert!(matches!(
        status_log.events.last(),
        Some(Event::Completed { .. })
    ));
    Ok(())
}

#[tokio::test]
async fn job_output_can_be_retrieved_via_patches() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let job_id = task_id("t-output");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles.store.mark_task_running(&job_id, Utc::now()).await?;
    let patch_id = handles
        .store
        .add_patch(Patch {
            title: "all good".to_string(),
            description: "all good".to_string(),
            diff: patch_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: Some(job_id.clone()),
            reviews: Vec::new(),
            service_repo_name: service_repo_name(),
            github: None,
        })
        .await?;
    handles
        .store
        .mark_task_complete(&job_id, Ok(()), None, Utc::now())
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!("{}/v1/jobs/{job_id}", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let summary: JobRecord = response.json().await?;
    assert_eq!(summary.status_log.current_status(), Status::Complete);

    let patch_response = client
        .get(format!("{}/v1/patches/{patch_id}", server.base_url()))
        .send()
        .await?;
    assert!(patch_response.status().is_success());
    let patch_record: metis_common::patches::PatchRecord = patch_response.json().await?;
    assert_eq!(patch_record.id, patch_id);
    let metis_common::patches::Patch {
        title,
        description,
        diff,
        ..
    } = patch_record.patch;
    assert_eq!(title, "all good");
    assert_eq!(description, "all good");
    assert_eq!(diff, patch_diff());
    assert_eq!(patch_record.patch.created_by, Some(job_id));
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
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let context_spec = BundleSpec::GitRepository {
        url: "https://example.com/repo.git".to_string(),
        rev: "main".to_string(),
    };
    let parent_job_id = task_id("t-parentjob");
    let ctx_job_id = task_id("t-ctxjob");
    handles
        .store
        .add_task_with_id(
            parent_job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    handles
        .store
        .mark_task_running(&parent_job_id, Utc::now())
        .await?;
    let _parent_patch_id = handles
        .store
        .add_patch(Patch {
            title: "done".to_string(),
            description: "done".to_string(),
            diff: patch_diff(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            created_by: Some(parent_job_id.clone()),
            reviews: Vec::new(),
            service_repo_name: service_repo_name(),
            github: None,
        })
        .await?;
    handles
        .store
        .mark_task_complete(&parent_job_id, Ok(()), None, Utc::now())
        .await?;
    handles
        .store
        .add_task_with_id(
            ctx_job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: context_spec.clone(),
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

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
    assert_eq!(body.prompt, "0");
    Ok(())
}

#[tokio::test]
async fn get_job_context_includes_task_variables() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let job_id = task_id("t-envjob");
    handles
        .store
        .add_task_with_id(
            job_id.clone(),
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                image: Some(default_image.clone()),
                env_vars: HashMap::from([("SECRET_VALUE".to_string(), "keep-me-safe".to_string())]),
                cpu_limit: None,
                memory_limit: None,
            },
            Utc::now(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

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
