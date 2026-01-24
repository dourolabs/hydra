use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::ENV_METIS_SERVER_URL,
    issues::{IssueStatus, SearchIssuesQuery},
    users::{User, Username},
};
use metis_server::domain::issues::{Issue, IssueStatus as DomainIssueStatus, IssueType};
use metis_server::domain::users::Username as DomainUsername;
use metis_server::test_utils;
use std::{fs, path::Path};
use tempfile::tempdir;

const TEST_METIS_TOKEN: &str = "token-123";

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let state = test_utils::test_state();
    let parent_id = {
        let mut store = state.store.write().await;
        let user = User::new(Username::from("test-user"), TEST_METIS_TOKEN.to_string());
        store.add_user(user.into()).await?;
        let parent = Issue::new(
            IssueType::Task,
            "parent issue".to_string(),
            DomainUsername::from("test-user"),
            String::new(),
            DomainIssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        store.add_issue(parent).await?
    };
    let server = test_utils::spawn_test_server_with_state(state).await?;
    let app_config = AppConfig {
        server: ServerSection {
            url: server.base_url(),
        },
    };
    let client = MetisClient::from_config(&app_config, TEST_METIS_TOKEN)?;
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, TEST_METIS_TOKEN).context("write auth token")?;

    let description = "integration flow issue";

    run_metis_command(
        &[
            "issues",
            "create",
            "--deps",
            &format!("child-of:{parent_id}"),
            description,
        ],
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

async fn run_metis_command(args: &[&str], app_config: &AppConfig, home_dir: &Path) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(ENV_METIS_SERVER_URL, &app_config.server.url)
        .env("METIS_TOKEN", TEST_METIS_TOKEN)
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
