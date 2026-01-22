use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::ENV_METIS_SERVER_URL,
    issues::{IssueStatus, SearchIssuesQuery},
};
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

    run_metis_command(
        &["issues", "create", "--creator", "test-user", description],
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

    run_metis_command(&["issues", "list"], &app_config).await?;

    assert_eq!(created.issue.status, IssueStatus::Open);

    Ok(())
}

async fn run_metis_command(args: &[&str], app_config: &AppConfig) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(ENV_METIS_SERVER_URL, &app_config.server.url)
        .output()
        .await
        .context("failed to spawn metis CLI command")?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "metis {:?} failed with status {}.\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status,
            stdout,
            stderr,
        );
    }

    Ok(())
}
