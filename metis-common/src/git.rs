use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use gix::ThreadSafeRepository;
use gix::progress::Discard;
use tempfile::NamedTempFile;

/// Lightweight client for interacting with git repositories.
#[derive(Default, Clone)]
pub struct GitClient;

impl GitClient {
    /// Create a new git client instance.
    pub fn new() -> Self {
        Self
    }

    /// Clone a repository from `url` into `dest` and return a handle to the new repository.
    pub fn clone_checkout(&self, url: &str, dest: &Path) -> Result<GitRepository> {
        let mut prepare = gix::prepare_clone(url, dest)?;
        let interrupt = AtomicBool::new(false);
        let (mut checkout, _) = prepare.fetch_then_checkout(Discard, &interrupt)?;
        let (repo, _) = checkout.main_worktree(Discard, &interrupt)?;
        Ok(GitRepository {
            repo: repo.into_sync(),
        })
    }

    /// Discover a repository by walking up from `start_at`.
    pub fn discover(&self, start_at: &Path) -> Result<GitRepository> {
        let repo = ThreadSafeRepository::discover(start_at)?;
        Ok(GitRepository { repo })
    }

    /// Open a repository at `path`.
    pub fn open(&self, path: &Path) -> Result<GitRepository> {
        let repo = ThreadSafeRepository::open(path)?;
        Ok(GitRepository { repo })
    }
}

/// Represents a git repository and provides common operations.
#[derive(Clone)]
pub struct GitRepository {
    repo: ThreadSafeRepository,
}

impl GitRepository {
    /// Return the working directory for this repository.
    pub fn workdir(&self) -> Result<PathBuf> {
        self.repo
            .work_dir()
            .map(PathBuf::from)
            .or_else(|| self.repo.path().canonicalize().ok())
            .ok_or_else(|| anyhow!("repository has no working directory"))
    }

    fn git_command(&self) -> Result<Command> {
        let mut command = Command::new("git");
        command.current_dir(self.workdir()?);
        Ok(command)
    }

    fn run_git(&self, args: &[&str]) -> Result<Output> {
        let output = self
            .git_command()?
            .args(args)
            .output()
            .with_context(|| format!("failed to run git {}", args.join(" ")))?;
        Ok(output)
    }

    fn run_git_with_index(&self, args: &[&str], index_file: Option<&Path>) -> Result<Output> {
        let mut command = self.git_command()?;
        command.args(args);
        if let Some(index_file) = index_file {
            command.env("GIT_INDEX_FILE", index_file);
        }
        let output = command
            .output()
            .with_context(|| format!("failed to run git {}", args.join(" ")))?;
        Ok(output)
    }

    /// Checkout `rev` in this repository.
    pub fn checkout(&self, rev: &str) -> Result<()> {
        let status = self
            .git_command()?
            .args(["checkout", rev])
            .status()
            .context("failed to spawn git checkout")?;
        if status.success() {
            return Ok(());
        }
        bail!("git checkout failed with status {status}");
    }

    /// Configure repository user identity.
    pub fn set_user_config(&self, name: &str, email: &str) -> Result<()> {
        let status = self
            .git_command()?
            .args(["config", "user.name", name])
            .status()
            .context("failed to set git user.name")?;
        if !status.success() {
            bail!("git config user.name failed with status {status}");
        }

        let status = self
            .git_command()?
            .args(["config", "user.email", email])
            .status()
            .context("failed to set git user.email")?;
        if !status.success() {
            bail!("git config user.email failed with status {status}");
        }

        Ok(())
    }

    /// Stage all tracked and untracked changes.
    pub fn stage_all(&self, index_file: Option<&Path>) -> Result<()> {
        let status = self
            .run_git_with_index(&["add", "-A", "--", "."], index_file)?
            .status;
        if status.success() {
            return Ok(());
        }
        bail!("git add failed while staging changes");
    }

    /// Unstage the provided path from the index.
    pub fn reset_path(&self, path: &str, index_file: Option<&Path>) -> Result<()> {
        let status = self
            .run_git_with_index(&["reset", "-q", "--", path], index_file)?
            .status;
        if status.success() {
            return Ok(());
        }
        bail!("git reset failed for path {path}");
    }

    /// Return the cached diff as a string, optionally excluding an additional glob.
    pub fn diff_cached_excluding(
        &self,
        exclusion_glob: Option<&str>,
        index_file: Option<&Path>,
    ) -> Result<String> {
        let mut args = vec!["diff", "--cached", "--", "."];
        if let Some(glob) = exclusion_glob {
            args.push(glob);
        }
        let output = self.run_git_with_index(&args, index_file)?;
        if output.status.success() || output.status.code() == Some(1) {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
        bail!("git diff failed with status {}", output.status);
    }

    /// Return true if staged changes exist.
    pub fn has_staged_changes(&self) -> Result<bool> {
        let status = self
            .git_command()?
            .args(["diff", "--cached", "--quiet"])
            .status()
            .context("failed to check staged changes")?;
        match status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => bail!("failed to check staged changes before committing"),
        }
    }

    /// Return the current branch name.
    pub fn current_branch(&self) -> Result<String> {
        let output = self.run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        if !output.status.success() {
            bail!("git rev-parse --abbrev-ref failed");
        }
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() {
            bail!("unable to determine current branch");
        }
        Ok(branch)
    }

    /// Check whether `branch` exists locally.
    pub fn branch_exists(&self, branch: &str) -> Result<bool> {
        let status = self
            .git_command()?
            .args(["show-ref", "--verify", &format!("refs/heads/{branch}")])
            .status()
            .context("failed to check for existing branch")?;
        Ok(status.success())
    }

    /// Create and check out a new branch.
    pub fn checkout_new_branch(&self, branch: &str) -> Result<()> {
        let status = self
            .git_command()?
            .args(["checkout", "-b", branch])
            .status()
            .context("failed to create feature branch")?;
        if status.success() {
            return Ok(());
        }
        bail!("failed to create branch '{branch}'");
    }

    /// Commit staged changes with `message`.
    pub fn commit(&self, message: &str) -> Result<()> {
        let status = self
            .git_command()?
            .args(["commit", "-m", message])
            .status()
            .context("failed to commit changes")?;
        if status.success() {
            return Ok(());
        }
        bail!("failed to commit changes");
    }

    /// Push branch to remote with upstream tracking.
    pub fn push(&self, remote: &str, branch: &str) -> Result<()> {
        let status = self
            .git_command()?
            .args(["push", "-u", remote, branch])
            .status()
            .context("failed to push branch to remote")?;
        if status.success() {
            return Ok(());
        }
        bail!("failed to push branch '{branch}' to {remote}");
    }

    /// Apply a patch with three-way merge and update the index.
    pub fn apply_patch(&self, patch: &str) -> Result<()> {
        if patch.trim().is_empty() {
            bail!("Patch is empty. Nothing to apply.");
        }

        let patch_file =
            NamedTempFile::new().context("failed to create temporary file for patch")?;
        std::fs::write(patch_file.path(), patch)
            .context("failed to write patch to temporary file")?;

        let output = self
            .git_command()?
            .arg("apply")
            .args(["--3way", "--index"])
            .arg(patch_file.path())
            .output()
            .context("failed to execute git apply with 3-way merge")?;

        if output.status.success() {
            return Ok(());
        }

        let conflicts = self.conflicted_files()?;
        if !conflicts.is_empty() {
            bail!(
                "Merge conflicts detected while applying patch; resolve these files and continue:\n{}",
                conflicts.join("\n")
            );
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to apply patch. Exit code: {}. Error: {}",
            output.status.code().unwrap_or(-1),
            stderr
        );
    }

    /// Return list of paths with unresolved conflicts.
    pub fn conflicted_files(&self) -> Result<Vec<String>> {
        let output = self.run_git(&["diff", "--name-only", "--diff-filter=U"])?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants;
    use std::fs;
    use std::process::Command;

    fn init_repo_with_commit() -> Result<(tempfile::TempDir, PathBuf, GitRepository)> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for git test")?;
        let repo_path = tempdir.path().to_path_buf();
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

        fs::write(repo_path.join("README.md"), "initial\n").context("failed to write README.md")?;
        Command::new("git")
            .args(["-C", repo_str, "add", "README.md"])
            .status()
            .context("failed to add README.md")?
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

        let git_repo = GitClient::new().open(&repo_path)?;

        Ok((tempdir, repo_path, git_repo))
    }

    #[test]
    fn diff_cached_excludes_metis_content() -> Result<()> {
        let (_tempdir, repo_path, repo) = init_repo_with_commit()?;
        fs::write(repo_path.join("README.md"), "updated\n")
            .context("failed to modify README.md")?;

        let metis_dir = repo_path.join(constants::METIS_DIR);
        fs::create_dir_all(&metis_dir).context("failed to create .metis directory")?;
        fs::write(metis_dir.join("internal.txt"), "ignored")
            .context("failed to write .metis/internal.txt")?;

        let temp_index = tempfile::tempdir().context("failed to create tempdir for index")?;
        let temp_index_path = temp_index.path().join("index");

        repo.stage_all(Some(&temp_index_path))?;
        repo.reset_path(constants::METIS_DIR, Some(&temp_index_path))?;
        let diff = repo.diff_cached_excluding(
            Some(&format!(":!{}/**", constants::METIS_DIR)),
            Some(&temp_index_path),
        )?;

        assert!(
            diff.contains("README.md"),
            "cached diff should include tracked file changes"
        );
        assert!(
            !diff.contains(constants::METIS_DIR),
            "cached diff should exclude .metis contents"
        );

        Ok(())
    }

    #[test]
    fn has_staged_changes_tracks_state() -> Result<()> {
        let (_tempdir, repo_path, repo) = init_repo_with_commit()?;
        fs::write(repo_path.join("notes.txt"), "notes\n").context("failed to add notes.txt")?;

        repo.stage_all(None)?;
        assert!(repo.has_staged_changes()?, "expected staged changes");

        repo.commit("add notes")?;
        assert!(
            !repo.has_staged_changes()?,
            "staged changes should be cleared after commit"
        );

        Ok(())
    }
}
