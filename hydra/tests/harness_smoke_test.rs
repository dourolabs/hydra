mod harness;

use anyhow::Result;
use hydra_common::issues::SearchIssuesQuery;

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

/// Verify that multiple repos and users can be registered, with separate
/// authentication tokens and functional API access for each user.
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
