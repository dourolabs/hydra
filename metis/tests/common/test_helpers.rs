#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use httpmock::prelude::*;
use metis::client::MetisClient;
use metis::config::{AppConfig, ServerSection};
use metis_common::{
    constants::{ENV_GITHUB_ACCESS_TOKEN_URL, ENV_GITHUB_DEVICE_CODE_URL, ENV_METIS_TOKEN},
    jobs::SearchJobsQuery,
    task_status::Status,
    users::{User, Username},
    RepoName, TaskId,
};
use metis_server::{
    app::AppState,
    test_utils::{spawn_test_server_with_state, test_state_with_github_client},
};
use octocrab::Octocrab;
use serde_json::json;
use std::{path::Path, process::Command, str::FromStr};
use tempfile::TempDir;

use super::bash_commands::{BashCommands, CommandOutput};

pub struct GithubLoginMock {
    pub server: MockServer,
    pub device_code_url: String,
    pub access_token_url: String,
    pub github_token: String,
}

impl GithubLoginMock {
    pub fn new() -> Self {
        let server = MockServer::start();
        let github_token = "gh-token".to_string();
        server.mock(|when, then| {
            when.method(POST).path("/login/device/code");
            then.status(200).json_body(json!({
                "device_code": "device-code",
                "user_code": "USER-CODE",
                "verification_uri": "https://example.com/verify",
                "expires_in": 600,
                "interval": 1
            }));
        });
        server.mock(|when, then| {
            when.method(POST).path("/login/oauth/access_token");
            then.status(200).json_body(json!({
                "access_token": github_token
            }));
        });
        server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200).json_body(json!({
                "login": "octo",
                "id": 42,
                "node_id": "NODEID",
                "avatar_url": "https://example.com/avatar",
                "gravatar_id": "gravatar",
                "url": "https://example.com/user",
                "html_url": "https://example.com/user",
                "followers_url": "https://example.com/followers",
                "following_url": "https://example.com/following",
                "gists_url": "https://example.com/gists",
                "starred_url": "https://example.com/starred",
                "subscriptions_url": "https://example.com/subscriptions",
                "organizations_url": "https://example.com/orgs",
                "repos_url": "https://example.com/repos",
                "events_url": "https://example.com/events",
                "received_events_url": "https://example.com/received_events",
                "type": "User",
                "site_admin": false,
                "name": null,
                "patch_url": null,
                "email": null
            }));
        });

        let device_code_url = format!("{}/login/device/code", server.base_url());
        let access_token_url = format!("{}/login/oauth/access_token", server.base_url());

        Self {
            server,
            device_code_url,
            access_token_url,
            github_token,
        }
    }

    pub fn client(&self) -> Octocrab {
        Octocrab::builder()
            .base_uri(self.server.base_url())
            .expect("valid github base url")
            .personal_token(self.github_token.clone())
            .build()
            .expect("build github client")
    }
}

pub struct TestEnvironment {
    pub server: metis_server::test_utils::TestServer,
    pub app_config: AppConfig,
    pub client: MetisClient,
    pub _tempdir: TempDir,
    pub home_dir: TempDir,
    pub service_repo_name: RepoName,
    pub auth_token: String,
    pub github_mock: GithubLoginMock,
}

pub fn metis_bin() -> std::path::PathBuf {
    // Cargo exposes the compiled binary location to integration tests via CARGO_BIN_EXE_<binname>
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metis"))
}

impl TestEnvironment {
    pub async fn login(&self) -> Result<()> {
        self.run_as_user(vec!["metis login".to_string()]).await
    }

    pub async fn run_as_user(&self, commands: Vec<String>) -> Result<()> {
        for command in commands {
            if command.trim().is_empty() {
                continue;
            }

            let first_token = command.split_whitespace().next();
            let command_to_run = if first_token == Some("metis") {
                let metis_path = metis_bin();
                command.replacen("metis", &metis_path.to_string_lossy(), 1)
            } else {
                let metis_path = metis_bin();
                format!("{} {}", metis_path.to_string_lossy(), command)
            };

            let output = tokio::process::Command::new("bash")
                .arg("-c")
                .arg(&command_to_run)
                .env("HOME", self.home_dir.path())
                .env("METIS_SERVER_URL", &self.app_config.server.url)
                .env(
                    ENV_GITHUB_DEVICE_CODE_URL,
                    &self.github_mock.device_code_url,
                )
                .env(
                    ENV_GITHUB_ACCESS_TOKEN_URL,
                    &self.github_mock.access_token_url,
                )
                .env_remove(ENV_METIS_TOKEN)
                .env_remove("METIS_ISSUE_ID")
                .output()
                .await
                .context("failed to spawn metis command")?;

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow!(
                    "metis command '{command_to_run}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                    status = output.status
                ));
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn run_as_worker(
        &self,
        commands: Vec<String>,
        job_id: TaskId,
    ) -> Result<Vec<CommandOutput>> {
        self.run_as_worker_with_failure(commands, job_id, false)
            .await
    }

    pub async fn run_as_worker_with_failure(
        &self,
        commands: Vec<String>,
        job_id: TaskId,
        fail_after_run: bool,
    ) -> Result<Vec<CommandOutput>> {
        let temp_dir =
            tempfile::tempdir().context("failed to create temporary directory for worker")?;
        let worker_dir = temp_dir.path().to_path_buf();
        let token_dir = tempfile::tempdir().context("failed to create temp dir for token")?;
        let token_path = token_dir.path().join("auth-token");
        std::fs::write(&token_path, &self.auth_token)
            .context("failed to write worker auth token")?;

        let bash_commands = BashCommands::new_with_failure(commands, fail_after_run);

        let run_result = metis::command::jobs::worker_run::run(
            &self.client,
            job_id,
            worker_dir,
            None,
            None,
            &bash_commands,
            &token_path,
        )
        .await;

        let outputs = bash_commands.outputs();

        if let Err(err) = run_result {
            let formatted_output = format_command_outputs(&outputs);
            return Err(anyhow!(
                "failed to run worker commands: {err}\ncommand output:\n{formatted_output}"
            ));
        }

        Ok(outputs)
    }
}

fn format_command_outputs(outputs: &[CommandOutput]) -> String {
    outputs
        .iter()
        .map(|output| {
            format!(
                "command: {command}\nstatus: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                command = output.command,
                status = output.status,
                stdout = output.stdout.trim_end(),
                stderr = output.stderr.trim_end(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

pub async fn init_test_server_with_remote(repo_name: &str) -> Result<TestEnvironment> {
    let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
    let remote_url = init_service_remote(tempdir.path())?;
    let service_repo_name = RepoName::from_str(repo_name)
        .with_context(|| format!("failed to parse service repo name: {repo_name}"))?;
    let github_mock = GithubLoginMock::new();
    let (state, auth_token) =
        app_state_with_repo(&remote_url, &service_repo_name, github_mock.client()).await?;
    let server = spawn_test_server_with_state(state)
        .await
        .context("failed to start test server")?;
    let server_url = server.base_url();
    let home_dir = tempfile::tempdir().context("failed to create temp home for cli")?;

    let app_config = AppConfig {
        server: ServerSection {
            url: server_url.clone(),
        },
    };
    let client = MetisClient::from_config(&app_config, auth_token.clone())?;

    Ok(TestEnvironment {
        server,
        app_config,
        client,
        _tempdir: tempdir,
        home_dir,
        service_repo_name,
        auth_token,
        github_mock,
    })
}

pub async fn job_id_for_prompt(client: &MetisClient, prompt: &str) -> Result<TaskId> {
    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    jobs.into_iter()
        .find(|job| job.task.prompt == prompt)
        .map(|job| job.id)
        .ok_or_else(|| anyhow!("job with prompt '{prompt}' not found"))
}

pub async fn wait_for_status(
    client: &MetisClient,
    job_id: &TaskId,
    expected: Status,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("timed out waiting for job '{job_id}' to reach status {expected:?}");
        }

        let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
        if let Some(job) = jobs.iter().find(|job| &job.id == job_id) {
            if job.status_log.current_status() == expected {
                return Ok(());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn init_service_remote(base_dir: &Path) -> Result<String> {
    let workdir = base_dir.join("workdir");
    let remote_dir = base_dir.join("remote.git");
    let workdir_str = workdir
        .to_str()
        .ok_or_else(|| anyhow!("workdir path contains invalid UTF-8"))?;
    let remote_dir_str = remote_dir
        .to_str()
        .ok_or_else(|| anyhow!("remote dir path contains invalid UTF-8"))?;

    Command::new("git")
        .args(["init", workdir_str])
        .status()
        .context("failed to init workdir")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "checkout", "-b", "main"])
        .status()
        .context("failed to create main branch in workdir")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git checkout returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "-C",
            workdir_str,
            "config",
            "user.name",
            "Worker Integration",
        ])
        .status()
        .context("failed to set git user.name")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "-C",
            workdir_str,
            "config",
            "user.email",
            "worker@example.com",
        ])
        .status()
        .context("failed to set git user.email")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;
    std::fs::write(workdir.join("README.md"), "base content\n")
        .context("failed to write initial README")?;
    Command::new("git")
        .args(["-C", workdir_str, "add", "README.md"])
        .status()
        .context("failed to add README.md")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "commit", "-m", "initial commit"])
        .status()
        .context("failed to commit README")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

    Command::new("git")
        .args(["init", "--bare", remote_dir_str])
        .status()
        .context("failed to init bare remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git init --bare returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "remote", "add", "origin", remote_dir_str])
        .status()
        .context("failed to add remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git remote add returned non-zero exit code"))?;
    Command::new("git")
        .args(["-C", workdir_str, "push", "-u", "origin", "main"])
        .status()
        .context("failed to push initial commit to remote")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git push returned non-zero exit code"))?;
    Command::new("git")
        .args([
            "--git-dir",
            remote_dir_str,
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .context("failed to set remote HEAD")?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("git symbolic-ref returned non-zero exit code"))?;

    Ok(remote_dir_str.to_string())
}

async fn app_state_with_repo(
    remote_url: &str,
    repo_name: &RepoName,
    github_client: Octocrab,
) -> Result<(AppState, String)> {
    let state = test_state_with_github_client(github_client);
    let auth_token = {
        let mut store = state.store.write().await;
        store
            .add_repository(
                repo_name.clone(),
                metis_common::repositories::ServiceRepositoryConfig::new(
                    remote_url.to_string(),
                    Some("main".to_string()),
                    None,
                ),
            )
            .await?;

        let (_actor, auth_token) = store.create_actor_for_task(TaskId::new()).await?;
        let user = User::new(Username::from("test-user"), auth_token.clone());
        store.add_user(user.into()).await?;
        auth_token
    };

    Ok((state, auth_token))
}
