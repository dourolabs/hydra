use super::common::{default_image, patch_diff, service_repo_name, service_repository, task_id};
use crate::app::{AppState, ServiceState};
use crate::config::BuildCacheSection;
use crate::domain::{
    actors::{ActorRef, store_github_token_secrets},
    patches::{Patch, PatchStatus},
    sessions::{AgentConfig, SessionMode},
    users::{User, Username},
};
use crate::routes::sessions::mount_spec_from_create_request;
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
use hydra_common::api::v1::sessions::Bundle;
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
        .json(&json!({
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "0" }
        }))
        .send()
        .await?;

    assert!(response.status().is_success());
    let body: CreateSessionResponse = response.json().await?;
    assert!(!body.session_id.as_ref().trim().is_empty());

    let task = check_state.get_session(&body.session_id).await?;
    let resolved = resolver_state.resolve_task(&task).await?;

    assert!(matches!(&task.mode, SessionMode::Headless));
    assert_eq!(task.agent_config.system_prompt.as_deref(), Some("0"));
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
async fn create_session_passes_through_caller_supplied_bundle() -> anyhow::Result<()> {
    // Post-PR-1: `create_session` no longer derives mount_spec from a
    // `spawned_from` issue's session_settings. Callers (CLI, automations)
    // pre-lower the bundle and submit a fully-resolved `mount_spec`.
    let (repo_name, repo) = service_repository();

    let handles = crate::test_utils::test_state_handles();
    let state2 = handles.state;
    add_repository(&state2, repo_name.clone(), repo.clone()).await?;
    let resolver_state2 = state2.clone();
    let check_state2 = state2.clone();
    let server2 = spawn_test_server_with_state(state2, handles.store.clone()).await?;
    let client = test_client();
    let response = client
        .post(format!("{}/v1/sessions", server2.base_url()))
        .json(&json!({
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "test" },
            "mount_spec": {
                "working_dir": "repo",
                "mounts": [
                    {
                        "type": "bundle",
                        "target": "repo",
                        "bundle": {
                            "type": "git_repository",
                            "url": repo.remote_url.clone(),
                            "rev": "develop",
                        },
                    },
                    { "type": "documents", "target": "documents" }
                ],
            },
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
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "test" },
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
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "test" },
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
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "custom prompt" },
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

// PR-1 removed `create_session`'s derivation from the `spawned_from`
// issue's `session_settings`. The tests that pinned that behavior
// (`session_settings_override_request_with_remote_url_priority`,
// `session_settings_use_repo_name_and_branch_overrides`,
// `create_session_rejects_unknown_service_repository`) no longer apply —
// callers now lower the bundle / image / cpu / memory themselves before
// calling.

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
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session {
            creator: Username::from("test-creator"),
            spawned_from: None,
            resumed_from: None,
            agent_config: AgentConfig::default(),
            mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            image: Some(default_image.clone()),
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            mode: SessionMode::Headless,
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
        .json(&json!({
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "test" }
        }))
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
        .json(&json!({
            "mode": { "type": "headless" },
            "agent_config": { "type": "adhoc", "system_prompt": "test" }
        }))
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
    let context_bundle = Bundle::GitRepository {
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                agent_config: AgentConfig::new(None, None, Some("0".to_string()), None),
                mount_spec: mount_spec_from_create_request(context_bundle.clone(), None),
                image: Some(default_image.clone()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
        .mounts
        .first()
        .expect("mounts must have at least the bundle item");
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
    assert_eq!(body.mode_kind, v1::sessions::SessionModeKind::Headless);
    // Note: system_prompt no longer flows through WorkerContext — it is
    // delivered to the worker via Phase 2 `FirstMessage` over the relay
    // websocket.
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                creator: Username::from("test-creator"),
                spawned_from: None,
                resumed_from: None,
                agent_config: AgentConfig::default(),
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image.clone()),
                env_vars: HashMap::from([("SECRET_VALUE".to_string(), "keep-me-safe".to_string())]),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
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
                    greet_user: false,
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
    assert_eq!(body.mode_kind, v1::sessions::SessionModeKind::Interactive);
    assert_eq!(body.idle_timeout_secs, Some(expected_idle_timeout));
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

    let context_bundle = Bundle::GitRepository {
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
                mount_spec: mount_spec_from_create_request(context_bundle, None),
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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
                mount_spec: crate::routes::sessions::mount_spec_from_create_request(
                    hydra_common::api::v1::sessions::Bundle::None,
                    None,
                ),
                image: Some(default_image()),
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                secrets: None,
                mode: SessionMode::Headless,
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

/// Build a session whose persisted `mount_spec` mirrors the lowered shape
/// `create_session` would emit for a service-repo issue — fully-resolved
/// `Bundle::GitRepository` plus, when `build_cache` is `Some`, an interior
/// `BuildCache` item. PR-F dropped the fetch-time re-derivation; the
/// persisted spec is now what the worker sees, so test fixtures construct
/// it directly.
fn make_session_with_service_repo(
    repo: &crate::app::Repository,
    repo_name: hydra_common::RepoName,
    env_vars: HashMap<String, String>,
    build_cache: Option<hydra_common::BuildCacheContext>,
) -> Session {
    let bundle = Bundle::GitRepository {
        url: repo.remote_url.clone(),
        rev: repo
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".to_string()),
    };
    let cache_pair = build_cache.map(|ctx| (repo_name, ctx));
    let mount_spec = mount_spec_from_create_request(bundle, cache_pair);
    Session {
        creator: Username::from("test-creator"),
        spawned_from: None,
        resumed_from: None,
        agent_config: AgentConfig::default(),
        mount_spec,
        image: Some(default_image()),
        env_vars,
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless,
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
        mount_spec: crate::routes::sessions::mount_spec_from_create_request(
            hydra_common::api::v1::sessions::Bundle::None,
            None,
        ),
        image: Some(default_image()),
        env_vars,
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless,
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

    let build_cache_ctx = state
        .config
        .build_cache
        .to_context()
        .expect("build_cache must be configured for this test");
    let (session_id, _) = store
        .add_session(
            make_session_with_service_repo(
                &repo,
                repo_name.clone(),
                HashMap::new(),
                Some(build_cache_ctx),
            ),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, store).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    let spec = v1::sessions::MountSpec::new(context.working_dir.clone(), context.mounts.clone());
    let spec = &spec;
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
            make_session_with_service_repo(&repo, repo_name, HashMap::new(), None),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, handles.store.clone()).await?;
    let context = fetch_worker_context(&server, &session_id).await?;

    let spec = v1::sessions::MountSpec::new(context.working_dir.clone(), context.mounts.clone());
    let spec = &spec;
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
    let bundle = Bundle::GitRepository {
        url: "https://example.com/repo.git".to_string(),
        rev: "main".to_string(),
    };
    let session = Session {
        creator: Username::from("test-creator"),
        spawned_from: None,
        resumed_from: None,
        agent_config: AgentConfig::default(),
        mount_spec: mount_spec_from_create_request(bundle, None),
        image: Some(default_image()),
        env_vars: HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        mode: SessionMode::Headless,
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
    let spec = v1::sessions::MountSpec::new(context.working_dir.clone(), context.mounts.clone());
    let spec = &spec;
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

    let spec = v1::sessions::MountSpec::new(context.working_dir.clone(), context.mounts.clone());
    let spec = &spec;
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

/// PR-F: after dropping `Session.context`, the WorkerContext fetch path is a
/// straight read of `Session.mount_spec`. Verify it: the mount_spec the
/// worker sees through `GET /v1/sessions/:id/context` matches what
/// `GET /v1/sessions/:id` returns, including the `BuildCache` item when
/// `build_cache` is configured.
#[tokio::test]
async fn get_session_context_mount_spec_matches_get_session_with_build_cache() -> anyhow::Result<()>
{
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

    let build_cache_ctx = state
        .config
        .build_cache
        .to_context()
        .expect("build_cache must be configured for this test");
    let (session_id, _) = store
        .add_session(
            make_session_with_service_repo(
                &repo,
                repo_name.clone(),
                HashMap::new(),
                Some(build_cache_ctx),
            ),
            Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    let server = spawn_test_server_with_state(state, store).await?;
    let client = test_client();

    let context = fetch_worker_context(&server, &session_id).await?;
    let session_record: v1::sessions::SessionVersionRecord = client
        .get(format!(
            "{}/v1/sessions/{}",
            server.base_url(),
            session_id.as_ref()
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        v1::sessions::MountSpec::new(context.working_dir.clone(), context.mounts.clone()),
        session_record.session.mount_spec,
        "WorkerContext.session.mount_spec must equal GET /v1/sessions/:id mount_spec",
    );
    let _ = repo_name;
    Ok(())
}
