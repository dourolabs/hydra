use super::common::{default_image, patch_diff, service_repo_name, service_repository, task_id};
use crate::app::{AppState, ServiceState, sessions::mount_spec_for_session};
use crate::config::BuildCacheSection;
use crate::domain::{
    actors::{ActorRef, store_github_token_secrets},
    issues::{Issue, IssueStatus, IssueType, SessionSettings},
    patches::{Patch, PatchStatus},
    sessions::{AgentConfig, Bundle, BundleSpec, SessionMode},
    users::{User, Username},
};
use crate::{
    job_engine::JobStatus,
    store::{MemoryStore, Session, Status},
    test_utils::{
        MockJobEngine, add_repository, github_user_response, spawn_test_server,
        spawn_test_server_with_state, test_app_config, test_client, test_secret_manager,
        test_state_handles, test_state_with_engine_handles, test_state_with_github_urls,
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
        .json(&json!({ "mode": { "type": "headless", "prompt": "0" } }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    assert!(!body.session_id.as_ref().trim().is_empty());

    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;

    assert_eq!(task.mode.prompt_for_legacy_wire(), "0");
    assert!(task.service_repo_name().is_none());
    assert_eq!(resolved.context.bundle, Bundle::None);
    assert_eq!(resolved.image, resolver_state.config.job.default_image);

    let status = check_state.get_session(&body.session_id).await?.status;
    // The start_created_sessions automation transitions Created → Pending
    // immediately via the event bus, so by the time we check the session
    // it may already be Pending.
    assert!(
        status == Status::Created || status == Status::Pending,
        "expected Created or Pending, got {status:?}"
    );
    Ok(())
}

#[tokio::test]
async fn create_session_allows_service_repository_bundle() -> anyhow::Result<()> {
    let (repo_name, repo) = service_repository();

    // Post-PR-E: route the service-repo lookup through `spawned_from`. The
    // server-side `mount_spec_from_session_settings` resolves the repo to
    // a fully-lowered Bundle::GitRepository and `mount_spec` carries no
    // BuildCache (no build_cache config in this test), so `session.context`
    // ends up as GitRepository — matching how the new CLI sends sessions.
    let handles = crate::test_utils::test_state_handles();
    let state2 = handles.state;
    add_repository(&state2, repo_name.clone(), repo.clone()).await?;
    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue {
                issue_type: IssueType::Task,
                title: String::new(),
                description: "linked to service repo".to_string(),
                creator: Username::from("tester"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                session_settings: SessionSettings {
                    repo_name: Some(repo_name.clone()),
                    branch: Some("develop".to_string()),
                    ..Default::default()
                },
                todo_list: Vec::new(),
                dependencies: Vec::new(),
                patches: Vec::new(),
                deleted: false,
                form: None,
                form_response: None,
                feedback: None,
            },
            &ActorRef::test(),
        )
        .await?;
    let resolver_state2 = state2.clone();
    let check_state2 = state2.clone();
    let server2 = spawn_test_server_with_state(state2, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server2.base_url()))
        .json(&json!({
            "mode": { "type": "headless", "prompt": "0" },
            "spawned_from": issue_id,
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    let task = check_state2.get_session(&body.session_id).await?;
    let resolved = resolver_state2.resolve_task(&task).await?;
    assert_eq!(
        resolved.context.bundle,
        Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "develop".to_string()
        }
    );

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
            "mode": { "type": "headless", "prompt": "0" },
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
            "mode": { "type": "headless", "prompt": "0" },
            "image": "ghcr.io/example/override:main"
        }))
        .send()
        .await?;
    let _ = repo_name;

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
            "mode": { "type": "headless", "prompt": "0" },
            "env_vars": { "FOO": "bar", "PROMPT": "custom prompt" }
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
                form: None,
                form_response: None,
                feedback: None,
            },
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "mode": { "type": "headless", "prompt": "0" },
            "spawned_from": issue_id,
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
    let bundle_item = worker_context
        .session
        .mount_spec
        .mounts
        .first()
        .expect("mount_spec must have at least the bundle item");
    let v1::sessions::MountItem::Bundle { bundle, .. } = bundle_item else {
        panic!("expected Bundle item first, got {bundle_item:?}");
    };
    assert_eq!(
        bundle,
        &v1::sessions::Bundle::GitRepository {
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
                form: None,
                form_response: None,
                feedback: None,
            },
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&json!({
            "mode": { "type": "headless", "prompt": "0" },
            "spawned_from": issue_id,
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
    // Image comes from session_settings, not from repo default_image
    // (the resolver no longer looks up the repo via ServiceRepository
    // because mount_spec is fully-lowered post-PR-E).
    let _ = repo.default_image;

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
    let bundle_item = worker_context
        .session
        .mount_spec
        .mounts
        .first()
        .expect("mount_spec must have at least the bundle item");
    let v1::sessions::MountItem::Bundle { bundle, .. } = bundle_item else {
        panic!("expected Bundle item first, got {bundle_item:?}");
    };
    assert_eq!(
        bundle,
        &v1::sessions::Bundle::GitRepository {
            url: repo.remote_url.clone(),
            rev: "issue-branch".to_string(),
        }
    );

    Ok(())
}

#[tokio::test]
async fn create_session_rejects_unknown_service_repository() -> anyhow::Result<()> {
    // Post-PR-E: clients send a fully-lowered Bundle::GitRepository in
    // `mount_spec`, so the server no longer encounters
    // `BundleSpec::ServiceRepository` with an unknown name through the
    // request body. The equivalent failure now surfaces from the CLI's
    // `list_repositories` lookup. This test pins the resolver path:
    // sending a session linked (via `spawned_from`) to an issue whose
    // `session_settings.repo_name` references an unregistered repo
    // surfaces the same `unknown repository` error from
    // `mount_spec_from_session_settings`.
    let handles = crate::test_utils::test_state_handles();
    let state = handles.state;
    let (issue_id, _) = handles
        .store
        .add_issue(
            Issue {
                issue_type: IssueType::Task,
                title: String::new(),
                description: "missing service repo".to_string(),
                creator: Username::from("tester"),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                session_settings: SessionSettings {
                    repo_name: Some(hydra_common::RepoName::new("missing", "repo").unwrap()),
                    ..Default::default()
                },
                todo_list: Vec::new(),
                dependencies: Vec::new(),
                patches: Vec::new(),
                deleted: false,
                form: None,
                form_response: None,
                feedback: None,
            },
            &ActorRef::test(),
        )
        .await?;
    let server2 = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server2.base_url()))
        .json(&json!({
            "mode": { "type": "headless", "prompt": "0" },
            "spawned_from": issue_id,
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
async fn list_sessions_includes_usage_in_summary() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let default_image = default_image();
    let usage = hydra_common::api::v1::sessions::TokenUsage {
        input_tokens: 1234,
        output_tokens: 567,
        cache_read_input_tokens: 89,
        cache_creation_input_tokens: 12,
    };
    let session = {
        use crate::app::sessions::mount_spec_for_session;
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session {
            creator: Username::from("test-creator"),
            spawned_from: None,
            resumed_from: None,
            agent_config: AgentConfig::default(),
            mount_spec: mount_spec_for_session(&BundleSpec::None),
            context: BundleSpec::None,
            image: Some(default_image.clone()),
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            mode: SessionMode::Headless {
                prompt: "with-usage".to_string(),
            },
            status: Status::Complete,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
            usage: Some(usage.clone()),
        }
    };
    let (session_id, _) = store
        .add_session(session, Utc::now(), &ActorRef::test())
        .await?;

    let server = spawn_test_server_with_state(handles.state, store).await?;
    let client = test_client();
    let response = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: ListSessionsResponse = response.json().await?;
    let summary = body
        .sessions
        .iter()
        .find(|record| record.session_id == session_id)
        .expect("expected session in list response");
    assert_eq!(summary.session.usage.as_ref(), Some(&usage));
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
        .json(&json!({ "mode": { "type": "headless", "prompt": "0" } }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let created: CreateSessionResponse = response.json().await?;

    // The start_created_sessions automation transitions Created → Pending
    // automatically via the event bus. Wait for it to complete.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let session = state.get_session(&created.session_id).await?;
        if session.status != Status::Created {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for session to transition from Created");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

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
        .json(&json!({ "mode": { "type": "headless", "prompt": "0" } }))
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
        .transition_task_to_completion(&parent_job_id, Ok(()), None, None, ActorRef::test())
        .await?;
    let (ctx_job_id, _) = handles
        .store
        .add_session(
            Session {
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&context_spec),
                context: context_spec.clone(),
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
    let bundle_item = body
        .session
        .mount_spec
        .mounts
        .first()
        .expect("mount_spec must have at least the bundle item");
    let v1::sessions::MountItem::Bundle { bundle, .. } = bundle_item else {
        panic!("expected Bundle item first, got {bundle_item:?}");
    };
    assert_eq!(
        bundle,
        &v1::sessions::Bundle::GitRepository {
            url: "https://example.com/repo.git".to_string(),
            rev: "main".to_string(),
        }
    );
    let v1::sessions::SessionMode::Headless { prompt } = &body.session.mode else {
        panic!("expected Headless mode, got {:?}", body.session.mode);
    };
    assert_eq!(prompt, "0");
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::new(
                    None,
                    Some("claude-3-5-sonnet".to_string()),
                    None,
                    None,
                ),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
        body.session.agent_config.model.as_deref(),
        Some("claude-3-5-sonnet")
    );
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image.clone()),
                env_vars: HashMap::from([("SECRET_VALUE".to_string(), "keep-me-safe".to_string())]),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
        body.resolved_env.get("SECRET_VALUE").map(String::as_str),
        Some("keep-me-safe")
    );
    assert_eq!(
        body.resolved_env.get("HYDRA_ID").map(String::as_str),
        Some(job_id.as_ref())
    );
    Ok(())
}

#[tokio::test]
async fn get_session_context_populates_idle_timeout_from_config() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let expected_idle_timeout = state.config.job.interactive_idle_timeout_secs;
    let default_image = default_image();
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                // Interactive sessions surface the server-configured
                // idle-timeout through `session.mode.Interactive`.
                mode: SessionMode::Interactive {
                    conversation_id: hydra_common::ConversationId::new(),
                    idle_timeout_secs: None,
                    conversation_resume_from: None,
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
    let v1::sessions::SessionMode::Interactive {
        idle_timeout_secs, ..
    } = &body.session.mode
    else {
        panic!("expected Interactive mode, got {:?}", body.session.mode);
    };
    assert_eq!(*idle_timeout_secs, Some(expected_idle_timeout));
    Ok(())
}

/// Regression: `/v1/sessions/:id/context` must populate
/// `WorkerContext.github_token` from the creator's stored GitHub token. After
/// PR-6 part A removed the worker's `client.get_github_token()` fallback,
/// `WorkerContext.github_token` is the only source of truth for clone auth
/// on the worker, so a `None` here is a hard outage for any session whose
/// bundle requires authenticated clone.
#[tokio::test]
async fn get_session_context_populates_github_token_from_creator_secret() -> anyhow::Result<()> {
    // Mock GitHub `/user` so `get_github_token_for_user`'s validity check
    // accepts the stored token without going out to api.github.com.
    let github_server = httpmock::MockServer::start_async().await;
    let _user_mock = github_server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/user");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(github_user_response("octo", 42));
    });

    let handles = test_state_with_github_urls(github_server.base_url(), github_server.base_url());
    let state = handles.state.clone();
    let creator = Username::from("test-creator");

    // Seed the creator user and an encrypted GitHub token in user_secrets so
    // `get_github_token_for_user` can read it back.
    handles
        .store
        .add_user(
            User::new(creator.clone(), Some(101), false),
            &ActorRef::test(),
        )
        .await?;
    store_github_token_secrets(&state, &creator, "creator-token", "creator-refresh").await;

    let context_spec = BundleSpec::GitRepository {
        url: "https://example.com/repo.git".to_string(),
        rev: "main".to_string(),
    };
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
                creator: creator.clone(),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&context_spec),
                context: context_spec.clone(),
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
    assert_eq!(body.github_token.as_deref(), Some("creator-token"));
    Ok(())
}

/// Sessions whose creator has no GitHub token on file must still get a
/// `WorkerContext` back — the field is `None` and the worker fails later at
/// clone time with a clear auth error (matching the pre-refactor
/// `client.get_github_token().await.ok()` semantics).
#[tokio::test]
async fn get_session_context_returns_none_token_when_creator_has_no_secret() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let (job_id, _) = handles
        .store
        .add_session(
            Session {
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: mount_spec_for_session(&BundleSpec::None),
                context: BundleSpec::None,
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless {
                    prompt: "0".to_string(),
                },
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
                creation_time: None,
                start_time: None,
                end_time: None,
                usage: None,
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
    assert_eq!(body.github_token, None);
    Ok(())
}

// --- MountSpec population tests ----------------------------------------------

fn mount_spec_test_config_with_build_cache() -> crate::config::AppConfig {
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
    config
}

fn make_session_with_service_repo(
    repo_name: hydra_common::RepoName,
    env_vars: HashMap<String, String>,
) -> Session {
    let context = BundleSpec::ServiceRepository {
        name: repo_name,
        rev: None,
    };
    Session {
        creator: Username::from("test-creator"),
        spawned_from: None,
        resumed_from: None,
        agent_config: AgentConfig::default(),
        mount_spec: mount_spec_for_session(&context),
        context: context.clone(),
        image: Some(default_image()),
        env_vars,
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless {
            prompt: "prompt".to_string(),
        },
        status: Status::Created,
        last_message: None,
        error: None,
        deleted: false,
        creation_time: None,
        start_time: None,
        end_time: None,
        usage: None,
    }
}

fn make_session_no_bundle(env_vars: HashMap<String, String>) -> Session {
    Session {
        creator: Username::from("test-creator"),
        spawned_from: None,
        resumed_from: None,
        agent_config: AgentConfig::default(),
        mount_spec: mount_spec_for_session(&BundleSpec::None),
        context: BundleSpec::None,
        image: Some(default_image()),
        env_vars,
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless {
            prompt: "prompt".to_string(),
        },
        status: Status::Created,
        last_message: None,
        error: None,
        deleted: false,
        creation_time: None,
        start_time: None,
        end_time: None,
        usage: None,
    }
}

async fn fetch_worker_context(
    server: &crate::test_utils::TestServer,
    session_id: &hydra_common::SessionId,
) -> anyhow::Result<v1::sessions::WorkerContext> {
    let client = test_client();
    let response = client
        .get(format!(
            "{}/v1/sessions/{}/context",
            server.base_url(),
            session_id.as_ref()
        ))
        .send()
        .await?;
    assert!(response.status().is_success());
    Ok(response.json().await?)
}

#[tokio::test]
async fn get_session_context_populates_three_item_mount_spec_for_standard_session()
-> anyhow::Result<()> {
    use hydra_common::api::v1::sessions::MountItem;

    let config = mount_spec_test_config_with_build_cache();
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;

    let (session_id, _) = store
        .add_session(
            make_session_with_service_repo(repo_name.clone(), HashMap::new()),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, store).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    let spec = &context.session.mount_spec;
    assert_eq!(spec.working_dir.as_path().to_str(), Some("repo"));
    assert_eq!(spec.mounts.len(), 3);
    match &spec.mounts[0] {
        MountItem::Bundle { target, bundle } => {
            assert_eq!(target.as_path().to_str(), Some("repo"));
            assert_eq!(
                bundle,
                &v1::sessions::Bundle::GitRepository {
                    url: repo.remote_url.clone(),
                    rev: "develop".to_string(),
                }
            );
        }
        other => panic!("expected Bundle item first, got {other:?}"),
    }
    match &spec.mounts[1] {
        MountItem::BuildCache {
            repo_target,
            service_repo_name,
            context: cache_context,
        } => {
            assert_eq!(repo_target.as_path().to_str(), Some("repo"));
            assert_eq!(service_repo_name, &repo_name);
            assert_eq!(
                cache_context.storage,
                BuildCacheStorageConfig::FileSystem {
                    root_dir: "/tmp/hydra-build-cache".to_string(),
                }
            );
        }
        other => panic!("expected BuildCache item second, got {other:?}"),
    }
    match &spec.mounts[2] {
        MountItem::Documents { target } => {
            assert_eq!(target.as_path().to_str(), Some("documents"));
        }
        other => panic!("expected Documents item last, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn get_session_context_omits_build_cache_when_cache_unconfigured() -> anyhow::Result<()> {
    use hydra_common::api::v1::sessions::MountItem;

    let handles = test_state_handles();
    let state = handles.state;
    let (repo_name, repo) = service_repository();
    add_repository(&state, repo_name.clone(), repo.clone()).await?;

    let (session_id, _) = handles
        .store
        .add_session(
            make_session_with_service_repo(repo_name, HashMap::new()),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    let spec = &context.session.mount_spec;
    assert_eq!(spec.mounts.len(), 2);
    assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
    assert!(matches!(spec.mounts[1], MountItem::Documents { .. }));
    Ok(())
}

#[tokio::test]
async fn get_session_context_omits_build_cache_when_no_service_repo() -> anyhow::Result<()> {
    use hydra_common::api::v1::sessions::MountItem;

    let config = mount_spec_test_config_with_build_cache();
    let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );

    // Session uses a raw git_repository bundle (no service_repo_name available).
    let context = BundleSpec::GitRepository {
        url: "https://example.com/repo.git".to_string(),
        rev: "main".to_string(),
    };
    let session = Session {
        creator: Username::from("test-creator"),
        spawned_from: None,
        resumed_from: None,
        agent_config: AgentConfig::default(),
        mount_spec: mount_spec_for_session(&context),
        context: context.clone(),
        image: Some(default_image()),
        env_vars: HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless {
            prompt: "prompt".to_string(),
        },
        status: Status::Created,
        last_message: None,
        error: None,
        deleted: false,
        creation_time: None,
        start_time: None,
        end_time: None,
        usage: None,
    };
    let (session_id, _) = store
        .add_session(session, Utc::now(), &ActorRef::test())
        .await?;

    let server = spawn_test_server_with_state(state, store).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    // The spec has no BuildCache item because there's no service repo name,
    // even though the server itself has build_cache configured.
    let spec = &context.session.mount_spec;
    assert_eq!(spec.mounts.len(), 2);
    assert!(matches!(spec.mounts[0], MountItem::Bundle { .. }));
    assert!(matches!(spec.mounts[1], MountItem::Documents { .. }));
    Ok(())
}

#[tokio::test]
async fn get_session_context_emits_bundle_item_for_none_bundle() -> anyhow::Result<()> {
    use hydra_common::api::v1::sessions::MountItem;

    let handles = test_state_handles();
    let state = handles.state;
    let (session_id, _) = handles
        .store
        .add_session(
            make_session_no_bundle(HashMap::new()),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    let spec = &context.session.mount_spec;
    assert_eq!(spec.mounts.len(), 2);
    match &spec.mounts[0] {
        MountItem::Bundle { bundle, .. } => {
            assert_eq!(bundle, &v1::sessions::Bundle::None);
        }
        other => panic!("expected Bundle item first, got {other:?}"),
    }
    assert!(matches!(spec.mounts[1], MountItem::Documents { .. }));
    Ok(())
}

/// PR-D moved `issue_branch_id` off `MountItem::Bundle` and onto the
/// worker-side `InstantiateInputs`. The server now just forwards the
/// raw `ENV_HYDRA_ISSUE_ID` env var through `resolved_env` — the worker
/// reads it at mount instantiation time. This test pins the new
/// behavior end-to-end.
#[tokio::test]
async fn get_session_context_forwards_issue_branch_env_var_through_resolved_env()
-> anyhow::Result<()> {
    let handles = test_state_handles();
    let state = handles.state;
    let env_vars = HashMap::from([(
        hydra_common::constants::ENV_HYDRA_ISSUE_ID.to_string(),
        "i-abcdefg".to_string(),
    )]);
    let (session_id, _) = handles
        .store
        .add_session(
            make_session_no_bundle(env_vars),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    assert_eq!(
        context
            .resolved_env
            .get(hydra_common::constants::ENV_HYDRA_ISSUE_ID)
            .map(String::as_str),
        Some("i-abcdefg"),
    );
    Ok(())
}
