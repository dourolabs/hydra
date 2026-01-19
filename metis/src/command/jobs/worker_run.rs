use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use git2::{build::CheckoutBuilder, BranchType, Commit, ErrorCode, Repository};
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_METIS_ISSUE_ID},
    job_status::JobStatusUpdate,
    jobs::{Bundle, WorkerContext},
    patches::GitOid,
    RepoName, TaskId,
};
use tempfile::Builder;

use crate::client::MetisClientInterface;
use crate::command::patches::{create_patch_artifact_from_repo, resolve_service_repo_name};
use crate::git::{clone_repo, configure_repo, push_branch, resolve_head_oid, workdir_diff};
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

    if base_commit.is_some() {
        if let Some(issue_id) = execution_env.get(ENV_METIS_ISSUE_ID) {
            initialize_issue_branches(&dest, issue_id, github_token.as_deref())
                .context("failed to initialize issue tracking branches")?;
        }
    }

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

fn initialize_issue_branches(
    repo_root: &Path,
    issue_id: &str,
    github_token: Option<&str>,
) -> Result<()> {
    println!("Ensuring git tracking branches exist for issue '{issue_id}' before starting work…");
    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;
    let head_commit = repo
        .head()
        .context("failed to resolve HEAD for issue branch initialization")?
        .peel_to_commit()
        .context("failed to peel HEAD to commit for issue branch initialization")?;

    let remote_name = "origin";
    let base_branch = format!("metis/{issue_id}/base");
    let head_branch = format!("metis/{issue_id}/head");

    let base_remote_exists = remote_branch_exists(&repo, remote_name, &base_branch);
    if base_remote_exists {
        ensure_local_branch_from_remote(&repo, &base_branch, remote_name)
            .with_context(|| format!("failed to track remote base branch '{base_branch}'"))?;
    } else {
        ensure_local_branch(&repo, &base_branch, &head_commit)
            .with_context(|| format!("failed to create base branch '{base_branch}'"))?;
        push_branch(repo_root, &base_branch, github_token).with_context(|| {
            format!("failed to push base branch '{base_branch}' to remote origin")
        })?;
    }

    if remote_branch_exists(&repo, remote_name, &head_branch) {
        ensure_local_branch_from_remote(&repo, &head_branch, remote_name).with_context(|| {
            format!("failed to create local tracking branch for '{head_branch}'")
        })?;
    } else {
        let base_commit = find_branch_commit(&repo, &base_branch, remote_name)?
            .unwrap_or_else(|| head_commit.clone());
        ensure_local_branch(&repo, &head_branch, &base_commit)
            .with_context(|| format!("failed to create head branch '{head_branch}'"))?;
        push_branch(repo_root, &head_branch, github_token).with_context(|| {
            format!("failed to push head branch '{head_branch}' to remote origin")
        })?;
    }

    checkout_local_branch(&repo, &head_branch).with_context(|| {
        format!("failed to checkout issue head branch '{head_branch}' before worker run")
    })?;

    Ok(())
}

fn ensure_local_branch<'repo>(
    repo: &'repo Repository,
    branch: &str,
    commit: &Commit<'repo>,
) -> Result<()> {
    match repo.find_branch(branch, BranchType::Local) {
        Ok(_) => Ok(()),
        Err(err) if err.code() == ErrorCode::NotFound => {
            repo.branch(branch, commit, false)
                .with_context(|| format!("failed to create branch '{branch}'"))?;
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("failed to resolve branch '{branch}'")),
    }
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

fn find_branch_commit<'repo>(
    repo: &'repo Repository,
    branch: &str,
    remote: &str,
) -> Result<Option<Commit<'repo>>> {
    if let Ok(local_branch) = repo.find_branch(branch, BranchType::Local) {
        return local_branch
            .into_reference()
            .peel_to_commit()
            .map(Some)
            .with_context(|| format!("failed to peel local branch '{branch}' to commit"));
    }

    let remote_reference = format!("refs/remotes/{remote}/{branch}");
    let reference = match repo.find_reference(&remote_reference) {
        Ok(reference) => reference,
        Err(err) if err.code() == ErrorCode::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to resolve remote branch '{remote_reference}'"))
        }
    };

    reference
        .peel_to_commit()
        .map(Some)
        .with_context(|| format!("failed to peel remote branch '{remote_reference}' to commit"))
}

fn checkout_local_branch(repo: &Repository, branch: &str) -> Result<()> {
    repo.set_head(&format!("refs/heads/{branch}"))
        .with_context(|| format!("failed to set HEAD to branch '{branch}'"))?;
    let mut checkout = CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_head(Some(&mut checkout))
        .with_context(|| format!("failed to checkout branch '{branch}'"))?;
    Ok(())
}

fn remote_branch_exists(repo: &Repository, remote: &str, branch: &str) -> bool {
    let reference = format!("refs/remotes/{remote}/{branch}");
    repo.find_reference(&reference).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MockMetisClient,
        git::{
            clone_repo as git_clone_repo, commit_changes as git_commit_changes,
            configure_repo as git_configure_repo, current_branch as git_current_branch,
            push_branch as git_push_branch, stage_all_changes as git_stage_all_changes,
        },
        test_utils::ids::{patch_id, task_id},
    };
    use git2::{build::CheckoutBuilder, Repository};
    use metis_common::patches::UpsertPatchResponse;
    use std::{collections::HashMap, path::Path, str::FromStr};

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

    #[test]
    fn initialize_issue_branches_creates_remote_and_local_branches() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;

        let issue_id = "i-worker-123";
        initialize_issue_branches(clone_dir.path(), issue_id, None)?;

        let base_branch = format!("metis/{issue_id}/base");
        let head_branch = format!("metis/{issue_id}/head");
        let repo = Repository::open(clone_dir.path())
            .context("failed to open cloned repository for assertions")?;
        assert!(
            repo.find_branch(&base_branch, BranchType::Local).is_ok(),
            "base branch should be created locally"
        );
        assert!(
            repo.find_branch(&head_branch, BranchType::Local).is_ok(),
            "head branch should be created locally"
        );
        assert_eq!(
            git_current_branch(clone_dir.path())?,
            head_branch,
            "issue head branch should be checked out for worker execution"
        );

        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for assertions")?;
        assert!(
            remote_repo
                .find_reference(&format!("refs/heads/{base_branch}"))
                .is_ok(),
            "base branch should be pushed to remote"
        );
        assert!(
            remote_repo
                .find_reference(&format!("refs/heads/{head_branch}"))
                .is_ok(),
            "head branch should be pushed to remote"
        );

        Ok(())
    }

    #[test]
    fn initialize_issue_branches_keeps_existing_remote_base_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let issue_id = "i-worker-456";
        let first_clone = tempfile::tempdir().context("failed to create first clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", first_clone.path(), None)?;
        initialize_issue_branches(first_clone.path(), issue_id, None)?;

        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repo for initial base ref")?;
        let base_branch = format!("metis/{issue_id}/base");
        let base_ref_name = format!("refs/heads/{base_branch}");
        let initial_base_target = remote_repo
            .find_reference(&base_ref_name)
            .and_then(|reference| {
                reference
                    .target()
                    .ok_or_else(|| git2::Error::from_str("missing base ref target"))
            })
            .context("failed to resolve initial base branch target")?;

        fixture.push_new_main_commit("NOTES.md", "new work on main\n")?;

        let second_clone = tempfile::tempdir().context("failed to create second clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", second_clone.path(), None)?;
        initialize_issue_branches(second_clone.path(), issue_id, None)?;

        let updated_remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repo for updated base ref")?;
        let updated_base_target = updated_remote_repo
            .find_reference(&base_ref_name)
            .and_then(|reference| {
                reference
                    .target()
                    .ok_or_else(|| git2::Error::from_str("missing base ref target"))
            })
            .context("failed to resolve updated base branch target")?;
        assert_eq!(
            initial_base_target, updated_base_target,
            "existing base branch should not be rewritten when initializing issue branches"
        );

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
            git_push_branch(upstream_dir.path(), "main", None)
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
            git_push_branch(self.upstream_dir.path(), "main", None)
                .context("failed to push updated main branch")?;
            Ok(())
        }
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
}
