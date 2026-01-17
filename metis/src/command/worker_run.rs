use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_OPENAI_API_KEY},
    job_status::JobStatusUpdate,
    jobs::{Bundle, WorkerContext},
    patches::{GitOid, PatchCommitRange},
    RepoName, TaskId,
};

use crate::client::MetisClientInterface;
use crate::command::patches::{create_patch_artifact_from_repo, resolve_service_repo_name};
use crate::exec::{codex_output_path, run_codex};

pub async fn run(client: &dyn MetisClientInterface, job: TaskId, dest: PathBuf) -> Result<()> {
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
    create_output_directory(&dest)?;

    login_codex()?;
    configure_git_repo(&dest)?;
    let base_commit = resolve_head_oid(&dest)?;

    run_codex(&prompt, &dest, &execution_env)
        .await
        .with_context(|| "failed to execute codex for worker context")?;

    let last_message = read_last_message(&dest)?;
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

fn login_codex() -> Result<()> {
    let openai_api_key = std::env::var(ENV_OPENAI_API_KEY)
        .with_context(|| format!("{ENV_OPENAI_API_KEY} is not set; unable to login Codex CLI"))?;

    let mut login_cmd = Command::new("codex")
        .args(["login", "--with-api-key"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn codex login")?;

    {
        let mut stdin = login_cmd
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open stdin for codex login"))?;
        stdin
            .write_all(format!("{openai_api_key}\n").as_bytes())
            .with_context(|| format!("failed to write {ENV_OPENAI_API_KEY} to codex login"))?;
    }

    let status = login_cmd
        .wait()
        .context("failed waiting for codex login to finish")?;
    if !status.success() {
        return Err(anyhow!("codex login failed with status {status}"));
    }

    Ok(())
}

fn create_output_directory(dest: &Path) -> Result<()> {
    let output_dir = codex_output_path(dest)
        .parent()
        .ok_or_else(|| anyhow!("failed to compute codex output directory"))?
        .to_path_buf();
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create output directory at {output_dir:?}"))?;
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
    base_commit: GitOid,
) -> Result<()> {
    let (title, description) = patch_metadata(job, last_message);
    let create_github_pr = false;
    let is_automatic_backup = true;

    let head_commit = match commit_pending_changes(dest, "Metis patch snapshot")?.or_else(|| {
        resolve_head_oid(dest)
            .ok()
            .filter(|current| current != &base_commit)
    }) {
        Some(oid) => oid,
        None => {
            println!("No changes detected; skipping patch submission for job '{job}'.");
            return Ok(());
        }
    };

    let commit_range = PatchCommitRange {
        base: base_commit,
        head: head_commit,
    };

    let response = create_patch_artifact_from_repo(
        client,
        dest,
        commit_range,
        title,
        description,
        Some(job.clone()),
        create_github_pr,
        is_automatic_backup,
        service_repo_name.clone(),
    )
    .await?;

    println!("Submitted patch '{}' for job '{}'.", response.patch_id, job);

    Ok(())
}

fn last_message_path(dest: &Path) -> PathBuf {
    codex_output_path(dest)
}

fn read_last_message(dest: &Path) -> Result<String> {
    let last_message_file = last_message_path(dest);
    fs::read_to_string(&last_message_file).with_context(|| {
        format!(
            "failed to read last message output at '{}'",
            last_message_file.display()
        )
    })
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

fn commit_pending_changes(dest: &Path, message: &str) -> Result<Option<GitOid>> {
    stage_pending_changes(dest)?;
    if !has_staged_changes(dest)? {
        return Ok(None);
    }

    let commit_status = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dest)
        .status()
        .context("failed to commit repository changes")?;
    if !commit_status.success() {
        bail!("git commit failed while preparing patch contents");
    }

    resolve_head_oid(dest).map(Some)
}

fn stage_pending_changes(dest: &Path) -> Result<()> {
    let add_status = Command::new("git")
        .args(["add", "-A", "--", "."])
        .current_dir(dest)
        .status()
        .context("failed to stage repository changes")?;
    if !add_status.success() {
        bail!("git add failed while preparing patch contents");
    }

    Ok(())
}

fn has_staged_changes(dest: &Path) -> Result<bool> {
    let status = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(dest)
        .status()
        .context("failed to check staged changes")?;

    match status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => bail!("failed to check staged changes before committing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MockMetisClient,
        constants,
        test_utils::ids::{patch_id, task_id},
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

    fn diff_for_range(repo_path: &Path, range: &PatchCommitRange) -> Result<String> {
        let output = Command::new("git")
            .args(["diff", &format!("{}..{}", range.base, range.head)])
            .current_dir(repo_path)
            .output()
            .context("failed to generate diff for commit range")?;
        if !output.status.success() {
            bail!(
                "git diff failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

    #[test]
    fn commit_pending_changes_captures_untracked_files() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;

        // Create new untracked files
        std::fs::write(repo_path.join("new_file.txt"), "new content")
            .context("failed to write new file")?;
        std::fs::create_dir_all(repo_path.join("src")).context("failed to create src directory")?;
        std::fs::write(repo_path.join("src").join("main.rs"), "fn main() {}")
            .context("failed to write main.rs")?;

        let base_commit = resolve_head_oid(repo_path)?;
        let head_commit = commit_pending_changes(repo_path, "commit new files")?
            .expect("commit should be created for new files");
        let patch_content = diff_for_range(
            repo_path,
            &PatchCommitRange {
                base: base_commit,
                head: head_commit,
            },
        )?;

        assert!(
            patch_content.contains("new_file.txt"),
            "patch should include new_file.txt"
        );
        assert!(
            patch_content.contains("new content"),
            "patch should include content of new_file.txt"
        );
        assert!(
            patch_content.contains("src/main.rs"),
            "patch should include src/main.rs"
        );
        assert!(
            patch_content.contains("fn main() {}"),
            "patch should include content of src/main.rs"
        );

        Ok(())
    }

    #[test]
    fn commit_pending_changes_includes_hidden_directories() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;

        // Create files in a hidden directory that should now be included
        let hidden_dir = repo_path.join(".metis");
        std::fs::create_dir_all(&hidden_dir).context("failed to create hidden directory")?;
        std::fs::write(hidden_dir.join("internal_file.txt"), "internal content")
            .context("failed to write file in hidden directory")?;
        std::fs::create_dir_all(hidden_dir.join("subdir"))
            .context("failed to create subdir in hidden directory")?;
        std::fs::write(
            hidden_dir.join("subdir").join("nested.txt"),
            "nested content",
        )
        .context("failed to write nested file in hidden directory")?;

        // Also create a regular file that should be included
        std::fs::write(repo_path.join("regular_file.txt"), "regular content")
            .context("failed to write regular file")?;

        let base_commit = resolve_head_oid(repo_path)?;
        let head_commit = commit_pending_changes(repo_path, "commit regular changes")?
            .expect("commit should be created when repository has changes");
        let patch_content = diff_for_range(
            repo_path,
            &PatchCommitRange {
                base: base_commit,
                head: head_commit,
            },
        )?;

        assert!(
            patch_content.contains(".metis/internal_file.txt"),
            "patch should include .metis/internal_file.txt"
        );
        assert!(
            patch_content.contains("internal content"),
            "patch should include content from .metis/internal_file.txt"
        );
        assert!(
            patch_content.contains(".metis/subdir/nested.txt"),
            "patch should include .metis/subdir/nested.txt"
        );
        assert!(
            patch_content.contains("nested content"),
            "patch should include content from .metis/subdir/nested.txt"
        );

        // Verify regular file is included
        assert!(
            patch_content.contains("regular_file.txt"),
            "patch should include regular_file.txt"
        );
        assert!(
            patch_content.contains("regular content"),
            "patch should include content of regular_file.txt"
        );

        Ok(())
    }

    #[test]
    fn commit_pending_changes_returns_none_when_clean() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();

        setup_git_repo_with_initial_commit(repo_path)?;
        let base_commit = resolve_head_oid(repo_path)?;

        let patch_content = commit_pending_changes(repo_path, "noop commit")?;

        assert!(patch_content.is_none(), "expected no commit for clean tree");
        assert_eq!(
            resolve_head_oid(repo_path)?,
            base_commit,
            "HEAD should remain unchanged when no commit is created"
        );

        Ok(())
    }

    #[tokio::test]
    async fn submit_patch_artifact_if_present_creates_patch() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let repo_path = tempdir.path();
        setup_git_repo_with_initial_commit(repo_path)?;
        let base_commit = resolve_head_oid(repo_path)?;
        std::fs::write(repo_path.join("README.md"), "updated content\n")
            .context("failed to update README.md")?;

        let output_dir = repo_path.join(constants::OUTPUT_DIR);
        std::fs::create_dir_all(&output_dir)
            .context("failed to create output directory for test repo")?;
        std::fs::write(
            output_dir.join(constants::OUTPUT_TXT_FILE),
            "final output line",
        )
        .context("failed to write output.txt for test repo")?;

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
            base_commit,
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
        let diff = diff_for_range(repo_path, &request.patch.commit_range)?;
        assert!(
            diff.contains("updated content"),
            "patch should include modifications made by the worker"
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
            base_commit,
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
