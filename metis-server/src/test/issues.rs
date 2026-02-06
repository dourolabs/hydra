use super::common::default_image;
use crate::{
    domain::{
        issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, TodoItem},
        jobs::BundleSpec,
        users::Username,
    },
    job_engine::{JobEngine, JobStatus},
    store::{Status, Task},
    test_utils::{
        MockJobEngine, spawn_test_server, spawn_test_server_with_state, test_client,
        test_state_with_engine_handles,
    },
};
use chrono::Utc;
use metis_common::{
    IssueId, PatchId, TaskId,
    api::v1::issues::{
        AddTodoItemRequest, IssueRecord, IssueVersionRecord, ListIssueVersionsResponse,
        ListIssuesResponse, ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest,
        TodoListResponse, UpsertIssueRequest, UpsertIssueResponse,
    },
};
use reqwest::StatusCode;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

fn issue(
    issue_type: IssueType,
    description: &str,
    creator: Username,
    progress: String,
    status: IssueStatus,
    assignee: Option<&str>,
    todo_list: Vec<TodoItem>,
    dependencies: Vec<IssueDependency>,
    patches: Vec<PatchId>,
) -> Issue {
    Issue::new(
        issue_type,
        description.to_string(),
        creator,
        progress,
        status,
        assignee.map(str::to_string),
        None,
        todo_list,
        dependencies,
        patches,
    )
}

fn user(username: &str) -> Username {
    Username::from(username)
}

fn default_user() -> Username {
    user("creator")
}

fn missing_user() -> Username {
    Username::from("")
}

fn todo(description: &str, is_done: bool) -> TodoItem {
    TodoItem::new(description.to_string(), is_done)
}

#[tokio::test]
async fn update_issue_replaces_existing_value() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "original details".to_string(),
                default_user(),
                "Initial progress".to_string(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let updated: UpsertIssueResponse = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "updated details".to_string(),
                default_user(),
                "Updated progress".to_string(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(updated.issue_id, created.issue_id);

    let fetched: IssueRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched.issue,
        metis_common::api::v1::issues::Issue::from(Issue::new(
            IssueType::Task,
            "updated details".to_string(),
            default_user(),
            "Updated progress".to_string(),
            IssueStatus::InProgress,
            None,
            None,
            Vec::new(),
            vec![],
            Vec::new(),
        ))
    );
    Ok(())
}

#[tokio::test]
async fn issue_versions_endpoints_return_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "initial".to_string(),
                default_user(),
                "Initial progress".to_string(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let _updated: UpsertIssueResponse = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "updated".to_string(),
                default_user(),
                "Updated progress".to_string(),
                IssueStatus::InProgress,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let versions: ListIssueVersionsResponse = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 2);
    assert_eq!(versions.versions[0].issue_id, created.issue_id);
    assert_eq!(versions.versions[0].version, 1);
    assert_eq!(versions.versions[0].issue.description, "initial");
    assert_eq!(versions.versions[1].issue_id, created.issue_id);
    assert_eq!(versions.versions[1].version, 2);
    assert_eq!(versions.versions[1].issue.description, "updated");

    let version: IssueVersionRecord = client
        .get(format!(
            "{}/v1/issues/{}/versions/2",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(version.version, 2);
    assert_eq!(version.issue_id, created.issue_id);
    assert_eq!(version.issue.description, "updated");

    Ok(())
}

#[tokio::test]
async fn issue_version_endpoints_return_404s() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let missing: IssueId = "i-missing".parse().expect("valid issue id");
    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            missing
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "initial".to_string(),
                default_user(),
                "Initial progress".to_string(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/issues/{}/versions/99",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn create_issue_inherits_creator_from_parent_when_missing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let parent_creator = user("parent-creator");
    let parent: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                parent_creator.clone(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let child_dependencies = vec![IssueDependency::new(
        IssueDependencyType::ChildOf,
        parent.issue_id.clone(),
    )];
    let child: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "child",
                missing_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                child_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let fetched: IssueRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            child.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched.issue.creator,
        metis_common::api::v1::users::Username::from(parent_creator)
    );

    let explicit_creator = user("explicit-creator");
    let explicit_child: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "explicit child",
                explicit_creator.clone(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                child_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let fetched_explicit: IssueRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            explicit_child.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(
        fetched_explicit.issue.creator,
        metis_common::api::v1::users::Username::from(explicit_creator)
    );

    Ok(())
}

#[tokio::test]
async fn create_issue_rejects_missing_creator_without_parent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "missing creator",
                missing_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_when_blocked() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let blocker: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocker",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let blocked_dependencies = vec![IssueDependency::new(
        IssueDependencyType::BlockedOn,
        blocker.issue_id.clone(),
    )];
    let blocked: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocked",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                blocked_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            blocked.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocked",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                blocked_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!(
            "blocked issues cannot close until blockers are closed: {}",
            blocker.issue_id
        )})
    );

    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            blocker.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocker",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            blocked.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "blocked",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                blocked_dependencies,
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_with_open_children() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let parent: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let child_dependencies = vec![IssueDependency::new(
        IssueDependencyType::ChildOf,
        parent.issue_id.clone(),
    )];
    let child: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "child",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                child_dependencies.clone(),
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            parent.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": format!(
            "cannot close issue with open child issues: {}",
            child.issue_id
        )})
    );

    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            child.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "child",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                child_dependencies,
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            parent.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "parent",
                default_user(),
                String::new(),
                IssueStatus::Closed,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?;

    assert!(response.status().is_success());
    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_with_open_todos() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let todo_list = vec![todo("write tests", false), todo("review PR", false)];
    let base_issue = issue(
        IssueType::Task,
        "issue with todos",
        default_user(),
        String::new(),
        IssueStatus::Open,
        None,
        todo_list,
        vec![],
        Vec::new(),
    );

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(base_issue.clone().into(), None))
        .send()
        .await?
        .json()
        .await?;

    let mut closed_issue = base_issue.clone();
    closed_issue.status = IssueStatus::Closed;
    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(closed_issue.clone().into(), None))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body,
        json!({ "error": "cannot close issue with incomplete todo items: 1, 2" })
    );

    for item_number in [1, 2] {
        client
            .post(format!(
                "{}/v1/issues/{}/todo-items/{}",
                server.base_url(),
                created.issue_id,
                item_number
            ))
            .json(&SetTodoItemStatusRequest::new(true))
            .send()
            .await?
            .error_for_status()?;
    }

    let mut completed_issue = closed_issue;
    completed_issue.todo_list = vec![todo("write tests", true), todo("review PR", true)];
    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(completed_issue.into(), None))
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

#[tokio::test]
async fn dropping_issue_kills_spawned_tasks() -> anyhow::Result<()> {
    let engine = Arc::new(MockJobEngine::new());
    let handles = test_state_with_engine_handles(engine.clone());
    let server = spawn_test_server_with_state(handles.state.clone(), handles.store.clone()).await?;
    let client = test_client();

    let base_issue = issue(
        IssueType::Task,
        "dropped issue",
        default_user(),
        String::new(),
        IssueStatus::Open,
        None,
        Vec::new(),
        vec![],
        Vec::new(),
    );

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(base_issue.clone().into(), None))
        .send()
        .await?
        .json()
        .await?;

    let task_id = TaskId::new();
    handles
        .store
        .add_task_with_id(
            task_id.clone(),
            Task {
                prompt: "do work".to_string(),
                context: BundleSpec::None,
                spawned_from: Some(created.issue_id.clone()),
                image: Some(default_image()),
                model: None,
                env_vars: HashMap::new(),
                cpu_limit: None,
                memory_limit: None,
                status: Status::Created,
                last_message: None,
                error: None,
                deleted: false,
            },
            Utc::now(),
        )
        .await?;
    handles.state.transition_task_to_pending(&task_id).await?;
    handles.state.transition_task_to_running(&task_id).await?;
    engine.insert_job(&task_id, JobStatus::Running).await;

    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            {
                let mut dropped_issue = base_issue.clone();
                dropped_issue.status = IssueStatus::Dropped;
                dropped_issue.into()
            },
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    let job = engine.find_job_by_metis_id(&task_id).await?;
    assert_eq!(job.status, JobStatus::Failed);

    Ok(())
}

#[tokio::test]
async fn list_issues_supports_filters() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let base_issue = issue(
        IssueType::Bug,
        "login fails for guests",
        default_user(),
        String::new(),
        IssueStatus::Open,
        None,
        Vec::new(),
        vec![],
        Vec::new(),
    );
    let assigned_issue = issue(
        IssueType::Task,
        "assigned issue",
        default_user(),
        String::new(),
        IssueStatus::Open,
        Some("owner-1"),
        Vec::new(),
        vec![],
        Vec::new(),
    );
    let closed_issue = issue(
        IssueType::Task,
        "retire old endpoint",
        default_user(),
        String::new(),
        IssueStatus::Closed,
        None,
        Vec::new(),
        vec![],
        Vec::new(),
    );

    for issue in [
        base_issue.clone(),
        assigned_issue.clone(),
        closed_issue.clone(),
    ] {
        let response = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&UpsertIssueRequest::new(issue.into(), None))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let filtered_issues: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Bug),
            None,
            None,
            None,
            Vec::new(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_issues.issues.len(), 1);
    assert_eq!(
        filtered_issues.issues[0].issue,
        metis_common::api::v1::issues::Issue::from(base_issue)
    );

    let filtered_by_assignee: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            None,
            Some("OWNER-1".to_string()),
            None,
            Vec::new(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_assignee.issues.len(), 1);
    assert_eq!(
        filtered_by_assignee.issues[0].issue,
        metis_common::api::v1::issues::Issue::from(assigned_issue)
    );

    let filtered_by_status: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            Some(metis_common::api::v1::issues::IssueStatus::Closed),
            None,
            None,
            Vec::new(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_status.issues.len(), 1);
    assert_eq!(
        filtered_by_status.issues[0].issue,
        metis_common::api::v1::issues::Issue::from(closed_issue)
    );
    Ok(())
}

#[tokio::test]
async fn todo_list_endpoints_append_update_and_replace() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let initial_todo = todo("write tests", false);
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "issue with todos",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                vec![initial_todo.clone()],
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    let added: TodoListResponse = client
        .post(format!(
            "{}/v1/issues/{}/todo-items",
            server.base_url(),
            created.issue_id
        ))
        .json(&AddTodoItemRequest::new("review PR".to_string(), false))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        added.todo_list,
        vec![
            metis_common::api::v1::issues::TodoItem::from(initial_todo.clone()),
            metis_common::api::v1::issues::TodoItem::from(todo("review PR", false))
        ]
    );

    let updated: TodoListResponse = client
        .post(format!(
            "{}/v1/issues/{}/todo-items/2",
            server.base_url(),
            created.issue_id
        ))
        .json(&SetTodoItemStatusRequest::new(true))
        .send()
        .await?
        .json()
        .await?;
    assert!(updated.todo_list[1].is_done);

    let repeated: TodoListResponse = client
        .post(format!(
            "{}/v1/issues/{}/todo-items/2",
            server.base_url(),
            created.issue_id
        ))
        .json(&SetTodoItemStatusRequest::new(true))
        .send()
        .await?
        .json()
        .await?;
    assert!(repeated.todo_list[1].is_done);

    let unset: TodoListResponse = client
        .post(format!(
            "{}/v1/issues/{}/todo-items/2",
            server.base_url(),
            created.issue_id
        ))
        .json(&SetTodoItemStatusRequest::new(false))
        .send()
        .await?
        .json()
        .await?;
    assert!(!unset.todo_list[1].is_done);

    let replacement = ReplaceTodoListRequest::new(vec![
        todo("rewrite docs", false).into(),
        todo("review PR", true).into(),
    ]);

    let replaced: TodoListResponse = client
        .put(format!(
            "{}/v1/issues/{}/todo-items",
            server.base_url(),
            created.issue_id
        ))
        .json(&replacement)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(replaced.todo_list, replacement.todo_list);

    let fetched: IssueRecord = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(fetched.issue.todo_list, replacement.todo_list);

    let invalid_update = client
        .post(format!(
            "{}/v1/issues/{}/todo-items/99",
            server.base_url(),
            created.issue_id
        ))
        .json(&SetTodoItemStatusRequest::new(true))
        .send()
        .await?;
    assert_eq!(invalid_update.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}

// ===== Deletion Tests =====

#[tokio::test]
async fn delete_issue_basic_operation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "issue to delete",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // Delete the issue
    let deleted: IssueRecord = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    // Verify the response has deleted=true
    assert!(deleted.issue.deleted);

    // Verify listing excludes the deleted issue
    let list: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!list.issues.iter().any(|i| i.id == created.issue_id));

    Ok(())
}

#[tokio::test]
async fn delete_issue_include_deleted_in_listing() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "deleted issue",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // List without include_deleted - verify not present
    let list_without: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .send()
        .await?
        .json()
        .await?;

    assert!(!list_without.issues.iter().any(|i| i.id == created.issue_id));

    // List with include_deleted=true - verify present with deleted=true
    let list_with: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            Vec::new(),
            Some(true),
        ))
        .send()
        .await?
        .json()
        .await?;

    let deleted_issue = list_with.issues.iter().find(|i| i.id == created.issue_id);

    assert!(deleted_issue.is_some());
    assert!(deleted_issue.unwrap().issue.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_issue_get_deleted_by_id() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "get deleted issue",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // GET by ID should return 404 for deleted issues
    let response = client
        .get(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // But get_issue_versions should still return all versions including deleted
    let versions: ListIssueVersionsResponse = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 2);
    assert!(versions.versions.last().unwrap().issue.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_issue_version_history() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create issue (v1)
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "version history test",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // Update issue (v2)
    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "updated description",
                default_user(),
                "Updated progress".to_string(),
                IssueStatus::InProgress,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .error_for_status()?;

    // Delete issue (v3)
    client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .error_for_status()?;

    // Get versions - verify deletion creates new version with deleted=true
    let versions: ListIssueVersionsResponse = client
        .get(format!(
            "{}/v1/issues/{}/versions",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(versions.versions.len(), 3);
    assert!(!versions.versions[0].issue.deleted);
    assert!(!versions.versions[1].issue.deleted);
    assert!(versions.versions[2].issue.deleted);

    Ok(())
}

#[tokio::test]
async fn delete_issue_idempotency() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and delete an issue
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest::new(
            issue(
                IssueType::Task,
                "idempotency test",
                default_user(),
                String::new(),
                IssueStatus::Open,
                None,
                Vec::new(),
                vec![],
                Vec::new(),
            )
            .into(),
            None,
        ))
        .send()
        .await?
        .json()
        .await?;

    // First delete
    let first_delete = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert!(first_delete.status().is_success());

    // Second delete - should return 200 (idempotent)
    let second_delete = client
        .delete(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .send()
        .await?;

    assert!(second_delete.status().is_success());

    Ok(())
}

#[tokio::test]
async fn delete_issue_non_existent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Attempt to delete non-existent ID
    let missing: IssueId = "i-nonexistent".parse().expect("valid issue id");
    let response = client
        .delete(format!("{}/v1/issues/{}", server.base_url(), missing))
        .send()
        .await?;

    // Verify 404 response
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
