use anyhow::{Context, Result};
use metis_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, SearchIssuesQuery,
        UpsertIssueRequest,
    },
    task_status::Status,
    users::Username,
};
use std::fs;
use tempfile::tempdir;

mod common;

use common::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

#[tokio::test]
async fn worker_rejects_closing_parent_with_open_child_issue() -> Result<()> {
    let env: common::test_helpers::TestEnvironment =
        init_test_server_with_remote("acme/worker-issue-children").await?;
    let prompt = "worker issue parent closure";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, "token-123").context("write auth token")?;
    let parent_issue = Issue::new(
        IssueType::Task,
        "parent issue".to_string(),
        Username::from("worker"),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let parent_id = env
        .client
        .create_issue(&UpsertIssueRequest::new(parent_issue, None))
        .await?
        .issue_id;
    let child_issue = Issue::new(
        IssueType::Task,
        "open child".to_string(),
        Username::from("worker"),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )],
        Vec::new(),
    );
    let child_id = env
        .client
        .create_issue(&UpsertIssueRequest::new(child_issue, None))
        .await?
        .issue_id;

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var HOME={} {}",
        repo_arg,
        server_url,
        temp_home.path().display(),
        prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue child test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env
        .run_as_worker(
            vec![format!("metis issues update {parent_id} --status closed")],
            job_id,
        )
        .await;
    let error = worker_result.expect_err("closing a parent with an open child should fail");
    let message = error.to_string();
    assert!(
        message.contains("failed to update issue")
            && message.contains("400 Bad Request")
            && message.contains("cannot close issue with open child issues"),
        "worker output did not include expected child error: {message}"
    );

    let issues = env
        .client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let parent = issues
        .iter()
        .find(|issue| issue.id == parent_id)
        .context("expected parent issue to exist")?;
    let child = issues
        .iter()
        .find(|issue| issue.id == child_id)
        .context("expected child issue to exist")?;
    assert_eq!(parent.issue.status, IssueStatus::Open);
    assert_eq!(child.issue.status, IssueStatus::Open);

    Ok(())
}

#[tokio::test]
async fn worker_rejects_closing_issue_with_open_todos() -> Result<()> {
    let env: common::test_helpers::TestEnvironment =
        init_test_server_with_remote("acme/worker-issue-todos").await?;
    let prompt = "worker issue todo closure";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, "token-123").context("write auth token")?;
    let todo_issue = Issue::new(
        IssueType::Task,
        "issue with todos".to_string(),
        Username::from("worker"),
        String::new(),
        IssueStatus::Open,
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let todo_issue_id = env
        .client
        .create_issue(&UpsertIssueRequest::new(todo_issue, None))
        .await?
        .issue_id;

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var HOME={} {}",
        repo_arg,
        server_url,
        temp_home.path().display(),
        prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue todo test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env
        .run_as_worker(
            vec![
                format!("metis issues todo {todo_issue_id} --add \"write more tests\""),
                format!("metis issues update {todo_issue_id} --status closed"),
            ],
            job_id,
        )
        .await;
    let error = worker_result.expect_err("closing an issue with incomplete todos should fail");
    let message = error.to_string();
    assert!(
        message.contains("failed to update issue")
            && message.contains("400 Bad Request")
            && message.contains("cannot close issue with incomplete todo items"),
        "worker output did not include expected todo error: {message}"
    );

    let issues = env
        .client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let issue = issues
        .iter()
        .find(|issue| issue.id == todo_issue_id)
        .context("expected todo issue to exist")?;
    assert_eq!(issue.issue.status, IssueStatus::Open);
    assert_eq!(issue.issue.todo_list.len(), 1);
    assert!(!issue.issue.todo_list[0].is_done);

    Ok(())
}
