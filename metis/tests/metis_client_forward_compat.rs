use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use httpmock::prelude::*;
use metis::client::MetisClient;
use metis_common::{
    issues::{
        AddTodoItemRequest, Issue, IssueDependencyType, IssueStatus, IssueType,
        ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest, TodoItem,
        UpsertIssueRequest,
    },
    job_status::JobStatusUpdate,
    jobs::{Bundle, BundleSpec, CreateJobRequest, SearchJobsQuery},
    logs::LogsQuery,
    patches::{GithubCiState, Patch, PatchStatus, SearchPatchesQuery, UpsertPatchRequest},
    repositories::{CreateRepositoryRequest, ServiceRepositoryConfig, UpdateRepositoryRequest},
    task_status::{Event, Status},
    users::{CreateUserRequest, UpdateGithubTokenRequest, User, Username},
    IssueId, PatchId, RepoName, TaskId,
};
use reqwest::Client as HttpClient;
use serde_json::{json, Value};

#[tokio::test]
async fn metis_client_handles_forward_compatible_payloads() -> Result<()> {
    let server = MockServer::start();
    let client = MetisClient::with_http_client(server.base_url(), HttpClient::new())?;

    let now = Utc::now();
    let job_id = TaskId::new();
    let issue_id = IssueId::new();
    let dependency_id = IssueId::new();
    let patch_id = PatchId::new();
    let repo_name = RepoName::new("dourolabs", "metis")?;
    let username: Username = "future-user".into();

    let status_log_json = forward_status_log_json(now);
    let job_record_body = forward_job_json(&job_id, status_log_json.clone());
    let job_record_body_for_list = job_record_body.clone();
    let job_record_body_for_get = job_record_body.clone();
    let issue_record_body = forward_issue_json(&issue_id, &dependency_id, &patch_id);
    let issue_record_for_get = issue_record_body.clone();
    let issue_record_for_list = issue_record_body.clone();
    let patch_record_body = forward_patch_json(&patch_id, &repo_name, &job_id, now);
    let patch_record_for_get = patch_record_body.clone();
    let patch_record_for_list = patch_record_body.clone();
    let repository_body = forward_repo_info(&repo_name);
    let todo_response = forward_todo_response(&issue_id);
    let todo_response_for_replace = todo_response.clone();
    let todo_response_for_status = todo_response.clone();

    let job_path = format!("/v1/jobs/{job_id}");
    let job_logs_path = format!("/v1/jobs/{job_id}/logs");
    let job_status_path = format!("/v1/jobs/{job_id}/status");
    let job_context_path = format!("/v1/jobs/{job_id}/context");
    let issue_path = format!("/v1/issues/{issue_id}");
    let todo_path = format!("/v1/issues/{issue_id}/todo-items");
    let todo_item_path = format!("{todo_path}/1");
    let patch_path = format!("/v1/patches/{patch_id}");
    let repo_path = format!(
        "/v1/repositories/{}/{}",
        repo_name.organization, repo_name.repo
    );
    let user_path = format!("/v1/users/{username}");
    let github_token_path = format!("/v1/users/{username}/github-token");
    let merge_queue_path = format!(
        "/v1/merge-queues/{}/{}/main/patches",
        repo_name.organization, repo_name.repo
    );
    let job_id_for_create = job_id.clone();
    let job_id_for_get = job_id.clone();
    let job_id_for_kill = job_id.clone();
    let job_id_for_status_post = job_id.clone();
    let job_id_for_status_get = job_id.clone();
    let issue_id_for_create = issue_id.clone();
    let issue_id_for_update = issue_id.clone();
    let patch_id_for_create = patch_id.clone();
    let patch_id_for_update = patch_id.clone();
    let patch_id_for_merge = patch_id.clone();
    let patch_id_for_enqueue = patch_id.clone();
    let username_for_users = username.clone();
    let username_for_delete = username.clone();
    let username_for_token = username.clone();

    server.mock(move |when, then| {
        when.method(POST).path("/v1/jobs");
        then.status(200)
            .json_body(json!({"job_id": job_id_for_create.clone(), "unexpected": "field"}));
    });

    server.mock(move |when, then| {
        when.method(GET).path("/v1/jobs/");
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
            "id": job_id_for_get_clone,
            "task": job_record_body_for_get_clone["task"].clone(),
            "notes": "note",
            "status_log": job_record_body_for_get_clone["status_log"].clone()
        }));
    });

    let kill_job_path = job_path.clone();
    let job_id_for_kill_clone = job_id_for_kill.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(kill_job_path.as_str());
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

    let job_status_get_path = job_status_path.clone();
    let job_id_for_status_get_clone = job_id_for_status_get.clone();
    let status_log_json_for_get = status_log_json.clone();
    server.mock(move |when, then| {
        when.method(GET).path(job_status_get_path.as_str());
        then.status(200).json_body(
            json!({ "job_id": job_id_for_status_get_clone, "status_log": status_log_json_for_get, "next": "status" }),
        );
    });

    let context_path_clone = job_context_path.clone();
    server.mock(move |when, then| {
        when.method(GET).path(context_path_clone.as_str());
        then.status(200).json_body(forward_worker_context_json());
    });

    let issue_id_for_create_clone = issue_id_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/issues");
        then.status(200)
            .json_body(json!({ "issue_id": issue_id_for_create_clone, "extra": "create-issue" }));
    });

    let issue_update_path = issue_path.clone();
    let issue_id_for_update_clone = issue_id_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(issue_update_path.as_str());
        then.status(200)
            .json_body(json!({ "issue_id": issue_id_for_update_clone, "future": true }));
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

    let todo_path_clone = todo_path.clone();
    let todo_response_for_add = todo_response.clone();
    server.mock(move |when, then| {
        when.method(POST).path(todo_path_clone.as_str());
        then.status(200).json_body(todo_response_for_add.clone());
    });

    let todo_replace_path = todo_path.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(todo_replace_path.as_str());
        then.status(200)
            .json_body(todo_response_for_replace.clone());
    });

    let todo_item_path_clone = todo_item_path.clone();
    server.mock(move |when, then| {
        when.method(POST).path(todo_item_path_clone.as_str());
        then.status(200).json_body(todo_response_for_status.clone());
    });

    let patch_id_for_create_clone = patch_id_for_create.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/patches");
        then.status(200).json_body(
            json!({ "patch_id": patch_id_for_create_clone, "additional": "create-patch" }),
        );
    });

    let patch_update_path = patch_path.clone();
    let patch_id_for_update_clone = patch_id_for_update.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(patch_update_path.as_str());
        then.status(200)
            .json_body(json!({ "patch_id": patch_id_for_update_clone, "note": "update" }));
    });

    let patch_get_path = patch_path.clone();
    let patch_record_for_get_clone = patch_record_for_get.clone();
    server.mock(move |when, then| {
        when.method(GET).path(patch_get_path.as_str());
        then.status(200).json_body(patch_record_for_get_clone);
    });

    let patch_record_for_list_clone = patch_record_for_list.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/patches");
        then.status(200)
            .json_body(json!({ "patches": [patch_record_for_list_clone], "extra": "list" }));
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

    let username_for_users_clone = username_for_users.clone();
    server.mock(move |when, then| {
        when.method(GET).path("/v1/users");
        then.status(200)
            .json_body(json!({ "users": [{"username": username_for_users_clone, "github_user_id": 123, "scope": "extra"}], "extra": "list-users" }));
    });

    let username_for_create_clone = username.clone();
    server.mock(move |when, then| {
        when.method(POST).path("/v1/users");
        then.status(200).json_body(json!({
            "user": { "username": username_for_create_clone, "github_user_id": 123, "bonus": "field" }
        }));
    });

    let user_delete_path = user_path.clone();
    let username_for_delete_clone = username_for_delete.clone();
    server.mock(move |when, then| {
        when.method(DELETE).path(user_delete_path.as_str());
        then.status(200)
            .json_body(json!({ "username": username_for_delete_clone, "archived": true }));
    });

    let github_token_path_clone = github_token_path.clone();
    let username_for_token_clone = username_for_token.clone();
    server.mock(move |when, then| {
        when.method(PUT).path(github_token_path_clone.as_str());
        then.status(200).json_body(json!({
            "user": { "username": username_for_token_clone, "github_user_id": 123, "scopes": ["repo"] },
            "updated": true
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

    server.mock(|when, then| {
        when.method(GET).path("/v1/github/app/client-id");
        then.status(200)
            .json_body(json!({ "client_id": "abc123", "note": "github" }));
    });

    // Job endpoints
    let create_job_request = CreateJobRequest::new(
        "test prompt".to_string(),
        None,
        BundleSpec::None,
        HashMap::new(),
    );
    let created_job = client.create_job(&create_job_request).await?;
    assert_eq!(created_job.job_id, job_id);

    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?;
    let listed_job = jobs.jobs.first().expect("job from list");
    assert!(matches!(listed_job.task.context, BundleSpec::Unknown));
    assert!(matches!(
        listed_job.status_log.events.first(),
        Some(Event::Created {
            status: Status::Unknown,
            ..
        })
    ));
    assert!(matches!(
        listed_job.status_log.events.get(1),
        Some(Event::Unknown)
    ));
    assert!(listed_job.status_log.current_status().is_failure());

    let fetched_job = client.get_job(&job_id).await?;
    assert!(matches!(fetched_job.task.context, BundleSpec::Unknown));

    let kill_response = client.kill_job(&job_id).await?;
    assert_eq!(kill_response.job_id, job_id);

    let mut logs = client.get_job_logs(&job_id, &LogsQuery::default()).await?;
    let mut collected = Vec::new();
    while let Some(item) = logs.next().await {
        collected.push(item?);
    }
    assert_eq!(collected.len(), 2);
    assert!(collected[1].contains("second log line"));

    let status_response = client
        .set_job_status(
            &job_id,
            &JobStatusUpdate::Failed {
                reason: "test".to_string(),
            },
        )
        .await?;
    assert!(matches!(status_response.status, Status::Unknown));

    let job_status = client.get_job_status(&job_id).await?;
    assert!(matches!(
        job_status.status_log.events.first(),
        Some(Event::Created {
            status: Status::Unknown,
            ..
        })
    ));

    let context = client.get_job_context(&job_id).await?;
    assert!(matches!(context.request_context, Bundle::Unknown));

    // Issues and todos
    let issue = Issue::new(
        IssueType::Bug,
        "desc".to_string(),
        User::new(Username::from("creator"), "token".to_string()),
        "progress".to_string(),
        IssueStatus::Open,
        Some("assignee".to_string()),
        None,
        vec![TodoItem::new("existing".to_string(), false)],
        vec![],
        vec![],
    );
    let issue_request = UpsertIssueRequest::new(issue, None);

    let created_issue = client.create_issue(&issue_request).await?;
    assert_eq!(created_issue.issue_id, issue_id);

    let updated_issue = client.update_issue(&issue_id, &issue_request).await?;
    assert_eq!(updated_issue.issue_id, issue_id);

    let fetched_issue = client.get_issue(&issue_id).await?;
    assert!(matches!(fetched_issue.issue.status, IssueStatus::Unknown));
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

    let todo_added = client
        .add_todo_item(
            &issue_id,
            &AddTodoItemRequest::new("new item".to_string(), true),
        )
        .await?;
    assert_eq!(todo_added.todo_list.len(), 1);

    let todo_replaced = client
        .replace_todo_list(
            &issue_id,
            &ReplaceTodoListRequest::new(vec![TodoItem::new("replacement".to_string(), false)]),
        )
        .await?;
    assert_eq!(todo_replaced.todo_list.len(), 1);

    let todo_status = client
        .set_todo_item_status(&issue_id, 1, &SetTodoItemStatusRequest::new(false))
        .await?;
    assert_eq!(todo_status.todo_list.len(), 1);

    // Patches
    let patch = Patch::new(
        "title".to_string(),
        "desc".to_string(),
        "diff".to_string(),
        PatchStatus::Open,
        false,
        None,
        vec![],
        repo_name.clone(),
        None,
    );
    let upsert_patch = UpsertPatchRequest::new(patch);

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

    // Repositories
    let repo_config = ServiceRepositoryConfig::new(
        "https://example.com/repo.git".to_string(),
        Some("main".to_string()),
        None,
        None,
    );
    let repo_create = CreateRepositoryRequest::new(repo_name.clone(), repo_config.clone());
    let repo_update = UpdateRepositoryRequest::new(repo_config);

    let created_repo = client.create_repository(&repo_create).await?;
    assert_eq!(created_repo.repository.name, repo_name);

    let updated_repo = client.update_repository(&repo_name, &repo_update).await?;
    assert_eq!(updated_repo.repository.name, repo_name);

    let repos = client.list_repositories().await?;
    assert_eq!(repos.repositories.len(), 1);

    // Users
    let create_user = CreateUserRequest::new(username.clone(), "token".to_string());
    let created_user = client.create_user(&create_user).await?;
    assert_eq!(created_user.user.username, username);

    let listed_users = client.list_users().await?;
    assert_eq!(listed_users.users.len(), 1);

    let deleted_user = client.delete_user(&username).await?;
    assert_eq!(deleted_user.username, username);

    let token_update = UpdateGithubTokenRequest::new("new-token".to_string());
    let updated_user = client
        .set_user_github_token(&username, &token_update)
        .await?;
    assert_eq!(updated_user.user.username, username);

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

    let github_client = client.get_github_app_client_id().await?;
    assert_eq!(github_client.client_id, "abc123");

    // Ensure unknown job status variants remain deserializable.
    let delayed_status: JobStatusUpdate = serde_json::from_value(json!({ "status": "delayed" }))?;
    assert!(matches!(delayed_status, JobStatusUpdate::Unknown));

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

fn forward_job_json(job_id: &TaskId, status_log: Value) -> Value {
    json!({
        "id": job_id,
        "task": {
            "prompt": "future job",
            "context": {
                "type": "archive_bundle",
                "url": "https://example.com/archive.tar.gz",
                "rev": "v2",
                "experimental": true
            },
            "env_vars": { "DEBUG": "true" },
            "extra": "task"
        },
        "notes": "note",
        "status_log": status_log,
        "unexpected": "job"
    })
}

fn forward_issue_json(issue_id: &IssueId, dependency_id: &IssueId, patch_id: &PatchId) -> Value {
    json!({
        "id": issue_id,
        "issue": {
            "type": "epic",
            "description": "future issue",
            "creator": {
                "username": "alice",
                "github_user_id": null,
                "github_token": "token"
            },
            "progress": "blocked",
            "status": "on-hold",
            "assignee": "robot",
            "todo_list": [
                { "description": "investigate", "is_done": true, "priority": 1 }
            ],
            "dependencies": [
                { "type": "relates-to", "issue_id": dependency_id }
            ],
            "patches": [patch_id],
            "surprise": "field"
        },
        "extra": "issue"
    })
}

fn forward_patch_json(
    patch_id: &PatchId,
    repo_name: &RepoName,
    job_id: &TaskId,
    now: DateTime<Utc>,
) -> Value {
    json!({
        "id": patch_id,
        "patch": {
            "title": "future patch",
            "description": "desc",
            "diff": "diff",
            "status": "stale",
            "is_automatic_backup": false,
            "created_by": job_id,
            "reviews": [
                { "contents": "looks ok", "is_approved": true, "author": "reviewer", "submitted_at": now, "confidence": "medium" }
            ],
            "service_repo_name": repo_name,
            "github": {
                "owner": "dourolabs",
                "repo": "metis",
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
        }
    })
}

fn forward_repo_info(repo_name: &RepoName) -> Value {
    json!({
        "name": repo_name,
        "remote_url": "https://example.com/repo.git",
        "default_branch": "main",
        "default_image": "ghcr.io/dourolabs/metis:main",
        "github_token_present": true,
        "sync": "on"
    })
}

fn forward_worker_context_json() -> Value {
    json!({
        "request_context": { "type": "workspace_snapshot", "path": "/tmp/work", "details": "future" },
        "prompt": "worker prompt",
        "variables": { "foo": "bar" },
        "note": "context"
    })
}

fn forward_todo_response(issue_id: &IssueId) -> Value {
    json!({
        "issue_id": issue_id,
        "todo_list": [
            { "description": "forward compatible", "is_done": false, "priority": "high" }
        ],
        "note": "todos"
    })
}

trait StatusSummary {
    fn is_failure(&self) -> bool;
}

impl StatusSummary for Status {
    fn is_failure(&self) -> bool {
        matches!(self, Status::Failed | Status::Unknown)
    }
}
