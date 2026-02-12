mod harness;

use anyhow::Result;
use metis_common::issues::{IssueStatus, SearchIssuesQuery};
use std::str::FromStr;

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
