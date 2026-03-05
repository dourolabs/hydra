use anyhow::{Context, anyhow};
use std::process::Command;
use tempfile::TempDir;

/// A bare git repository backed by a temporary directory, suitable for use as a
/// remote in integration tests. The repository is created with an initial commit
/// on the `main` branch.
///
/// The temporary directory is automatically cleaned up when the `GitRemote` is
/// dropped.
pub struct GitRemote {
    /// The `file://` URL used as the git remote (enforces fast-forward checks).
    url: String,
    /// The bare filesystem path, used for `--git-dir` operations.
    path: String,
    _tempdir: TempDir,
}

impl GitRemote {
    /// Create a new bare remote repository with an initial commit on `main`.
    ///
    /// The repository is initialised in a temporary directory. A working
    /// directory is used to create the initial commit and push it to the bare
    /// remote.
    pub fn new() -> anyhow::Result<Self> {
        let tempdir = TempDir::new().context("failed to create tempdir for GitRemote")?;
        let base = tempdir.path();

        let workdir = base.join("workdir");
        let remote_dir = base.join("remote.git");
        let workdir_str = workdir
            .to_str()
            .ok_or_else(|| anyhow!("workdir path contains invalid UTF-8"))?;
        let remote_dir_str = remote_dir
            .to_str()
            .ok_or_else(|| anyhow!("remote dir path contains invalid UTF-8"))?;

        run_git(&["init", workdir_str])?;
        run_git(&["-C", workdir_str, "checkout", "-b", "main"])?;
        run_git(&["-C", workdir_str, "config", "user.name", "Test"])?;
        run_git(&[
            "-C",
            workdir_str,
            "config",
            "user.email",
            "test@example.com",
        ])?;

        std::fs::write(workdir.join("README.md"), "initial content\n")
            .context("failed to write initial README")?;
        run_git(&["-C", workdir_str, "add", "README.md"])?;
        run_git(&["-C", workdir_str, "commit", "-m", "initial commit"])?;

        run_git(&["init", "--bare", remote_dir_str])?;
        run_git(&["-C", workdir_str, "remote", "add", "origin", remote_dir_str])?;
        run_git(&["-C", workdir_str, "push", "-u", "origin", "main"])?;
        run_git(&[
            "--git-dir",
            remote_dir_str,
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])?;

        Ok(Self {
            url: format!("file://{remote_dir_str}"),
            path: remote_dir_str.to_string(),
            _tempdir: tempdir,
        })
    }

    /// Return the file-system URL of the bare repository, suitable for use
    /// as a git remote.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Create a new branch with a single commit that writes `content` to `file`.
    ///
    /// The branch is created from the current `main` HEAD. Returns the SHA of
    /// the new commit.
    pub fn create_branch(&self, name: &str, file: &str, content: &str) -> anyhow::Result<String> {
        let tempdir = TempDir::new().context("failed to create tempdir for branch work")?;
        let work = tempdir.path();
        let work_str = work
            .to_str()
            .ok_or_else(|| anyhow!("work path contains invalid UTF-8"))?;

        run_git(&["clone", &self.url, work_str])?;
        run_git(&["-C", work_str, "config", "user.name", "Test"])?;
        run_git(&["-C", work_str, "config", "user.email", "test@example.com"])?;
        run_git(&["-C", work_str, "checkout", "-b", name])?;

        let file_path = work.join(file);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)
                .context("failed to create parent directories for file")?;
        }
        std::fs::write(&file_path, content).context("failed to write file for branch")?;

        run_git(&["-C", work_str, "add", file])?;
        run_git(&[
            "-C",
            work_str,
            "commit",
            "-m",
            &format!("add {file} on {name}"),
        ])?;
        run_git(&["-C", work_str, "push", "origin", name])?;

        self.branch_sha(name)
    }

    /// Return the HEAD SHA of the given branch in the bare remote.
    pub fn branch_sha(&self, branch: &str) -> anyhow::Result<String> {
        let output = run_git_output(&[
            "--git-dir",
            &self.path,
            "rev-parse",
            &format!("refs/heads/{branch}"),
        ])?;
        Ok(output.trim().to_string())
    }

    /// Check whether a branch exists in the bare remote.
    pub fn branch_exists(&self, branch: &str) -> bool {
        run_git_output(&[
            "--git-dir",
            &self.path,
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}"),
        ])
        .is_ok()
    }

    /// Push an additional commit to an existing branch, appending `content` to `file`.
    ///
    /// Returns the SHA of the new commit.
    pub fn push_commit(&self, branch: &str, file: &str, content: &str) -> anyhow::Result<String> {
        let tempdir = TempDir::new().context("failed to create tempdir for push_commit")?;
        let work = tempdir.path();
        let work_str = work
            .to_str()
            .ok_or_else(|| anyhow!("work path contains invalid UTF-8"))?;

        run_git(&["clone", &self.url, work_str])?;
        run_git(&["-C", work_str, "config", "user.name", "Test"])?;
        run_git(&["-C", work_str, "config", "user.email", "test@example.com"])?;
        run_git(&["-C", work_str, "fetch", "origin", branch])?;
        run_git(&[
            "-C",
            work_str,
            "checkout",
            "-B",
            branch,
            &format!("origin/{branch}"),
        ])?;

        let file_path = work.join(file);
        let mut existing = std::fs::read_to_string(&file_path).unwrap_or_default();
        existing.push_str(content);
        std::fs::write(&file_path, &existing).context("failed to write file for push_commit")?;

        run_git(&["-C", work_str, "add", file])?;
        run_git(&["-C", work_str, "commit", "-m", "additional change"])?;
        run_git(&["-C", work_str, "push", "origin", branch])?;

        self.branch_sha(branch)
    }

    /// Set the HEAD symbolic reference of the bare remote to the given branch.
    pub fn set_head(&self, branch: &str) -> anyhow::Result<()> {
        run_git(&[
            "--git-dir",
            &self.path,
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{branch}"),
        ])
    }

    /// Return the unified diff between two branches.
    pub fn diff(&self, base: &str, head: &str) -> anyhow::Result<String> {
        run_git_output(&[
            "--git-dir",
            &self.path,
            "diff",
            &format!("refs/heads/{base}"),
            &format!("refs/heads/{head}"),
        ])
    }

    /// Read the contents of a file at a given path on the specified branch.
    pub fn read_file(&self, branch: &str, path: &str) -> anyhow::Result<String> {
        run_git_output(&[
            "--git-dir",
            &self.path,
            "show",
            &format!("refs/heads/{branch}:{path}"),
        ])
    }

    /// Count the number of commits between two SHAs (exclusive base, inclusive head).
    pub fn commit_count(&self, base_sha: &str, head_sha: &str) -> anyhow::Result<usize> {
        let output = run_git_output(&[
            "--git-dir",
            &self.path,
            "rev-list",
            "--count",
            &format!("{base_sha}..{head_sha}"),
        ])?;
        output
            .trim()
            .parse()
            .context("failed to parse commit count")
    }

    /// Return the full commit message of the given SHA.
    pub fn commit_message(&self, sha: &str) -> anyhow::Result<String> {
        run_git_output(&["--git-dir", &self.path, "log", "-1", "--format=%B", sha])
    }
}

/// Run a git command, returning an error if it exits with a non-zero status.
fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !status.success() {
        anyhow::bail!("git {} exited with {status}", args.join(" "));
    }
    Ok(())
}

/// Run a git command and capture its stdout.
fn run_git_output(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .stderr(std::process::Stdio::piped())
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {} exited with {}: {stderr}",
            args.join(" "),
            output.status
        );
    }
    String::from_utf8(output.stdout).context("git output contained invalid UTF-8")
}
