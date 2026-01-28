use anyhow::{Context, Result};
use metis_common::{
    constants::ENV_METIS_TOKEN,
    issues::{Issue, IssueStatus, IssueType, SearchIssuesQuery, UpsertIssueRequest},
    task_status::Status,
    users::Username,
};
use std::fs;
use tempfile::tempdir;

mod common;

use common::test_helpers::{init_test_server_with_remote, job_id_for_prompt, wait_for_status};

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
    fs::write(&auth_token_path, &env.auth_token).context("write auth token")?;
    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var HOME={} --var METIS_ISSUE_ID={} --var {}={} {}",
        repo_arg,
        server_url,
        temp_home.path().display(),
        env.current_issue_id,
        ENV_METIS_TOKEN,
        env.auth_token,
        prompt
    )])
    .await?;

    let grandparent_issue = UpsertIssueRequest::new(
        Issue::new(
            IssueType::Task,
            "grandparent issue".into(),
            Username::from("test-user"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        None,
    );
    let grandparent_id = env.client.create_issue(&grandparent_issue).await?.issue_id;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue child test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env.run_as_worker(
        vec![
            format!(
                "metis --output-format jsonl issues create --deps child-of:{grandparent_id} \"parent issue\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p' | tee parent_id.txt"
            ),
            "metis --output-format jsonl issues create --deps child-of:$(cat parent_id.txt) \"open child\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p' | tee child_id.txt"
                .to_string(),
            format!(
                "metis issues update $(cat parent_id.txt) --status closed --deps child-of:{grandparent_id}"
            ),
        ],
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
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, &env.auth_token).context("write auth token")?;
    env.run_as_user(vec![format!(
        "metis jobs create --repo {} --var METIS_SERVER_URL={} --var HOME={} --var METIS_ISSUE_ID={} --var {}={} {}",
        repo_arg,
        server_url,
        temp_home.path().display(),
        env.current_issue_id,
        ENV_METIS_TOKEN,
        env.auth_token,
        prompt
    )])
    .await?;

    let grandparent_issue = UpsertIssueRequest::new(
        Issue::new(
            IssueType::Task,
            "todo grandparent issue".into(),
            Username::from("test-user"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
        None,
    );
    let grandparent_id = env.client.create_issue(&grandparent_issue).await?.issue_id;

    let job_id = job_id_for_prompt(&env.client, prompt)
        .await
        .context("expected job to be created for worker issue todo test")?;
    wait_for_status(&env.client, &job_id, Status::Running).await?;

    let worker_result: Result<Vec<common::bash_commands::CommandOutput>, _> = env
        .run_as_worker(
            vec![
                format!(
                    "metis --output-format jsonl issues create --deps child-of:{grandparent_id} \"issue with todos\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p' | tee issue_id.txt"
                ),
                "metis issues todo $(cat issue_id.txt) --add \"write more tests\"".to_string(),
                format!(
                    "metis issues update $(cat issue_id.txt) --status closed --deps child-of:{grandparent_id}"
                ),
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
        .find(|issue| issue.issue.description == "issue with todos")
        .context("expected todo issue to exist")?;
    assert_eq!(issue.issue.status, IssueStatus::Open);
    assert_eq!(issue.issue.todo_list.len(), 1);
    assert!(!issue.issue.todo_list[0].is_done);

    Ok(())
}
