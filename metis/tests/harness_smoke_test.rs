mod harness;

use anyhow::Result;
use harness::{IssueAssertions, JobAssertions, PatchAssertions};
use metis_common::issues::{IssueStatus, SearchIssuesQuery};
use metis_common::patches::PatchStatus;
use metis_common::task_status::Status;
use std::str::FromStr;
use std::time::Duration;

/// Verify that `TestHarness::new().await` succeeds and produces a reachable server.
#[tokio::test]
async fn harness_new_creates_server() -> Result<()> {
    let harness = harness::TestHarness::new().await?;

    // state() is accessible
    let _state = harness.state();

    // server_url() returns a reachable URL
    let url = harness.server_url();
    assert!(url.starts_with("http://"));

    // Can make a simple API call
    let client = harness.client()?;
    let response = client.list_issues(&SearchIssuesQuery::default()).await?;
    assert!(response.issues.is_empty());

    Ok(())
}

/// Verify that the builder with `.with_repo()` and `.with_github()` works.
#[tokio::test]
async fn harness_builder_with_repo_and_github() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/test")
        .with_github()
        .build()
        .await?;

    // Server is reachable
    let client = harness.client()?;
    let response = client.list_issues(&SearchIssuesQuery::default()).await?;
    assert!(response.issues.is_empty());

    // GitHub mock is configured
    assert!(harness.github().is_some());

    // Git remote is registered
    let remote = harness.remote("acme/test");
    assert!(remote.branch_exists("main"));

    Ok(())
}

/// Verify that multiple repos and users can be registered.
#[tokio::test]
async fn harness_builder_multiple_repos_and_users() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("org/repo-a")
        .with_repo("org/repo-b")
        .with_user("alice")
        .with_user("bob")
        .build()
        .await?;

    // Both remotes exist
    assert!(harness.remote("org/repo-a").branch_exists("main"));
    assert!(harness.remote("org/repo-b").branch_exists("main"));

    // Both named users + default user exist
    let _default_token = harness.default_user_token();
    let alice = harness.user("alice");
    let bob = harness.user("bob");
    assert_ne!(alice.token(), bob.token());

    // Clients for named users can also make API calls
    let alice_client = harness.client_for("alice")?;
    let response = alice_client
        .list_issues(&SearchIssuesQuery::default())
        .await?;
    assert!(response.issues.is_empty());

    Ok(())
}

/// Verify that `default_user()` returns a UserHandle and that issue
/// operations work through the typed API.
#[tokio::test]
async fn user_handle_create_and_get_issue() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // Create an issue via the typed API
    let issue_id = user.create_issue("test issue via UserHandle").await?;

    // Retrieve it
    let record = user.get_issue(&issue_id).await?;
    assert_eq!(record.issue.description, "test issue via UserHandle");
    assert_eq!(record.issue.status, IssueStatus::Open);

    // List issues — should contain the one we just created
    let list = user.list_issues().await?;
    assert_eq!(list.issues.len(), 1);
    assert_eq!(list.issues[0].id, issue_id);

    Ok(())
}

/// Verify that `create_child_issue` creates an issue with the correct dependency.
#[tokio::test]
async fn user_handle_create_child_issue() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent_id = user.create_issue("parent issue").await?;
    let child_id = user.create_child_issue(&parent_id, "child issue").await?;

    let child = user.get_issue(&child_id).await?;
    assert_eq!(child.issue.description, "child issue");
    assert!(!child.issue.dependencies.is_empty());
    assert_eq!(child.issue.dependencies[0].issue_id, parent_id);

    Ok(())
}

/// Verify that `update_issue_status` changes the status of an issue.
#[tokio::test]
async fn user_handle_update_issue_status() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let issue_id = user.create_issue("status test").await?;

    // Update status to InProgress
    user.update_issue_status(&issue_id, IssueStatus::InProgress)
        .await?;

    let record = user.get_issue(&issue_id).await?;
    assert_eq!(record.issue.status, IssueStatus::InProgress);

    Ok(())
}

/// Verify that named users have separate UserHandle instances with typed APIs.
#[tokio::test]
async fn user_handle_named_user() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("test-org/test-repo")
        .with_user("agent-1")
        .build()
        .await?;

    let agent = harness.user("agent-1");
    assert_eq!(agent.name(), "agent-1");

    let issue_id = agent.create_issue("agent-created issue").await?;
    let record = agent.get_issue(&issue_id).await?;
    assert_eq!(record.issue.description, "agent-created issue");

    Ok(())
}

/// Verify that `create_job` works through the UserHandle API.
#[tokio::test]
async fn user_handle_create_job() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/app")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/app")?;
    let job_id = user.create_job(&repo, "do something").await?;

    // Verify the job exists via the underlying client
    let job = user.client().get_job(&job_id).await?;
    assert_eq!(job.task.prompt, "do something");

    Ok(())
}

/// Verify that CLI subprocess invocation works through UserHandle.
#[tokio::test]
async fn user_handle_cli_list_issues() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // `issues list` with no issues should succeed
    let output = user.cli(&["issues", "list"]).await?;
    assert!(output.status.success());

    Ok(())
}

/// Verify that `cli_expect_failure` captures non-zero exit.
#[tokio::test]
async fn user_handle_cli_expect_failure() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    // A clearly invalid subcommand should fail
    let output = user.cli_expect_failure(&["--invalid-flag"]).await?;
    assert!(!output.status.success());

    Ok(())
}

// ── Assertion trait smoke tests ─────────────────────────────────────

/// Verify `IssueAssertions::assert_status` passes for correct status.
#[tokio::test]
async fn issue_assert_status() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let issue_id = user.create_issue("assert status test").await?;
    let issue = user.get_issue(&issue_id).await?;
    issue.assert_status(IssueStatus::Open);

    user.update_issue_status(&issue_id, IssueStatus::InProgress)
        .await?;
    let issue = user.get_issue(&issue_id).await?;
    issue.assert_status(IssueStatus::InProgress);

    Ok(())
}

/// Verify `IssueAssertions::assert_has_child_with_status` finds matching children.
#[tokio::test]
async fn issue_assert_has_child_with_status() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let parent_id = user.create_issue("parent for child assertion").await?;
    let _child_id = user
        .create_child_issue(&parent_id, "child task one")
        .await?;

    let all_issues = user.list_issues().await?.issues;
    let parent = user.get_issue(&parent_id).await?;

    // Child exists with Open status and matching description
    parent.assert_has_child_with_status(&all_issues, "child task", IssueStatus::Open);

    Ok(())
}

/// Verify `IssueAssertions::assert_todo_count` checks the count correctly.
#[tokio::test]
async fn issue_assert_todo_count() -> Result<()> {
    let harness = harness::TestHarness::new().await?;
    let user = harness.default_user();

    let issue_id = user.create_issue("todo count test").await?;
    let issue = user.get_issue(&issue_id).await?;

    // New issue has no todos
    issue.assert_todo_count(0);

    Ok(())
}

/// Verify `PatchAssertions::assert_status` works on patches.
#[tokio::test]
async fn patch_assert_status() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/patch-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/patch-test")?;
    let patch_id = user
        .create_patch("test patch", "a description", &repo)
        .await?;
    let patch = user.get_patch(&patch_id).await?;

    patch.assert_status(PatchStatus::Open);

    Ok(())
}

/// Verify `JobAssertions::assert_status` works on job records.
#[tokio::test]
async fn job_assert_status() -> Result<()> {
    let harness = harness::TestHarness::builder()
        .with_repo("acme/job-test")
        .build()
        .await?;
    let user = harness.default_user();

    let repo = metis_common::RepoName::from_str("acme/job-test")?;
    let job_id = user.create_job(&repo, "assertion test job").await?;
    let job = user.client().get_job(&job_id).await?;

    job.assert_status(Status::Created);

    Ok(())
}

/// Verify `wait_until` returns Ok when the condition is immediately true.
#[tokio::test]
async fn wait_until_immediate_success() -> Result<()> {
    harness::wait_until(
        Duration::from_secs(1),
        Duration::from_millis(10),
        "condition that is immediately true",
        || async { true },
    )
    .await?;

    Ok(())
}

/// Verify `wait_until` returns an error on timeout.
#[tokio::test]
async fn wait_until_timeout_error() -> Result<()> {
    let result = harness::wait_until(
        Duration::from_millis(100),
        Duration::from_millis(10),
        "condition that never becomes true",
        || async { false },
    )
    .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("timed out"),
        "error should mention timeout: {err_msg}"
    );
    assert!(
        err_msg.contains("condition that never becomes true"),
        "error should include description: {err_msg}"
    );

    Ok(())
}

/// Verify `wait_until` polls until condition becomes true.
#[tokio::test]
async fn wait_until_polls_until_true() -> Result<()> {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    harness::wait_until(
        Duration::from_secs(2),
        Duration::from_millis(10),
        "counter to reach 3",
        move || {
            let c = counter_clone.clone();
            async move {
                let val = c.fetch_add(1, Ordering::SeqCst);
                val >= 3
            }
        },
    )
    .await?;

    // Counter should have been incremented at least 4 times (0, 1, 2, 3 -> true at 3)
    assert!(counter.load(Ordering::SeqCst) >= 4);

    Ok(())
}
