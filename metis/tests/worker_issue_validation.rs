mod harness;

use anyhow::{Context, Result};
use metis_common::issues::IssueStatus;
use std::str::FromStr;

#[tokio::test]
async fn worker_rejects_closing_parent_with_open_child_issue() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-issue-children")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/worker-issue-children")?;

    let grandparent_id = user.create_issue("grandparent issue").await?;
    let issue_id = user.create_issue("worker issue parent closure").await?;
    let job_id = user
        .create_job_for_issue(&repo, "worker issue parent closure", &issue_id)
        .await?;

    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                &format!(
                    "metis --output-format jsonl issues create --deps child-of:{grandparent_id} \"parent issue\" | sed -n 's/^{{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee parent_id.txt"
                ),
                "metis --output-format jsonl issues create --deps child-of:$(cat parent_id.txt) \"open child\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee child_id.txt",
                &format!(
                    "metis issues update $(cat parent_id.txt) --status closed --deps child-of:{grandparent_id}"
                ),
            ],
        )
        .await?;

    let message = format!("{:#}", failure.error);
    assert!(
        message.contains("failed to update issue")
            && message.contains("400 Bad Request")
            && message.contains("cannot close issue with open child issues"),
        "worker output did not include expected child error: {message}"
    );

    let issues = user.list_issues().await?.issues;
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
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-issue-todos")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/worker-issue-todos")?;

    let grandparent_id = user.create_issue("todo grandparent issue").await?;
    let issue_id = user.create_issue("worker issue todo closure").await?;
    let job_id = user
        .create_job_for_issue(&repo, "worker issue todo closure", &issue_id)
        .await?;

    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                &format!(
                    "metis --output-format jsonl issues create --deps child-of:{grandparent_id} \"issue with todos\" | sed -n 's/^{{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee issue_id.txt"
                ),
                "metis issues todo $(cat issue_id.txt) --add \"write more tests\"",
                &format!(
                    "metis issues update $(cat issue_id.txt) --status closed --deps child-of:{grandparent_id}"
                ),
            ],
        )
        .await?;

    let message = format!("{:#}", failure.error);
    assert!(
        message.contains("failed to update issue")
            && message.contains("400 Bad Request")
            && message.contains("cannot close issue with incomplete todo items"),
        "worker output did not include expected todo error: {message}"
    );

    let issues = user.list_issues().await?.issues;
    let issue = issues
        .iter()
        .find(|issue| issue.issue.description == "issue with todos")
        .context("expected todo issue to exist")?;
    assert_eq!(issue.issue.status, IssueStatus::Open);
    assert_eq!(issue.issue.todo_list.len(), 1);
    assert!(!issue.issue.todo_list[0].is_done);

    Ok(())
}
