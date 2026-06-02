//! `BundleMount` — pre-agent repo checkout and post-agent git finalize.
//!
//! Always constructed exactly once per worker run. For [`Bundle::None`]
//! the mount simply creates an empty destination directory and has no
//! save phase. For [`Bundle::GitRepository`] it runs the existing
//! `clone_repo` → `configure_repo` → `fetch_remote` → `resolve_head_oid` →
//! `initialize_tracking_branches` sequence at setup time, and
//! `finalize_task_run` at save time.
//!
//! The git helpers (`initialize_tracking_branches`, `finalize_task_run`,
//! and their private companions) used to live in `worker_run.rs`; PR4
//! moves them here as a pure code move with no behavior change. See
//! `/designs/worker-mount-trait.md` for the full design.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use git2::{build::CheckoutBuilder, BranchType, Oid, Repository};
use hydra_common::{sessions::Bundle, SessionId};

use crate::git::{
    clone_repo, commit_changes, configure_repo, fetch_remote, push_branch, resolve_head_oid,
    stage_all_changes, workdir_diff,
};

use super::{Mount, MountError, MountResult, Phase};

/// Per-phase timeout for the post-agent git finalize step.
pub const FINALIZE_TASK_RUN_TIMEOUT: Duration = Duration::from_secs(120);

pub struct BundleMount {
    repo_path: PathBuf,
    bundle: BundleSource,
}

enum BundleSource {
    None,
    GitRepository {
        url: String,
        rev: String,
        github_token: Option<String>,
        session_id: SessionId,
        issue_branch_id: Option<String>,
    },
}

impl BundleMount {
    pub fn empty(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            bundle: BundleSource::None,
        }
    }

    pub fn git_repository(
        repo_path: PathBuf,
        url: String,
        rev: String,
        github_token: Option<String>,
        session_id: SessionId,
        issue_branch_id: Option<String>,
    ) -> Self {
        Self {
            repo_path,
            bundle: BundleSource::GitRepository {
                url,
                rev,
                github_token,
                session_id,
                issue_branch_id,
            },
        }
    }
}

/// Construct the [`BundleMount`] for a given worker bundle.
///
/// Exactly one `BundleMount` is built per worker run; the flavor depends
/// on the bundle shape. `Bundle::Unknown` is rejected as an unsupported
/// bundle type, matching the prior `worker_run::run` behavior.
pub fn bundle_mount(
    bundle: &Bundle,
    repo_path: PathBuf,
    github_token: Option<String>,
    session_id: SessionId,
    issue_branch_id: Option<String>,
) -> Result<BundleMount> {
    match bundle {
        Bundle::None => Ok(BundleMount::empty(repo_path)),
        Bundle::GitRepository { url, rev } => Ok(BundleMount::git_repository(
            repo_path,
            url.clone(),
            rev.clone(),
            github_token,
            session_id,
            issue_branch_id,
        )),
        _ => Err(anyhow!("unsupported bundle type for worker context")),
    }
}

#[async_trait]
impl Mount for BundleMount {
    fn setup_phase(&self) -> Phase {
        Phase {
            label: "repo checkout",
            timeout: None,
        }
    }

    fn save_phase(&self) -> Option<Phase> {
        match self.bundle {
            BundleSource::None => None,
            BundleSource::GitRepository { .. } => Some(Phase {
                label: "git finalize",
                timeout: Some(FINALIZE_TASK_RUN_TIMEOUT),
            }),
        }
    }

    async fn setup(&mut self) -> MountResult {
        std::fs::create_dir_all(&self.repo_path)
            .with_context(|| format!("failed to create {:?}", self.repo_path))
            .map_err(MountError::fatal)?;

        match &self.bundle {
            BundleSource::None => Ok(()),
            BundleSource::GitRepository {
                url,
                rev,
                github_token,
                session_id,
                issue_branch_id,
            } => {
                clone_repo(url, rev, &self.repo_path, github_token.as_deref())
                    .context("failed to clone repository")
                    .map_err(MountError::fatal)?;
                configure_repo(&self.repo_path, "Hydra Worker", "hydra-worker@example.com")
                    .context("failed to configure git repository")
                    .map_err(MountError::fatal)?;
                fetch_remote(&self.repo_path, github_token.as_deref())
                    .context("failed to fetch all remote branches")
                    .map_err(MountError::fatal)?;
                resolve_head_oid(&self.repo_path)
                    .context("failed to resolve HEAD commit")
                    .map_err(MountError::fatal)?;
                initialize_tracking_branches(
                    &self.repo_path,
                    issue_branch_id.as_deref(),
                    session_id,
                    github_token.as_deref(),
                )
                .context("failed to initialize tracking branches")
                .map_err(MountError::fatal)?;
                Ok(())
            }
        }
    }

    async fn save(&mut self) -> MountResult {
        let (session_id, issue_branch_id, github_token) = match &self.bundle {
            BundleSource::None => return Ok(()),
            BundleSource::GitRepository {
                github_token,
                session_id,
                issue_branch_id,
                ..
            } => (
                session_id.clone(),
                issue_branch_id.clone(),
                github_token.clone(),
            ),
        };
        let repo_path = self.repo_path.clone();
        let join_handle = tokio::task::spawn_blocking(move || {
            finalize_task_run(
                &repo_path,
                &session_id,
                issue_branch_id.as_deref(),
                github_token.as_deref(),
            )
        });
        match join_handle.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(MountError::tracked(
                err.context("failed to finalize task output branches"),
            )),
            Err(join_err) => Err(MountError::tracked(anyhow!(
                "git finalize task panicked: {join_err}"
            ))),
        }
    }
}

/// Pick the worker's working branch.
///
/// The worker only needs a single tracking branch — `hydra/<issue>/head` when
/// the session is attached to an issue (so subsequent sessions on the same
/// issue can resume from it), and `hydra/<session>/head` otherwise.
fn working_branch_name(issue_id: Option<&str>, task_id: &SessionId) -> String {
    match issue_id {
        Some(issue_id) => format!("hydra/{issue_id}/head"),
        None => format!("hydra/{task_id}/head"),
    }
}

fn initialize_tracking_branches(
    repo_root: &Path,
    issue_id: Option<&str>,
    task_id: &SessionId,
    github_token: Option<&str>,
) -> Result<()> {
    let issue_label = issue_id.unwrap_or("unknown");
    tracing::info!(
        target: "hydra::mounts::bundle",
        "Ensuring git working branch exists (issue: {issue_label}, task: {task_id}) before starting work…"
    );
    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;
    let head_commit = repo
        .head()
        .context("failed to resolve HEAD for working branch initialization")?
        .peel_to_commit()
        .context("failed to peel HEAD to commit for working branch initialization")?;

    let remote_name = "origin";
    let working_branch = working_branch_name(issue_id, task_id);

    if remote_branch_exists(&repo, remote_name, &working_branch) {
        ensure_local_branch_from_remote(&repo, &working_branch, remote_name)
            .with_context(|| format!("failed to track remote working branch '{working_branch}'"))?;
    } else {
        set_branch_to_commit(&repo, &working_branch, head_commit.id())
            .with_context(|| format!("failed to create working branch '{working_branch}'"))?;
        push_branch(repo_root, &working_branch, github_token, true).with_context(|| {
            format!("failed to push working branch '{working_branch}' to remote origin")
        })?;
    }

    checkout_local_branch(&repo, &working_branch).with_context(|| {
        format!("failed to checkout working branch '{working_branch}' before worker run")
    })?;

    Ok(())
}

fn finalize_task_run(
    repo_root: &Path,
    task_id: &SessionId,
    issue_id: Option<&str>,
    github_token: Option<&str>,
) -> Result<()> {
    tracing::info!(
        target: "hydra::mounts::bundle",
        "Auto-committing worker changes for task '{task_id}' and syncing working branch…"
    );
    let diff = workdir_diff(repo_root)?;
    let has_changes = !diff.trim().is_empty();

    if has_changes {
        stage_all_changes(repo_root).context("failed to stage repository changes")?;
        let message = format!("Hydra worker auto-commit for task {task_id}");
        commit_changes(repo_root, &message)
            .context("failed to auto-commit worker changes to git")?;
    } else {
        tracing::info!(
            target: "hydra::mounts::bundle",
            "No uncommitted changes detected after worker run for task '{task_id}'; skipping auto-commit."
        );
    }

    let working_branch = working_branch_name(issue_id, task_id);
    push_branch(repo_root, &working_branch, github_token, true).with_context(|| {
        format!("failed to push working branch '{working_branch}' to remote origin")
    })?;

    Ok(())
}

fn ensure_local_branch_from_remote(repo: &Repository, branch: &str, remote: &str) -> Result<()> {
    if repo.find_branch(branch, BranchType::Local).is_ok() {
        return Ok(());
    }

    let reference_name = format!("refs/remotes/{remote}/{branch}");
    let reference = repo
        .find_reference(&reference_name)
        .with_context(|| format!("failed to find remote branch '{reference_name}'"))?;
    let commit = reference
        .peel_to_commit()
        .with_context(|| format!("failed to peel remote branch '{reference_name}' to commit"))?;
    let mut local_branch = repo.branch(branch, &commit, false).with_context(|| {
        format!("failed to create local branch '{branch}' from remote '{remote}'")
    })?;
    local_branch
        .set_upstream(Some(&format!("{remote}/{branch}")))
        .with_context(|| format!("failed to set upstream for branch '{branch}'"))?;
    Ok(())
}

fn checkout_local_branch(repo: &Repository, branch: &str) -> Result<()> {
    repo.set_head(&format!("refs/heads/{branch}"))
        .with_context(|| format!("failed to set HEAD to branch '{branch}'"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout))
        .with_context(|| format!("failed to checkout branch '{branch}'"))?;
    Ok(())
}

fn remote_branch_exists(repo: &Repository, remote: &str, branch: &str) -> bool {
    let reference = format!("refs/remotes/{remote}/{branch}");
    repo.find_reference(&reference).is_ok()
}

fn set_branch_to_commit(repo: &Repository, branch: &str, commit: Oid) -> Result<()> {
    let reference_name = format!("refs/heads/{branch}");
    repo.reference(
        &reference_name,
        commit,
        true,
        "update hydra tracking branch reference",
    )
    .with_context(|| format!("failed to set branch '{branch}' reference to commit {commit}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        clone_repo as git_clone_repo, commit_changes as git_commit_changes,
        configure_repo as git_configure_repo, current_branch as git_current_branch,
        push_branch as git_push_branch, stage_all_changes as git_stage_all_changes,
    };
    use crate::test_utils::ids::task_id;
    use git2::{build::CheckoutBuilder, Repository};

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

    fn reference_target(repo: &Repository, reference: &str) -> Result<Oid> {
        repo.find_reference(reference)
            .and_then(|reference| {
                reference
                    .target()
                    .ok_or_else(|| git2::Error::from_str("reference missing target"))
            })
            .with_context(|| format!("failed to resolve reference '{reference}' target"))
    }

    fn promote_branch_to_main(repo: &Repository) -> Result<()> {
        let head_commit = repo
            .head()
            .context("failed to resolve HEAD commit for upstream repo")?
            .peel_to_commit()
            .context("failed to peel HEAD commit for upstream repo")?;
        repo.branch("main", &head_commit, true)
            .context("failed to create 'main' branch in upstream repo")?;
        repo.set_head("refs/heads/main")
            .context("failed to set HEAD to 'main' in upstream repo")?;
        let mut checkout = CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_head(Some(&mut checkout))
            .context("failed to checkout 'main' in upstream repo")?;
        Ok(())
    }

    struct RemoteFixture {
        remote_dir: tempfile::TempDir,
        upstream_dir: tempfile::TempDir,
        remote_path: String,
    }

    impl RemoteFixture {
        fn new() -> Result<Self> {
            let remote_dir = tempfile::tempdir().context("failed to create remote tempdir")?;
            Repository::init_bare(remote_dir.path())
                .context("failed to initialize bare remote repository")?;

            let upstream_dir = tempfile::tempdir().context("failed to create upstream tempdir")?;
            setup_git_repo_with_initial_commit(upstream_dir.path())?;
            let remote_path = remote_dir
                .path()
                .to_str()
                .ok_or_else(|| anyhow!("remote path contains invalid UTF-8"))?
                .to_string();

            let repo = Repository::open(upstream_dir.path())
                .context("failed to open upstream repository for configuration")?;
            repo.remote("origin", &remote_path)
                .context("failed to add origin remote to upstream repository")?;
            promote_branch_to_main(&repo)?;
            git_push_branch(upstream_dir.path(), "main", None, false)
                .context("failed to push main branch to remote fixture")?;
            let remote_repo = Repository::open_bare(remote_dir.path())
                .context("failed to reopen remote repository for head update")?;
            remote_repo
                .set_head("refs/heads/main")
                .context("failed to set remote HEAD to 'main'")?;

            Ok(Self {
                remote_dir,
                upstream_dir,
                remote_path,
            })
        }

        fn remote_path(&self) -> &str {
            &self.remote_path
        }

        fn remote_dir(&self) -> &Path {
            self.remote_dir.path()
        }

        fn push_new_main_commit(&self, filename: &str, contents: &str) -> Result<()> {
            std::fs::write(self.upstream_dir.path().join(filename), contents)
                .with_context(|| format!("failed to update {filename} in upstream repo"))?;
            git_stage_all_changes(self.upstream_dir.path())?;
            git_commit_changes(self.upstream_dir.path(), "upstream change")?;
            git_push_branch(self.upstream_dir.path(), "main", None, false)
                .context("failed to push updated main branch")?;
            Ok(())
        }
    }

    #[test]
    fn initialize_tracking_branches_creates_only_issue_head_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;

        let issue_id = "i-worker-123";
        let job_id = task_id("t-worker-123");
        initialize_tracking_branches(clone_dir.path(), Some(issue_id), &job_id, None)?;

        let head_branch = format!("hydra/{issue_id}/head");
        let repo = Repository::open(clone_dir.path())
            .context("failed to open cloned repository for assertions")?;
        assert_eq!(
            git_current_branch(clone_dir.path())?,
            head_branch,
            "issue head branch should be checked out for worker execution"
        );
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("HEAD missing target after initialize test"))?;
        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for assertions")?;
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{head_branch}"))?,
            head_oid,
            "issue head branch should start from the clone's HEAD"
        );
        for vestigial in [
            format!("refs/heads/hydra/{issue_id}/base"),
            format!("refs/heads/hydra/{job_id}/base"),
            format!("refs/heads/hydra/{job_id}/head"),
        ] {
            assert!(
                remote_repo.find_reference(&vestigial).is_err(),
                "vestigial branch '{vestigial}' should not be created"
            );
        }
        let working_diff = workdir_diff(clone_dir.path())?;
        assert!(
            working_diff.trim().is_empty(),
            "working directory should be clean after initialize_tracking_branches"
        );

        Ok(())
    }

    #[test]
    fn initialize_tracking_branches_without_issue_uses_session_head_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;

        let job_id = task_id("t-noissue-1");
        initialize_tracking_branches(clone_dir.path(), None, &job_id, None)?;

        let head_branch = format!("hydra/{job_id}/head");
        assert_eq!(
            git_current_branch(clone_dir.path())?,
            head_branch,
            "session head branch should be checked out when no issue id is provided"
        );
        let repo = Repository::open(clone_dir.path())
            .context("failed to open cloned repository for assertions")?;
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("HEAD missing target after initialize test"))?;
        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for assertions")?;
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{head_branch}"))?,
            head_oid,
            "session head branch should be pushed to origin"
        );
        let vestigial = format!("refs/heads/hydra/{job_id}/base");
        assert!(
            remote_repo.find_reference(&vestigial).is_err(),
            "session base branch should not be created"
        );

        Ok(())
    }

    #[test]
    fn initialize_tracking_branches_reuses_existing_issue_head_for_new_task() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let issue_id = "i-worker-456";
        let job_id = task_id("t-worker-456");
        let first_clone = tempfile::tempdir().context("failed to create first clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", first_clone.path(), None)?;
        initialize_tracking_branches(first_clone.path(), Some(issue_id), &job_id, None)?;

        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repo for initial head ref")?;
        let head_branch = format!("hydra/{issue_id}/head");
        let head_ref_name = format!("refs/heads/{head_branch}");
        let initial_issue_head_target = reference_target(&remote_repo, &head_ref_name)?;

        fixture.push_new_main_commit("NOTES.md", "new work on main\n")?;

        let next_job = task_id("t-worker-456b");
        let second_clone = tempfile::tempdir().context("failed to create second clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", second_clone.path(), None)?;
        initialize_tracking_branches(second_clone.path(), Some(issue_id), &next_job, None)?;

        let repo = Repository::open(second_clone.path())
            .context("failed to open second clone for assertions")?;
        let cloned_head = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("second clone HEAD missing target"))?;
        assert_eq!(
            cloned_head, initial_issue_head_target,
            "working branch should start from the existing issue head commit"
        );

        let updated_remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repo for updated branch assertions")?;
        assert_eq!(
            initial_issue_head_target,
            reference_target(&updated_remote_repo, &head_ref_name)?,
            "issue head branch should not move during initialization"
        );
        let working_diff = workdir_diff(second_clone.path())?;
        assert!(
            working_diff.trim().is_empty(),
            "working directory should be clean after initialize_tracking_branches"
        );

        Ok(())
    }

    #[test]
    fn finalize_task_run_commits_changes_and_pushes_issue_head_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let issue_id = "i-worker-789";
        let job_id = task_id("t-worker-789");
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;
        configure_repo(clone_dir.path(), "Hydra Worker", "hydra-worker@example.com")
            .context("failed to configure git repository")?;
        initialize_tracking_branches(clone_dir.path(), Some(issue_id), &job_id, None)?;

        let remote_repo_before = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for pre-finalize snapshot")?;
        let issue_head_before = reference_target(
            &remote_repo_before,
            &format!("refs/heads/hydra/{issue_id}/head"),
        )?;
        drop(remote_repo_before);

        std::fs::write(clone_dir.path().join("README.md"), "updated content\n")
            .context("failed to edit README during finalize test")?;
        std::fs::write(
            clone_dir.path().join("new_file.txt"),
            "new untracked content\n",
        )
        .context("failed to write new file during finalize test")?;

        finalize_task_run(clone_dir.path(), &job_id, Some(issue_id), None)?;

        let repo = Repository::open(clone_dir.path())
            .context("failed to open cloned repository for finalize assertions")?;
        let working_diff = workdir_diff(clone_dir.path())?;
        assert!(
            working_diff.trim().is_empty(),
            "auto-commit should leave a clean working tree"
        );
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("HEAD missing target after finalize"))?;
        assert_ne!(
            head_oid, issue_head_before,
            "HEAD should have advanced past the initial issue head commit"
        );

        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for finalize assertions")?;
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/hydra/{issue_id}/head"))?,
            head_oid,
            "issue head branch should advance to the new commit so subsequent sessions resume from it"
        );

        Ok(())
    }

    #[test]
    fn finalize_task_run_without_issue_pushes_session_head_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let job_id = task_id("t-noissue-finalize");
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;
        configure_repo(clone_dir.path(), "Hydra Worker", "hydra-worker@example.com")
            .context("failed to configure git repository")?;
        initialize_tracking_branches(clone_dir.path(), None, &job_id, None)?;

        std::fs::write(clone_dir.path().join("NEW.txt"), "new content\n")
            .context("failed to write new file during finalize test")?;

        finalize_task_run(clone_dir.path(), &job_id, None, None)?;

        let local_repo = Repository::open(clone_dir.path()).context("open clone")?;
        let local_head = local_repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("local HEAD missing"))?;
        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for finalize assertions")?;
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/hydra/{job_id}/head"))?,
            local_head,
            "session head branch should be pushed to the new commit when no issue is set"
        );

        Ok(())
    }

    #[tokio::test]
    async fn empty_setup_creates_directory_and_has_no_save_phase() -> Result<()> {
        let tempdir = tempfile::tempdir().context("create tempdir")?;
        let repo_path = tempdir.path().join("repo");
        assert!(!repo_path.exists(), "precondition: repo dir must not exist");

        let mut mount = BundleMount::empty(repo_path.clone());
        mount.setup().await.expect("empty setup must succeed");

        assert!(
            repo_path.is_dir(),
            "BundleMount::empty must create its target directory at setup"
        );
        assert!(
            mount.save_phase().is_none(),
            "Bundle::None has no post-agent save phase"
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_save_is_noop() -> Result<()> {
        let tempdir = tempfile::tempdir().context("create tempdir")?;
        let repo_path = tempdir.path().join("repo");
        let mut mount = BundleMount::empty(repo_path);
        mount.setup().await.expect("setup");
        // `save_phase` returns None for empty; calling save() directly still
        // returns Ok so the trait remains usable even if a future caller
        // forgets to gate on save_phase.
        let result = mount.save().await;
        assert!(result.is_ok(), "empty save must be a noop");
        Ok(())
    }

    #[tokio::test]
    async fn git_repository_setup_runs_clone_and_initialize_tracking_branches() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let tempdir = tempfile::tempdir().context("dest tempdir")?;
        let repo_path = tempdir.path().join("repo");

        let job = task_id("t-bundle-setup");
        let mut mount = BundleMount::git_repository(
            repo_path.clone(),
            fixture.remote_path().to_string(),
            "main".to_string(),
            None,
            job.clone(),
            Some("i-bundle-setup".to_string()),
        );
        mount.setup().await.expect("git setup must succeed");

        let repo = Repository::open(&repo_path).context("open cloned repo")?;
        assert_eq!(
            git_current_branch(&repo_path)?,
            "hydra/i-bundle-setup/head",
            "issue head branch should be checked out after setup"
        );
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("HEAD missing target after bundle setup"))?;
        let remote_repo =
            Repository::open(fixture.remote_dir()).context("open remote repo for assertions")?;
        assert_eq!(
            reference_target(&remote_repo, "refs/heads/hydra/i-bundle-setup/head")?,
            head_oid,
            "issue head branch should be pushed to origin"
        );
        assert!(
            mount.save_phase().is_some(),
            "GitRepository bundle must expose a save phase"
        );
        Ok(())
    }

    #[tokio::test]
    async fn git_repository_setup_clone_failure_is_fatal() -> Result<()> {
        let tempdir = tempfile::tempdir().context("dest tempdir")?;
        let repo_path = tempdir.path().join("repo");
        let bad_url = tempdir.path().join("does-not-exist");

        let job = task_id("t-bundle-fail");
        let mut mount = BundleMount::git_repository(
            repo_path,
            bad_url.to_string_lossy().into_owned(),
            "main".to_string(),
            None,
            job,
            None,
        );
        let err = mount
            .setup()
            .await
            .expect_err("clone of missing repo must fail");
        assert!(err.fatal, "clone failures must be fatal");
        Ok(())
    }

    #[tokio::test]
    async fn git_repository_save_runs_finalize_task_run() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let tempdir = tempfile::tempdir().context("dest tempdir")?;
        let repo_path = tempdir.path().join("repo");
        let job = task_id("t-bundle-save");

        let mut mount = BundleMount::git_repository(
            repo_path.clone(),
            fixture.remote_path().to_string(),
            "main".to_string(),
            None,
            job.clone(),
            None,
        );
        mount.setup().await.expect("setup");

        std::fs::write(repo_path.join("NEW.txt"), "after-agent edits\n")
            .context("write file post-setup")?;

        mount.save().await.expect("save must commit + push");

        let remote_repo = Repository::open(fixture.remote_dir()).context("open remote repo")?;
        let task_head_ref = format!("refs/heads/hydra/{job}/head");
        let pushed_oid = reference_target(&remote_repo, &task_head_ref)?;
        let local_repo = Repository::open(&repo_path).context("open clone")?;
        let local_head = local_repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("local HEAD missing"))?;
        assert_eq!(pushed_oid, local_head, "save should push the new HEAD");
        Ok(())
    }

    #[tokio::test]
    async fn bundle_mount_factory_rejects_unknown_bundle() {
        let tempdir = tempfile::tempdir().expect("dest tempdir");
        let job = task_id("t-bundle-unknown");
        let result = bundle_mount(
            &Bundle::Unknown,
            tempdir.path().join("repo"),
            None,
            job,
            None,
        );
        assert!(
            result.is_err(),
            "Bundle::Unknown must be rejected at construction"
        );
    }
}
