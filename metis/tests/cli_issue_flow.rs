use anyhow::{anyhow, Context, Result};
use metis::{
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::{ENV_METIS_ISSUE_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN},
    issues::{Issue, IssueStatus, IssueType, JobSettings, SearchIssuesQuery},
    users::Username,
    IssueId, RepoName, TaskId,
};
use metis_server::test_utils;
use std::{fs, path::Path, str::FromStr};
use tempfile::tempdir;

#[tokio::test]
async fn cli_issue_flow_creates_and_lists_issue() -> Result<()> {
    let handles = test_utils::test_state_handles();
    let state = handles.state;
    let (auth_token, parent_id) = {
        let (actor, auth_token) = metis_server::domain::actors::Actor::new_for_task(TaskId::new());
        handles.store.add_actor(actor).await?;
        let mut parent_job_settings = JobSettings::default();
        parent_job_settings.repo_name = Some(RepoName::from_str("acme/cli-flow").unwrap());
        parent_job_settings.remote_url = Some("https://example.com/cli-flow.git".into());
        parent_job_settings.image = Some("worker:latest".into());
        parent_job_settings.branch = Some("feature/cli-flow".into());
        let parent_issue = Issue::new(
            IssueType::Task,
            "parent issue".into(),
            Username::from("test-user"),
            String::new(),
            IssueStatus::Open,
            None,
            Some(parent_job_settings.clone()),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        );
        let parent_id = handles.store.add_issue(parent_issue.into()).await?;
        (auth_token, parent_id)
    };
    let server = test_utils::spawn_test_server_with_state(state, handles.store).await?;
    let app_config = AppConfig {
        servers: vec![ServerSection {
            url: server.base_url(),
            auth_token: None,
            default: true,
        }],
    };
    let client = MetisClient::from_config(&app_config, &auth_token)?;
    let temp_home = tempdir().context("create temp home")?;
    let auth_token_path = temp_home.path().join(".local/share/metis/auth-token");
    fs::create_dir_all(auth_token_path.parent().expect("auth token parent"))
        .context("create auth token dir")?;
    fs::write(&auth_token_path, &auth_token).context("write auth token")?;

    let description = "integration flow issue";

    eprintln!("running metis issues create");
    let deps_arg = format!("child-of:{parent_id}");
    run_metis_command(
        &["issues", "create", "--deps", &deps_arg, description],
        &app_config,
        temp_home.path(),
        &auth_token,
        &parent_id,
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
        &parent_id,
    )
    .await?;
    eprintln!("metis issues list complete");

    assert_eq!(created.issue.status, IssueStatus::Open);
    assert_eq!(
        created.issue.job_settings.repo_name,
        Some(RepoName::from_str("acme/cli-flow").unwrap())
    );
    assert_eq!(
        created.issue.job_settings.remote_url,
        Some("https://example.com/cli-flow.git".into())
    );
    assert_eq!(
        created.issue.job_settings.image,
        Some("worker:latest".into())
    );
    assert_eq!(
        created.issue.job_settings.branch,
        Some("feature/cli-flow".into())
    );

    Ok(())
}

async fn run_metis_command(
    args: &[&str],
    app_config: &AppConfig,
    home_dir: &Path,
    auth_token: &str,
    current_issue_id: &IssueId,
) -> Result<()> {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_metis"))
        .args(args)
        .env(
            ENV_METIS_SERVER_URL,
            &app_config.default_server().expect("default server").url,
        )
        .env(ENV_METIS_TOKEN, auth_token)
        .env("HOME", home_dir)
        .env(ENV_METIS_ISSUE_ID, current_issue_id.as_ref())
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
