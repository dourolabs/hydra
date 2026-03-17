use super::common::{default_image, patch_diff, service_repo_name, service_repository, task_id};
use crate::app::{AppState, ServiceState};
use crate::config::BuildCacheSection;
use crate::domain::{
    actors::ActorRef,
    issues::{Issue, IssueStatus, IssueType, SessionSettings},
    patches::{Patch, PatchStatus},
    sessions::{Bundle, BundleSpec},
    users::Username,
};
use crate::{
    job_engine::JobStatus,
    store::{MemoryStore, Session, Status},
    test_utils::{
        MockJobEngine, add_repository, spawn_test_server, spawn_test_server_with_state,
        test_app_config, test_client, test_secret_manager, test_state_handles,
        test_state_with_engine_handles,
    },
};
use chrono::{Duration, Utc};
use hydra_common::{
    BuildCacheStorageConfig,
    api::v1::{
        self,
        sessions::{
            CreateSessionResponse, ListSessionVersionsResponse, ListSessionsResponse,
            SessionVersionRecord,
        },
    },
};
use reqwest::StatusCode;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn create_session_enqueues_task() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    assert!(!body.session_id.as_ref().trim().is_empty());

    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    let Session {
        context, prompt, ..
    } = task;

    assert_eq!(prompt, "0");
    assert_eq!(context, BundleSpec::None);
    assert_eq!(resolved.context.bundle, Bundle::None);
    assert_eq!(resolved.image, resolver_state.config.job.default_image);

    let status = check_state.get_session(&body.session_id).await?.status;
    assert_eq!(status, Status::Created);
    Ok(())
}

#[tokio::test]
async fn create_session_allows_service_repository_bundle() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "service_repository", "name": repo_name.to_string() }
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    let Session { context, .. } = task;
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
async fn create_session_respects_image_override() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "image": "ghcr.io/example/custom:dev"
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(task.image, Some("ghcr.io/example/custom:dev".to_string()));
    assert_eq!(resolved.image, "ghcr.io/example/custom:dev");

    Ok(())
}

#[tokio::test]
async fn create_session_image_override_beats_repo_default() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "service_repository", "name": repo_name.to_string() },
            "image": "ghcr.io/example/override:main"
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;
    assert_eq!(resolved.image, "ghcr.io/example/override:main");

    Ok(())
}

#[tokio::test]
async fn create_session_stores_provided_variables() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let check_state = state.clone();
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "variables": { "FOO": "bar", "PROMPT": "custom prompt" }
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let task = check_state.get_session(&body.session_id).await?;
    let Session { env_vars, .. } = task;
    assert_eq!(env_vars.get("FOO"), Some(&"bar".to_string()));
    assert_eq!(env_vars.get("PROMPT"), Some(&"custom prompt".to_string()));

    Ok(())
}

#[tokio::test]
async fn session_settings_override_request_with_remote_url_priority() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();

    let session_settings = SessionSettings {
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
                title: String::new(),
                description: "use overrides".to_string(),
                creator: Username::from("tester"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                session_settings: session_settings.clone(),
                todo_list: Vec::new(),
                dependencies: Vec::new(),
                patches: Vec::new(),
                deleted: false,
            },
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "git_repository", "url": "https://task.example.com/base.git", "rev": "task-branch" },
            "issue_id": issue_id
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;

    let task = check_state.get_session(&body.session_id).await?;
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
            "{}/v1/sessions/{}/context",
            server.base_url(),
            body.session_id.as_ref()
        ))
        .send()
        .await?;
    assert!(context_response.status().is_success());
    let worker_context: v1::sessions::WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        v1::sessions::Bundle::GitRepository {
            url: "https://override.example.com/repo.git".to_string(),
            rev: "issue-branch".to_string(),
        }
    );
    Ok(())
}

#[tokio::test]
async fn session_settings_use_repo_name_and_branch_overrides() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;
    let resolver_state = state.clone();
    let check_state = state.clone();

    let session_settings = SessionSettings {
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
                title: String::new(),
                description: "use repo override".to_string(),
                creator: Username::from("tester"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                session_settings: session_settings.clone(),
                todo_list: Vec::new(),
                dependencies: Vec::new(),
                patches: Vec::new(),
                deleted: false,
            },
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "prompt": "0",
            "context": { "type": "git_repository", "url": "https://task.example.com/base.git", "rev": "task-branch" },
            "issue_id": issue_id
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;

    let task = check_state.get_session(&body.session_id).await?;
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
            "{}/v1/sessions/{}/context",
            server.base_url(),
            body.session_id.as_ref()
        ))
        .send()
        .await?;
    assert!(context_response.status().is_success());
    let worker_context: v1::sessions::WorkerContext = context_response.json().await?;
    assert_eq!(
        worker_context.request_context,
        v1::sessions::Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "issue-branch".to_string(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn session_context_includes_build_cache_settings() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.build_cache = BuildCacheSection {
        storage: Some(BuildCacheStorageConfig::FileSystem {
            root_dir: "/tmp/hydra-build-cache".to_string(),
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
        test_secret_manager(),
    );
    let server = spawn_test_server_with_state(state, store).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let context_response = client
        .get(format!(
            "{}/v1/sessions/{}/context",
            server.base_url(),
            body.session_id.as_ref()
        ))
        .send()
        .await?;

    assert!(context_response.status().is_success());
    let worker_context: v1::sessions::WorkerContext = context_response.json().await?;
    let build_cache = worker_context.build_cache.expect("build cache");
    assert_eq!(
        build_cache.storage,
        BuildCacheStorageConfig::FileSystem {
            root_dir: "/tmp/hydra-build-cache".to_string(),
        }
    );
    assert_eq!(build_cache.settings.max_entries_per_repo, Some(5));
    Ok(())
}

#[tokio::test]
async fn create_session_rejects_unknown_service_repository() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
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
async fn list_sessions_returns_empty_list_when_store_is_empty() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: ListSessionsResponse = response.json().await?;
    assert!(body.sessions.is_empty());
    Ok(())
}

#[tokio::test]
async fn session_versions_endpoints_return_history() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state.clone();
    let server = spawn_test_server_with_state(state.clone(), handles.store).await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: CreateSessionResponse = response.json().await?;

    state
        .transition_task_to_pending(&created.session_id, ActorRef::test())
        .await?;
    state
        .transition_task_to_running(&created.session_id, ActorRef::test())
        .await?;

    let response = client
        .post(format!(
            "{}/v1/sessions/{}/status",
            server.base_url(),
            created.session_id
        ))
        .json(&json!({ "status": "complete" }))
        .send()
        .await?;

    assert!(response.status().is_success());

    let versions: ListSessionVersionsResponse = client
        .get(format!(
            "{}/v1/sessions/{}/versions",
            server.base_url(),
            created.session_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 4);
    assert_eq!(versions.versions[0].session_id, created.session_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(
        versions.versions[0].session.status,
        v1::task_status::Status::Created
    );
    assert_eq!(versions.versions[1].session_id, created.session_id);
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(
        versions.versions[1].session.status,
        v1::task_status::Status::Pending
    );
    assert_eq!(versions.versions[2].session_id, created.session_id);
    assert_eq!(versions.versions[2].version, 3);
    assert_eq!(
        versions.versions[2].session.status,
        v1::task_status::Status::Running
    );
    assert_eq!(versions.versions[3].session_id, created.session_id);
    assert_eq!(versions.versions[3].version, 4);
    assert_eq!(
        versions.versions[3].session.status,
        v1::task_status::Status::Complete
    );

    let version: SessionVersionRecord = client
        .get(format!(
            "{}/v1/sessions/{}/versions/4",
            server.base_url(),
            created.session_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(version.version, 4);
    assert_eq!(version.session_id, created.session_id);
    assert_eq!(version.session.status, v1::task_status::Status::Complete);

    Ok(())
}

#[tokio::test]
async fn session_version_endpoints_return_404s() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let missing_id = task_id("s-missing");
    let response = client
        .get(format!(
            "{}/v1/sessions/{}/versions",
            server.base_url(),
            missing_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({ "prompt": "0" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: CreateSessionResponse = response.json().await?;

    let response = client
        .get(format!(
            "{}/v1/sessions/{}/versions/99",
            server.base_url(),
            created.session_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn get_session_rejects_empty_session_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions/%20", server.base_url()))
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
async fn get_session_rejects_session_id_with_whitespace_padding() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let server = spawn_test_server_with_state(state.clone(), handles.store.clone()).await?;
    let now = Utc::now();
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
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
        .get(format!(
            "{}/v1/sessions/%20{}%20",
            server.base_url(),
            job_id
        ))
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
async fn get_session_returns_not_found_for_missing_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_id = task_id("s-missing");
    let response = client
        .get(format!("{}/v1/sessions/{missing_id}", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!("session '{missing_id}' not found") })
    );
    Ok(())
}

#[tokio::test]
async fn get_session_logs_rejects_empty_session_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions/ /logs", server.base_url()))
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
async fn get_session_logs_returns_bad_request_when_multiple_sessions_found() -> anyhow::Result<()> {
    let engine = Arc::new(MockJobEngine::new());
    let job_id = task_id("s-jobaa");
    engine.insert_job(&job_id, JobStatus::Running).await;
    engine.insert_job(&job_id, JobStatus::Failed).await;
    let handles = test_state_with_engine_handles(engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;

    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions/{job_id}/logs", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!("Multiple sessions found for hydra-id '{job_id}'") })
    );
    Ok(())
}

#[tokio::test]
async fn get_session_logs_returns_not_found_for_missing_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_id = task_id("s-missing");
    let response = client
        .get(format!(
            "{}/v1/sessions/{missing_id}/logs",
            server.base_url()
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!("Session '{missing_id}' not found") })
    );
    Ok(())
}

#[tokio::test]
async fn get_session_logs_streams_when_watching_running_session() -> anyhow::Result<()> {
    let engine = Arc::new(MockJobEngine::new());
    let job_id = task_id("s-stream");
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
            "{}/v1/sessions/{job_id}/logs?watch=true",
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
async fn kill_session_rejects_empty_session_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .delete(format!("{}/v1/sessions/%20", server.base_url()))
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
async fn kill_session_returns_not_found_for_unknown_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_id = task_id("s-missing");
    let response = client
        .delete(format!("{}/v1/sessions/{missing_id}", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!("Session '{missing_id}' not found") })
    );
    Ok(())
}

#[tokio::test]
async fn kill_session_handles_multiple_matches_conflict() -> anyhow::Result<()> {
    let engine = Arc::new(MockJobEngine::new());
    let job_id = task_id("s-dupe");
    engine.insert_job(&job_id, JobStatus::Running).await;
    engine.insert_job(&job_id, JobStatus::Running).await;
    let handles = test_state_with_engine_handles(engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;

    let client = test_client();
    let response = client
        .delete(format!("{}/v1/sessions/{job_id}", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!("Multiple sessions found for hydra-id '{job_id}'") })
    );
    Ok(())
}

#[tokio::test]
async fn set_session_status_rejects_empty_session_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions/ /status", server.base_url()))
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
async fn set_session_status_returns_not_found_for_missing_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_id = task_id("s-missing");
    let response = client
        .post(format!(
            "{}/v1/sessions/{missing_id}/status",
            server.base_url()
        ))
        .json(&json!({ "status": "complete" }))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await?;
    assert!(body["error"].as_str().unwrap_or("").contains("not found"));
    Ok(())
}

#[tokio::test]
async fn set_session_status_persists_result_for_spawn_tasks() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
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
            },
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions/{job_id}/status", server.base_url()))
        .json(&json!({ "status": "complete" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "session_id": job_id.as_ref(), "status": "complete" })
    );
    Ok(())
}

#[tokio::test]
async fn set_session_status_can_mark_failed() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
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
        .post(format!("{}/v1/sessions/{job_id}/status", server.base_url()))
        .json(&json!({ "status": "failed", "reason": "boom" }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "session_id": job_id.as_ref(), "status": "failed" })
    );
    Ok(())
}

#[tokio::test]
async fn get_session_context_rejects_empty_session_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions/ /context", server.base_url()))
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
async fn get_session_context_returns_not_found_for_unknown_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();
    let missing_id = task_id("s-missing");
    let response = client
        .get(format!(
            "{}/v1/sessions/{missing_id}/context",
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
async fn get_session_context_returns_context_for_spawn_tasks() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let context_spec = BundleSpec::GitRepository {
        url: "https://example.com/repo.git".to_string(),
        rev: "main".to_string(),
    };
    let (parent_job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
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
            },
            &ActorRef::test(),
        )
        .await?;
    state
        .transition_task_to_completion(&parent_job_id, Ok(()), None, ActorRef::test())
        .await?;
    let (ctx_job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!(
            "{}/v1/sessions/{ctx_job_id}/context",
            server.base_url()
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: v1::sessions::WorkerContext = response.json().await?;
    assert_eq!(
        body.request_context,
        v1::sessions::Bundle::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        }
    );
    assert_eq!(body.prompt, "0");
    Ok(())
}

#[tokio::test]
async fn get_session_context_includes_model_from_task() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!(
            "{}/v1/sessions/{job_id}/context",
            server.base_url()
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: v1::sessions::WorkerContext = response.json().await?;
    assert_eq!(body.model.as_deref(), Some("claude-3-5-sonnet"));
    Ok(())
}

#[tokio::test]
async fn get_session_context_includes_task_variables() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
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
                creation_time: None,
                start_time: None,
                end_time: None,
            },
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;

    let client = test_client();
    let response = client
        .get(format!(
            "{}/v1/sessions/{job_id}/context",
            server.base_url()
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: v1::sessions::WorkerContext = response.json().await?;
    assert_eq!(
        body.variables.get("SECRET_VALUE").map(String::as_str),
        Some("keep-me-safe")
    );
    assert_eq!(
        body.variables.get("HYDRA_ID").map(String::as_str),
        Some(job_id.as_ref())
    );
    Ok(())
}
