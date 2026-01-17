use anyhow::{anyhow, bail, Context, Result};
use metis::{
    cli,
    client::MetisClient,
    config::{AppConfig, ServerSection},
};
use metis_common::{
    constants::ENV_METIS_SERVER_URL, jobs::SearchJobsQuery, patches::SearchPatchesQuery,
    task_status::Status, RepoName, TaskId,
};
use metis_server::{
    app::{AppState, ServiceState},
    config::{Repository as ServiceRepo, ServiceSection},
    job_engine::MockJobEngine,
    store::{MemoryStore, Store},
    test_utils::{spawn_test_server_with_state, test_app_config},
};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::Arc,
    time::Instant,
};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn worker_run_creates_patch_via_override_command() -> Result<()> {
    let tempdir = tempfile::tempdir().context("failed to create tempdir for worker test")?;
    let remote_url = init_service_remote(tempdir.path())?;
    let service_repo_name = RepoName::from_str("acme/worker-test")
        .context("failed to parse service repo name for worker test")?;
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
    let prompt = "worker integration patch flow";
    let repo_arg = service_repo_name.to_string();

    cli::run_with_client_and_config(
        [
            "metis",
            "jobs",
            "create",
            "--repo",
            &repo_arg,
            "--var",
            &format!("{ENV_METIS_SERVER_URL}={server_url}"),
            prompt,
        ],
        &client,
        &app_config,
    )
    .await?;

    let job_id = job_id_for_prompt(&client, prompt)
        .await
        .context("expected job to be created for worker test")?;
    wait_for_status(&client, &job_id, Status::Running).await?;

    let metis_bin = metis_binary_path()?;
    let metis_bin = metis_bin
        .to_str()
        .ok_or_else(|| anyhow!("metis binary path contains invalid UTF-8"))?;
    let run_guard = EnvGuard::set(
        "METIS_WORKER_RUN_CMD",
        format!(
            r#"
set -e
echo "worker content" >> README.md
git add README.md
git commit -m "worker update" >/dev/null
git push origin HEAD >/dev/null
"{metis_bin}" patches create --title "integration worker patch" --description "created by worker override" --base "${{METIS_BASE_COMMIT:-$(git rev-parse HEAD~1)}}" >/dev/null
echo "worker run finished"
"#
        ),
    );
    let login_guard = EnvGuard::set("METIS_WORKER_LOGIN_CMD", "echo \"skipping codex login\"");

    let worker_dir = tempdir.path().join("worker-dir");
    cli::run_with_client_and_config(
        [
            "metis",
            "jobs",
            "worker-run",
            job_id.as_ref(),
            worker_dir
                .to_str()
                .ok_or_else(|| anyhow!("worker path contains invalid UTF-8"))?,
        ],
        &client,
        &app_config,
    )
    .await?;
    drop(run_guard);
    drop(login_guard);

    let patches = client
        .list_patches(&SearchPatchesQuery { q: None })
        .await?
        .patches;
    let non_backup_patch = patches
        .iter()
        .find(|patch| !patch.patch.is_automatic_backup)
        .ok_or_else(|| anyhow!("expected worker override to create a non-backup patch"))?;
    assert_eq!(non_backup_patch.patch.service_repo_name, service_repo_name);
    assert_eq!(non_backup_patch.patch.title, "integration worker patch");

    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    let status = jobs
        .iter()
        .find(|job| job.id == job_id)
        .map(|job| job.status_log.current_status())
        .ok_or_else(|| anyhow!("job should still exist after worker run"))?;
    assert_eq!(status, Status::Complete);

    Ok(())
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

async fn job_id_for_prompt(client: &MetisClient, prompt: &str) -> Result<TaskId> {
    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    jobs.into_iter()
        .find(|job| job.task.prompt == prompt)
        .map(|job| job.id)
        .ok_or_else(|| anyhow!("job with prompt '{prompt}' not found"))
}

async fn wait_for_status(client: &MetisClient, job_id: &TaskId, expected: Status) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            bail!("timed out waiting for job '{job_id}' to reach status {expected:?}");
        }

        let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
        if let Some(job) = jobs.iter().find(|job| &job.id == job_id) {
            if job.status_log.current_status() == expected {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(50)).await;
    }
}

fn workspace_target_dir() -> PathBuf {
    match env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("CARGO_MANIFEST_DIR should have a parent for workspace root")
            .join("target"),
    }
}

fn metis_binary_path() -> Result<PathBuf> {
    let mut candidate = workspace_target_dir().join("debug").join("metis");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }

    if !candidate.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "metis", "--bin", "metis"])
            .status()
            .context("failed to build metis binary for worker integration test")?;
        if !status.success() {
            bail!("cargo build for metis binary failed with status {status}");
        }
    }

    if candidate.exists() {
        return Ok(candidate);
    }

    bail!("metis binary not found at {}", candidate.display());
}

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl Into<String>) -> Self {
        let original = env::var(key).ok();
        env::set_var(key, value.into());
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.original {
            env::set_var(self.key, value);
        } else {
            env::remove_var(self.key);
        }
    }
}
