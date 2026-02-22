mod harness;

use anyhow::Result;
use harness::find_summary_issue_by_description;
use std::str::FromStr;

/// Scenario 7a: Cannot close an issue with open children.
///
/// Creates a parent with an open child via a worker, attempts to close the
/// parent (expecting failure with a clear error message), then closes the
/// child and retries closing the parent (expecting success).
#[tokio::test]
async fn cannot_close_issue_with_open_children() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/lifecycle-7a")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/lifecycle-7a")?;

    // Create a parent issue and a job for the worker.
    let parent_id = user.create_issue("parent for 7a").await?;
    let issue_id = user.create_issue("worker issue 7a").await?;
    let job_id = user
        .create_job_for_issue(&repo, "test lifecycle 7a", &issue_id)
        .await?;

    // Worker creates a child under the parent, then tries to close the parent.
    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                // Create a child issue under the parent.
                &format!(
                    "metis --output-format jsonl issues create --deps child-of:{parent_id} \"open child\""
                ),
                // Try to close the parent — should fail because the child is open.
                &format!("metis issues update {parent_id} --status closed"),
            ],
        )
        .await?;

    let message = format!("{:#}", failure.error);
    assert!(
        message.contains("cannot close issue with open child issues"),
        "expected error about open children, got: {message}"
    );

    // Now close the child via CLI, then close the parent.
    let issue_id2 = user.create_issue("closer worker 7a").await?;
    let job_id2 = user
        .create_job_for_issue(&repo, "close child 7a", &issue_id2)
        .await?;

    // List children of the parent to find the child ID.
    let issues = user.list_issues().await?;
    let child = find_summary_issue_by_description(&issues.issues, "open child")
        .expect("child issue should exist");
    let child_id = child.issue_id.clone();

    harness
        .run_worker(
            &job_id2,
            vec![
                // Close the child first.
                &format!("metis issues update {child_id} --status closed"),
                // Now close the parent — should succeed.
                &format!("metis issues update {parent_id} --status closed"),
            ],
        )
        .await?;

    // Verify both are closed.
    let parent = user.get_issue(&parent_id).await?;
    harness::IssueAssertions::assert_status(&parent, metis_common::issues::IssueStatus::Closed);

    let child = user.get_issue(&child_id).await?;
    harness::IssueAssertions::assert_status(&child, metis_common::issues::IssueStatus::Closed);

    Ok(())
}

/// Scenario 7b: Cannot close an issue with incomplete todos.
///
/// Creates an issue with todo items via a worker. Tries to close it with
/// incomplete todos (expecting failure), then marks all todos done and
/// closes successfully.
#[tokio::test]
async fn cannot_close_issue_with_incomplete_todos() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/lifecycle-7b")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/lifecycle-7b")?;

    let parent_id = user.create_issue("parent for 7b").await?;
    let issue_id = user.create_issue("worker issue 7b").await?;
    let job_id = user
        .create_job_for_issue(&repo, "test lifecycle 7b", &issue_id)
        .await?;

    // Worker creates an issue with todos, marks some done, then tries to close.
    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                // Create a fresh issue to work with.
                &format!(
                    "metis --output-format jsonl issues create --deps child-of:{parent_id} \"issue with todos\" | sed -n 's/^{{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee target_id.txt"
                ),
                // Add 3 todo items.
                "metis issues todo $(cat target_id.txt) --add \"task one\"",
                "metis issues todo $(cat target_id.txt) --add \"task two\"",
                "metis issues todo $(cat target_id.txt) --add \"task three\"",
                // Mark only 2 of 3 as done.
                "metis issues todo $(cat target_id.txt) --done 1",
                "metis issues todo $(cat target_id.txt) --done 2",
                // Try to close — should fail because todo 3 is incomplete.
                &format!(
                    "metis issues update $(cat target_id.txt) --status closed --deps child-of:{parent_id}"
                ),
            ],
        )
        .await?;

    let message = format!("{:#}", failure.error);
    assert!(
        message.contains("cannot close issue with incomplete todo items"),
        "expected error about incomplete todos, got: {message}"
    );

    // Now mark the last todo done and close successfully.
    let issue_id2 = user.create_issue("closer worker 7b").await?;
    let job_id2 = user
        .create_job_for_issue(&repo, "close todos 7b", &issue_id2)
        .await?;

    // Find the target issue.
    let issues = user.list_issues().await?;
    let target = find_summary_issue_by_description(&issues.issues, "issue with todos")
        .expect("target issue should exist");
    let target_id = target.issue_id.clone();

    harness
        .run_worker(
            &job_id2,
            vec![
                // Mark the last todo as done.
                &format!("metis issues todo {target_id} --done 3"),
                // Close the issue — should succeed now.
                &format!(
                    "metis issues update {target_id} --status closed --deps child-of:{parent_id}"
                ),
            ],
        )
        .await?;

    let target = user.get_issue(&target_id).await?;
    harness::IssueAssertions::assert_status(&target, metis_common::issues::IssueStatus::Closed);

    Ok(())
}

/// Scenario 7c: Cannot close an issue with open blockers.
///
/// Creates issue A and issue B (blocked-on A). Tries to close B (expecting
/// failure), then closes A and retries closing B (expecting success).
#[tokio::test]
async fn cannot_close_issue_with_open_blockers() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/lifecycle-7c")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/lifecycle-7c")?;

    let issue_id = user.create_issue("worker issue 7c").await?;
    let job_id = user
        .create_job_for_issue(&repo, "test lifecycle 7c", &issue_id)
        .await?;

    // Worker creates issue A, then B blocked-on A, then tries to close B.
    let failure = harness
        .run_worker_expect_failure(
            &job_id,
            vec![
                // Create blocker issue A.
                "metis --output-format jsonl issues create \"blocker A\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee blocker_id.txt",
                // Create issue B blocked-on A.
                "metis --output-format jsonl issues create --deps blocked-on:$(cat blocker_id.txt) \"blocked B\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee blocked_id.txt",
                // Try to close B — should fail because blocker A is open.
                "metis issues update $(cat blocked_id.txt) --status closed",
            ],
        )
        .await?;

    let message = format!("{:#}", failure.error);
    assert!(
        message.contains("blocked issues cannot close until blockers are closed"),
        "expected error about open blockers, got: {message}"
    );

    // Find the issues we created.
    let issues = user.list_issues().await?;
    let blocker = find_summary_issue_by_description(&issues.issues, "blocker A")
        .expect("blocker issue should exist");
    let blocked = find_summary_issue_by_description(&issues.issues, "blocked B")
        .expect("blocked issue should exist");
    let blocker_id = blocker.issue_id.clone();
    let blocked_id = blocked.issue_id.clone();

    // Close blocker A first, then close B.
    let issue_id2 = user.create_issue("closer worker 7c").await?;
    let job_id2 = user
        .create_job_for_issue(&repo, "close blocker 7c", &issue_id2)
        .await?;

    harness
        .run_worker(
            &job_id2,
            vec![
                &format!("metis issues update {blocker_id} --status closed"),
                &format!("metis issues update {blocked_id} --status closed"),
            ],
        )
        .await?;

    let blocker = user.get_issue(&blocker_id).await?;
    harness::IssueAssertions::assert_status(&blocker, metis_common::issues::IssueStatus::Closed);
    let blocked = user.get_issue(&blocked_id).await?;
    harness::IssueAssertions::assert_status(&blocked, metis_common::issues::IssueStatus::Closed);

    Ok(())
}

/// Scenario 7d: Failed/Dropped blockers are treated as terminal and unblock
/// the dependent issue.
///
/// The policy engine considers Closed, Dropped, Rejected, and Failed as
/// terminal blocker states. This test verifies that when a blocker is marked
/// as Failed, the blocked issue can be closed (the failed blocker counts as
/// resolved). Contrast with 7c where an *Open* blocker prevents closure.
#[tokio::test]
async fn failed_blocker_allows_closure() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/lifecycle-7d")
        .build()
        .await?;
    let user = harness.default_user();
    let repo = metis_common::RepoName::from_str("acme/lifecycle-7d")?;

    let issue_id = user.create_issue("worker issue 7d").await?;
    let job_id = user
        .create_job_for_issue(&repo, "test lifecycle 7d", &issue_id)
        .await?;

    // Worker creates issue A, then B blocked-on A, fails A, then closes B.
    // This should succeed because Failed is a terminal state for blockers.
    harness
        .run_worker(
            &job_id,
            vec![
                // Create blocker issue A.
                "metis --output-format jsonl issues create \"blocker A\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee blocker_id.txt",
                // Create issue B blocked-on A.
                "metis --output-format jsonl issues create --deps blocked-on:$(cat blocker_id.txt) \"blocked B\" | sed -n 's/^{\"issue_id\":\"\\([^\"]*\\)\".*/\\1/p' | tee blocked_id.txt",
                // Mark A as failed.
                "metis issues update $(cat blocker_id.txt) --status failed",
                // Close B — should succeed because failed blocker is terminal.
                "metis issues update $(cat blocked_id.txt) --status closed",
            ],
        )
        .await?;

    // Verify final states.
    let issues = user.list_issues().await?;
    let blocker = find_summary_issue_by_description(&issues.issues, "blocker A")
        .expect("blocker issue should exist");
    let blocked = find_summary_issue_by_description(&issues.issues, "blocked B")
        .expect("blocked issue should exist");

    harness::IssueAssertions::assert_status(blocker, metis_common::issues::IssueStatus::Failed);
    harness::IssueAssertions::assert_status(blocked, metis_common::issues::IssueStatus::Closed);

    Ok(())
}
