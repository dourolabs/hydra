use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_METIS_BASE_COMMIT, ENV_METIS_ISSUE_ID},
    job_status::JobStatusUpdate,
    jobs::{Bundle, WorkerContext},
    patches::GitOid,
    IssueId, RepoName, TaskId,
};

use crate::client::MetisClientInterface;
use crate::command::patches::{
    create_patch_artifact_from_repo, git_workdir_diff, resolve_service_repo_name,
};
use tempfile::Builder;

use crate::command::worker_run::worker_commands::WorkerCommands;

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
    // Startup tasks: set up context
    ensure_clean_destination(&dest)?;
    let github_token = variables.get(ENV_GH_TOKEN).cloned();
    let mut execution_env = variables;
    ensure_color_output_env(&mut execution_env);
    match request_context {
        Bundle::None => {
            fs::create_dir_all(&dest).with_context(|| format!("failed to create {dest:?}"))?;
        }
        Bundle::GitRepository { url, rev } => {
            clone_git_repo(&url, &rev, &dest, github_token.as_deref())?;
        }
    }

    let output_dir = Builder::new()
        .prefix("codex-output")
        .tempdir()
        .context("failed to create temporary codex output directory")?;
    let output_path = output_dir.path().join(crate::constants::OUTPUT_TXT_FILE);

    configure_git_repo(&dest)?;
    let base_commit = if dest.join(".git").exists() {
        match resolve_issue_id(&execution_env) {
            Ok(issue_id) => {
                let fork_point = resolve_head_oid(&dest)?;
                let issue_base_commit = setup_tracking_branches(
                    &dest,
                    &issue_id,
                    &job,
                    fork_point,
                    github_token.as_deref(),
                )?;
                set_base_commit_env(&mut execution_env, Some(issue_base_commit));
                Some(issue_base_commit)
            }
            Err(err) => {
                eprintln!(
                    "Skipping tracking branch setup because {ENV_METIS_ISSUE_ID} is missing or invalid: {err}"
                );
                let head_commit = resolve_head_oid(&dest)?;
                set_base_commit_env(&mut execution_env, Some(head_commit));
                Some(head_commit)
            }
        }
    } else {
        None
    };

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
    // Submit job status (merge of worker-submit functionality)
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

fn clone_git_repo(url: &str, rev: &str, dest: &Path, github_token: Option<&str>) -> Result<()> {
    if let Some(_token) = github_token {
        // The token is also present as an environment variable so it doesn't need to be explicitly
        // passed to authenticate.
        authenticate_github()?;
    }

    let status = Command::new("git")
        .args(["clone", "--no-checkout", url, dest.to_str().unwrap()])
        .status()
        .context("failed to spawn git clone")?;
    if !status.success() {
        return Err(anyhow!("git clone failed with status {status}"));
    }

    let status = Command::new("git")
        .args(["-C", dest.to_str().unwrap(), "checkout", rev])
        .status()
        .context("failed to spawn git checkout")?;
    if !status.success() {
        return Err(anyhow!("git checkout failed with status {status}"));
    }
    Ok(())
}

fn authenticate_github() -> Result<()> {
    let status = Command::new("gh")
        .args(["auth", "setup-git"])
        .status()
        .context("failed to spawn gh auth setup-git")?;
    if !status.success() {
        return Err(anyhow!("gh auth setup-git failed with status {status}"));
    }

    Ok(())
}

fn configure_git_repo(dest: &Path) -> Result<()> {
    let git_dir = dest.join(".git");
    if !git_dir.exists() {
        return Ok(());
    }

    let repo_path = dest
        .to_str()
        .ok_or_else(|| anyhow!("destination path contains invalid UTF-8"))?;

    let status = Command::new("git")
        .args(["-C", repo_path, "config", "user.name", "Metis Worker"])
        .status()
        .context("failed to set git user.name")?;
    if !status.success() {
        return Err(anyhow!("git config user.name failed with status {status}"));
    }

    let status = Command::new("git")
        .args([
            "-C",
            repo_path,
            "config",
            "user.email",
            "metis-worker@example.com",
        ])
        .status()
        .context("failed to set git user.email")?;
    if !status.success() {
        return Err(anyhow!("git config user.email failed with status {status}"));
    }

    Ok(())
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

fn set_base_commit_env(env: &mut HashMap<String, String>, base_commit: Option<GitOid>) {
    if let Some(base_commit) = base_commit {
        env.insert(ENV_METIS_BASE_COMMIT.to_string(), base_commit.to_string());
    }
}

fn resolve_issue_id(env: &HashMap<String, String>) -> Result<IssueId> {
    let issue_id = env
        .get(ENV_METIS_ISSUE_ID)
        .ok_or_else(|| anyhow!("{ENV_METIS_ISSUE_ID} is required for tracking branches"))?;

    IssueId::from_str(issue_id)
        .with_context(|| format!("invalid issue id in {ENV_METIS_ISSUE_ID}: '{issue_id}'"))
}

#[derive(Debug, Clone)]
struct TrackingBranchNames {
    issue_base: String,
    issue_head: String,
    task_base: String,
    task_head: String,
}

impl TrackingBranchNames {
    fn new(issue_id: &IssueId, task_id: &TaskId) -> Result<Self> {
        let issue_segment = sanitize_branch_segment(issue_id.as_ref());
        let task_segment = sanitize_branch_segment(task_id.as_ref());
        if issue_segment.is_empty() {
            bail!("failed to build tracking branches: issue id produced an empty branch segment");
        }
        if task_segment.is_empty() {
            bail!("failed to build tracking branches: task id produced an empty branch segment");
        }

        Ok(Self {
            issue_base: format!("metis/{issue_segment}/base"),
            issue_head: format!("metis/{issue_segment}/head"),
            task_base: format!("metis/{task_segment}/base"),
            task_head: format!("metis/{task_segment}/head"),
        })
    }
}

fn sanitize_branch_segment(input: &str) -> String {
    let mut normalized = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    while normalized.contains("--") {
        normalized = normalized.replace("--", "-");
    }

    normalized.trim_matches('-').to_string()
}

fn setup_tracking_branches(
    repo_root: &Path,
    issue_id: &IssueId,
    task_id: &TaskId,
    fork_point: GitOid,
    github_token: Option<&str>,
) -> Result<GitOid> {
    if let Some(_token) = github_token {
        authenticate_github()?;
    }

    let remote = "origin";
    ensure_remote_exists(repo_root, remote)?;
    fetch_remote(repo_root, remote)?;
    let branches = TrackingBranchNames::new(issue_id, task_id)?;
    let fork_point_ref = fork_point.to_string();

    let issue_base_remote = format!("{remote}/{}", branches.issue_base);
    let issue_base_commit = if remote_branch_exists(repo_root, remote, &branches.issue_base)? {
        let commit = resolve_ref(repo_root, &issue_base_remote)?;
        create_or_reset_branch(repo_root, &branches.issue_base, &commit.to_string())?;
        set_branch_upstream(repo_root, &branches.issue_base, &issue_base_remote)?;
        commit
    } else {
        create_or_reset_branch(repo_root, &branches.issue_base, &fork_point_ref)?;
        push_branch(repo_root, remote, &branches.issue_base)?;
        fork_point
    };

    let issue_head_remote = format!("{remote}/{}", branches.issue_head);
    let issue_head_commit = if remote_branch_exists(repo_root, remote, &branches.issue_head)? {
        let commit = resolve_ref(repo_root, &issue_head_remote)?;
        create_or_reset_branch(repo_root, &branches.issue_head, &commit.to_string())?;
        set_branch_upstream(repo_root, &branches.issue_head, &issue_head_remote)?;
        commit
    } else {
        create_or_reset_branch(repo_root, &branches.issue_head, &branches.issue_base)?;
        push_branch(repo_root, remote, &branches.issue_head)?;
        issue_base_commit
    };

    let task_base_remote = format!("{remote}/{}", branches.task_base);
    let task_base_commit = if remote_branch_exists(repo_root, remote, &branches.task_base)? {
        let commit = resolve_ref(repo_root, &task_base_remote)?;
        create_or_reset_branch(repo_root, &branches.task_base, &commit.to_string())?;
        set_branch_upstream(repo_root, &branches.task_base, &task_base_remote)?;
        commit
    } else {
        create_or_reset_branch(
            repo_root,
            &branches.task_base,
            &issue_head_commit.to_string(),
        )?;
        push_branch(repo_root, remote, &branches.task_base)?;
        issue_head_commit
    };

    let task_head_remote = format!("{remote}/{}", branches.task_head);
    if remote_branch_exists(repo_root, remote, &branches.task_head)? {
        let commit = resolve_ref(repo_root, &task_head_remote)?;
        create_or_reset_branch(repo_root, &branches.task_head, &commit.to_string())?;
        set_branch_upstream(repo_root, &branches.task_head, &task_head_remote)?;
    } else {
        create_or_reset_branch(
            repo_root,
            &branches.task_head,
            &task_base_commit.to_string(),
        )?;
        push_branch(repo_root, remote, &branches.task_head)?;
    }
    checkout_branch(repo_root, &branches.task_head)?;

    Ok(issue_base_commit)
}

fn ensure_remote_exists(repo_root: &Path, remote: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["remote", "get-url", remote])
        .current_dir(repo_root)
        .status()
        .context("failed to read git remotes")?;

    if status.success() {
        return Ok(());
    }

    bail!("repository does not have a '{remote}' remote; tracking branches require a configured remote");
}

fn fetch_remote(repo_root: &Path, remote: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["fetch", "--prune", remote])
        .current_dir(repo_root)
        .status()
        .context("failed to fetch repository remote")?;

    if status.success() {
        return Ok(());
    }

    bail!("git fetch failed for remote '{remote}'");
}

fn remote_branch_exists(repo_root: &Path, remote: &str, branch: &str) -> Result<bool> {
    branch_exists(repo_root, &format!("refs/remotes/{remote}/{branch}"))
}

fn branch_exists(repo_root: &Path, reference: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["show-ref", "--verify", reference])
        .current_dir(repo_root)
        .status()
        .context("failed to inspect git references")?;

    Ok(status.success())
}

fn create_or_reset_branch(repo_root: &Path, branch: &str, source: &str) -> Result<()> {
    let target_ref = format!("refs/heads/{branch}");
    let status = Command::new("git")
        .args(["update-ref", &target_ref, source])
        .current_dir(repo_root)
        .status()
        .with_context(|| format!("failed to create or reset branch '{branch}'"))?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to create or reset branch '{branch}'");
}

fn push_branch(repo_root: &Path, remote: &str, branch: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["push", "-u", remote, branch])
        .current_dir(repo_root)
        .status()
        .with_context(|| format!("failed to push branch '{branch}' to remote '{remote}'"))?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to push branch '{branch}' to remote '{remote}'");
}

fn set_branch_upstream(repo_root: &Path, branch: &str, upstream: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["branch", "--set-upstream-to", upstream, branch])
        .current_dir(repo_root)
        .status()
        .with_context(|| format!("failed to set upstream for branch '{branch}'"))?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to set upstream for branch '{branch}'");
}

fn checkout_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["checkout", branch])
        .current_dir(repo_root)
        .status()
        .with_context(|| format!("failed to checkout branch '{branch}'"))?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to checkout branch '{branch}'");
}

fn resolve_ref(repo_root: &Path, reference: &str) -> Result<GitOid> {
    let output = Command::new("git")
        .args(["rev-parse", &format!("{reference}^{{commit}}")])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to resolve reference '{reference}'"))?;

    if !output.status.success() {
        bail!("failed to resolve reference '{reference}'");
    }

    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    GitOid::from_str(&oid).context("failed to parse commit oid")
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
    let diff = git_workdir_diff(dest)?;
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

fn resolve_head_oid(dest: &Path) -> Result<GitOid> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD^{commit}"])
        .current_dir(dest)
        .output()
        .context("failed to resolve HEAD commit")?;
    if !output.status.success() {
        bail!("failed to resolve HEAD commit");
    }

    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    GitOid::from_str(&oid).context("failed to parse HEAD commit oid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MockMetisClient,
        test_utils::ids::{issue_id, patch_id, task_id},
    };
    use metis_common::patches::UpsertPatchResponse;
    use std::collections::HashMap;
    use std::process::Command;
    use std::str::FromStr;

    fn init_git_repo(repo_path: &Path) -> Result<String> {
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("repo path contains invalid UTF-8"))?;

        Command::new("git")
            .args(["init", repo_str])
            .status()
            .context("failed to init git repo for test")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "config", "user.name", "Test User"])
            .status()
            .context("failed to set git user.name")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "config", "user.email", "test@example.com"])
            .status()
            .context("failed to set git user.email")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;

        Ok(repo_str.to_string())
    }

    fn create_initial_commit(
        repo_path: &Path,
        repo_str: &str,
        filename: &str,
        content: &str,
    ) -> Result<()> {
        std::fs::write(repo_path.join(filename), content)
            .with_context(|| format!("failed to write initial file {filename}"))?;

        Command::new("git")
            .args(["-C", repo_str, "add", filename])
            .status()
            .with_context(|| format!("failed to add initial file {filename}"))?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;

        Command::new("git")
            .args(["-C", repo_str, "commit", "-m", "initial commit"])
            .status()
            .context("failed to create initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

        Ok(())
    }

    fn setup_git_repo_with_initial_commit(repo_path: &Path) -> Result<String> {
        let repo_str = init_git_repo(repo_path)?;
        create_initial_commit(repo_path, &repo_str, "README.md", "initial content")?;
        Ok(repo_str)
    }

    fn setup_bare_remote_repo() -> Result<(tempfile::TempDir, GitOid)> {
        let remote_dir =
            tempfile::tempdir().context("failed to create temporary directory for remote repo")?;
        let remote_str = remote_dir
            .path()
            .to_str()
            .ok_or_else(|| anyhow!("remote path contains invalid UTF-8"))?;
        Command::new("git")
            .args(["init", "--bare", remote_str])
            .status()
            .context("failed to init bare remote repo")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init --bare returned non-zero exit code"))?;

        let seed_dir =
            tempfile::tempdir().context("failed to create temporary directory for seed repo")?;
        let seed_str = seed_dir
            .path()
            .to_str()
            .ok_or_else(|| anyhow!("seed path contains invalid UTF-8"))?;
        Command::new("git")
            .args(["init", seed_str])
            .status()
            .context("failed to init seed repo for remote")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init returned non-zero exit code for seed repo"))?;
        Command::new("git")
            .args(["-C", seed_str, "checkout", "-b", "main"])
            .status()
            .context("failed to create main branch in seed repo")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git checkout returned non-zero exit code for seed repo"))?;
        configure_git_repo(seed_dir.path())?;
        std::fs::write(seed_dir.path().join("README.md"), "seed content\n")
            .context("failed to write seed file for remote")?;
        Command::new("git")
            .args(["-C", seed_str, "add", "README.md"])
            .status()
            .context("failed to add seed file to repo")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code for seed repo"))?;
        Command::new("git")
            .args(["-C", seed_str, "commit", "-m", "seed"])
            .status()
            .context("failed to commit seed repo")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code for seed repo"))?;
        Command::new("git")
            .args(["-C", seed_str, "remote", "add", "origin", remote_str])
            .status()
            .context("failed to add remote to seed repo")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git remote add returned non-zero exit code for seed repo"))?;
        Command::new("git")
            .args(["-C", seed_str, "push", "-u", "origin", "main"])
            .status()
            .context("failed to push seed repo to remote")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git push returned non-zero exit code for seed repo"))?;

        let main_commit = resolve_ref(seed_dir.path(), "HEAD")?;
        Ok((remote_dir, main_commit))
    }

    fn clone_remote_repo(remote: &Path) -> Result<tempfile::TempDir> {
        let clone_dir =
            tempfile::tempdir().context("failed to create temporary clone directory")?;
        let remote_str = remote
            .to_str()
            .ok_or_else(|| anyhow!("remote path contains invalid UTF-8"))?;
        clone_git_repo(remote_str, "main", clone_dir.path(), None)?;
        configure_git_repo(clone_dir.path())?;
        Ok(clone_dir)
    }

    fn current_branch(repo_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .context("failed to read current branch")?;
        if !output.status.success() {
            bail!("failed to read current branch");
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    #[test]
    fn configure_git_repo_sets_user_config_and_branch() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("tempdir path contains invalid UTF-8"))?;

        Command::new("git")
            .args(["init", repo_str])
            .status()
            .context("failed to init git repo for test")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git init returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", repo_str, "config", "user.name", "Initial User"])
            .status()
            .context("failed to set initial git user.name")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.name returned non-zero exit code"))?;
        Command::new("git")
            .args([
                "-C",
                repo_str,
                "config",
                "user.email",
                "initial@example.com",
            ])
            .status()
            .context("failed to set initial git user.email")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git config user.email returned non-zero exit code"))?;
        std::fs::write(repo_path.join("README.md"), "hello world")
            .context("failed to write initial file for git repo")?;
        Command::new("git")
            .args(["-C", repo_str, "add", "."])
            .status()
            .context("failed to add file for initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", repo_str, "commit", "-m", "init"])
            .status()
            .context("failed to create initial commit")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

        configure_git_repo(repo_path)?;

        let user_name = Command::new("git")
            .args(["-C", repo_str, "config", "user.name"])
            .output()
            .context("failed to read git user.name")?;
        assert!(user_name.status.success());
        assert_eq!(
            String::from_utf8_lossy(&user_name.stdout).trim(),
            "Metis Worker"
        );

        let user_email = Command::new("git")
            .args(["-C", repo_str, "config", "user.email"])
            .output()
            .context("failed to read git user.email")?;
        assert!(user_email.status.success());
        assert_eq!(
            String::from_utf8_lossy(&user_email.stdout).trim(),
            "metis-worker@example.com"
        );

        Ok(())
    }

    #[test]
    fn sanitize_branch_segment_normalizes_segments() {
        assert_eq!(sanitize_branch_segment("I-This_is.TEST"), "i-this-is-test");
        assert_eq!(sanitize_branch_segment("..."), "");
    }

    #[test]
    fn set_base_commit_env_sets_value_when_present() -> Result<()> {
        let mut env = HashMap::new();
        let commit = GitOid::from_str("0123456789abcdef0123456789abcdef01234567")?;

        set_base_commit_env(&mut env, Some(commit));

        assert_eq!(
            env.get(ENV_METIS_BASE_COMMIT).map(String::as_str),
            Some("0123456789abcdef0123456789abcdef01234567")
        );

        Ok(())
    }

    #[test]
    fn set_base_commit_env_ignores_missing_commit() {
        let mut env = HashMap::new();

        set_base_commit_env(&mut env, None);

        assert!(!env.contains_key(ENV_METIS_BASE_COMMIT));
    }

    #[test]
    fn setup_tracking_branches_bootstraps_issue_and_task_branches() -> Result<()> {
        let (remote_dir, main_commit) = setup_bare_remote_repo()?;
        let repo = clone_remote_repo(remote_dir.path())?;
        let issue = issue_id("bootstrap");
        let task = task_id("bootstrap-task");
        let fork_point = resolve_head_oid(repo.path())?;

        let branches = TrackingBranchNames::new(&issue, &task)?;
        let base_commit = setup_tracking_branches(repo.path(), &issue, &task, fork_point, None)?;

        assert_eq!(base_commit, main_commit);
        assert_eq!(current_branch(repo.path())?, branches.task_head);
        assert_eq!(resolve_ref(repo.path(), &branches.task_head)?, main_commit);
        assert!(branch_exists(
            repo.path(),
            &format!("refs/remotes/origin/{}", branches.issue_base)
        )?);
        assert!(branch_exists(
            repo.path(),
            &format!("refs/remotes/origin/{}", branches.task_head)
        )?);

        Ok(())
    }

    #[test]
    fn setup_tracking_branches_resumes_remote_head() -> Result<()> {
        let (remote_dir, main_commit) = setup_bare_remote_repo()?;
        let issue = issue_id("resume");
        let task = task_id("resume-task");
        let branches = TrackingBranchNames::new(&issue, &task)?;

        let bootstrap_repo = clone_remote_repo(remote_dir.path())?;
        let fork_point = resolve_head_oid(bootstrap_repo.path())?;
        setup_tracking_branches(bootstrap_repo.path(), &issue, &task, fork_point, None)?;

        let work_repo = clone_remote_repo(remote_dir.path())?;
        create_or_reset_branch(
            work_repo.path(),
            &branches.issue_head,
            &format!("origin/{}", branches.issue_head),
        )?;
        checkout_branch(work_repo.path(), &branches.issue_head)?;
        std::fs::write(work_repo.path().join("README.md"), "next change\n")
            .context("failed to update seed file")?;
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(work_repo.path())
            .status()
            .context("failed to add updated seed file")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code for updated seed file"))?;
        Command::new("git")
            .args(["commit", "-m", "advance head"])
            .current_dir(work_repo.path())
            .status()
            .context("failed to commit head advancement")?
            .success()
            .then_some(())
            .ok_or_else(|| {
                anyhow!("git commit returned non-zero exit code for head advancement")
            })?;
        Command::new("git")
            .args(["push", "origin", &branches.issue_head])
            .current_dir(work_repo.path())
            .status()
            .context("failed to push updated head to remote")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git push returned non-zero exit code for updated head"))?;
        let updated_head = resolve_ref(work_repo.path(), "HEAD")?;

        let resume_repo = clone_remote_repo(remote_dir.path())?;
        let fork_point = resolve_head_oid(resume_repo.path())?;
        let base_commit =
            setup_tracking_branches(resume_repo.path(), &issue, &task, fork_point, None)?;

        assert_eq!(base_commit, main_commit);
        assert_eq!(
            resolve_ref(resume_repo.path(), &branches.issue_head)?,
            updated_head
        );
        let remote_task_head = resolve_ref(
            resume_repo.path(),
            &format!("refs/remotes/origin/{}", branches.task_head),
        )?;
        assert_eq!(
            resolve_ref(resume_repo.path(), &branches.task_head)?,
            remote_task_head
        );
        assert_eq!(current_branch(resume_repo.path())?, branches.task_head);

        Ok(())
    }

    #[test]
    fn setup_tracking_branches_errors_without_remote() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        setup_git_repo_with_initial_commit(repo_path)?;
        let issue = issue_id("no-remote");
        let task = task_id("no-remote-task");
        let original_branch = current_branch(repo_path)?;

        let fork_point = resolve_head_oid(repo_path)?;
        let err = setup_tracking_branches(repo_path, &issue, &task, fork_point, None)
            .expect_err("missing remote should produce an error");

        assert!(
            err.to_string().contains("remote"),
            "unexpected error from missing remote: {err}"
        );

        assert_eq!(current_branch(repo_path)?, original_branch);

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
        let base_commit = resolve_head_oid(repo_path)?;
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
        let base_commit = resolve_head_oid(repo_path)?;

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
