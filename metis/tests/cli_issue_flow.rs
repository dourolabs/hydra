use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::ENV_METIS_SERVER_URL,
    issues::{IssueStatus, SearchIssuesQuery},
    users::{CreateUserRequest, Username},
};
use metis_server::test_utils;
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let server = test_utils::spawn_test_server().await?;
    let app_config = AppConfig {
        server: ServerSection {
            url: server.base_url(),
        },
    };
    let client = MetisClient::from_config(&app_config)?;
    let temp_home = tempfile::tempdir().context("temp home")?;
    let token = "integration-token";
    write_auth_token(&temp_home, token)?;
    client
        .create_user(&CreateUserRequest::new(
            Username::from("integration-user"),
            token.to_string(),
        ))
        .await
        .context("create integration user")?;

    let description = "integration flow issue";

    run_metis_command(
        &["issues", "create", description],
        &app_config,
        temp_home.path(),
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

    run_metis_command(&["issues", "list"], &app_config, temp_home.path()).await?;

    assert_eq!(created.issue.status, IssueStatus::Open);

    Ok(())
}

fn write_auth_token(temp_home: &TempDir, token: &str) -> Result<()> {
    let path = temp_home.path().join(".local/share/metis/auth-token");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create auth token dir")?;
    }
    fs::write(&path, token).context("write auth token")?;
    Ok(())
}

async fn run_metis_command(
    args: &[&str],
    app_config: &AppConfig,
    home_dir: &std::path::Path,
) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(ENV_METIS_SERVER_URL, &app_config.server.url)
        .env("HOME", home_dir)
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
