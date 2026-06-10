use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use httpmock::prelude::*;
use httpmock::Method::PATCH;
use hydra::client::{HydraClient, HydraClientUnauthenticated};
use hydra_common::{
    api::v1::projects::{
        ProjectKey, ProjectRef, StatusDefinition, StatusKey, UpsertProjectRequest,
    },
    conversations::{
        ConversationStatus, CreateConversationRequest, SearchConversationsQuery,
        SendMessageRequest, UpdateConversationRequest,
    },
    documents::{Document, SearchDocumentsQuery, UpsertDocumentRequest},
    issues::{IssueDependencyType, IssueInput, IssueType, SearchIssuesQuery, UpsertIssueRequest},
    login::LoginRequest,
    logs::LogsQuery,
    patches::{GithubCiState, Patch, PatchStatus, SearchPatchesQuery, UpsertPatchRequest},
    repositories::{
        CreateRepositoryRequest, Repository, SearchRepositoriesQuery, UpdateRepositoryRequest,
    },
    session_status::SessionStatusUpdate,
    sessions::{Bundle, CreateSessionRequest, SearchSessionsQuery},
    task_status::Status,
    test_utils::status::status,
    users::Username,
    whoami::ActorIdentity,
    ConversationId, DocumentId, IssueId, PatchId, ProjectId, RelativeVersionNumber, RepoName,
    SessionId,
};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};

const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

#[tokio::test]
async fn hydra_client_handles_forward_compatible_payloads() -> Result<()> {
    let server = MockServer::start();
    let client =
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
    let unauth_client =
        HydraClientUnauthenticated::with_http_client(server.base_url(), HttpClient::new())?;

    let now = Utc::now();
    let job_id = SessionId::new();
    let issue_id = IssueId::new();
    let dependency_id = IssueId::new();
    let patch_id = PatchId::new();
    let project_id = ProjectId::new();
    let conversation_id = ConversationId::new();
    let repo_name = RepoName::new("dourolabs", "hydra")?;
    let username: Username = "future-user".into();

    let status_log_json = forward_status_log_json(now);
    let job_record_body = forward_job_json(&job_id, status_log_json.clone());
    let job_record_body_for_list = job_record_body.clone();
    let job_record_body_for_get = job_record_body.clone();
    let issue_record_body = forward_issue_json(&issue_id, &dependency_id, &patch_id);
    let issue_record_for_get = issue_record_body.clone();
    let issue_record_for_list = issue_record_body.clone();
    let patch_record_body = forward_patch_json(&patch_id, &repo_name, now);
    let patch_record_for_get = patch_record_body.clone();
    let patch_summary_record = forward_patch_summary_json(&patch_id, &repo_name);
    let document_id = DocumentId::new();
    let document_record_body = forward_document_json(&document_id, &job_id);
    let document_record_for_get = document_record_body.clone();
    let document_record_for_list = document_record_body.clone();
    let document_version_body = forward_document_version_json(&document_id, 2, now, &job_id);
    let repository_body = forward_repo_info(&repo_name);
    let conversation_record_body = forward_conversation_json(&conversation_id);
    let conversation_record_for_get = conversation_record_body.clone();
    let conversation_record_for_create = conversation_record_body.clone();
    let conversation_record_for_close = conversation_record_body.clone();
    let conversation_record_for_update = conversation_record_body.clone();
    let conversation_record_for_delete = conversation_record_body.clone();
    let conversation_summary_record = forward_conversation_summary_json(&conversation_id);
    let conversation_version_body = forward_conversation_version_json(&conversation_id, 2, now);

    let job_path = format!("/v1/sessions/{job_id}");
    let job_logs_path = format!("/v1/sessions/{job_id}/logs");
    let job_status_path = format!("/v1/sessions/{job_id}/status");
    let job_context_path = format!("/v1/sessions/{job_id}/context");
    let issue_path = format!("/v1/issues/{issue_id}");
    let patch_path = format!("/v1/patches/{patch_id}");
    let document_path = format!("/v1/documents/{document_id}");
    let document_versions_path = format!("{document_path}/versions");
    let document_version_path = format!("{document_versions_path}/2");
    let repo_path = format!(
        "/v1/repositories/{}/{}",
        repo_name.organization, repo_name.repo
    );
    let github_token_lookup_path = "/v1/github/token";
    let whoami_path = "/v1/whoami";
    let project_path = format!("/v1/projects/{project_id}");
    let project_statuses_path = format!("{project_path}/statuses");
    let project_record_body = forward_project_json(&project_id);
    let project_record_for_get = project_record_body.clone();
    let project_record_for_list = project_record_body.clone();
    let merge_queue_path = format!(
        "/v1/merge-queues/{}/{}/main/patches",
        repo_name.organization, repo_name.repo
    );
    let conversation_path = format!("/v1/conversations/{conversation_id}");
    let conversation_messages_path = format!("{conversation_path}/messages");
    let conversation_close_path = format!("{conversation_path}/close");
    let conversation_versions_path = format!("{conversation_path}/versions");
    let conversation_version_path = format!("{conversation_versions_path}/2");
    let job_id_for_create = job_id.clone();
    let job_id_for_get = job_id.clone();
    let job_id_for_kill = job_id.clone();
    let job_id_for_status_post = job_id.clone();
    let issue_id_for_create = issue_id.clone();
    let issue_id_for_update = issue_id.clone();
    let patch_id_for_create = patch_id.clone();
    let patch_id_for_update = patch_id.clone();
    let patch_id_for_merge = patch_id.clone();
    let patch_id_for_enqueue = patch_id.clone();
    let document_id_for_create = document_id.clone();
    let document_id_for_update = document_id.clone();
    let username_for_whoami = username.clone();

    server.mock(|when, then| {
        when.method(POST).path("/v1/login").json_body(json!({
            "github_token": "gho_forward_compat",
            "github_refresh_token": "ghr_forward_compat"
        }));
        then.status(200).json_body(json!({
            "login_token": "login-token",
            "user": {
                "username": "future-user",
                "github_user_id": 4242
            },
            "extra": "login"
        }));
    });

    let job_record_body_for_create = job_record_body.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/sessions");
        then.status(200).json_body(json!({
            "session_id": job_id_for_create.clone(),
            "session": job_record_body_for_create["task"].clone(),
            "unexpected": "field"
        }));
    });

    server.mock(move |when, then| {
        when.method(GET).path("/v1/sessions");
        then.status(200).json_body(json!({
            "jobs": [job_record_body_for_list.clone()],
            "future": "job-list"
        }));
    });

    let job_path_clone = job_path.clone();
    let job_record_body_for_get_clone = job_record_body_for_get.clone();
    let job_id_for_get_clone = job_id_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(job_path_clone.as_str());
        then.status(200).json_body(json!({
            "extra": "job",
            "job_id": job_id_for_get_clone,
            "version": 0,
            "timestamp": Utc::now(),
            "task": job_record_body_for_get_clone["task"].clone(),
            "notes": "note",
            "status_log": job_record_body_for_get_clone["status_log"].clone()
        }));
    });

    let kill_session_path = job_path.clone();
    let job_id_for_kill_clone = job_id_for_kill.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(kill_session_path.as_str());
        then.status(200).json_body(json!({
            "job_id": job_id_for_kill_clone,
            "status": "terminated",
            "note": "extra-status"
        }));
    });

    let logs_path_clone = job_logs_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(logs_path_clone.as_str());
        then.status(200)
            .header("content-type", "text/event-stream")
            .body("data: first log line\n\nevent: info\ndata: second log line\n\n");
    });

    let job_status_path_clone = job_status_path.clone();
    let job_id_for_status_post_clone = job_id_for_status_post.clone();
    server.mock(move |when, then| {
        when.method(POST).path(job_status_path_clone.as_str());
        then.status(200).json_body(
            json!({ "job_id": job_id_for_status_post_clone, "status": "draining", "future": true }),
        );
    });

    let context_path_clone = job_context_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(context_path_clone.as_str());
        then.status(200).json_body(forward_worker_context_json());
    });

    let issue_id_for_create_clone = issue_id_for_create.clone();
    let issue_record_for_create = issue_record_body.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/issues");
        then.status(200).json_body(json!({
            "issue_id": issue_id_for_create_clone,
            "version": 0,
            "issue": issue_record_for_create["issue"].clone(),
            "extra": "create-issue",
        }));
    });

    let issue_update_path = issue_path.clone();
    let issue_id_for_update_clone = issue_id_for_update.clone();
    let issue_record_for_update = issue_record_body.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(issue_update_path.as_str());
        then.status(200).json_body(json!({
            "issue_id": issue_id_for_update_clone,
            "version": 1,
            "issue": issue_record_for_update["issue"].clone(),
            "future": true,
        }));
    });

    let issue_get_path = issue_path.clone();
    let issue_record_for_get_clone = issue_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(issue_get_path.as_str());
        then.status(200).json_body(issue_record_for_get_clone);
    });

    let issue_record_for_list_clone = issue_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/issues");
        then.status(200).json_body(json!({
            "issues": [issue_record_for_list_clone],
            "note": "list issues"
        }));
    });

    let patch_id_for_create_clone = patch_id_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/patches");
        then.status(200).json_body(
            json!({ "patch_id": patch_id_for_create_clone, "version": 0, "additional": "create-patch" }),
        );
    });

    let patch_update_path = patch_path.clone();
    let patch_id_for_update_clone = patch_id_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(patch_update_path.as_str());
        then.status(200).json_body(
            json!({ "patch_id": patch_id_for_update_clone, "version": 1, "note": "update" }),
        );
    });

    let patch_get_path = patch_path.clone();
    let patch_record_for_get_clone = patch_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(patch_get_path.as_str());
        then.status(200).json_body(patch_record_for_get_clone);
    });

    let patch_summary_record_clone = patch_summary_record.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/patches");
        then.status(200)
            .json_body(json!({ "patches": [patch_summary_record_clone], "extra": "list" }));
    });

    let document_id_for_create_clone = document_id_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/documents");
        then.status(200).json_body(json!({
            "document_id": document_id_for_create_clone,
            "version": 0,
            "note": "create-document"
        }));
    });

    let document_update_path = document_path.clone();
    let document_id_for_update_clone = document_id_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(document_update_path.as_str());
        then.status(200).json_body(json!({
            "document_id": document_id_for_update_clone,
            "version": 1,
            "extra": "update-document"
        }));
    });

    let document_get_path = document_path.clone();
    let document_record_for_get_clone = document_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(document_get_path.as_str());
        then.status(200)
            .json_body(document_record_for_get_clone.clone());
    });

    let document_record_for_list_clone = document_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/documents");
        then.status(200).json_body(json!({
            "documents": [document_record_for_list_clone.clone()],
            "extra": "documents"
        }));
    });

    let document_versions_path_clone = document_versions_path.clone();
    let document_version_body_clone = document_version_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(document_versions_path_clone.as_str());
        then.status(200).json_body(json!({
            "versions": [document_version_body_clone.clone()],
            "note": "document-versions"
        }));
    });

    let document_version_path_clone = document_version_path.clone();
    let document_version_body_for_get = document_version_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(document_version_path_clone.as_str());
        then.status(200)
            .json_body(document_version_body_for_get.clone());
    });

    let repository_body_for_list = repository_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/repositories");
        then.status(200).json_body(
            json!({ "repositories": [repository_body_for_list.clone()], "meta": "list-repos" }),
        );
    });

    let repository_body_for_create = repository_body.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/repositories");
        then.status(200).json_body(
            json!({ "repository": repository_body_for_create.clone(), "extra": "create" }),
        );
    });

    let repo_update_path = repo_path.clone();
    let repository_body_for_update = repository_body.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(repo_update_path.as_str());
        then.status(200).json_body(
            json!({ "repository": repository_body_for_update.clone(), "note": "update-repo" }),
        );
    });

    server.mock(move |when, then| {
        when.method(GET).path(github_token_lookup_path);
        then.status(200).json_body(json!({
            "github_token": "gho_forward_compat",
            "extra": "github-token"
        }));
    });

    server.mock(move |when, then| {
        when.method(GET).path(whoami_path);
        then.status(200).json_body(json!({
            "actor": {
                "type": "user",
                "username": username_for_whoami,
                "role": "extra"
            },
            "note": "whoami"
        }));
    });

    let merge_queue_path_clone = merge_queue_path.clone();
    let patch_id_for_merge_clone = patch_id_for_merge.clone();
    server.mock(move |when, then| {
        when.method(GET).path(merge_queue_path_clone.as_str());
        then.status(200)
            .json_body(json!({ "patches": [patch_id_for_merge_clone], "extra": "merge-queue" }));
    });

    let enqueue_merge_queue_path = merge_queue_path.clone();
    let patch_id_for_enqueue_clone = patch_id_for_enqueue.clone();
    server.mock(move |when, then| {
        when.method(POST).path(enqueue_merge_queue_path.as_str());
        then.status(200)
            .json_body(json!({ "patches": [patch_id_for_enqueue_clone], "note": "enqueue" }));
    });

    server.mock(|when, then| {
        when.method(GET).path("/v1/agents");
        then.status(200).json_body(json!({
            "agents": [{"name": "bot", "capability": "new"}],
            "extra": "agents"
        }));
    });

    let project_id_for_create = project_id.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/projects");
        then.status(200).json_body(json!({
            "project_id": project_id_for_create,
            "version": 0,
            "note": "create-project"
        }));
    });

    let project_record_for_list_clone = project_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/projects");
        then.status(200).json_body(json!({
            "projects": [project_record_for_list_clone.clone()],
            "extra": "list-projects"
        }));
    });

    let project_get_path = project_path.clone();
    let project_record_for_get_clone = project_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(project_get_path.as_str());
        then.status(200)
            .json_body(project_record_for_get_clone.clone());
    });

    let project_update_path = project_path.clone();
    let project_id_for_update = project_id.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(project_update_path.as_str());
        then.status(200).json_body(json!({
            "project_id": project_id_for_update,
            "version": 1,
            "note": "update-project"
        }));
    });

    let project_delete_path = project_path.clone();
    let project_id_for_delete = project_id.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(project_delete_path.as_str());
        then.status(200).json_body(json!({
            "project_id": project_id_for_delete,
            "version": 2,
            "note": "delete-project"
        }));
    });

    let project_statuses_path_clone = project_statuses_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(project_statuses_path_clone.as_str());
        then.status(200).json_body(json!({
            "statuses": [
                {
                    "key": "open",
                    "label": "Open",
                    "color": "#abcdef",
                    "unblocks_parents": false,
                    "unblocks_dependents": false,
                    "cascades_to_children": false,
                    "future": "field"
                }
            ],
            "extra": "statuses"
        }));
    });

    // POST /v1/projects/:project_ref/statuses
    let project_status_create_path = project_statuses_path.clone();
    let project_id_for_status_post = project_id.clone();
    server.mock(move |when, then| {
        when.method(POST).path(project_status_create_path.as_str());
        then.status(200).json_body(json!({
            "project_id": project_id_for_status_post,
            "version": 3,
            "status": {
                "key": "open",
                "label": "Open",
                "color": "#abcdef",
                "unblocks_parents": false,
                "unblocks_dependents": false,
                "cascades_to_children": false,
                "future": "field"
            },
            "extra": "create-status"
        }));
    });

    // PUT /v1/projects/:project_ref/statuses/:status_key
    let project_status_update_path = format!("{project_statuses_path}/open");
    let project_id_for_status_put = project_id.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(project_status_update_path.as_str());
        then.status(200).json_body(json!({
            "project_id": project_id_for_status_put,
            "version": 4,
            "status": {
                "key": "open",
                "label": "Renamed",
                "color": "#abcdef",
                "unblocks_parents": false,
                "unblocks_dependents": false,
                "cascades_to_children": false,
            },
            "extra": "update-status"
        }));
    });

    // DELETE /v1/projects/:project_ref/statuses/:status_key
    let project_status_delete_path = format!("{project_statuses_path}/open");
    let project_id_for_status_delete = project_id.clone();
    server.mock(move |when, then| {
        when.method(DELETE)
            .path(project_status_delete_path.as_str());
        then.status(200).json_body(json!({
            "project_id": project_id_for_status_delete,
            "version": 5,
            "extra": "delete-status"
        }));
    });

    server.mock(|when, then| {
        when.method(GET).path("/v1/github/app/client-id");
        then.status(200)
            .json_body(json!({ "client_id": "abc123", "note": "github" }));
    });

    // Conversations
    server.mock(move |when, then| {
        when.method(POST).path("/v1/conversations");
        then.status(200).json_body(conversation_record_for_create);
    });

    let conversation_summary_for_list = conversation_summary_record.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/conversations");
        then.status(200)
            .json_body(json!([conversation_summary_for_list]));
    });

    let conversation_get_path = conversation_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(conversation_get_path.as_str());
        then.status(200).json_body(conversation_record_for_get);
    });

    let conversation_update_path = conversation_path.clone();
    server.mock(move |when, then| {
        when.method(PATCH).path(conversation_update_path.as_str());
        then.status(200).json_body(conversation_record_for_update);
    });

    let conversation_delete_path = conversation_path.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(conversation_delete_path.as_str());
        then.status(200).json_body(conversation_record_for_delete);
    });

    let conversation_versions_path_clone = conversation_versions_path.clone();
    let conversation_version_body_for_list = conversation_version_body.clone();
    server.mock(move |when, then| {
        when.method(GET)
            .path(conversation_versions_path_clone.as_str());
        then.status(200)
            .json_body(json!([conversation_version_body_for_list]));
    });

    let conversation_version_path_clone = conversation_version_path.clone();
    let conversation_version_body_for_get = conversation_version_body.clone();
    server.mock(move |when, then| {
        when.method(GET)
            .path(conversation_version_path_clone.as_str());
        then.status(200)
            .json_body(conversation_version_body_for_get);
    });

    let conversation_messages_path_clone = conversation_messages_path.clone();
    server.mock(move |when, then| {
        when.method(POST)
            .path(conversation_messages_path_clone.as_str());
        then.status(200).json_body(json!({
            "type": "future-event-kind",
            "content": "from the future",
            "extra": "send-message"
        }));
    });

    let conversation_close_path_clone = conversation_close_path.clone();
    server.mock(move |when, then| {
        when.method(POST)
            .path(conversation_close_path_clone.as_str());
        then.status(200).json_body(conversation_record_for_close);
    });

    let login_request = LoginRequest::new(
        "gho_forward_compat".to_string(),
        "ghr_forward_compat".to_string(),
    );
    let (login_token, _login_client) = unauth_client.login(&login_request).await?;
    assert_eq!(login_token, "login-token");

    // Job endpoints
    use hydra_common::api::v1::sessions::{
        AgentSpec as ApiAgentSpec, MountSpec as ApiMountSpec, SessionMode as ApiSessionMode,
    };
    let create_session_request = CreateSessionRequest {
        mode: ApiSessionMode::Headless,
        agent_config: ApiAgentSpec::Adhoc {
            system_prompt: "test prompt".to_string(),
            mcp_config: None,
        },
        model: None,
        mount_spec: ApiMountSpec::default(),
        image: None,
        env_vars: HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
    };
    let created_session = client.create_session(&create_session_request).await?;
    assert_eq!(created_session.session_id, job_id);

    let jobs = client
        .list_sessions(&SearchSessionsQuery::default())
        .await?;
    let listed_job = jobs.sessions.first().expect("job from list");
    // Summary records do not include context; verify core summary fields.
    assert_eq!(listed_job.session_id, job_id);

    // PR-F dropped `Session.context` from the wire shape. The forward-compat
    // fixture above still includes a (now-unknown) `context` field with an
    // exotic variant; the client must tolerate it (serde silently ignores
    // unknown fields). The assertion below just verifies the fetch succeeds.
    let fetched_session = client.get_session(&job_id).await?;
    assert_eq!(fetched_session.session_id, job_id);

    let kill_response = client.kill_session(&job_id).await?;
    assert_eq!(kill_response.session_id, job_id);

    let mut logs = client
        .get_session_logs(&job_id, &LogsQuery::default())
        .await?;
    let mut collected = Vec::new();
    while let Some(item) = logs.next().await {
        collected.push(item?);
    }
    assert_eq!(collected.len(), 2);
    assert!(collected[1].contains("second log line"));

    let status_response = client
        .set_session_status(
            &job_id,
            &SessionStatusUpdate::Failed {
                reason: "test".to_string(),
            },
        )
        .await?;
    assert!(matches!(status_response.status, Status::Unknown));

    // Verify the job can still be fetched after a status update
    let fetched_session_after_status = client.get_session(&job_id).await?;
    assert_eq!(fetched_session_after_status.session_id, job_id);

    let context = client.get_session_context(&job_id).await?;
    let bundle_item = context
        .mounts
        .first()
        .expect("mounts must include at least one item");
    let hydra_common::sessions::MountItem::Bundle { bundle, .. } = bundle_item else {
        panic!("expected Bundle item first, got {bundle_item:?}");
    };
    assert!(matches!(bundle, Bundle::Unknown));

    // Issues
    let input = IssueInput::new(
        IssueType::Bug,
        "Test Title".to_string(),
        "desc".to_string(),
        Username::from("creator"),
        "progress".to_string(),
        status("open"),
        ProjectId::default_project(),
        Some(hydra_common::principal::Principal::User {
            name: hydra_common::api::v1::users::Username::try_new("assignee").unwrap(),
        }),
        None,
        vec![],
        vec![],
        false,
        None,
        None,
        None,
    );
    let issue_request = UpsertIssueRequest::new(input, None);

    let created_issue = client.create_issue(&issue_request).await?;
    assert_eq!(created_issue.issue_id, issue_id);

    let updated_issue = client.update_issue(&issue_id, &issue_request).await?;
    assert_eq!(updated_issue.issue_id, issue_id);

    let fetched_issue = client.get_issue(&issue_id, false).await?;
    // PR 3 wire change: status is a newtyped string (`StatusKey`). The
    // forward-compat payload's `"on-hold"` value now round-trips
    // verbatim instead of being collapsed into the `Unknown` enum variant.
    assert_eq!(fetched_issue.issue.status.key.as_str(), "on-hold");
    assert!(matches!(fetched_issue.issue.issue_type, IssueType::Unknown));
    assert!(matches!(
        fetched_issue
            .issue
            .dependencies
            .first()
            .map(|d| d.dependency_type),
        Some(IssueDependencyType::Unknown)
    ));

    let list_issues = client.list_issues(&SearchIssuesQuery::default()).await?;
    assert_eq!(list_issues.issues.len(), 1);

    // Patches
    let patch = Patch::new(
        "title".to_string(),
        "desc".to_string(),
        "diff".to_string(),
        PatchStatus::Open,
        false,
        Username::from("test-creator"),
        vec![],
        repo_name.clone(),
        None,
        false,
        None,
        None,
        None,
    );
    let upsert_patch = UpsertPatchRequest::new(patch.into());

    let created_patch = client.create_patch(&upsert_patch).await?;
    assert_eq!(created_patch.patch_id, patch_id);

    let updated_patch = client.update_patch(&patch_id, &upsert_patch).await?;
    assert_eq!(updated_patch.patch_id, patch_id);

    let fetched_patch = client.get_patch(&patch_id).await?;
    assert!(matches!(fetched_patch.patch.status, PatchStatus::Unknown));
    if let Some(ci_status) = fetched_patch.patch.github.and_then(|github| github.ci) {
        assert!(matches!(ci_status.state, GithubCiState::Unknown));
    }

    let patches = client.list_patches(&SearchPatchesQuery::default()).await?;
    assert_eq!(patches.patches.len(), 1);

    // Projects
    let upsert_project = UpsertProjectRequest::new(
        ProjectKey::try_new("future-project").unwrap(),
        "Future Project".to_string(),
    );

    let created_project = client.create_project(&upsert_project).await?;
    assert_eq!(created_project.project_id, project_id);

    let project_ref = ProjectRef::Id(project_id.clone());

    // Add a status via the per-status route so the read-back below
    // sees one entry.
    let status = StatusDefinition::new(
        StatusKey::try_new("open").unwrap(),
        "Open".to_string(),
        "#abcdef".parse().unwrap(),
        false,
        false,
        false,
        None,
    );
    client.create_project_status(&project_ref, &status).await?;

    let updated_project = client.update_project(&project_ref, &upsert_project).await?;
    assert_eq!(updated_project.project_id, project_id);

    let fetched_project = client.get_project(&project_ref).await?;
    assert_eq!(fetched_project.project_id, project_id);

    let listed_projects = client.list_projects().await?;
    assert_eq!(listed_projects.projects.len(), 1);

    let statuses = client.get_project_statuses(&project_ref).await?;
    assert_eq!(statuses.statuses.len(), 1);

    let updated_status = StatusDefinition::new(
        StatusKey::try_new("open").unwrap(),
        "Renamed".to_string(),
        "#abcdef".parse().unwrap(),
        false,
        false,
        false,
        None,
    );
    client
        .update_project_status(
            &project_ref,
            &StatusKey::try_new("open").unwrap(),
            &updated_status,
        )
        .await?;
    client
        .delete_project_status(&project_ref, &StatusKey::try_new("open").unwrap())
        .await?;

    let deleted_project = client.delete_project(&project_ref).await?;
    assert_eq!(deleted_project.project_id, project_id);

    // Documents
    let document = Document::new(
        "forward doc".to_string(),
        "# Runbook".to_string(),
        Some("docs/runbook.md".to_string()),
        false,
    )
    .unwrap();
    let upsert_document = UpsertDocumentRequest::new(document);

    let created_document = client.create_document(&upsert_document).await?;
    assert_eq!(created_document.document_id, document_id);

    let updated_document = client
        .update_document(&document_id, &upsert_document)
        .await?;
    assert_eq!(updated_document.document_id, document_id);

    let fetched_document = client.get_document(&document_id, false).await?;
    assert_eq!(fetched_document.document_id, document_id);
    assert_eq!(
        fetched_document.document.path.as_deref(),
        Some("/docs/runbook.md")
    );

    let documents = client
        .list_documents(&SearchDocumentsQuery::new(
            Some("runbook".to_string()),
            Some("docs/".to_string()),
            None,
            None,
        ))
        .await?;
    assert_eq!(documents.documents.len(), 1);

    let versions = client.list_document_versions(&document_id).await?;
    assert_eq!(versions.versions.len(), 1);
    let version_number = versions.versions[0].version;
    let document_version = client
        .get_document_version(
            &document_id,
            RelativeVersionNumber::new(version_number as i64),
        )
        .await?;
    assert_eq!(document_version.version, version_number);

    // Repositories
    let repo_config = Repository::new(
        "https://example.com/repo.git".to_string(),
        Some("main".to_string()),
        None,
    );
    let repo_create = CreateRepositoryRequest::new(repo_name.clone(), repo_config.clone());
    let repo_update = UpdateRepositoryRequest::new(repo_config);

    let created_repo = client.create_repository(&repo_create).await?;
    assert_eq!(created_repo.repository.name, repo_name);

    let updated_repo = client.update_repository(&repo_name, &repo_update).await?;
    assert_eq!(updated_repo.repository.name, repo_name);

    let repos = client
        .list_repositories(&SearchRepositoriesQuery::default())
        .await?;
    assert_eq!(repos.repositories.len(), 1);

    let github_token = client.get_github_token().await?;
    assert_eq!(github_token, "gho_forward_compat");

    let whoami = client.whoami().await?;
    assert!(matches!(
        whoami.actor,
        ActorIdentity::User { username: ref found } if found == &username
    ));

    // Conversations
    let create_conversation_request = CreateConversationRequest {
        message: Some("hello future".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created_conversation = client
        .create_conversation(&create_conversation_request)
        .await?;
    assert_eq!(created_conversation.conversation_id, conversation_id);
    assert!(matches!(
        created_conversation.status,
        ConversationStatus::Unknown
    ));

    let listed_conversations = client
        .list_conversations(&SearchConversationsQuery::default())
        .await?;
    assert_eq!(listed_conversations.len(), 1);
    assert!(matches!(
        listed_conversations[0].status,
        ConversationStatus::Unknown
    ));

    let fetched_conversation = client.get_conversation(&conversation_id).await?;
    assert_eq!(fetched_conversation.conversation_id, conversation_id);
    assert!(matches!(
        fetched_conversation.status,
        ConversationStatus::Unknown
    ));

    let updated_conversation = client
        .update_conversation(
            &conversation_id,
            &UpdateConversationRequest {
                title: Some("future title".to_string()),
            },
        )
        .await?;
    assert_eq!(updated_conversation.conversation_id, conversation_id);
    assert!(matches!(
        updated_conversation.status,
        ConversationStatus::Unknown
    ));

    let deleted_conversation = client.delete_conversation(&conversation_id).await?;
    assert_eq!(deleted_conversation.conversation_id, conversation_id);
    assert!(matches!(
        deleted_conversation.status,
        ConversationStatus::Unknown
    ));

    let conversation_versions = client.get_conversation_versions(&conversation_id).await?;
    assert_eq!(conversation_versions.len(), 1);
    assert!(matches!(
        conversation_versions[0].item.status,
        ConversationStatus::Unknown
    ));

    let single_conversation_version = client
        .get_conversation_version(&conversation_id, RelativeVersionNumber::new(2))
        .await?;
    assert_eq!(
        single_conversation_version.item.conversation_id,
        conversation_id
    );
    assert!(matches!(
        single_conversation_version.item.status,
        ConversationStatus::Unknown
    ));

    let send_message_response = client
        .send_message(
            &conversation_id,
            &SendMessageRequest {
                content: "ping".to_string(),
            },
        )
        .await?;
    assert!(matches!(
        send_message_response,
        hydra_common::api::v1::sessions::SessionEvent::Unknown
    ));

    let closed_conversation = client.close_conversation(&conversation_id).await?;
    assert_eq!(closed_conversation.conversation_id, conversation_id);
    assert!(matches!(
        closed_conversation.status,
        ConversationStatus::Unknown
    ));

    // Merge queue
    let merge_queue = client.get_merge_queue(&repo_name, "main").await?;
    assert_eq!(merge_queue.patches, vec![patch_id.clone()]);

    let enqueued_queue = client
        .enqueue_merge_patch(&repo_name, "main", &patch_id)
        .await?;
    assert_eq!(enqueued_queue.patches, vec![patch_id.clone()]);

    // Agents and GitHub
    let agents = client.list_agents().await?;
    assert_eq!(agents.agents.len(), 1);

    let github_client = unauth_client.get_github_app_client_id().await?;
    assert_eq!(github_client.client_id, "abc123");

    // Ensure unknown job status variants remain deserializable.
    let delayed_status: SessionStatusUpdate =
        serde_json::from_value(json!({ "status": "delayed" }))?;
    assert!(matches!(delayed_status, SessionStatusUpdate::Unknown));

    Ok(())
}

fn forward_status_log_json(now: DateTime<Utc>) -> Value {
    json!({
        "events": [
            { "created": { "at": now, "status": "paused", "note": "new-status" } },
            { "blocked": { "at": now, "reason": "maintenance" } },
            { "failed": { "at": now, "error": { "timeout": { "message": "slow" } }, "trail": "info" } }
        ],
        "tracker": "future"
    })
}

fn forward_job_json(job_id: &SessionId, status_log: Value) -> Value {
    json!({
        "job_id": job_id,
        "version": 0,
        "timestamp": Utc::now(),
        "task": {
            "prompt": "future job",
            "context": {
                "type": "archive_bundle",
                "url": "https://example.com/archive.tar.gz",
                "rev": "v2",
                "experimental": true
            },
            "creator": "future-creator",
            "env_vars": { "DEBUG": "true" },
            "mount_spec": {
                "working_dir": "repo",
                "mounts": [
                    {
                        "type": "bundle",
                        "target": "repo",
                        "bundle": { "type": "none" },
                        "session_id": job_id,
                    },
                    { "type": "documents", "target": "documents" }
                ]
            },
            "mode": { "type": "headless", "prompt": "future job" },
            "extra": "task"
        },
        "notes": "note",
        "status_log": status_log,
        "unexpected": "job"
    })
}

fn forward_issue_json(issue_id: &IssueId, dependency_id: &IssueId, patch_id: &PatchId) -> Value {
    json!({
        "issue_id": issue_id,
        "version": 0,
        "timestamp": Utc::now(),
        "issue": {
            "type": "epic",
            "description": "future issue",
            "creator": "alice",
            "progress": "blocked",
            "status": {
                "key": "on-hold",
                "label": "On hold",
                "color": "#abcdef",
                "unblocks_parents": false,
                "unblocks_dependents": false,
                "cascades_to_children": false,
                "future": "status-field"
            },
            "project_id": "j-defaul",
            "assignee": {"Agent": {"name": "robot"}},
            "dependencies": [
                { "type": "relates-to", "issue_id": dependency_id }
            ],
            "patches": [patch_id],
            "surprise": "field"
        },
        "extra": "issue",
        "creation_time": Utc::now()
    })
}

fn forward_project_json(project_id: &ProjectId) -> Value {
    json!({
        "project_id": project_id,
        "version": 0,
        "project": {
            "key": "future-project",
            "name": "Future Project",
            "statuses": [
                {
                    "key": "open",
                    "label": "Open",
                    "color": "#abcdef",
                    "unblocks_parents": false,
                    "unblocks_dependents": false,
                    "cascades_to_children": false,
                    "future": "status-field"
                }
            ],
            "creator": "test-creator",
            "future": "project-field"
        },
        "extra": "project"
    })
}

fn forward_patch_json(patch_id: &PatchId, repo_name: &RepoName, now: DateTime<Utc>) -> Value {
    json!({
        "patch_id": patch_id,
        "version": 0,
        "timestamp": Utc::now(),
        "patch": {
            "title": "future patch",
            "description": "desc",
            "diff": "diff",
            "status": "stale",
            "is_automatic_backup": false,
            "creator": "test-creator",
            "reviews": [
                { "contents": "looks ok", "is_approved": true, "author": "reviewer", "submitted_at": now, "confidence": "medium" }
            ],
            "service_repo_name": repo_name,
            "github": {
                "owner": "dourolabs",
                "repo": "hydra",
                "number": 1,
                "head_ref": "future-head",
                "base_ref": "main",
                "url": "https://example.com/pr/1",
                "ci": {
                    "state": "flaky",
                    "failure": {
                        "name": "lint",
                        "summary": "lint failed",
                        "details_url": "https://example.com/lint",
                        "retry_after": 30
                    },
                    "extra": true
                },
                "unexpected": "field"
            },
            "bonus": "field"
        },
        "creation_time": now
    })
}

fn forward_patch_summary_json(patch_id: &PatchId, repo_name: &RepoName) -> Value {
    json!({
        "patch_id": patch_id,
        "version": 0,
        "timestamp": Utc::now(),
        "patch": {
            "title": "future patch",
            "status": "stale",
            "is_automatic_backup": false,
            "creator": "test-creator",
            "review_summary": { "count": 1, "approved": true },
            "service_repo_name": repo_name,
            "github": {
                "owner": "dourolabs",
                "repo": "hydra",
                "number": 1,
                "head_ref": "future-head",
                "base_ref": "main",
                "url": "https://example.com/pr/1",
                "ci": {
                    "state": "flaky",
                    "failure": {
                        "name": "lint",
                        "summary": "lint failed",
                        "details_url": "https://example.com/lint",
                        "retry_after": 30
                    },
                    "extra": true
                },
                "unexpected": "field"
            },
            "bonus": "field"
        },
        "creation_time": Utc::now()
    })
}

fn forward_document_json(document_id: &DocumentId, job_id: &SessionId) -> Value {
    json!({
        "document_id": document_id,
        "version": 0,
        "timestamp": Utc::now(),
        "document": {
            "title": "forward doc",
            "body_markdown": "# Runbook",
            "path": "docs/runbook.md",
            "created_by": job_id,
            "extra": "document"
        },
        "note": "document",
        "creation_time": Utc::now()
    })
}

fn forward_document_version_json(
    document_id: &DocumentId,
    version: u64,
    timestamp: DateTime<Utc>,
    job_id: &SessionId,
) -> Value {
    json!({
        "document_id": document_id,
        "version": version,
        "timestamp": timestamp,
        "document": {
            "title": format!("forward doc v{version}"),
            "body_markdown": "# Body",
            "path": "docs/runbook.md",
            "created_by": job_id,
            "extra": "document-version"
        },
        "note": "document-version",
        "creation_time": timestamp
    })
}

fn forward_repo_info(repo_name: &RepoName) -> Value {
    json!({
        "name": repo_name,
        "repository": {
            "remote_url": "https://example.com/repo.git",
            "default_branch": "main",
            "default_image": "ghcr.io/dourolabs/hydra:main"
        },
        "sync": "on"
    })
}

fn forward_conversation_json(conversation_id: &ConversationId) -> Value {
    json!({
        "conversation_id": conversation_id,
        "title": "future chat",
        "agent_name": "claude",
        "status": "archived",
        "creator": "future-user",
        "session_settings": {
            "repo_name": "dourolabs/hydra",
            "future": "settings-field"
        },
        "spawned_from": null,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "extra": "conversation"
    })
}

fn forward_conversation_summary_json(conversation_id: &ConversationId) -> Value {
    json!({
        "conversation_id": conversation_id,
        "title": "future chat",
        "agent_name": "claude",
        "status": "archived",
        "event_count": 7,
        "last_event_preview": "future preview",
        "creator": "future-user",
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "extra": "conversation-summary"
    })
}

fn forward_conversation_version_json(
    conversation_id: &ConversationId,
    version: u64,
    timestamp: DateTime<Utc>,
) -> Value {
    json!({
        "item": forward_conversation_json(conversation_id),
        "version": version,
        "timestamp": timestamp,
        "creation_time": timestamp,
        "future": "version-field"
    })
}

fn forward_worker_context_json() -> Value {
    json!({
        "session_id": "s-forwardct",
        "mode_kind": "headless",
        "mounts": [
            {
                "type": "bundle",
                "target": "repo",
                "bundle": { "type": "workspace_snapshot", "path": "/tmp/work", "details": "future" },
                "session_id": "s-forwardct",
            },
            {"type": "documents", "target": "documents"}
        ],
        "working_dir": "repo",
        "model": "future-model",
        "mcp_config": null,
        "idle_timeout_secs": null,
        "resolved_env": { "foo": "bar" },
        "github_token": null,
        "note": "context"
    })
}
