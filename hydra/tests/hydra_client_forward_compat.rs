use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use httpmock::prelude::*;
use httpmock::Method::PATCH;
use hydra::client::{HydraClient, HydraClientUnauthenticated};
use hydra_common::{
    agents::UpsertAgentRequest,
    api::v1::events::EventsQuery,
    api::v1::merge_check::MergeCheckResponse,
    api::v1::projects::{
        ProjectKey, ProjectRef, StatusDefinition, StatusKey, UpsertProjectRequest,
    },
    api::v1::relations::{CreateRelationRequest, ListRelationsRequest, RemoveRelationRequest},
    api::v1::triggers::{
        Action as TriggerAction, CreateIssueAction, Schedule as TriggerSchedule,
        SearchTriggersQuery, UpsertTriggerRequest,
    },
    conversations::{
        ConversationStatus, CreateConversationRequest, SearchConversationsQuery,
        SendMessageRequest, UpdateConversationRequest,
    },
    documents::{Document, SearchDocumentsQuery, UpsertDocumentRequest},
    issues::{
        IssueDependencyType, IssueInput, IssueType, SearchIssuesQuery, SubmitFormRequest,
        UpsertIssueRequest,
    },
    labels::{Label, SearchLabelsQuery, UpsertLabelRequest},
    login::{DevicePollStatus, LoginRequest},
    logs::LogsQuery,
    patches::{GithubCiState, Patch, PatchStatus, SearchPatchesQuery, UpsertPatchRequest},
    repositories::{
        CreateRepositoryRequest, Repository, SearchRepositoriesQuery, UpdateRepositoryRequest,
    },
    session_status::SessionStatusUpdate,
    sessions::{Bundle, CreateSessionRequest, SearchSessionsQuery},
    task_status::Status,
    test_utils::status::status,
    users::{SearchUsersQuery, Username},
    whoami::ActorIdentity,
    ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, ProjectId,
    RelativeVersionNumber, RepoName, SessionId, TriggerId,
};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};
use tempfile::NamedTempFile;

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
    let trigger_id = TriggerId::new();
    let label_id = LabelId::new();
    let repo_name = RepoName::new("dourolabs", "hydra")?;
    let username: Username = "future-user".into();
    let label_object_id: HydraId = issue_id.clone().into();
    let relation_source_id: HydraId = issue_id.clone().into();
    let relation_target_id: HydraId = patch_id.clone().into();

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
    let session_versions_path = format!("{job_path}/versions");
    let session_version_path = format!("{session_versions_path}/1");
    let issue_path = format!("/v1/issues/{issue_id}");
    let issue_versions_path = format!("{issue_path}/versions");
    let issue_version_path = format!("{issue_versions_path}/1");
    let patch_path = format!("/v1/patches/{patch_id}");
    let patch_versions_path = format!("{patch_path}/versions");
    let patch_version_path = format!("{patch_versions_path}/1");
    let trigger_path = format!("/v1/triggers/{trigger_id}");
    let trigger_versions_path = format!("{trigger_path}/versions");
    let trigger_version_path = format!("{trigger_versions_path}/1");
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
    let trigger_record_body = forward_trigger_json(&trigger_id, &project_id, now);
    let trigger_record_for_get = trigger_record_body.clone();
    let trigger_record_for_list = trigger_record_body.clone();
    let trigger_record_for_delete = trigger_record_body.clone();
    let trigger_id_for_create = trigger_id.clone();
    let trigger_id_for_update = trigger_id.clone();
    let agent_name = "future-agent";
    let secret_name = "FUTURE_SECRET";
    let user_path = format!("/v1/users/{username}");
    let user_secrets_path = format!("/v1/users/{username}/secrets");
    let user_secret_path = format!("/v1/users/{username}/secrets/{secret_name}");
    let agent_path = format!("/v1/agents/{agent_name}");
    let user_record_body = forward_user_json(&username);
    let user_record_for_get = user_record_body.clone();
    let user_record_for_list = user_record_body.clone();
    let agent_record_body = forward_agent_json(agent_name);
    let agent_record_for_get = agent_record_body.clone();
    let agent_record_for_create = agent_record_body.clone();
    let agent_record_for_update = agent_record_body.clone();
    let agent_record_for_delete = agent_record_body.clone();
    let issue_record_for_delete = issue_record_body.clone();
    let patch_record_for_delete = patch_record_body.clone();
    let document_version_body_for_delete = document_version_body.clone();
    let repository_body_for_delete = repository_body.clone();
    let issue_id_for_submit_form = issue_id.clone();
    let patch_id_for_merge_check = patch_id.clone();
    let patch_id_for_asset = patch_id.clone();
    let label_record_body = forward_label_json(&label_id, now);
    let label_record_for_list = label_record_body.clone();
    let relation_record_body = forward_relation_json(&relation_source_id, &relation_target_id, now);
    let relation_record_for_list = relation_record_body.clone();
    let label_object_path = format!("/v1/labels/{label_id}/objects/{label_object_id}");
    let merge_queue_path = format!(
        "/v1/merge-queues/{}/{}/main/patches",
        repo_name.organization, repo_name.repo
    );
    let device_session_id = "device-future-id";
    let proxy_port: u16 = 7777;
    let proxy_targets_path = format!("/v1/sessions/{job_id}/proxy-targets");
    let proxy_target_delete_path = format!("{proxy_targets_path}/{proxy_port}");
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

    let session_versions_path_clone = session_versions_path.clone();
    let job_record_body_for_list_versions = job_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(session_versions_path_clone.as_str());
        then.status(200).json_body(json!({
            "versions": [job_record_body_for_list_versions.clone()],
            "future": "session-versions"
        }));
    });

    let session_version_path_clone = session_version_path.clone();
    let job_record_body_for_get_version = job_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(session_version_path_clone.as_str());
        then.status(200)
            .json_body(job_record_body_for_get_version.clone());
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

    let issue_versions_path_clone = issue_versions_path.clone();
    let issue_record_for_list_versions = issue_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(issue_versions_path_clone.as_str());
        then.status(200).json_body(json!({
            "versions": [issue_record_for_list_versions.clone()],
            "future": "issue-versions"
        }));
    });

    let issue_version_path_clone = issue_version_path.clone();
    let issue_record_for_get_version = issue_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(issue_version_path_clone.as_str());
        then.status(200)
            .json_body(issue_record_for_get_version.clone());
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

    let patch_versions_path_clone = patch_versions_path.clone();
    let patch_record_for_list_versions = patch_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(patch_versions_path_clone.as_str());
        then.status(200).json_body(json!({
            "versions": [patch_record_for_list_versions.clone()],
            "future": "patch-versions"
        }));
    });

    let patch_version_path_clone = patch_version_path.clone();
    let patch_record_for_get_version = patch_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(patch_version_path_clone.as_str());
        then.status(200)
            .json_body(patch_record_for_get_version.clone());
    });

    let trigger_versions_path_clone = trigger_versions_path.clone();
    let trigger_record_for_list_clone = trigger_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path(trigger_versions_path_clone.as_str());
        then.status(200).json_body(json!({
            "versions": [trigger_record_for_list_clone.clone()],
            "future": "trigger-versions"
        }));
    });

    let trigger_version_path_clone = trigger_version_path.clone();
    let trigger_record_for_get_version = trigger_record_body.clone();
    server.mock(move |when, then| {
        when.method(GET).path(trigger_version_path_clone.as_str());
        then.status(200)
            .json_body(trigger_record_for_get_version.clone());
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

    // POST /v1/triggers
    let trigger_id_for_create_clone = trigger_id_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/triggers");
        then.status(200).json_body(json!({
            "trigger_id": trigger_id_for_create_clone,
            "version": 1,
            "extra": "create-trigger"
        }));
    });

    // PUT /v1/triggers/:trigger_id
    let trigger_update_path = trigger_path.clone();
    let trigger_id_for_update_clone = trigger_id_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(trigger_update_path.as_str());
        then.status(200).json_body(json!({
            "trigger_id": trigger_id_for_update_clone,
            "version": 2,
            "extra": "update-trigger"
        }));
    });

    // GET /v1/triggers/:trigger_id
    let trigger_get_path = trigger_path.clone();
    let trigger_record_for_get_clone = trigger_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(trigger_get_path.as_str());
        then.status(200)
            .json_body(trigger_record_for_get_clone.clone());
    });

    // GET /v1/triggers
    let trigger_record_for_list_clone = trigger_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/triggers");
        then.status(200).json_body(json!({
            "triggers": [trigger_record_for_list_clone.clone()],
            "extra": "list-triggers"
        }));
    });

    // DELETE /v1/triggers/:trigger_id
    let trigger_delete_path = trigger_path.clone();
    let trigger_record_for_delete_clone = trigger_record_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(trigger_delete_path.as_str());
        then.status(200)
            .json_body(trigger_record_for_delete_clone.clone());
    });

    // GET /v1/users
    let user_record_for_list_clone = user_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/users");
        then.status(200).json_body(json!({
            "users": [user_record_for_list_clone.clone()],
            "extra": "list-users"
        }));
    });

    // GET /v1/users/:username
    let user_get_path = user_path.clone();
    let user_record_for_get_clone = user_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(user_get_path.as_str());
        then.status(200)
            .json_body(user_record_for_get_clone.clone());
    });

    // GET /v1/users/:username/secrets
    let user_secrets_list_path = user_secrets_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(user_secrets_list_path.as_str());
        then.status(200).json_body(json!({
            "secrets": ["FOO_KEY", "BAR_KEY"],
            "extra": "list-secrets"
        }));
    });

    // PUT /v1/users/:username/secrets/:name
    let user_secret_set_path = user_secret_path.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(user_secret_set_path.as_str());
        then.status(200).json_body(json!({ "extra": "set-secret" }));
    });

    // DELETE /v1/users/:username/secrets/:name
    let user_secret_delete_path = user_secret_path.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(user_secret_delete_path.as_str());
        then.status(200)
            .json_body(json!({ "extra": "delete-secret" }));
    });

    // POST /v1/agents (list_agents is already mocked above)
    let agent_record_for_create_clone = agent_record_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/agents");
        then.status(200)
            .json_body(agent_record_for_create_clone.clone());
    });

    // PUT /v1/agents/:name
    let agent_update_path = agent_path.clone();
    let agent_record_for_update_clone = agent_record_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(agent_update_path.as_str());
        then.status(200)
            .json_body(agent_record_for_update_clone.clone());
    });

    // GET /v1/agents/:name
    let agent_get_path = agent_path.clone();
    let agent_record_for_get_clone = agent_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(agent_get_path.as_str());
        then.status(200)
            .json_body(agent_record_for_get_clone.clone());
    });

    // DELETE /v1/agents/:name
    let agent_delete_path = agent_path.clone();
    let agent_record_for_delete_clone = agent_record_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(agent_delete_path.as_str());
        then.status(200)
            .json_body(agent_record_for_delete_clone.clone());
    });

    // DELETE /v1/repositories/:org/:repo
    let delete_repo_path = repo_path.clone();
    let repository_body_for_delete_clone = repository_body_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(delete_repo_path.as_str());
        then.status(200).json_body(json!({
            "repository": repository_body_for_delete_clone.clone(),
            "future": "delete-repo"
        }));
    });

    // DELETE /v1/issues/:issue_id
    let delete_issue_path = issue_path.clone();
    let issue_record_for_delete_clone = issue_record_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(delete_issue_path.as_str());
        then.status(200)
            .json_body(issue_record_for_delete_clone.clone());
    });

    // DELETE /v1/patches/:patch_id
    let delete_patch_path = patch_path.clone();
    let patch_record_for_delete_clone = patch_record_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(delete_patch_path.as_str());
        then.status(200)
            .json_body(patch_record_for_delete_clone.clone());
    });

    // DELETE /v1/documents/:document_id
    let delete_document_path = document_path.clone();
    let document_version_body_for_delete_clone = document_version_body_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(delete_document_path.as_str());
        then.status(200)
            .json_body(document_version_body_for_delete_clone.clone());
    });

    // POST /v1/patches/:patch_id/merge_check — success body. `MergeCheckOk`
    // has no enum fields to test forward-compat on, so only the top-level
    // unknown field exercises serde's tolerance here.
    let merge_check_path = format!("/v1/patches/{patch_id_for_merge_check}/merge_check");
    server.mock(move |when, then| {
        when.method(POST).path(merge_check_path.as_str());
        then.status(200)
            .json_body(json!({ "ok": true, "future": "merge-check" }));
    });

    // POST /v1/patches/:patch_id/assets — server returns a JSON envelope with
    // the uploaded asset URL plus unknown extras.
    let patch_asset_path = format!("/v1/patches/{patch_id_for_asset}/assets");
    server.mock(move |when, then| {
        when.method(POST).path(patch_asset_path.as_str());
        then.status(200).json_body(json!({
            "asset_url": "https://example.com/assets/forward-compat",
            "future": "asset"
        }));
    });

    // POST /v1/issues/:issue_id/actions — submit form response envelope.
    // `FormResponse` has no `Unknown`-style enum variants of its own;
    // forward-compat tolerance is exercised via unknown extras at the
    // top level and on the embedded `form_response`.
    let submit_form_path = format!("/v1/issues/{issue_id_for_submit_form}/actions");
    let issue_id_for_submit_form_response = issue_id_for_submit_form.clone();
    server.mock(move |when, then| {
        when.method(POST).path(submit_form_path.as_str());
        then.status(200).json_body(json!({
            "issue_id": issue_id_for_submit_form_response,
            "version": 7,
            "form_response": {
                "action_id": "future-action",
                "actor": { "User": { "name": "future-user" } },
                "values": {},
                "submitted_at": Utc::now(),
                "future": "form-response-field"
            },
            "future": "submit-form"
        }));
    });

    // GET /v1/labels — envelope with extra top-level field; embedded
    // LabelRecord carries unknown fields that serde should silently drop.
    let label_record_for_list_clone = label_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/labels");
        then.status(200).json_body(json!({
            "labels": [label_record_for_list_clone.clone()],
            "extra": "list-labels"
        }));
    });

    // POST /v1/labels — UpsertLabelResponse envelope with extra fields.
    let label_id_for_create = label_id.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/labels");
        then.status(200).json_body(json!({
            "label_id": label_id_for_create,
            "extra": "create-label"
        }));
    });

    // PUT /v1/labels/:label_id/objects/:object_id — `()`-return; surface
    // forward-compat by responding with a body the client never decodes.
    let label_object_path_for_put = label_object_path.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(label_object_path_for_put.as_str());
        then.status(200)
            .json_body(json!({ "extra": "add-label-association" }));
    });

    // DELETE /v1/labels/:label_id/objects/:object_id — `()`-return.
    let label_object_path_for_delete = label_object_path.clone();
    server.mock(move |when, then| {
        when.method(DELETE)
            .path(label_object_path_for_delete.as_str());
        then.status(200)
            .json_body(json!({ "extra": "remove-label-association" }));
    });

    // POST /v1/relations — returns `bool` based on HTTP status (201 ⇒ true).
    // No body is decoded; emit a JSON envelope anyway to confirm the client
    // tolerates an unexpected payload alongside the status check.
    server.mock(|when, then| {
        when.method(POST).path("/v1/relations");
        then.status(201)
            .json_body(json!({ "created": true, "extra": "create-relation" }));
    });

    // DELETE /v1/relations — decodes RemoveRelationResponse { removed: bool };
    // include extra top-level field so serde must skip it.
    server.mock(|when, then| {
        when.method(DELETE).path("/v1/relations");
        then.status(200)
            .json_body(json!({ "removed": true, "extra": "remove-relation" }));
    });

    // GET /v1/relations — envelope with extra top-level field; embedded
    // RelationResponse carries unknown fields.
    let relation_record_for_list_clone = relation_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/relations");
        then.status(200).json_body(json!({
            "relations": [relation_record_for_list_clone.clone()],
            "extra": "list-relations"
        }));
    });

    // POST /v1/login/device/start (unauth) — device flow init.
    let device_start_body = forward_device_start_json(device_session_id);
    server.mock(move |when, then| {
        when.method(POST).path("/v1/login/device/start");
        then.status(200).json_body(device_start_body.clone());
    });

    // POST /v1/login/device/poll (unauth) — device flow poll. The status
    // carries an unknown `DevicePollStatus` variant so the embedded enum
    // exercises forward-compat.
    let device_session_id_for_poll = device_session_id.to_string();
    server.mock(move |when, then| {
        when.method(POST)
            .path("/v1/login/device/poll")
            .json_body(json!({ "device_session_id": device_session_id_for_poll.clone() }));
        then.status(200).json_body(forward_device_poll_json());
    });

    // GET /v1/sessions/:session_id/proxy-targets — list.
    let proxy_list_path = proxy_targets_path.clone();
    let proxy_list_body = forward_proxy_target_list_json(proxy_port);
    server.mock(move |when, then| {
        when.method(GET).path(proxy_list_path.as_str());
        then.status(200).json_body(proxy_list_body.clone());
    });

    // POST /v1/sessions/:session_id/proxy-targets — upsert. Returns an
    // unexpected body to assert the client ignores it (response type is
    // `()`).
    let proxy_upsert_path = proxy_targets_path.clone();
    let proxy_port_for_upsert = proxy_port;
    server.mock(move |when, then| {
        when.method(POST)
            .path(proxy_upsert_path.as_str())
            .json_body(json!({
                "port": proxy_port_for_upsert,
                "ready_path": "/healthz"
            }));
        then.status(200)
            .json_body(json!({ "extra": "upsert-proxy-target" }));
    });

    // DELETE /v1/sessions/:session_id/proxy-targets/:port — empty/unknown
    // body must be tolerated since the client discards it.
    let proxy_delete_path = proxy_target_delete_path.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(proxy_delete_path.as_str());
        then.status(200)
            .json_body(json!({ "extra": "delete-proxy-target" }));
    });

    // GET /v1/events — SSE stream. Includes one event with an unknown event
    // type (skipped by the SSE parser without erroring) plus a known
    // `heartbeat` event whose payload carries extra unknown fields.
    server.mock(|when, then| {
        when.method(GET).path("/v1/events");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(concat!(
                "event: future_event_type\n",
                "id: 1\n",
                "data: {\"entity_type\":\"trigger\",\"entity_id\":\"t-abc\",\"version\":1,\"timestamp\":\"2026-01-01T00:00:00Z\",\"future\":\"field\"}\n",
                "\n",
                "event: heartbeat\n",
                "id: 2\n",
                "data: {\"server_time\":\"2026-01-01T00:00:00Z\",\"future\":\"field\"}\n",
                "\n",
            ));
    });

    let login_request = LoginRequest::new(
        "gho_forward_compat".to_string(),
        "ghr_forward_compat".to_string(),
    );
    let (login_token, _login_client) = unauth_client.login(&login_request).await?;
    assert_eq!(login_token, "login-token");

    // `login_with_http_client` shares the same `/v1/login` endpoint and
    // response shape — the existing mock above services this call too.
    let (login_token_with_http, _login_client_with_http) = unauth_client
        .login_with_http_client(HttpClient::new(), &login_request)
        .await?;
    assert_eq!(login_token_with_http, "login-token");

    // Device-code flow (unauth). The poll mock returns an unknown
    // `DevicePollStatus` variant; older clients must decode it as
    // `DevicePollStatus::Unknown` rather than erroring.
    let device_start = unauth_client.device_start().await?;
    assert_eq!(device_start.device_session_id, device_session_id);
    let device_poll = unauth_client.device_poll(device_session_id).await?;
    assert!(matches!(device_poll.status, DevicePollStatus::Unknown));

    // Job endpoints
    use hydra_common::api::v1::sessions::{
        AgentSpec as ApiAgentSpec, MountSpec as ApiMountSpec, SessionMode as ApiSessionMode,
        UpsertProxyTargetRequest,
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

    // Proxy targets (session-scoped).
    let proxy_targets = client.list_proxy_targets(&job_id).await?;
    assert_eq!(proxy_targets.targets.len(), 1);
    assert_eq!(proxy_targets.targets[0].port, proxy_port);

    client
        .upsert_proxy_target(
            &job_id,
            &UpsertProxyTargetRequest {
                port: proxy_port,
                ready_path: Some("/healthz".to_string()),
            },
        )
        .await?;

    client.delete_proxy_target(&job_id, proxy_port).await?;

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

    let submit_form_response = client
        .submit_form(
            &issue_id,
            &SubmitFormRequest::new("future-action".to_string(), HashMap::new()),
        )
        .await?;
    assert_eq!(submit_form_response.issue_id, issue_id);
    assert_eq!(
        submit_form_response.form_response.action_id,
        "future-action"
    );

    let deleted_issue = client.delete_issue(&issue_id).await?;
    assert_eq!(deleted_issue.issue_id, issue_id);
    assert!(matches!(deleted_issue.issue.issue_type, IssueType::Unknown));

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

    let merge_check_response = client.merge_check(&patch_id).await?;
    assert!(matches!(merge_check_response, MergeCheckResponse::Ok(_)));

    let asset_file = NamedTempFile::new()?;
    let asset_url = client
        .create_patch_asset(&patch_id, asset_file.path())
        .await?;
    assert!(!asset_url.is_empty());

    let deleted_patch = client.delete_patch(&patch_id).await?;
    assert_eq!(deleted_patch.patch_id, patch_id);
    assert!(matches!(deleted_patch.patch.status, PatchStatus::Unknown));

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

    let document_by_path = client
        .get_document_by_path("docs/runbook.md", false)
        .await?;
    assert_eq!(document_by_path.document_id, document_id);

    let deleted_document = client.delete_document(&document_id).await?;
    assert_eq!(deleted_document.document_id, document_id);

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

    let deleted_repo = client.delete_repository(&repo_name).await?;
    assert_eq!(deleted_repo.name, repo_name);

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

    // Triggers
    let trigger_request = UpsertTriggerRequest::new(
        true,
        TriggerSchedule::Cron {
            expression: "0 9 * * MON".to_string(),
            timezone: Some("UTC".to_string()),
        },
        vec![TriggerAction::CreateIssue(CreateIssueAction::new(
            IssueType::Task,
            "Daily triage".to_string(),
            "Run triage for {{ now.date }}".to_string(),
            None,
            project_id.clone(),
            StatusKey::try_new("open").unwrap(),
            Default::default(),
        ))],
        Username::from("test-creator"),
    );

    let created_trigger = client.create_trigger(&trigger_request).await?;
    assert_eq!(created_trigger.trigger_id, trigger_id);

    let updated_trigger = client.update_trigger(&trigger_id, &trigger_request).await?;
    assert_eq!(updated_trigger.trigger_id, trigger_id);

    let fetched_trigger = client.get_trigger(&trigger_id, false).await?;
    assert_eq!(fetched_trigger.trigger_id, trigger_id);
    let fetched_action = fetched_trigger
        .trigger
        .actions
        .first()
        .expect("fetched trigger has at least one action");
    let TriggerAction::CreateIssue(fetched_create_issue) = fetched_action;
    assert!(matches!(
        fetched_create_issue.issue_type,
        IssueType::Unknown
    ));

    let listed_triggers = client
        .list_triggers(&SearchTriggersQuery::default())
        .await?;
    assert_eq!(listed_triggers.triggers.len(), 1);
    let listed_action = listed_triggers.triggers[0]
        .trigger
        .actions
        .first()
        .expect("listed trigger has at least one action");
    let TriggerAction::CreateIssue(listed_create_issue) = listed_action;
    assert!(matches!(listed_create_issue.issue_type, IssueType::Unknown));

    let deleted_trigger = client.delete_trigger(&trigger_id).await?;
    assert_eq!(deleted_trigger.trigger_id, trigger_id);

    // Users + secrets. Neither `UserSummary` nor `ListSecretsResponse`
    // carries an enum member with an `Unknown` variant, so forward-compat
    // coverage here is unknown-extra-field tolerance plus a successful
    // decode. `set_user_secret` / `delete_user_secret` return `()`; the
    // successful await confirms the (extra-field-bearing) response body
    // decoded.
    let listed_users = client.list_users(&SearchUsersQuery::default()).await?;
    assert_eq!(listed_users.users.len(), 1);
    assert_eq!(listed_users.users[0].username, username);

    let fetched_user = client.get_user(username.as_str()).await?;
    assert_eq!(fetched_user.username, username);

    let listed_secrets = client.list_user_secrets(username.as_str()).await?;
    assert_eq!(listed_secrets.secrets.len(), 2);

    client
        .set_user_secret(username.as_str(), secret_name, "ssshhh")
        .await?;
    client
        .delete_user_secret(username.as_str(), secret_name)
        .await?;

    // Agents (CRUD; `list_agents` is covered above). `AgentRecord` and the
    // `AgentResponse` / `DeleteAgentResponse` envelopes carry no enum
    // members with an `Unknown` variant, so coverage here is the
    // unknown-extra-field tolerance plus a successful decode.
    let upsert_agent = UpsertAgentRequest::new(
        agent_name,
        "future agent prompt".to_string(),
        3,
        5,
        None,
        None,
        true,
        false,
        vec!["FUTURE_KEY".to_string()],
    );

    let created_agent = client.create_agent(&upsert_agent).await?;
    assert_eq!(created_agent.agent.name, agent_name);

    let updated_agent = client.update_agent(agent_name, &upsert_agent).await?;
    assert_eq!(updated_agent.agent.name, agent_name);

    let fetched_agent = client.get_agent(agent_name).await?;
    assert_eq!(fetched_agent.agent.name, agent_name);

    let deleted_agent = client.delete_agent(agent_name).await?;
    assert_eq!(deleted_agent.agent.name, agent_name);

    // Labels. Neither `Label` nor `LabelRecord` carries an enum with an
    // `Unknown` variant, so the forward-compat surface is purely about
    // serde silently dropping unknown top-level/inner fields.
    let listed_labels = client.list_labels(&SearchLabelsQuery::default()).await?;
    assert_eq!(listed_labels.labels.len(), 1);
    assert_eq!(listed_labels.labels[0].label_id, label_id);

    let upsert_label = UpsertLabelRequest::new(Label::new(
        "future-label".to_string(),
        Some("#abcdef".parse().unwrap()),
    ));
    let created_label = client.create_label(&upsert_label).await?;
    assert_eq!(created_label.label_id, label_id);

    client
        .add_label_association(&label_id, &label_object_id)
        .await?;
    client
        .remove_label_association(&label_id, &label_object_id)
        .await?;

    // Relations. `create_relation` returns its bool from the HTTP status
    // (201 = newly created) without decoding the body; `remove_relation`
    // decodes a `RemoveRelationResponse` envelope.
    let create_relation_request = CreateRelationRequest {
        source_id: relation_source_id.clone(),
        target_id: relation_target_id.clone(),
        rel_type: "future-rel".to_string(),
    };
    let created_relation = client.create_relation(&create_relation_request).await?;
    assert!(created_relation);

    let remove_relation_request = RemoveRelationRequest {
        source_id: relation_source_id.clone(),
        target_id: relation_target_id.clone(),
        rel_type: "future-rel".to_string(),
    };
    let removed_relation = client.remove_relation(&remove_relation_request).await?;
    assert!(removed_relation);

    let listed_relations = client
        .list_relations(&ListRelationsRequest::default())
        .await?;
    assert_eq!(listed_relations.relations.len(), 1);
    assert_eq!(listed_relations.relations[0].source_id, relation_source_id);

    // Events (SSE). The mock emits one block with an unknown event type
    // (the SSE parser skips it without erroring) and one `heartbeat` block
    // whose payload carries an extra unknown field. Driving the stream to
    // completion verifies that an older client can decode at least one
    // forward-compat event payload from a newer server.
    let mut events = client
        .subscribe_events(&EventsQuery::default(), None)
        .await?;
    let mut collected_events = Vec::new();
    while let Some(item) = events.next().await {
        collected_events.push(item?);
    }
    assert!(!collected_events.is_empty());
    let heartbeat = collected_events
        .iter()
        .find(|e| {
            matches!(
                e.event_type,
                hydra_common::api::v1::events::SseEventType::Heartbeat
            )
        })
        .expect("heartbeat event survives forward-compat payload");
    // The extra `future` field is silently ignored by serde.
    heartbeat.as_heartbeat()?;

    // Ensure unknown job status variants remain deserializable.
    let delayed_status: SessionStatusUpdate =
        serde_json::from_value(json!({ "status": "delayed" }))?;
    assert!(matches!(delayed_status, SessionStatusUpdate::Unknown));

    // Version-history methods. SessionVersionRecord has no `Unknown`
    // variant of its own; the existing get_session/get_issue/get_patch
    // assertions already cover the embedded-enum case, so here we just
    // verify the version-list/get pair decodes without error and that
    // the embedded payloads still collapse unknown variants to `Unknown`
    // where applicable.
    let session_versions = client.list_session_versions(&job_id).await?;
    assert_eq!(session_versions.versions.len(), 1);
    assert_eq!(session_versions.versions[0].session_id, job_id);
    let session_version = client
        .get_session_version(&job_id, RelativeVersionNumber::new(1))
        .await?;
    assert_eq!(session_version.session_id, job_id);

    let issue_versions = client.list_issue_versions(&issue_id).await?;
    assert_eq!(issue_versions.versions.len(), 1);
    assert!(matches!(
        issue_versions.versions[0].issue.issue_type,
        IssueType::Unknown
    ));
    let issue_version = client
        .get_issue_version(&issue_id, RelativeVersionNumber::new(1))
        .await?;
    assert_eq!(issue_version.issue_id, issue_id);
    assert!(matches!(issue_version.issue.issue_type, IssueType::Unknown));

    let patch_versions = client.list_patch_versions(&patch_id).await?;
    assert_eq!(patch_versions.versions.len(), 1);
    assert!(matches!(
        patch_versions.versions[0].patch.status,
        PatchStatus::Unknown
    ));
    let patch_version = client
        .get_patch_version(&patch_id, RelativeVersionNumber::new(1))
        .await?;
    assert_eq!(patch_version.patch_id, patch_id);
    assert!(matches!(patch_version.patch.status, PatchStatus::Unknown));

    let trigger_versions = client.list_trigger_versions(&trigger_id).await?;
    assert_eq!(trigger_versions.versions.len(), 1);
    let TriggerAction::CreateIssue(create) = &trigger_versions.versions[0].trigger.actions[0];
    assert!(matches!(create.issue_type, IssueType::Unknown));
    let trigger_version = client
        .get_trigger_version(&trigger_id, RelativeVersionNumber::new(1))
        .await?;
    assert_eq!(trigger_version.trigger_id, trigger_id);
    let TriggerAction::CreateIssue(create) = &trigger_version.trigger.actions[0];
    assert!(matches!(create.issue_type, IssueType::Unknown));

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

fn forward_user_json(username: &Username) -> Value {
    json!({
        "username": username,
        "github_user_id": 4242,
        "future": "user-field"
    })
}

fn forward_agent_json(agent_name: &str) -> Value {
    json!({
        "agent": {
            "name": agent_name,
            "prompt": "future agent prompt",
            "prompt_path": "/agents/future/prompt.md",
            "mcp_config_path": null,
            "mcp_config": null,
            "max_tries": 3,
            "max_simultaneous": 5,
            "is_assignment_agent": true,
            "is_default_conversation_agent": false,
            "secrets": ["FUTURE_KEY"],
            "future": "agent-record-field"
        },
        "extra": "agent-response"
    })
}

fn forward_label_json(label_id: &LabelId, now: DateTime<Utc>) -> Value {
    json!({
        "label_id": label_id,
        "name": "future-label",
        "color": "#abcdef",
        "recurse": true,
        "hidden": false,
        "created_at": now,
        "updated_at": now,
        "future": "label-field"
    })
}

fn forward_relation_json(source_id: &HydraId, target_id: &HydraId, now: DateTime<Utc>) -> Value {
    json!({
        "source_id": source_id,
        "target_id": target_id,
        "rel_type": "future-rel",
        "created_at": now,
        "future": "relation-field"
    })
}

fn forward_trigger_json(
    trigger_id: &TriggerId,
    project_id: &ProjectId,
    now: DateTime<Utc>,
) -> Value {
    json!({
        "trigger_id": trigger_id,
        "version": 1,
        "timestamp": now,
        "trigger": {
            "enabled": true,
            "schedule": {
                "Cron": {
                    "expression": "0 9 * * MON",
                    "timezone": "UTC",
                    "future": "schedule-field"
                }
            },
            "actions": [
                {
                    "CreateIssue": {
                        // Unknown `IssueType` tag — older clients must decode
                        // this as `IssueType::Unknown`, not error.
                        "type": "future-issue-type",
                        "title": "Daily triage",
                        "description": "Run triage for {{ now.date }}",
                        "assignee": "users/alice",
                        "project_id": project_id,
                        "status": "open",
                        "session_settings": {},
                        "future": "action-field"
                    }
                }
            ],
            "creator": "test-creator",
            "last_fired_at": null,
            "deleted": false,
            "future": "trigger-field"
        },
        "actor": null,
        "creation_time": now,
        "extra": "trigger-version-record"
    })
}

fn forward_device_start_json(device_session_id: &str) -> Value {
    json!({
        "device_session_id": device_session_id,
        "user_code": "FUTURE-CODE",
        "verification_uri": "https://example.com/device",
        "expires_in": 600,
        "interval": 5,
        "extra": "device-start"
    })
}

fn forward_device_poll_json() -> Value {
    json!({
        // Unknown `DevicePollStatus` variant — older clients must decode
        // this as `DevicePollStatus::Unknown`, not error.
        "status": "future-status",
        "login_token": null,
        "user": null,
        "error": null,
        "extra": "device-poll"
    })
}

fn forward_proxy_target_list_json(port: u16) -> Value {
    json!({
        "targets": [
            {
                "port": port,
                "ready_path": "/healthz",
                "future": "field"
            }
        ],
        "extra": "list-proxy-targets"
    })
}
