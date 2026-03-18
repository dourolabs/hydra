mod harness;

use anyhow::Result;
use std::str::FromStr;

/// Verify that the CLI surfaces an appropriate error when a worker attempts to
/// close an issue that has open children. The policy logic itself is tested in
/// policy/restrictions/issue_lifecycle.rs; this test only checks that the error
/// propagates through the worker execution path to the CLI.
#[tokio::test]
async fn worker_surfaces_error_for_closing_issue_with_open_children() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-issue-children")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/worker-issue-children")?;

    let parent_id = user.create_issue("parent for children test").await?;
    let issue_id = user.create_issue("worker child closure test").await?;
    let job_id = user
        .create_session_for_issue(&repo, "worker child closure test", &issue_id)
        .await?;

    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                &format!(
                    "hydra --output-format jsonl issues create --deps child-of:{parent_id} \"issue with child\" | sed -n 's/^{{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee parent_id.txt"
                ),
                "hydra --output-format jsonl issues create --deps child-of:$(cat parent_id.txt) \"open child\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee child_id.txt",
                &format!(
                    "hydra issues update $(cat parent_id.txt) --status closed --deps child-of:{parent_id}"
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

    Ok(())
}

/// Verify that the CLI surfaces an appropriate error when a worker attempts to
/// close an issue that has incomplete todos. The policy logic itself is tested in
/// policy/restrictions/issue_lifecycle.rs; this test only checks that the error
/// propagates through the worker execution path to the CLI.
#[tokio::test]
async fn worker_surfaces_error_for_closing_issue_with_open_todos() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/worker-issue-todos")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = hydra_common::RepoName::from_str("acme/worker-issue-todos")?;

    let parent_id = user.create_issue("parent for todos test").await?;
    let issue_id = user.create_issue("worker todo closure test").await?;
    let job_id = user
        .create_session_for_issue(&repo, "worker todo closure test", &issue_id)
        .await?;

    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                &format!(
                    "hydra --output-format jsonl issues create --deps child-of:{parent_id} \"issue with todos\" | sed -n 's/^{{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee issue_id.txt"
                ),
                "hydra issues todo $(cat issue_id.txt) --add \"write more tests\"",
                &format!(
                    "hydra issues update $(cat issue_id.txt) --status closed --deps child-of:{parent_id}"
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

    Ok(())
}
