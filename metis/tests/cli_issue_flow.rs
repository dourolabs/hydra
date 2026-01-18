use anyhow::{anyhow, Result};
use metis::{
    cli,
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::issues::{IssueStatus, SearchIssuesQuery};
use metis_server::test_utils;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let server = test_utils::spawn_test_server().await?;
    let app_config = AppConfig {
        server: ServerSection {
            url: server.base_url(),
        },
    };
    let client = MetisClient::from_config(&app_config)?;

    let description = "integration flow issue";

    cli::run_with_client_and_config(
        ["metis", "issues", "create", description],
        &client,
        &app_config,
    )
    .await?;

    let issues = client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let created = issues
        .iter()
        .find(|issue| issue.issue.description == description)
        .ok_or_else(|| anyhow!("expected issue to be created"))?;

    cli::run_with_client_and_config(["metis", "issues", "list"], &client, &app_config).await?;

    assert_eq!(created.issue.status, IssueStatus::Open);

    Ok(())
}
