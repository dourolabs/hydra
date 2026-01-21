use anyhow::{Context, Result};
use metis_common::{
    issues::{IssueStatus, SearchIssuesQuery},
    task_status::Status,
};

mod common;

use common::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

#[tokio::test]
async fn worker_rejects_closing_parent_with_open_child_issue() -> Result<()> {
    let env: common::test_helpers::TestEnvironment =
        init_test_server_with_remote("acme/worker-issue-children").await?;
    let prompt = "worker issue parent closure";
    let repo_arg = env.service_repo_name.to_string();
    let server_url = env.server.base_url();

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} {}",
        repo_arg, server_url, prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue child test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env.run_as_worker(
        vec![
            "metis issues create \"parent issue\" | tee parent_id.txt".to_string(),
            "metis issues create --deps child-of:$(cat parent_id.txt) \"open child\" | tee child_id.txt"
                .to_string(),
            "metis issues update $(cat parent_id.txt) --status closed".to_string(),
        ],
        job_id,
    )
    .await;
    let error = worker_result.expect_err("closing a parent with an open child should fail");
    let message = error.to_string();
    assert!(
        message.contains("failed to update issue") && message.contains("400 Bad Request"),
        "worker output did not include expected child error: {message}"
    );

    let issues = env
        .client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let parent = issues
        .iter()
        .find(|issue| issue.issue.description == "parent issue")
        .context("expected parent issue to exist")?;
    let child = issues
        .iter()
        .find(|issue| issue.issue.description == "open child")
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

    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} {}",
        repo_arg, server_url, prompt
    )])
    .await?;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue todo test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env
        .run_as_worker(
            vec![
                "metis issues create \"issue with todos\" | tee issue_id.txt".to_string(),
                "metis issues todo $(cat issue_id.txt) --add \"write more tests\"".to_string(),
                "metis issues update $(cat issue_id.txt) --status closed".to_string(),
            ],
            job_id,
        )
        .await;
    let error = worker_result.expect_err("closing an issue with incomplete todos should fail");
    let message = error.to_string();
    assert!(
        message.contains("failed to update issue") && message.contains("400 Bad Request"),
        "worker output did not include expected todo error: {message}"
    );

    let issues = env
        .client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let issue = issues
        .iter()
        .find(|issue| issue.issue.description == "issue with todos")
        .context("expected todo issue to exist")?;
    assert_eq!(issue.issue.status, IssueStatus::Open);
    assert_eq!(issue.issue.todo_list.len(), 1);
    assert!(!issue.issue.todo_list[0].is_done);

    Ok(())
}
