use anyhow::{anyhow, Context, Result};
use metis::cli;
use metis::client::MetisClient;
use metis::config::{AppConfig, ServerSection};
use metis_common::RepoName;
use metis_server::{
    app::{AppState, ServiceState},
    config::{Repository as ServiceRepo, ServiceSection},
    store::{MemoryStore, Store},
    test_utils::{spawn_test_server_with_state, test_app_config, MockJobEngine},
};
use std::{path::Path, process::Command, str::FromStr, sync::Arc};
use tempfile::TempDir;
use tokio::sync::RwLock;

use crate::BashCommands;

pub struct TestEnvironment {
    pub server: metis_server::test_utils::TestServer,
    pub app_config: AppConfig,
    pub client: MetisClient,
    pub tempdir: TempDir,
    pub remote_url: String,
    pub service_repo_name: RepoName,
}

impl TestEnvironment {
    /// Run metis commands as a user via the CLI.
    pub async fn run_as_user(&self, commands: Vec<String>) -> Result<()> {
        for command in commands {
            let tokens: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();

            // Skip if empty
            if tokens.is_empty() {
                continue;
            }

            // Build args starting with "metis" if not already present
            let args: Vec<&str> = if tokens.first().map(|s| s.as_str()) == Some("metis") {
                tokens.iter().map(|s| s.as_str()).collect()
            } else {
                let mut args = vec!["metis"];
                args.extend(tokens.iter().map(|s| s.as_str()));
                args
            };

            cli::run_with_client_and_config(args, &self.client, &self.app_config).await?;
        }

        Ok(())
    }

    /// Run commands as a worker using BashCommands and worker-run functionality.
    pub async fn run_as_worker(
        &self,
        commands: Vec<String>,
        job_id: metis_common::TaskId,
    ) -> Result<()> {
        // Create a tempdir that will be kept alive for the duration of worker_run
        let temp_dir =
            tempfile::tempdir().context("failed to create temporary directory for worker")?;
        let worker_dir = temp_dir.path().to_path_buf();

        // Create a new client and config clone for BashCommands
        let client_clone = MetisClient::new(&self.app_config.server.url)?;
        let app_config_clone = AppConfig {
            server: ServerSection {
                url: self.app_config.server.url.clone(),
            },
        };

        let bash_commands = BashCommands {
            commands,
            client: Box::new(client_clone),
            app_config: app_config_clone,
        };

        // Keep temp_dir alive for the duration of worker_run (it will be dropped at end of function)
        metis::command::worker_run::run(&self.client, job_id, worker_dir, None, &bash_commands)
            .await
            .context("failed to run worker commands")?;

        Ok(())
    }
}

/// Initialize a test server with a remote repository and return the test environment.
pub async fn init_test_server_with_remote(repo_name: &str) -> Result<TestEnvironment> {
    let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
    let remote_url = init_service_remote(tempdir.path())?;
    let service_repo_name = RepoName::from_str(repo_name)
        .with_context(|| format!("failed to parse service repo name: {repo_name}"))?;
    let state = app_state_with_repo(&remote_url, &service_repo_name)?;
    let server = spawn_test_server_with_state(state)
        .await
        .context("failed to start test server")?;
    let server_url = server.base_url();

    let app_config = AppConfig {
        server: ServerSection {
            url: server_url.clone(),
        },
    };
    let client = MetisClient::from_config(&app_config)?;

    Ok(TestEnvironment {
        server,
        app_config,
        client,
        tempdir,
        remote_url,
        service_repo_name,
    })
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

fn app_state_with_repo(remote_url: &str, repo_name: &RepoName) -> Result<AppState> {
    let mut service_section = ServiceSection::default();
    service_section.repositories.insert(
        repo_name.clone(),
        ServiceRepo {
            remote_url: remote_url.to_string(),
            default_branch: Some("main".to_string()),
            github_token: None,
            default_image: None,
        },
    );

    let mut server_config = test_app_config();
    server_config.service = service_section.clone();

    Ok(AppState {
        config: Arc::new(server_config),
        service_state: Arc::new(ServiceState::from_config(&service_section)),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()) as Box<dyn Store>)),
        job_engine: Arc::new(MockJobEngine::new()),
        spawners: Vec::new(),
    })
}
