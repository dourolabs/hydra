use super::common::default_image;
use crate::{
    domain::{
        issues::{
            AddTodoItemRequest, Issue, IssueDependency, IssueDependencyType, IssueRecord,
            IssueStatus, IssueType, ListIssuesResponse, ReplaceTodoListRequest, SearchIssuesQuery,
            SetTodoItemStatusRequest, TodoItem, TodoListResponse, UpsertIssueRequest,
            UpsertIssueResponse,
        },
        jobs::BundleSpec,
        users::{User, Username},
    },
    job_engine::{JobEngine, JobStatus},
    store::Task,
    test_utils::{
        MockJobEngine, spawn_test_server, spawn_test_server_with_state, test_client,
        test_state_with_engine,
    },
};
use chrono::Utc;
use metis_common::{PatchId, TaskId};
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
            ),
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
            ),
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
    );
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
            ),
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
            ),
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

    assert_eq!(fetched.issue.creator, parent_creator);

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
            ),
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

    assert_eq!(fetched_explicit.issue.creator, explicit_creator);

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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
            ),
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
        .json(&UpsertIssueRequest::new(base_issue.clone(), None))
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
        .json(&UpsertIssueRequest::new(closed_issue.clone(), None))
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
            .json(&SetTodoItemStatusRequest { is_done: true })
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
        .json(&UpsertIssueRequest::new(completed_issue, None))
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

#[tokio::test]
async fn dropping_issue_kills_spawned_tasks() -> anyhow::Result<()> {
    let engine = Arc::new(MockJobEngine::new());
    let state = test_state_with_engine(engine.clone());
    let server = spawn_test_server_with_state(state.clone()).await?;
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
        .json(&UpsertIssueRequest::new(base_issue.clone(), None))
        .send()
        .await?
        .json()
        .await?;

    let task_id = TaskId::new();
    {
        let mut store = state.store.write().await;
        store
            .add_task_with_id(
                task_id.clone(),
                Task {
                    prompt: "do work".to_string(),
                    context: BundleSpec::None,
                    spawned_from: Some(created.issue_id.clone()),
                    image: Some(default_image()),
                    env_vars: HashMap::new(),
                },
                Utc::now(),
            )
            .await?;
        store.mark_task_running(&task_id, Utc::now()).await?;
    }
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
                dropped_issue
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
            .json(&UpsertIssueRequest::new(issue, None))
            .send()
            .await?;
        assert!(response.status().is_success());
    }

    let filtered_issues: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery {
            issue_type: Some(IssueType::Bug),
            status: None,
            assignee: None,
            q: None,
            graph_filters: Vec::new(),
        })
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_issues.issues.len(), 1);
    assert_eq!(filtered_issues.issues[0].issue, base_issue);

    let filtered_by_assignee: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery {
            issue_type: None,
            status: None,
            assignee: Some("OWNER-1".to_string()),
            q: None,
            graph_filters: Vec::new(),
        })
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_assignee.issues.len(), 1);
    assert_eq!(filtered_by_assignee.issues[0].issue, assigned_issue);

    let filtered_by_status: ListIssuesResponse = client
        .get(format!("{}/v1/issues", server.base_url()))
        .query(&SearchIssuesQuery {
            issue_type: None,
            status: Some(IssueStatus::Closed),
            assignee: None,
            q: None,
            graph_filters: Vec::new(),
        })
        .send()
        .await?
        .json()
        .await?;

    assert_eq!(filtered_by_status.issues.len(), 1);
    assert_eq!(filtered_by_status.issues[0].issue, closed_issue);
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
            ),
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
        .json(&AddTodoItemRequest {
            description: "review PR".to_string(),
            is_done: false,
        })
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        added.todo_list,
        vec![initial_todo.clone(), todo("review PR", false)]
    );

    let updated: TodoListResponse = client
        .post(format!(
            "{}/v1/issues/{}/todo-items/2",
            server.base_url(),
            created.issue_id
        ))
        .json(&SetTodoItemStatusRequest { is_done: true })
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
        .json(&SetTodoItemStatusRequest { is_done: true })
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
        .json(&SetTodoItemStatusRequest { is_done: false })
        .send()
        .await?
        .json()
        .await?;
    assert!(!unset.todo_list[1].is_done);

    let replacement = ReplaceTodoListRequest {
        todo_list: vec![todo("rewrite docs", false), todo("review PR", true)],
    };

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
        .json(&SetTodoItemStatusRequest { is_done: true })
        .send()
        .await?;
    assert_eq!(invalid_update.status(), reqwest::StatusCode::BAD_REQUEST);

    Ok(())
}
