use super::common::default_image;
use crate::{
    job_engine::{JobEngine, JobStatus},
    store::Task,
    test_utils::{
        MockJobEngine, spawn_test_server, spawn_test_server_with_state, test_client,
        test_state_with_engine,
    },
};
use chrono::Utc;
use metis_common::{
    TaskId,
    issues::{
        AddTodoItemRequest, Issue, IssueDependency, IssueDependencyType, IssueRecord, IssueStatus,
        IssueType, ListIssuesResponse, ReplaceTodoListRequest, SearchIssuesQuery,
        SetTodoItemStatusRequest, TodoItem, TodoListResponse, UpsertIssueRequest,
        UpsertIssueResponse,
    },
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

#[tokio::test]
async fn update_issue_replaces_existing_value() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "original details".to_string(),
                creator: String::new(),
                progress: "Initial progress".to_string(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "updated details".to_string(),
                creator: String::new(),
                progress: "Updated progress".to_string(),
                status: IssueStatus::InProgress,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        Issue {
            issue_type: IssueType::Task,
            description: "updated details".to_string(),
            creator: String::new(),
            progress: "Updated progress".to_string(),
            status: IssueStatus::InProgress,
            assignee: None,
            todo_list: Vec::new(),
            dependencies: vec![],
            patches: Vec::new(),
        }
    );
    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_when_blocked() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let blocker: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "blocker".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let blocked_dependencies = vec![IssueDependency {
        dependency_type: IssueDependencyType::BlockedOn,
        issue_id: blocker.issue_id.clone(),
    }];
    let blocked: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "blocked".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: blocked_dependencies.clone(),
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "blocked".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: blocked_dependencies.clone(),
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "blocker".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
        .send()
        .await?
        .error_for_status()?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            blocked.issue_id
        ))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "blocked".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: blocked_dependencies,
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "parent".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let child_dependencies = vec![IssueDependency {
        dependency_type: IssueDependencyType::ChildOf,
        issue_id: parent.issue_id.clone(),
    }];
    let child: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "child".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: child_dependencies.clone(),
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "parent".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "child".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: child_dependencies,
                patches: Vec::new(),
            },
            job_id: None,
        })
        .send()
        .await?
        .error_for_status()?;

    let response = client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            parent.issue_id
        ))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "parent".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Closed,
                assignee: None,
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
        .send()
        .await?;

    assert!(response.status().is_success());
    Ok(())
}

#[tokio::test]
async fn update_issue_rejects_closing_with_open_todos() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let todo_list = vec![
        TodoItem {
            description: "write tests".to_string(),
            is_done: false,
        },
        TodoItem {
            description: "review PR".to_string(),
            is_done: false,
        },
    ];
    let base_issue = Issue {
        issue_type: IssueType::Task,
        description: "issue with todos".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Open,
        assignee: None,
        todo_list,
        dependencies: vec![],
        patches: Vec::new(),
    };

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: base_issue.clone(),
            job_id: None,
        })
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
        .json(&UpsertIssueRequest {
            issue: closed_issue.clone(),
            job_id: None,
        })
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
    completed_issue.todo_list = vec![
        TodoItem {
            description: "write tests".to_string(),
            is_done: true,
        },
        TodoItem {
            description: "review PR".to_string(),
            is_done: true,
        },
    ];
    client
        .put(format!(
            "{}/v1/issues/{}",
            server.base_url(),
            created.issue_id
        ))
        .json(&UpsertIssueRequest {
            issue: completed_issue,
            job_id: None,
        })
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

    let base_issue = Issue {
        issue_type: IssueType::Task,
        description: "dropped issue".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Open,
        assignee: None,
        todo_list: Vec::new(),
        dependencies: vec![],
        patches: Vec::new(),
    };

    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: base_issue.clone(),
            job_id: None,
        })
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
                    context: metis_common::jobs::BundleSpec::None,
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
        .json(&UpsertIssueRequest {
            issue: Issue {
                status: IssueStatus::Dropped,
                ..base_issue
            },
            job_id: None,
        })
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

    let issue = Issue {
        issue_type: IssueType::Bug,
        description: "login fails for guests".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Open,
        assignee: None,
        todo_list: Vec::new(),
        dependencies: vec![],
        patches: Vec::new(),
    };
    let assigned_issue = Issue {
        issue_type: IssueType::Task,
        description: "assigned issue".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Open,
        assignee: Some("owner-1".to_string()),
        todo_list: Vec::new(),
        dependencies: vec![],
        patches: Vec::new(),
    };
    let closed_issue = Issue {
        issue_type: IssueType::Task,
        description: "retire old endpoint".to_string(),
        creator: String::new(),
        progress: String::new(),
        status: IssueStatus::Closed,
        assignee: None,
        todo_list: Vec::new(),
        dependencies: vec![],
        patches: Vec::new(),
    };

    for issue in [issue.clone(), assigned_issue.clone(), closed_issue.clone()] {
        let response = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&UpsertIssueRequest {
                issue,
                job_id: None,
            })
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
    assert_eq!(filtered_issues.issues[0].issue, issue);

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

    let initial_todo = TodoItem {
        description: "write tests".to_string(),
        is_done: false,
    };
    let created: UpsertIssueResponse = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::Task,
                description: "issue with todos".to_string(),
                creator: String::new(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: None,
                todo_list: vec![initial_todo.clone()],
                dependencies: vec![],
                patches: Vec::new(),
            },
            job_id: None,
        })
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
        vec![
            initial_todo.clone(),
            TodoItem {
                description: "review PR".to_string(),
                is_done: false
            }
        ]
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
        todo_list: vec![
            TodoItem {
                description: "rewrite docs".to_string(),
                is_done: false,
            },
            TodoItem {
                description: "review PR".to_string(),
                is_done: true,
            },
        ],
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
