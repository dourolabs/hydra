use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::{ENV_METIS_SERVER_URL, ENV_METIS_TOKEN},
    issues::{IssueStatus, SearchIssuesQuery},
    users::{User, Username},
    TaskId,
};
use metis_server::test_utils;
use std::{fs, path::Path};
use tempfile::tempdir;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let state = test_utils::test_state();
    let auth_token = {
        let mut store = state.store.write().await;
        let (_actor, auth_token) = store.create_actor_for_task(TaskId::new()).await?;
        let user = User::new(Username::from("test-user"), auth_token.clone());
        store.add_user(user.into()).await?;
        auth_token
    };
    let server = test_utils::spawn_test_server_with_state(state).await?;
    let app_config = AppConfig {
        server: ServerSection {
            url: server.base_url(),
        },
    };
    let client = MetisClient::from_config(&app_config, &auth_token)?;
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, &auth_token).context("write auth token")?;

    let description = "integration flow issue";

    eprintln!("running metis issues create");
    run_metis_command(
        &["issues", "create", description],
        &app_config,
        temp_home.path(),
        &auth_token,
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
        &auth_token,
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
    auth_token: &str,
) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(ENV_METIS_SERVER_URL, &app_config.server.url)
        .env(ENV_METIS_TOKEN, auth_token)
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
