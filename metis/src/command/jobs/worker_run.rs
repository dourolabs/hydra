use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use git2::{build::CheckoutBuilder, BranchType, Commit, ErrorCode, Oid, Repository};
use metis_common::{
    constants::{ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_METIS_DOCUMENTS_DIR, ENV_METIS_ISSUE_ID},
    job_status::JobStatusUpdate,
    jobs::{Bundle, WorkerContext},
    IssueId, TaskId,
};
use tempfile::Builder;

use crate::build_cache::build_cache_client;
use crate::command::documents::{push_documents, sync_documents, PushArgs, SyncArgs};
use crate::command::patches::resolve_service_repo_name;
use crate::git::{
    clone_repo, commit_changes, configure_repo, fetch_remote, push_branch, resolve_head_oid,
    stage_all_changes, workdir_diff,
};
use crate::worker_commands::WorkerCommands;
use crate::{client::MetisClientInterface, command::output::CommandContext};

pub async fn run(
    client: &dyn MetisClientInterface,
    job: TaskId,
    dest: PathBuf,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    claude_code_oauth_token: Option<String>,
    issue_id: Option<IssueId>,
    use_tempdir: bool,
    commands: &dyn WorkerCommands,
    _context: &CommandContext,
) -> Result<()> {
    let WorkerContext {
        request_context,
        variables,
        prompt,
        model,
        build_cache,
        ..
    } = client.get_job_context(&job).await?;
    let service_repo_name = resolve_service_repo_name(client, Some(&job)).await?;
    let dest = if use_tempdir {
        let tmp = tempfile::tempdir().context("failed to create temporary working directory")?;
        let tmp_path = tmp.keep();
        log_status(format!("Using temporary directory: {}", tmp_path.display()));
        tmp_path
    } else {
        ensure_clean_destination(&dest)?;
        dest
    };
    let mut execution_env = variables;
    ensure_color_output_env(&mut execution_env);
    if let Some(token) = claude_code_oauth_token
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        execution_env.insert(ENV_CLAUDE_CODE_OAUTH_TOKEN.to_string(), token.clone());
    }
    let worker_home_dir = resolve_worker_home_dir();
    let issue_branch_id = issue_id
        .as_ref()
        .map(|value| value.to_string())
        .or_else(|| execution_env.get(ENV_METIS_ISSUE_ID).cloned());
    let github_token = client.get_github_token().await.ok();
    let repo_path = dest.join("repo");
    let base_commit = match request_context {
        Bundle::None => {
            fs::create_dir_all(&repo_path)
                .with_context(|| format!("failed to create {repo_path:?}"))?;
            None
        }
        Bundle::GitRepository { url, rev } => {
            clone_repo(&url, &rev, &repo_path, github_token.as_deref())
                .context("failed to clone repository")?;
            configure_repo(&repo_path, "Metis Worker", "metis-worker@example.com")
                .context("failed to configure git repository")?;
            fetch_remote(&repo_path, github_token.as_deref())
                .context("failed to fetch all remote branches")?;
            resolve_head_oid(&repo_path).context("failed to resolve HEAD commit")?
        }
        _ => bail!("unsupported bundle type for worker context"),
    };

    if base_commit.is_some() {
        initialize_tracking_branches(
            &repo_path,
            issue_branch_id.as_deref(),
            &job,
            github_token.as_deref(),
        )
        .context("failed to initialize tracking branches")?;
    }

    let mut downloaded_cache_sha: Option<String> = None;
    if base_commit.is_some() {
        if let (Some(build_cache), Some(service_repo_name)) =
            (build_cache.as_ref(), service_repo_name.as_ref())
        {
            let cache_apply_start = Instant::now();
            match build_cache_client(build_cache) {
                Ok(client) => match client
                    .apply_nearest_cache(
                        &repo_path,
                        worker_home_dir.as_deref(),
                        service_repo_name.clone(),
                    )
                    .await
                {
                    Ok((Some(key), timings)) => {
                        let elapsed = cache_apply_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Build cache download/apply completed in {elapsed:.2}s (applied entry '{}').",
                            key.object_key()
                        ));
                        log_apply_cache_timings(&timings);
                        downloaded_cache_sha = Some(key.git_sha.clone());
                    }
                    Ok((None, timings)) => {
                        let elapsed = cache_apply_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Build cache download/apply completed in {elapsed:.2}s (no entry found)."
                        ));
                        log_apply_cache_timings(&timings);
                    }
                    Err(err) => {
                        let elapsed = cache_apply_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Build cache download/apply completed in {elapsed:.2}s (skipped: {err})."
                        ))
                    }
                },
                Err(err) => {
                    let elapsed = cache_apply_start.elapsed().as_secs_f64();
                    log_status(format!(
                        "Build cache download/apply completed in {elapsed:.2}s (skipped: {err})."
                    ))
                }
            }
        }
    }

    // Sync documents to a well-known sibling directory next to the repo checkout (best-effort).
    let documents_path = dest.join("documents");
    std::fs::create_dir_all(&documents_path).context("failed to create documents directory")?;
    let documents_path = documents_path
        .canonicalize()
        .context("failed to canonicalize documents directory path")?;
    match sync_documents(
        client,
        SyncArgs {
            directory: Some(documents_path.clone()),
            path_prefix: None,
            clean: false,
        },
    )
    .await
    {
        Ok(()) => {
            execution_env.insert(
                ENV_METIS_DOCUMENTS_DIR.to_string(),
                documents_path.to_string_lossy().to_string(),
            );
        }
        Err(err) => {
            log_status(format!(
                "Warning: document sync failed, continuing without METIS_DOCUMENTS_DIR: {err}"
            ));
        }
    }

    let output_dir = Builder::new()
        .prefix("codex-output")
        .tempdir()
        .context("failed to create temporary codex output directory")?;
    let output_path = output_dir.path().join(crate::constants::OUTPUT_TXT_FILE);

    let mut errors = Vec::new();
    let last_message = match commands
        .run(
            &prompt,
            model.as_deref(),
            openai_api_key.clone(),
            anthropic_api_key.clone(),
            claude_code_oauth_token.clone(),
            &repo_path,
            &execution_env,
            &output_path,
        )
        .await
    {
        Ok(message) => message,
        Err(err) => {
            errors.push(err);
            errors
                .last()
                .map(|err| err.to_string())
                .unwrap_or_else(|| "worker command execution failed".to_string())
        }
    };

    if base_commit.is_some() {
        if let Err(err) = finalize_task_run(&repo_path, &job, github_token.as_deref()) {
            errors.push(err.context("failed to finalize task output branches"));
        }
    }

    // Push document changes back to the server (best-effort).
    if execution_env.contains_key(ENV_METIS_DOCUMENTS_DIR) {
        if let Err(err) = push_documents(
            client,
            PushArgs {
                directory: Some(documents_path.clone()),
                dry_run: false,
                path_prefix: None,
            },
        )
        .await
        {
            log_status(format!("Warning: document push failed, continuing: {err}"));
        }
    }

    if base_commit.is_some() {
        if let (Some(build_cache), Some(service_repo_name)) =
            (build_cache.as_ref(), service_repo_name.as_ref())
        {
            let cache_upload_start = Instant::now();
            match build_cache_client(build_cache) {
                Ok(client) => match resolve_head_oid(&repo_path) {
                    Ok(Some(head_oid)) => {
                        let git_sha = head_oid.to_string();
                        if downloaded_cache_sha.as_deref() == Some(git_sha.as_str()) {
                            let elapsed = cache_upload_start.elapsed().as_secs_f64();
                            log_status(format!(
                                "Build cache upload skipped (cache entry already up-to-date) in {elapsed:.2}s."
                            ));
                        } else {
                            const MAX_ATTEMPTS: u32 = 3;
                            let mut last_error = None;
                            for attempt in 1..=MAX_ATTEMPTS {
                                log_status(format!(
                                    "Uploading build cache (attempt {attempt}/{MAX_ATTEMPTS})..."
                                ));
                                match client
                                    .build_and_upload_cache(
                                        &repo_path,
                                        worker_home_dir.as_deref(),
                                        service_repo_name.clone(),
                                        &git_sha,
                                    )
                                    .await
                                {
                                    Ok((key, timings)) => {
                                        let elapsed = cache_upload_start.elapsed().as_secs_f64();
                                        log_status(format!(
                                            "Build cache create/upload completed in {elapsed:.2}s (uploaded entry '{}').",
                                            key.object_key()
                                        ));
                                        log_upload_cache_timings(&timings);
                                        last_error = None;
                                        break;
                                    }
                                    Err(err) => {
                                        last_error = Some(err);
                                        if attempt < MAX_ATTEMPTS {
                                            let delay_secs = 2u64.pow(attempt);
                                            log_status(format!(
                                                "Build cache upload attempt {attempt} failed, retrying in {delay_secs}s..."
                                            ));
                                            tokio::time::sleep(Duration::from_secs(delay_secs))
                                                .await;
                                        }
                                    }
                                }
                            }
                            if let Some(err) = last_error {
                                let elapsed = cache_upload_start.elapsed().as_secs_f64();
                                log_status(format!(
                                    "Build cache create/upload completed in {elapsed:.2}s (skipped after {MAX_ATTEMPTS} attempts: {err})."
                                ));
                            }
                        }
                    }
                    Ok(None) => {
                        let elapsed = cache_upload_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Build cache create/upload completed in {elapsed:.2}s (skipped: HEAD is unavailable)."
                        ))
                    }
                    Err(err) => {
                        let elapsed = cache_upload_start.elapsed().as_secs_f64();
                        log_status(format!(
                            "Build cache create/upload completed in {elapsed:.2}s (skipped: failed to resolve HEAD: {err})."
                        ))
                    }
                },
                Err(err) => {
                    let elapsed = cache_upload_start.elapsed().as_secs_f64();
                    log_status(format!(
                        "Build cache create/upload completed in {elapsed:.2}s (skipped: {err})."
                    ))
                }
            }
        }
    }

    let status_update = if errors.is_empty() {
        JobStatusUpdate::Complete {
            last_message: Some(last_message.clone()),
        }
    } else {
        JobStatusUpdate::Failed {
            reason: errors
                .first()
                .map(|err| err.to_string())
                .unwrap_or_else(|| "worker run failed for unknown reasons".to_string()),
        }
    };

    if let Err(err) = submit_job_status(client, &job, status_update).await {
        errors.push(err);
    }

    if let Some(err) = errors.into_iter().next() {
        Err(err)
    } else {
        Ok(())
    }
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
    status: JobStatusUpdate,
) -> Result<()> {
    log_status(format!("Updating status for job '{job}' via metis-server…"));
    match client.set_job_status(job, &status).await {
        Ok(response) => {
            let last_message_length = status
                .last_message()
                .map(|message| message.len())
                .unwrap_or(0);
            log_status(format!(
                "Status updated for job '{}'. Stored last message length: {}",
                response.job_id, last_message_length,
            ));
            Ok(())
        }
        Err(err) if err.to_string().contains("409 Conflict") => {
            log_status(format!(
                "Status for job '{job}' was already set (conflict); ignoring."
            ));
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn initialize_tracking_branches(
    repo_root: &Path,
    issue_id: Option<&str>,
    task_id: &TaskId,
    github_token: Option<&str>,
) -> Result<()> {
    let issue_label = issue_id.unwrap_or("unknown");
    log_status(format!(
        "Ensuring git tracking branches exist (issue: {issue_label}, task: {task_id}) before starting work…"
    ));
    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;
    let head_commit = repo
        .head()
        .context("failed to resolve HEAD for tracking branch initialization")?
        .peel_to_commit()
        .context("failed to peel HEAD to commit for tracking branch initialization")?;

    let remote_name = "origin";
    let mut task_branch_target = head_commit.id();
    let task_head_branch = format!("metis/{task_id}/head");

    if let Some(issue_id) = issue_id {
        let issue_base_branch = format!("metis/{issue_id}/base");
        let issue_head_branch = format!("metis/{issue_id}/head");
        let issue_head_exists = remote_branch_exists(&repo, remote_name, &issue_head_branch)
            || repo
                .find_branch(&issue_head_branch, BranchType::Local)
                .is_ok();

        if issue_head_exists {
            let issue_head_commit = find_branch_commit(&repo, &issue_head_branch, remote_name)?
                .ok_or_else(|| {
                    anyhow!(
                        "issue head branch '{issue_head_branch}' exists but failed to resolve commit"
                    )
                })?;

            if remote_branch_exists(&repo, remote_name, &issue_base_branch) {
                ensure_local_branch_from_remote(&repo, &issue_base_branch, remote_name)
                    .with_context(|| {
                        format!("failed to track remote issue base branch '{issue_base_branch}'")
                    })?;
            } else {
                set_branch_to_commit(&repo, &issue_base_branch, issue_head_commit.id())
                    .with_context(|| {
                        format!("failed to align issue base branch '{issue_base_branch}' with head")
                    })?;
            }

            if remote_branch_exists(&repo, remote_name, &issue_head_branch) {
                ensure_local_branch_from_remote(&repo, &issue_head_branch, remote_name)
                    .with_context(|| {
                        format!("failed to track remote issue head branch '{issue_head_branch}'")
                    })?;
            } else {
                set_branch_to_commit(&repo, &issue_head_branch, issue_head_commit.id())
                    .with_context(|| {
                        format!("failed to align issue head branch '{issue_head_branch}' locally")
                    })?;
            }

            task_branch_target = issue_head_commit.id();
        } else {
            set_branch_to_commit(&repo, &issue_base_branch, head_commit.id()).with_context(
                || format!("failed to create issue base branch '{issue_base_branch}'"),
            )?;
            push_branch(repo_root, &issue_base_branch, github_token, true).with_context(|| {
                format!("failed to push issue base branch '{issue_base_branch}' to remote origin")
            })?;

            set_branch_to_commit(&repo, &issue_head_branch, head_commit.id()).with_context(
                || format!("failed to create issue head branch '{issue_head_branch}'"),
            )?;
            push_branch(repo_root, &issue_head_branch, github_token, true).with_context(|| {
                format!("failed to push issue head branch '{issue_head_branch}' to remote origin")
            })?;
        }
    }

    let task_base_branch = format!("metis/{task_id}/base");
    set_branch_to_commit(&repo, &task_base_branch, task_branch_target)
        .with_context(|| format!("failed to update task base branch '{task_base_branch}'"))?;
    push_branch(repo_root, &task_base_branch, github_token, true).with_context(|| {
        format!("failed to push task base branch '{task_base_branch}' to remote origin")
    })?;

    set_branch_to_commit(&repo, &task_head_branch, task_branch_target)
        .with_context(|| format!("failed to update task head branch '{task_head_branch}'"))?;
    push_branch(repo_root, &task_head_branch, github_token, true).with_context(|| {
        format!("failed to push task head branch '{task_head_branch}' to remote origin")
    })?;

    let working_branch = if let Some(issue_id) = issue_id {
        format!("metis/{issue_id}/head")
    } else {
        task_head_branch.clone()
    };
    checkout_local_branch(&repo, &working_branch).with_context(|| {
        format!("failed to checkout working branch '{working_branch}' before worker run")
    })?;

    Ok(())
}

fn finalize_task_run(repo_root: &Path, task_id: &TaskId, github_token: Option<&str>) -> Result<()> {
    log_status(format!(
        "Auto-committing worker changes for task '{task_id}' and syncing tracking branches…"
    ));
    let diff = workdir_diff(repo_root)?;
    let has_changes = !diff.trim().is_empty();

    if has_changes {
        stage_all_changes(repo_root).context("failed to stage repository changes")?;
        let message = format!("Metis worker auto-commit for task {task_id}");
        commit_changes(repo_root, &message)
            .context("failed to auto-commit worker changes to git")?;
    } else {
        log_status(format!(
            "No uncommitted changes detected after worker run for task '{task_id}'; skipping auto-commit."
        ));
    }

    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repository at {}", repo_root.display()))?;

    let task_head_branch = format!("metis/{task_id}/head");
    update_branch_to_head(&repo, &task_head_branch).with_context(|| {
        format!("failed to update task head branch '{task_head_branch}' to latest commit")
    })?;
    push_branch(repo_root, &task_head_branch, github_token, true).with_context(|| {
        format!("failed to push task head branch '{task_head_branch}' to remote origin")
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
        "update metis tracking branch reference",
    )
    .with_context(|| format!("failed to set branch '{branch}' reference to commit {commit}"))?;
    Ok(())
}

fn update_branch_to_head(repo: &Repository, branch: &str) -> Result<()> {
    let head_commit = repo
        .head()
        .context("failed to resolve HEAD for tracking branch update")?
        .peel_to_commit()
        .context("failed to peel HEAD commit for tracking branch update")?;
    let reference_name = format!("refs/heads/{branch}");
    repo.reference(
        &reference_name,
        head_commit.id(),
        true,
        "update metis tracking branch",
    )
    .with_context(|| format!("failed to update branch '{branch}' to latest commit"))?;
    Ok(())
}

fn log_status(message: impl std::fmt::Display) {
    println!("{message}");
}

fn log_apply_cache_timings(timings: &metis_build_cache::ApplyCacheTimings) {
    log_status(format!(
        "  list_caches: {:.2}s",
        timings.list_caches.as_secs_f64()
    ));
    log_status(format!(
        "  find_nearest: {:.2}s",
        timings.find_nearest.as_secs_f64()
    ));
    if let Some(dl) = &timings.download {
        log_status(format!(
            "  download: {:.2}s ({} bytes)",
            dl.elapsed.as_secs_f64(),
            dl.file_size_bytes
        ));
    }
    if let Some(apply) = &timings.apply {
        log_status(format!("  apply: {:.2}s", apply.as_secs_f64()));
    }
}

fn log_upload_cache_timings(timings: &metis_build_cache::UploadCacheTimings) {
    log_status(format!(
        "  build_archive: {:.2}s ({} bytes)",
        timings.build_archive.elapsed.as_secs_f64(),
        timings.build_archive.file_size_bytes
    ));
    log_status(format!("  upload: {:.2}s", timings.upload.as_secs_f64()));
    log_status(format!("  evict: {:.2}s", timings.evict.as_secs_f64()));
}

fn resolve_worker_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        git::{
            clone_repo as git_clone_repo, commit_changes as git_commit_changes,
            configure_repo as git_configure_repo, current_branch as git_current_branch,
            push_branch as git_push_branch, stage_all_changes as git_stage_all_changes,
        },
        test_utils::ids::task_id,
    };
    use git2::{build::CheckoutBuilder, Oid, Repository};
    use std::{collections::HashMap, path::Path};

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

    #[test]
    fn initialize_tracking_branches_creates_issue_and_task_branches() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;

        let issue_id = "i-worker-123";
        let job_id = task_id("t-worker-123");
        initialize_tracking_branches(clone_dir.path(), Some(issue_id), &job_id, None)?;

        let base_branch = format!("metis/{issue_id}/base");
        let head_branch = format!("metis/{issue_id}/head");
        let task_base_branch = format!("metis/{job_id}/base");
        let task_head_branch = format!("metis/{job_id}/head");
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
            reference_target(&remote_repo, &format!("refs/heads/{base_branch}"))?,
            head_oid,
            "issue base branch should be synchronized to the clone's HEAD"
        );
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{head_branch}"))?,
            head_oid,
            "issue head branch should start from the clone's HEAD"
        );
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{task_base_branch}"))?,
            head_oid,
            "task base branch should match the clone's HEAD"
        );
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{task_head_branch}"))?,
            head_oid,
            "task head branch should match the clone's HEAD"
        );
        let working_diff = workdir_diff(clone_dir.path())?;
        assert!(
            working_diff.trim().is_empty(),
            "working directory should be clean after initialize_tracking_branches"
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
            .context("failed to open remote repo for initial base ref")?;
        let base_branch = format!("metis/{issue_id}/base");
        let head_branch = format!("metis/{issue_id}/head");
        let base_ref_name = format!("refs/heads/{base_branch}");
        let head_ref_name = format!("refs/heads/{head_branch}");
        let initial_base_target = reference_target(&remote_repo, &base_ref_name)?;
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
            "task branches should reuse the existing issue head commit"
        );

        let updated_remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repo for updated branch assertions")?;
        assert_eq!(
            reference_target(
                &updated_remote_repo,
                &format!("refs/heads/metis/{next_job}/base")
            )?,
            initial_issue_head_target,
            "task base branch should match the existing issue head commit"
        );
        assert_eq!(
            reference_target(
                &updated_remote_repo,
                &format!("refs/heads/metis/{next_job}/head")
            )?,
            initial_issue_head_target,
            "task head branch should match the existing issue head commit"
        );
        assert_eq!(
            initial_base_target,
            reference_target(&updated_remote_repo, &base_ref_name)?,
            "issue base branch should remain unchanged"
        );
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
    fn finalize_task_run_commits_changes_and_pushes_task_head_branch() -> Result<()> {
        let fixture = RemoteFixture::new()?;
        let issue_id = "i-worker-789";
        let job_id = task_id("t-worker-789");
        let clone_dir = tempfile::tempdir().context("failed to create clone tempdir")?;
        git_clone_repo(fixture.remote_path(), "main", clone_dir.path(), None)?;
        configure_repo(clone_dir.path(), "Metis Worker", "metis-worker@example.com")
            .context("failed to configure git repository")?;
        initialize_tracking_branches(clone_dir.path(), Some(issue_id), &job_id, None)?;

        let remote_repo_before = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for pre-finalize snapshot")?;
        let issue_head_before = reference_target(
            &remote_repo_before,
            &format!("refs/heads/metis/{issue_id}/head"),
        )?;
        drop(remote_repo_before);

        std::fs::write(clone_dir.path().join("README.md"), "updated content\n")
            .context("failed to edit README during finalize test")?;
        std::fs::write(
            clone_dir.path().join("new_file.txt"),
            "new untracked content\n",
        )
        .context("failed to write new file during finalize test")?;

        finalize_task_run(clone_dir.path(), &job_id, None)?;

        let repo = Repository::open(clone_dir.path())
            .context("failed to open cloned repository for finalize assertions")?;
        let working_diff = workdir_diff(clone_dir.path())?;
        assert!(
            working_diff.trim().is_empty(),
            "auto-commit should leave a clean working tree"
        );
        let task_head_branch = format!("metis/{job_id}/head");
        assert!(
            repo.find_branch(&task_head_branch, BranchType::Local)
                .is_ok(),
            "task head branch should exist locally after finalize"
        );
        let head_oid = repo
            .head()?
            .target()
            .ok_or_else(|| anyhow!("HEAD missing target after finalize"))?;
        let remote_repo = Repository::open(fixture.remote_dir())
            .context("failed to open remote repository for finalize assertions")?;
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/{task_head_branch}"))?,
            head_oid,
            "task head branch should be pushed to the new commit"
        );
        assert_ne!(
            head_oid, issue_head_before,
            "HEAD should have advanced past the initial issue head commit"
        );
        assert_eq!(
            reference_target(&remote_repo, &format!("refs/heads/metis/{issue_id}/head"))?,
            issue_head_before,
            "issue head branch should NOT advance during finalize_task_run"
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
    fn documents_path_is_absolute_after_canonicalize() -> Result<()> {
        // Simulate the worker_run logic: start with a relative dest, create the documents
        // directory, then canonicalize it. The resulting path must be absolute.
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test")?;
        let original_dir = std::env::current_dir().context("failed to get current dir")?;
        std::env::set_current_dir(tempdir.path()).context("failed to change to temp directory")?;

        let dest = PathBuf::from(".");
        let documents_path = dest.join("documents");
        std::fs::create_dir_all(&documents_path).context("failed to create documents directory")?;
        let documents_path = documents_path
            .canonicalize()
            .context("failed to canonicalize documents directory path")?;

        // Restore the original working directory before assertions so test cleanup works.
        std::env::set_current_dir(&original_dir).context("failed to restore original directory")?;

        assert!(
            documents_path.is_absolute(),
            "documents_path should be absolute after canonicalize, got: {documents_path:?}"
        );

        // Verify the path would be correct in execution_env.
        let mut execution_env = HashMap::new();
        execution_env.insert(
            ENV_METIS_DOCUMENTS_DIR.to_string(),
            documents_path.to_string_lossy().to_string(),
        );
        let env_value = execution_env.get(ENV_METIS_DOCUMENTS_DIR).unwrap();
        assert!(
            PathBuf::from(env_value).is_absolute(),
            "ENV_METIS_DOCUMENTS_DIR should be an absolute path, got: {env_value}"
        );

        Ok(())
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
