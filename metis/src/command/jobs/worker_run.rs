use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use metis_common::{
    constants::ENV_GH_TOKEN,
    job_status::JobStatusUpdate,
    jobs::{Bundle, WorkerContext},
    patches::GitOid,
    RepoName, TaskId,
};
use tempfile::Builder;

use crate::client::MetisClientInterface;
use crate::command::patches::{create_patch_artifact_from_repo, resolve_service_repo_name};
use crate::git::{clone_repo, configure_repo, resolve_head_oid, workdir_diff};
use crate::worker_commands::WorkerCommands;

pub async fn run(
    client: &dyn MetisClientInterface,
    job: TaskId,
    dest: PathBuf,
    openai_api_key: Option<String>,
    commands: &dyn WorkerCommands,
) -> Result<()> {
    let WorkerContext {
        request_context,
        variables,
        prompt,
        ..
    } = client.get_job_context(&job).await?;
    let service_repo_name = resolve_service_repo_name(client, Some(&job)).await?;
    ensure_clean_destination(&dest)?;
    let github_token = variables.get(ENV_GH_TOKEN).cloned();
    let mut execution_env = variables;
    ensure_color_output_env(&mut execution_env);
    let base_commit = match request_context {
        Bundle::None => {
            fs::create_dir_all(&dest).with_context(|| format!("failed to create {dest:?}"))?;
            None
        }
        Bundle::GitRepository { url, rev } => {
            clone_repo(&url, &rev, &dest, github_token.as_deref())
                .context("failed to clone repository")?;
            configure_repo(&dest, "Metis Worker", "metis-worker@example.com")
                .context("failed to configure git repository")?;
            resolve_head_oid(&dest).context("failed to resolve HEAD commit")?
        }
    };

    let output_dir = Builder::new()
        .prefix("codex-output")
        .tempdir()
        .context("failed to create temporary codex output directory")?;
    let output_path = output_dir.path().join(crate::constants::OUTPUT_TXT_FILE);

    let last_message = commands
        .run(
            &prompt,
            openai_api_key.clone(),
            &dest,
            &execution_env,
            &output_path,
        )
        .await?;

    submit_patch_artifact_if_present(
        client,
        &job,
        &dest,
        &last_message,
        &service_repo_name,
        base_commit,
    )
    .await?;
    submit_job_status(client, &job, &last_message).await?;

    Ok(())
}

fn ensure_clean_destination(dest: &Path) -> Result<()> {
    if dest.exists() {
        let mut entries =
            fs::read_dir(dest).with_context(|| format!("failed to read directory {dest:?}"))?;
        if entries.next().is_some() {
            return Err(anyhow!(
                "destination {dest:?} is not empty; choose an empty or new directory"
            ));
        }
        Ok(())
    } else {
        fs::create_dir_all(dest).with_context(|| format!("failed to create {dest:?}"))
    }
}

fn ensure_color_output_env(env: &mut HashMap<String, String>) {
    env.entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
    env.entry("CLICOLOR_FORCE".to_string())
        .or_insert_with(|| "1".to_string());
    env.entry("FORCE_COLOR".to_string())
        .or_insert_with(|| "1".to_string());
}

async fn submit_job_status(
    client: &dyn MetisClientInterface,
    job: &TaskId,
    last_message: &str,
) -> Result<()> {
    println!("Updating status for job '{job}' via metis-server…");
    let response = client
        .set_job_status(
            job,
            &JobStatusUpdate::Complete {
                last_message: Some(last_message.to_string()),
            },
        )
        .await?;
    println!(
        "Status updated for job '{}'. Stored last message length: {}",
        response.job_id,
        last_message.len(),
    );
    Ok(())
}

async fn submit_patch_artifact_if_present(
    client: &dyn MetisClientInterface,
    job: &TaskId,
    dest: &Path,
    last_message: &str,
    service_repo_name: &RepoName,
    base_commit: Option<GitOid>,
) -> Result<()> {
    let (title, description) = patch_metadata(job, last_message);
    let create_github_pr = false;
    let is_automatic_backup = true;

    let Some(_) = base_commit else {
        println!("No git repository detected; skipping patch submission for job '{job}'.");
        return Ok(());
    };
    let diff = workdir_diff(dest)?;
    if diff.trim().is_empty() {
        println!("No uncommitted changes detected; skipping patch submission for job '{job}'.");
        return Ok(());
    }

    let response = create_patch_artifact_from_repo(
        client,
        dest,
        diff,
        title,
        description,
        Some(job.clone()),
        create_github_pr,
        None,
        is_automatic_backup,
        service_repo_name.clone(),
    )
    .await?;

    println!("Submitted patch '{}' for job '{}'.", response.patch_id, job);

    Ok(())
}

fn patch_metadata(job: &TaskId, last_message: &str) -> (String, String) {
    let job_display = job.to_string();
    let trimmed_message = last_message.trim();
    let title = trimmed_message
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Patch for job {job_display}"));
    let description = if trimmed_message.is_empty() {
        format!("Patch generated for Metis job {job_display}")
    } else {
        trimmed_message.to_string()
    };

    (title, description)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MockMetisClient,
        git::{
            commit_changes as git_commit_changes, configure_repo as git_configure_repo,
            stage_all_changes as git_stage_all_changes,
        },
        test_utils::ids::{patch_id, task_id},
    };
    use git2::Repository;
    use metis_common::patches::UpsertPatchResponse;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn init_git_repo(repo_path: &Path) -> Result<String> {
        Repository::init(repo_path).context("failed to init git repo for test")?;
        git_configure_repo(repo_path, "Test User", "test@example.com")?;

        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("repo path contains invalid UTF-8"))?;
        Ok(repo_str.to_string())
    }

    fn create_initial_commit(repo_path: &Path, filename: &str, content: &str) -> Result<()> {
        std::fs::write(repo_path.join(filename), content)
            .with_context(|| format!("failed to write initial file {filename}"))?;

        git_stage_all_changes(repo_path)?;
        git_commit_changes(repo_path, "initial commit")?;

        Ok(())
    }

    fn setup_git_repo_with_initial_commit(repo_path: &Path) -> Result<String> {
        let repo_str = init_git_repo(repo_path)?;
        create_initial_commit(repo_path, "README.md", "initial content")?;
        Ok(repo_str)
    }

    #[test]
    fn configure_git_repo_sets_user_config_and_branch() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        Repository::init(repo_path).context("failed to init git repo for test")?;
        {
            let repo = Repository::open(repo_path).context("failed to reopen repo for config")?;
            let mut config = repo
                .config()
                .context("failed to load git config for repo")?;
            config
                .set_str("user.name", "Initial User")
                .context("failed to set initial git user.name")?;
            config
                .set_str("user.email", "initial@example.com")
                .context("failed to set initial git user.email")?;
        }
        std::fs::write(repo_path.join("README.md"), "hello world")
            .context("failed to write initial file for git repo")?;
        git_stage_all_changes(repo_path)?;
        git_commit_changes(repo_path, "init")?;

        git_configure_repo(repo_path, "Metis Worker", "metis-worker@example.com")?;

        let repo = Repository::open(repo_path).context("failed to reopen repo for assertions")?;
        let config = repo
            .config()
            .context("failed to read git config for assertions")?;
        assert_eq!(config.get_string("user.name")?, "Metis Worker");
        assert_eq!(config.get_string("user.email")?, "metis-worker@example.com");

        Ok(())
    }

    #[test]
    fn ensure_color_output_env_sets_defaults() {
        let mut env = HashMap::new();

        ensure_color_output_env(&mut env);

        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(env.get("CLICOLOR_FORCE").map(String::as_str), Some("1"));
        assert_eq!(env.get("FORCE_COLOR").map(String::as_str), Some("1"));
    }

    #[test]
    fn ensure_color_output_env_preserves_existing_entries() {
        let mut env = HashMap::from([
            ("TERM".to_string(), "vt100".to_string()),
            ("FORCE_COLOR".to_string(), "0".to_string()),
        ]);

        ensure_color_output_env(&mut env);

        assert_eq!(env.get("TERM").map(String::as_str), Some("vt100"));
        assert_eq!(env.get("FORCE_COLOR").map(String::as_str), Some("0"));
        assert_eq!(env.get("CLICOLOR_FORCE").map(String::as_str), Some("1"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
    }

    #[tokio::test]
    async fn submit_patch_artifact_if_present_creates_patch_from_uncommitted_changes() -> Result<()>
    {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        setup_git_repo_with_initial_commit(repo_path)?;
        let base_commit =
            resolve_head_oid(repo_path)?.expect("expected HEAD commit after initial setup");
        std::fs::write(repo_path.join("README.md"), "updated content\n")?;
        std::fs::write(repo_path.join("untracked.txt"), "untracked content\n")?;

        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-123"),
        });
        let job_id = task_id("t-job-123");
        let repo_name = RepoName::from_str("dourolabs/example")?;

        submit_patch_artifact_if_present(
            &client,
            &job_id,
            repo_path,
            "final output line",
            &repo_name,
            Some(base_commit),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected a single patch submission");
        let (_, request) = &requests[0];
        assert_eq!(request.job_id, Some(job_id));
        assert_eq!(request.patch.title, "final output line");
        assert_eq!(request.patch.description, "final output line");
        assert!(
            request.patch.is_automatic_backup,
            "worker-run patches should be marked as automatic backups"
        );
        assert_eq!(
            request.patch.service_repo_name, repo_name,
            "patch should record the provided service repository"
        );
        assert!(
            request.patch.diff.contains("updated content"),
            "patch should include modifications made by the worker"
        );
        assert!(
            request.patch.diff.contains("untracked.txt"),
            "patch should include untracked files"
        );

        Ok(())
    }

    #[tokio::test]
    async fn submit_patch_artifact_if_present_skips_without_changes() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        setup_git_repo_with_initial_commit(repo_path)?;
        let base_commit =
            resolve_head_oid(repo_path)?.expect("expected HEAD commit after initial setup");

        let client = MockMetisClient::default();
        let job_id = task_id("t-job-456");
        let repo_name = RepoName::from_str("dourolabs/example")?;
        submit_patch_artifact_if_present(
            &client,
            &job_id,
            repo_path,
            "done",
            &repo_name,
            Some(base_commit),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert!(
            requests.is_empty(),
            "no patch should be submitted when the repository has no changes"
        );

        Ok(())
    }
}
