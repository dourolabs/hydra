use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::{ENV_GITHUB_ACCESS_TOKEN_URL, ENV_GITHUB_DEVICE_CODE_URL, ENV_METIS_SERVER_URL},
    issues::{IssueStatus, SearchIssuesQuery},
};
use metis_server::test_utils;
use std::{fs, path::Path};
use tempfile::tempdir;

mod common;
use common::test_helpers::GithubLoginMock;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let github_mock = GithubLoginMock::new();
    let state = test_utils::test_state_with_github_client(github_mock.client());
    let server = test_utils::spawn_test_server_with_state(state).await?;
    let app_config = AppConfig {
        server: ServerSection {
            url: server.base_url(),
        },
    };
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");

    let description = "integration flow issue";

    eprintln!("running metis login");
    run_metis_command(&["login"], &app_config, temp_home.path(), &github_mock).await?;
    eprintln!("metis login complete");

    let auth_token = fs::read_to_string(&auth_token_path).context("read auth token")?;
    let client = MetisClient::from_config(&app_config, auth_token.trim())?;

    eprintln!("running metis issues create");
    run_metis_command(
        &["issues", "create", description],
        &app_config,
        temp_home.path(),
        &github_mock,
    )
    .await?;
    eprintln!("metis issues create complete");

    let issues = client
        .list_issues(&SearchIssuesQuery::default())
        .await?
        .issues;
    let created = issues
        .iter()
        .find(|issue| issue.issue.description == description)
        .ok_or_else(|| anyhow!("expected issue to be created"))?;

    eprintln!("running metis issues list");
    run_metis_command(
        &["issues", "list"],
        &app_config,
        temp_home.path(),
        &github_mock,
    )
    .await?;
    eprintln!("metis issues list complete");

    assert_eq!(created.issue.status, IssueStatus::Open);

    Ok(())
}

async fn run_metis_command(
    args: &[&str],
    app_config: &AppConfig,
    home_dir: &Path,
    github_mock: &GithubLoginMock,
) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(ENV_METIS_SERVER_URL, &app_config.server.url)
        .env(ENV_GITHUB_DEVICE_CODE_URL, &github_mock.device_code_url)
        .env(ENV_GITHUB_ACCESS_TOKEN_URL, &github_mock.access_token_url)
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
