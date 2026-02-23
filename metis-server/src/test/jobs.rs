use super::common::{default_image, patch_diff, service_repo_name, service_repository, task_id};
use crate::app::{AppState, ServiceState};
use crate::config::BuildCacheSection;
use crate::domain::{
    actors::ActorRef,
    issues::{Issue, IssueStatus, IssueType, JobSettings},
    jobs::{Bundle, BundleSpec},
    patches::{Patch, PatchStatus},
    users::Username,
};
use crate::{
    background::AgentQueue,
    job_engine::JobStatus,
    store::{MemoryStore, Status, Task},
    test_utils::{
        MockJobEngine, add_repository, spawn_test_server, spawn_test_server_with_state,
        test_app_config, test_client, test_state_handles, test_state_with_engine_handles,
    },
};
use chrono::{Duration, Utc};
use metis_common::{
    BuildCacheStorageConfig,
    api::v1::{
        self,
        jobs::{CreateJobResponse, JobVersionRecord, ListJobVersionsResponse, ListJobsResponse},
    },
};
use reqwest::StatusCode;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

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

    let status = check_state.get_task(&body.job_id).await?.status;
    assert_eq!(status, Status::Created);
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
        model: None,
        branch: Some("issue-branch".to_string()),
        max_retries: None,
        cpu_limit: Some("600m".to_string()),
        memory_limit: Some("512Mi".to_string()),
        secrets: None,
    };

    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue {
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
                deleted: false,
                creation_timestamp: None,
            },
            &ActorRef::test(),
        )
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
    let worker_context: v1::jobs::WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        v1::jobs::Bundle::GitRepository {
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
        model: None,
        branch: Some("issue-branch".to_string()),
        max_retries: None,
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
    };

    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue {
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
                deleted: false,
                creation_timestamp: None,
            },
            &ActorRef::test(),
        )
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
    let worker_context: v1::jobs::WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        v1::jobs::Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "issue-branch".to_string(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn job_context_includes_build_cache_settings() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.build_cache = BuildCacheSection {
        storage: Some(BuildCacheStorageConfig::FileSystem {
            root_dir: "/tmp/metis-build-cache".to_string(),
        }),
        include: Vec::new(),
        exclude: Vec::new(),
        home_include: Vec::new(),
        home_exclude: Vec::new(),
        max_entries_per_repo: Some(5),
    };

    let store = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        Arc::new(RwLock::new(Vec::<Arc<AgentQueue>>::new())),
    );
    let server = spawn_test_server_with_state(state, store).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateJobResponse = response.json().await?;
    let context_response = client
        .get(format!(
            "{}/v1/jobs/{}/context",
            server.base_url(),
            body.job_id.as_ref()
        ))
        .send()
        .await?;

    assert!(context_response.status().is_success());
    let worker_context: v1::jobs::WorkerContext = context_response.json().await?;
    let build_cache = worker_context.build_cache.expect("build cache");
    assert_eq!(
        build_cache.storage,
        BuildCacheStorageConfig::FileSystem {
            root_dir: "/tmp/metis-build-cache".to_string(),
        }
    );
    assert_eq!(build_cache.settings.max_entries_per_repo, Some(5));
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
        .get(format!("{}/v1/jobs", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: ListJobsResponse = response.json().await?;
    assert!(body.jobs.is_empty());
    Ok(())
}

#[tokio::test]
async fn job_versions_endpoints_return_history() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state.clone();
    let server = spawn_test_server_with_state(state.clone(), handles.store).await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: CreateJobResponse = response.json().await?;

    state
        .transition_task_to_pending(&created.job_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&created.job_id, ActorRef::test())
        .await?;

    let response = client
        .post(format!(
            "{}/v1/jobs/{}/status",
            server.base_url(),
            created.job_id
        ))
        .json(&json!({ "status": "complete" }))
        .send()
        .await?;

    assert!(response.status().is_success());

    let versions: ListJobVersionsResponse = client
        .get(format!(
            "{}/v1/jobs/{}/versions",
            server.base_url(),
            created.job_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 4);
    assert_eq!(versions.versions[0].job_id, created.job_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(
        versions.versions[0].task.status,
        v1::task_status::Status::Created
    );
    assert_eq!(versions.versions[1].job_id, created.job_id);
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(
        versions.versions[1].task.status,
        v1::task_status::Status::Pending
    );
    assert_eq!(versions.versions[2].job_id, created.job_id);
    assert_eq!(versions.versions[2].version, 3);
    assert_eq!(
        versions.versions[2].task.status,
        v1::task_status::Status::Running
    );
    assert_eq!(versions.versions[3].job_id, created.job_id);
    assert_eq!(versions.versions[3].version, 4);
    assert_eq!(
        versions.versions[3].task.status,
        v1::task_status::Status::Complete
    );

    let version: JobVersionRecord = client
        .get(format!(
            "{}/v1/jobs/{}/versions/4",
            server.base_url(),
            created.job_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(version.version, 4);
    assert_eq!(version.job_id, created.job_id);
    assert_eq!(version.task.status, v1::task_status::Status::Complete);

    Ok(())
}

#[tokio::test]
async fn job_version_endpoints_return_404s() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let missing_id = task_id("t-missing");
    let response = client
        .get(format!(
            "{}/v1/jobs/{}/versions",
            server.base_url(),
            missing_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = client
        .post(format!("{}/v1/jobs", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: CreateJobResponse = response.json().await?;

    let response = client
        .get(format!(
            "{}/v1/jobs/{}/versions/99",
            server.base_url(),
            created.job_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

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
    let server = spawn_test_server_with_state(state.clone(), handles.store.clone()).await?;
    let now = Utc::now();
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image.clone()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            now - Duration::seconds(30),
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_pending(&job_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&job_id, ActorRef::test())
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
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image.clone()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_pending(&job_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&job_id, ActorRef::test())
        .await?;
    let (_patch_id, _) = handles
        .store
        .add_patch(
            Patch {
                title: "done".to_string(),
                description: "done".to_string(),
                diff: patch_diff(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                created_by: Some(job_id.clone()),
                creator: Username::from("test-creator"),
                reviews: Vec::new(),
                service_repo_name: service_repo_name(),
                github: None,
                deleted: false,
                branch_name: None,
                commit_range: None,
                base_branch: None,
                creation_timestamp: None,
            },
            &ActorRef::test(),
        )
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
    Ok(())
}

#[tokio::test]
async fn set_job_status_can_mark_failed() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_pending(&job_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&job_id, ActorRef::test())
        .await?;
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
    let (parent_job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image.clone()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_pending(&parent_job_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&parent_job_id, ActorRef::test())
        .await?;
    let (_parent_patch_id, _) = handles
        .store
        .add_patch(
            Patch {
                title: "done".to_string(),
                description: "done".to_string(),
                diff: patch_diff(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                created_by: Some(parent_job_id.clone()),
                creator: Username::from("test-creator"),
                reviews: Vec::new(),
                service_repo_name: service_repo_name(),
                github: None,
                deleted: false,
                branch_name: None,
                commit_range: None,
                base_branch: None,
                creation_timestamp: None,
            },
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_completion(&parent_job_id, Ok(()), None, ActorRef::test())
        .await?;
    let (ctx_job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: context_spec.clone(),
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image.clone()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
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
    let body: v1::jobs::WorkerContext = response.json().await?;
    assert_eq!(
        body.request_context,
        v1::jobs::Bundle::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        }
    );
    assert_eq!(body.prompt, "0");
    Ok(())
}

#[tokio::test]
async fn get_job_context_includes_model_from_task() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image),
                model: Some("claude-3-5-sonnet".to_string()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!("{}/v1/jobs/{job_id}/context", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: v1::jobs::WorkerContext = response.json().await?;
    assert_eq!(body.model.as_deref(), Some("claude-3-5-sonnet"));
    Ok(())
}

#[tokio::test]
async fn get_job_context_includes_task_variables() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_task(
            Task {
                prompt: "0".to_string(),
                context: BundleSpec::None,
                spawned_from: None,
                creator: Username::from("test-creator"),
                image: Some(default_image.clone()),
                model: None,
                env_vars: HashMap::from([("SECRET_VALUE".to_string(), "keep-me-safe".to_string())]),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!("{}/v1/jobs/{job_id}/context", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: v1::jobs::WorkerContext = response.json().await?;
    assert_eq!(
        body.variables.get("SECRET_VALUE").map(String::as_str),
        Some("keep-me-safe")
    );
    assert_eq!(
        body.variables.get("METIS_ID").map(String::as_str),
        Some(job_id.as_ref())
    );
    Ok(())
}
