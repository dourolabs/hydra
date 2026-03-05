use std::{io::Write, path::Path, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use metis_common::{
    activity_log_for_patch_versions,
    constants::{ENV_METIS_ID, ENV_METIS_ISSUE_ID},
    issues::{IssueId, UpsertIssueRequest},
    jobs::BundleSpec,
    patches::{
        Patch, PatchStatus, PatchSummaryRecord, PatchVersionRecord, Review, SearchPatchesQuery,
        UpsertPatchRequest, UpsertPatchResponse,
    },
    repositories::SearchRepositoriesQuery,
    review_utils::{find_last_commit_range_change_timestamp, has_approved_non_dismissed_review},
    users::Username,
    whoami::ActorIdentity,
    PatchId, RelativeVersionNumber, RepoName, TaskId, Versioned,
};
use serde::Serialize;

use crate::git;
use crate::git::{
    apply_patch, current_branch, delete_local_branch as git_delete_local_branch,
    diff_commit_range as git_diff_commit_range, fetch_remote as git_fetch_remote,
    has_uncommitted_changes as git_has_uncommitted_changes, push_branch, push_to_ref,
    resolve_commit_range_from_merge_base as git_resolve_commit_range_from_merge_base,
    squash_merge_onto as git_squash_merge_onto, PushError,
};
use crate::{
    client::MetisClientInterface,
    command::{
        output::{
            render_patch_records, render_patch_summary_records, CommandContext,
            ResolvedOutputFormat,
        },
        utils::changelog::{summarize_activity_log, write_changelog_pretty},
    },
};
#[derive(Subcommand, Debug)]
pub enum PatchesCommand {
    /// List or search patches. Returns summary records without the full diff, description, or review contents; use `get` for complete details.
    List {
        /// Patch id to retrieve.
        #[arg(long = "id", value_name = "PATCH_ID")]
        id: Option<PatchId>,

        /// Query string to filter patches.
        #[arg(long = "query", value_name = "QUERY")]
        query: Option<String>,

        /// Include deleted patches in the listing.
        #[arg(long = "include-deleted")]
        include_deleted: bool,
    },

    /// Get the full details of a single patch by ID. Returns the complete patch including diff, description, and reviews.
    Get {
        /// Patch ID to retrieve.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,

        /// Retrieve a specific version (positive = exact version, negative = offset from latest).
        #[arg(long)]
        version: Option<i64>,
    },

    /// Create a patch from the current git repository.
    Create {
        /// Title for the patch.
        #[arg(long = "title", value_name = "TITLE", required = true)]
        title: String,

        /// Description for the patch.
        #[arg(long = "description", value_name = "DESCRIPTION", required = true)]
        description: String,

        /// Associate the patch with a Metis job.
        #[arg(long = "job", value_name = "METIS_ID", env = ENV_METIS_ID)]
        job: Option<TaskId>,

        /// Associate the merge-request issue with an existing issue id.
        #[arg(
            long = "issue-id",
            value_name = "ISSUE_ID",
            env = ENV_METIS_ISSUE_ID
        )]
        issue_id: IssueId,

        /// Allow creating a patch even when the working directory has uncommitted changes.
        #[arg(long = "allow-uncommitted")]
        allow_uncommitted: bool,

        /// Force push the branch to the remote.
        #[arg(long = "force")]
        force: bool,

        /// Base ref for computing the commit range (defaults to origin/main).
        #[arg(
            long = "base-ref",
            value_name = "BASE_REF",
            default_value = "origin/main"
        )]
        base_ref: String,
    },

    /// Apply a patch to the current git repository.
    Apply {
        /// Patch id to apply.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,
    },

    /// Add a review to an existing patch.
    Review {
        /// Patch id to review.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,

        /// Name of the reviewer.
        #[arg(long = "author", value_name = "AUTHOR", required = true)]
        author: String,

        /// Review contents in plaintext.
        #[arg(long = "contents", value_name = "CONTENTS", required = true)]
        contents: String,

        /// Mark the review as approved.
        #[arg(long = "approve")]
        approve: bool,

        /// Request changes on the patch.
        #[arg(long = "request-changes", conflicts_with = "approve")]
        request_changes: bool,
    },

    /// Update an existing patch.
    Update {
        /// Patch id to update.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,

        /// Updated title for the patch.
        #[arg(long = "title", value_name = "TITLE")]
        title: Option<String>,

        /// Updated description for the patch.
        #[arg(long = "description", value_name = "DESCRIPTION")]
        description: Option<String>,

        /// Updated status for the patch.
        #[arg(long = "status", value_name = "STATUS")]
        status: Option<PatchStatus>,

        /// Force push the branch to the remote.
        #[arg(long = "force")]
        force: bool,

        /// Base ref for computing the commit range (defaults to origin/main).
        #[arg(
            long = "base-ref",
            value_name = "BASE_REF",
            default_value = "origin/main"
        )]
        base_ref: String,
    },

    /// Merge a patch by squash-merging onto a base branch and pushing.
    Merge {
        /// Patch ID to merge.
        #[arg(value_name = "PATCH_ID")]
        patch_id: PatchId,

        /// Base branch to squash-merge onto (defaults to the repo's configured default branch or 'main').
        #[arg(long = "base", value_name = "BASE")]
        base: Option<String>,
    },
    /// Manage patch assets.
    Assets {
        #[command(subcommand)]
        command: PatchAssetsCommand,
    },
    /// Show changelog for a patch (most recent first).
    Changelog {
        /// Patch ID to show changelog for.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,

        /// Maximum number of changelog entries to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Delete a patch.
    Delete {
        /// Patch ID to delete.
        #[arg(value_name = "PATCH_ID")]
        id: PatchId,
    },
}

#[derive(Subcommand, Debug)]
pub enum PatchAssetsCommand {
    /// Upload an asset to a patch.
    Create(PatchAssetCreateArgs),
}

#[derive(Debug, Clone, Args)]
pub struct PatchAssetCreateArgs {
    /// Patch id to attach the asset to.
    #[arg(long = "patch-id", value_name = "PATCH_ID")]
    pub patch_id: PatchId,

    /// Path to the asset file to upload.
    #[arg(value_name = "FILE")]
    pub file_path: PathBuf,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: PatchesCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        PatchesCommand::List {
            id,
            query,
            include_deleted,
        } => list_patches(client, id, query, include_deleted, context.output_format).await,
        PatchesCommand::Get { id, version } => {
            get_patch_by_version(client, &id, version, context.output_format).await
        }
        PatchesCommand::Create {
            title,
            description,
            job,
            issue_id,
            allow_uncommitted,
            force,
            base_ref,
        } => {
            let patch = create_patch(
                client,
                title,
                description,
                job,
                issue_id,
                allow_uncommitted,
                force,
                &base_ref,
                None,
            )
            .await?;
            write_patch_output(context.output_format, &patch)?;
            Ok(())
        }
        PatchesCommand::Apply { id } => apply_patch_record(client, id).await,
        PatchesCommand::Review {
            id,
            author,
            contents,
            approve,
            request_changes,
        } => review_patch(client, id, author, contents, approve, request_changes).await,
        PatchesCommand::Update {
            id,
            title,
            description,
            status,
            force,
            base_ref,
        } => {
            let patch =
                update_patch(client, id, title, description, status, force, &base_ref).await?;
            write_patch_output(context.output_format, &patch)?;
            Ok(())
        }
        PatchesCommand::Merge { patch_id, base } => merge_patch(client, patch_id, base).await,
        PatchesCommand::Assets { command } => {
            patch_assets(client, command, context.output_format).await
        }
        PatchesCommand::Changelog { id, limit } => {
            changelog_patch(client, id, context.output_format, limit).await
        }
        PatchesCommand::Delete { id } => {
            let deleted = client
                .delete_patch(&id)
                .await
                .with_context(|| format!("failed to delete patch '{id}'"))?;
            println!("Deleted patch '{}'", deleted.patch_id);
            Ok(())
        }
    }
}

async fn patch_assets(
    client: &dyn MetisClientInterface,
    command: PatchAssetsCommand,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    match command {
        PatchAssetsCommand::Create(args) => create_patch_asset(client, args, output_format).await,
    }
}

async fn create_patch_asset(
    client: &dyn MetisClientInterface,
    args: PatchAssetCreateArgs,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let mut buffer = Vec::new();
    create_patch_asset_with_writer(
        client,
        &args.patch_id,
        &args.file_path,
        output_format,
        &mut buffer,
    )
    .await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn create_patch_asset_with_writer(
    client: &dyn MetisClientInterface,
    patch_id: &PatchId,
    file_path: &Path,
    output_format: ResolvedOutputFormat,
    writer: &mut impl Write,
) -> Result<()> {
    if !file_path.is_file() {
        bail!(
            "asset file '{}' does not exist or is not a file",
            file_path.display()
        );
    }

    let asset_url = client
        .create_patch_asset(patch_id, file_path)
        .await
        .with_context(|| format!("failed to upload asset for patch '{patch_id}'"))?;

    let output = PatchAssetOutput::new(patch_id.clone(), asset_url);
    render_patch_asset_output(output_format, &output, writer)?;
    Ok(())
}

fn render_patch_asset_output(
    format: ResolvedOutputFormat,
    output: &PatchAssetOutput,
    writer: &mut impl Write,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, output)?;
            writer.write_all(b"\n")?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(
                writer,
                "Uploaded asset for patch {}: {}",
                output.patch_id, output.asset_url
            )?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct PatchAssetOutput {
    patch_id: PatchId,
    asset_url: String,
}

impl PatchAssetOutput {
    fn new(patch_id: PatchId, asset_url: String) -> Self {
        Self {
            patch_id,
            asset_url,
        }
    }
}

async fn list_patches(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
    include_deleted: bool,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let mut buffer = Vec::new();
    list_patches_with_writer(
        client,
        id,
        query,
        include_deleted,
        output_format,
        &mut buffer,
    )
    .await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn list_patches_with_writer(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
    include_deleted: bool,
    output_format: ResolvedOutputFormat,
    writer: &mut impl Write,
) -> Result<()> {
    if let Some(id) = id {
        if query.is_some() {
            bail!("--id and --query cannot be combined");
        }

        let patch = client
            .get_patch(&id)
            .await
            .with_context(|| format!("failed to fetch patch '{id}'"))?;
        render_patch_records(output_format, &[patch], writer)?;
        return Ok(());
    }

    let patches = fetch_patches(client, query, include_deleted).await?;

    render_patch_summary_records(output_format, &patches, writer)?;

    Ok(())
}

async fn get_patch_by_version(
    client: &dyn MetisClientInterface,
    patch_id: &PatchId,
    version: Option<i64>,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let patch = match version {
        Some(0) => {
            bail!("--version 0 is not valid; use a positive version number or a negative offset")
        }
        Some(v) => client
            .get_patch_version(patch_id, RelativeVersionNumber::new(v))
            .await
            .with_context(|| format!("failed to fetch version {v} of patch '{patch_id}'"))?,
        None => client
            .get_patch(patch_id)
            .await
            .with_context(|| format!("failed to fetch patch '{patch_id}'"))?,
    };
    render_patch_records(output_format, &[patch], &mut std::io::stdout())?;
    Ok(())
}

async fn fetch_patches(
    client: &dyn MetisClientInterface,
    query: Option<String>,
    include_deleted: bool,
) -> Result<Vec<PatchSummaryRecord>> {
    let include_deleted_opt = if include_deleted { Some(true) } else { None };
    let response = client
        .list_patches(&SearchPatchesQuery::new(
            query,
            include_deleted_opt,
            vec![],
            None,
        ))
        .await
        .context("failed to search for patches")?;
    Ok(response.patches)
}

async fn create_patch(
    client: &dyn MetisClientInterface,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    issue_id: IssueId,
    allow_uncommitted: bool,
    force: bool,
    base_ref: &str,
    repo_root: Option<&Path>,
) -> Result<PatchVersionRecord> {
    let repo_root = match repo_root {
        Some(path) => path.to_path_buf(),
        None => git_repository_root()?,
    };

    if !allow_uncommitted && git_has_uncommitted_changes(&repo_root)? {
        bail!("Working directory has uncommitted changes. Commit them before creating a patch or re-run with --allow-uncommitted.");
    }

    // Derive commit range from merge-base with the provided base ref.
    let (base_oid, head_oid) = git_resolve_commit_range_from_merge_base(&repo_root, base_ref)?;
    let commit_range = format!("{base_oid}..{head_oid}");
    let diff = git_diff_commit_range(&repo_root, &commit_range)?;
    if diff.trim().is_empty() {
        bail!("No changes found in commit range '{commit_range}'.");
    }

    let service_repo_name = resolve_service_repo_name(client, job_id.as_ref()).await?;
    let service_repo_name = service_repo_name.ok_or_else(|| {
        let job_ref = job_id
            .as_ref()
            .map(|id| id.as_ref().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        anyhow!("job '{job_ref}' does not reference a service repository")
    })?;

    let is_automatic_backup = false;
    let response = create_patch_artifact_from_repo(
        client,
        &repo_root,
        diff,
        title,
        description,
        job_id.clone(),
        is_automatic_backup,
        force,
        service_repo_name,
        base_ref,
    )
    .await?;

    let patch = client
        .get_patch(&response.patch_id)
        .await
        .with_context(|| format!("failed to fetch patch '{}'", response.patch_id))?;

    // Link the newly created patch to the issue by appending its ID to the
    // issue's patches field. If the update fails, log a warning but do not
    // fail the command — the patch was already created successfully.
    match client.get_issue(&issue_id, false).await {
        Ok(issue_record) => {
            let mut updated_issue = issue_record.issue;
            if !updated_issue.patches.contains(&patch.patch_id) {
                updated_issue.patches.push(patch.patch_id.clone());
            }
            let request = UpsertIssueRequest::new(updated_issue, None);
            if let Err(e) = client.update_issue(&issue_id, &request).await {
                eprintln!("Warning: failed to update issue '{issue_id}' with patch id: {e}");
            }
        }
        Err(e) => {
            eprintln!("Warning: failed to fetch issue '{issue_id}' for patch linking: {e}");
        }
    }

    Ok(patch)
}

fn write_patch_output(
    output_format: ResolvedOutputFormat,
    patch: &PatchVersionRecord,
) -> Result<()> {
    let mut buffer = Vec::new();
    render_patch_records(output_format, std::slice::from_ref(patch), &mut buffer)?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn update_patch(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    title: Option<String>,
    description: Option<String>,
    status: Option<PatchStatus>,
    force: bool,
    base_ref: &str,
) -> Result<PatchVersionRecord> {
    update_patch_inner(
        client,
        patch_id,
        title,
        description,
        status,
        force,
        base_ref,
        None,
    )
    .await
}

async fn update_patch_inner(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    title: Option<String>,
    description: Option<String>,
    status: Option<PatchStatus>,
    force: bool,
    base_ref: &str,
    repo_root: Option<&Path>,
) -> Result<PatchVersionRecord> {
    let description = match description {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Patch description must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let title = match title {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("Patch title must not be empty.");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let has_field_updates = title.is_some() || description.is_some() || status.is_some();
    let in_git_repo = match repo_root {
        Some(path) => git::repository_root(Some(path)).ok(),
        None => git::repository_root(None).ok(),
    };

    if !has_field_updates && in_git_repo.is_none() {
        bail!("At least one field must be provided to update when not inside a git repository.");
    }

    let current = client
        .get_patch(&patch_id)
        .await
        .with_context(|| format!("failed to fetch patch '{patch_id}'"))?;

    let mut updated_patch = current.patch;
    if let Some(title) = title {
        updated_patch.title = title;
    }
    if let Some(description) = description {
        updated_patch.description = description;
    }
    if let Some(status) = status {
        updated_patch.status = status;
    }

    // Always re-read git state when inside a git repo: diff, branch name,
    // base branch, commit range SHAs, and push the branch.
    if let Some(repo_root) = in_git_repo {
        let branch_name = current_branch(&repo_root)?;
        updated_patch.branch_name = Some(branch_name.clone());
        updated_patch.base_branch = Some(
            base_ref
                .strip_prefix("origin/")
                .unwrap_or(base_ref)
                .to_string(),
        );

        // Derive commit range from the merge-base of HEAD with the provided base ref.
        let (base_oid, head_oid) = git_resolve_commit_range_from_merge_base(&repo_root, base_ref)?;
        let range_str = format!("{base_oid}..{head_oid}");
        let diff = git_diff_commit_range(&repo_root, &range_str)?;
        if !diff.trim().is_empty() {
            updated_patch.diff = diff;
        }
        updated_patch.commit_range =
            Some(metis_common::patches::CommitRange::new(base_oid, head_oid));

        // Try to get a GitHub token for pushing; fall back to pushing without one.
        let github_token = client.get_github_token().await.ok();
        push_branch(&repo_root, &branch_name, github_token.as_deref(), force)?;
    }

    let response = client
        .update_patch(&patch_id, &UpsertPatchRequest::new(updated_patch.clone()))
        .await
        .with_context(|| format!("failed to update patch '{patch_id}'"))?;

    Ok(PatchVersionRecord::new(
        response.patch_id,
        response.version,
        Utc::now(),
        updated_patch,
        None,
        Utc::now(),
    ))
}

async fn merge_patch(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    base_override: Option<String>,
) -> Result<()> {
    // 1. Fetch the patch and its version history.
    let patch_record = client
        .get_patch(&patch_id)
        .await
        .with_context(|| format!("failed to fetch patch '{patch_id}'"))?;
    let patch = &patch_record.patch;

    // 2. Validate review status.
    let versions_response = client
        .list_patch_versions(&patch_id)
        .await
        .with_context(|| format!("failed to fetch version history for patch '{patch_id}'"))?;
    let staleness_cutoff = find_last_commit_range_change_timestamp(&versions_response.versions);
    if !has_approved_non_dismissed_review(&patch.reviews, staleness_cutoff) {
        bail!(
            "Error: patch {patch_id} cannot be merged because it does not have an approved review.\n\n\
             The patch is pending code review. To proceed, end your session now. \
             A reviewer agent will provide a review, and the merge can be retried afterward."
        );
    }

    // 3. If the patch is linked to a GitHub PR, merge via the GitHub API.
    if let Some(github_pr) = &patch.github {
        let github_token = client
            .get_github_token()
            .await
            .context("GitHub token required to merge via GitHub API")?;
        let octocrab_client = metis_common::github::build_octocrab_client(&github_token)?;

        const MAX_API_MERGE_ATTEMPTS: u32 = 3;
        let mut last_error = None;

        for attempt in 1..=MAX_API_MERGE_ATTEMPTS {
            let merge_result = octocrab_client
                .pulls(&github_pr.owner, &github_pr.repo)
                .merge(github_pr.number)
                .method(octocrab::params::pulls::MergeMethod::Squash)
                .title(format!("{} ({})", patch.title, patch_id))
                .message(patch.description.clone())
                .send()
                .await;

            match merge_result {
                Ok(_) => {
                    let mut merged_patch = patch.clone();
                    merged_patch.status = PatchStatus::Merged;
                    client
                        .update_patch(&patch_id, &UpsertPatchRequest::new(merged_patch))
                        .await
                        .with_context(|| {
                            format!("failed to update patch '{patch_id}' status to Merged")
                        })?;
                    println!("Patch '{patch_id}' merged successfully via GitHub API.");
                    return Ok(());
                }
                Err(e) => {
                    let is_transient = match &e {
                        octocrab::Error::GitHub { source, .. } => {
                            source.status_code.is_server_error()
                        }
                        octocrab::Error::Http { .. }
                        | octocrab::Error::Service { .. }
                        | octocrab::Error::Hyper { .. } => true,
                        _ => false,
                    };

                    if !is_transient || attempt == MAX_API_MERGE_ATTEMPTS {
                        last_error = Some(e);
                        break;
                    }

                    eprintln!(
                        "GitHub API merge attempt {attempt}/{MAX_API_MERGE_ATTEMPTS} failed \
                         (transient error), retrying..."
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }

        if let Some(e) = last_error {
            bail!(
                "Error: GitHub API merge failed for patch {patch_id} \
                 (PR #{pr_number} in {owner}/{repo}).\n\n\
                 This could be due to merge conflicts with the target branch. To resolve:\n\
                 1. Check the PR on GitHub: https://github.com/{owner}/{repo}/pull/{pr_number}\n\
                 2. Rebase your changes onto the target branch if needed\n\
                 3. Try merging again: metis patches merge {patch_id}\n\n\
                 Underlying error: {e}",
                pr_number = github_pr.number,
                owner = github_pr.owner,
                repo = github_pr.repo,
            );
        }
    }

    // 4. Resolve the branch name from the patch.
    let branch_name = patch
        .branch_name
        .as_deref()
        .ok_or_else(|| anyhow!("patch '{patch_id}' does not have a branch name set"))?;

    // 5. Resolve the base branch.
    let base_branch = match &base_override {
        Some(b) => b.clone(),
        None => {
            // Try to get the default branch from the repository config.
            let repo_name = &patch.service_repo_name;
            let repos_response = client
                .list_repositories(&SearchRepositoriesQuery::new(None))
                .await
                .context("failed to list repositories")?;
            let repo_config = repos_response
                .repositories
                .iter()
                .find(|r| r.name == *repo_name);
            match repo_config {
                Some(r) => r
                    .repository
                    .default_branch
                    .clone()
                    .unwrap_or_else(|| "main".to_string()),
                None => "main".to_string(),
            }
        }
    };

    // 6. Ensure we are in a git repo and check working tree state.
    let repo_root = git_repository_root()?;

    if git_has_uncommitted_changes(&repo_root)? {
        bail!(
            "Error: the working tree has uncommitted changes.\n\n\
             Please commit or stash your changes before running 'metis patches merge', then retry."
        );
    }

    // Fetch from origin to ensure we have the latest refs.
    let github_token = client.get_github_token().await.ok();
    git_fetch_remote(&repo_root, github_token.as_deref())?;

    // 7. Squash merge and push with retry loop to handle concurrent pushes.
    //
    // The squash merge creates a single commit containing all patch changes
    // on a temporary local branch. This branch is then pushed to
    // origin/{base_branch}. If a concurrent push advances the remote,
    // the push fails with NotFastForward and we retry by re-fetching and
    // re-creating the squash merge on top of the updated base.
    //
    // The squash_merge_onto function does not modify HEAD or the working
    // directory — it only creates/updates a local branch ref.
    const MAX_MERGE_ATTEMPTS: u32 = 3;
    let onto_ref = format!("origin/{base_branch}");
    let source_ref = format!("origin/{branch_name}");
    let merge_branch = format!("metis-squash-merge/{}", patch_id.as_ref());
    let commit_message = format!("{} ({})\n\n{}", patch.title, patch_id, patch.description);
    let mut push_succeeded = false;

    for attempt in 1..=MAX_MERGE_ATTEMPTS {
        if attempt > 1 {
            // Re-fetch to pick up any new commits pushed to origin.
            git_fetch_remote(&repo_root, github_token.as_deref())?;
        }

        // Squash-merge the patch branch onto origin/<base>.
        if let Err(err) = git_squash_merge_onto(
            &repo_root,
            &onto_ref,
            &source_ref,
            &merge_branch,
            &commit_message,
        ) {
            bail!(
                "Error: failed to squash merge patch {patch_id} onto {base_branch}.\n\n\
                 The patch branch \"{branch_name}\" has conflicts with \"{base_branch}\". To resolve:\n\
                 1. Rebase your changes onto {base_branch} locally: git rebase origin/{base_branch}\n\
                 2. Resolve any conflicts\n\
                 3. Update the patch: metis patches update {patch_id}\n\
                 4. Try merging again: metis patches merge {patch_id}\n\n\
                 Underlying error: {err}"
            );
        }

        // Push the squash-merged branch to the base branch on origin.
        match push_to_ref(
            &repo_root,
            &merge_branch,
            &base_branch,
            github_token.as_deref(),
            false,
        ) {
            Ok(()) => {
                push_succeeded = true;
                break;
            }
            Err(PushError::NotFastForward { .. }) => {
                if attempt < MAX_MERGE_ATTEMPTS {
                    eprintln!(
                        "Push to origin/{base_branch} failed (not a fast-forward), \
                         retrying ({attempt}/{MAX_MERGE_ATTEMPTS})..."
                    );
                    continue;
                }
                // All retries exhausted — fall through to error below.
            }
            Err(err) => {
                // Non-retriable push error.
                bail!(
                    "Error: failed to push squash merge to origin/{base_branch}.\n\n\
                     Underlying error: {err}"
                );
            }
        }
    }

    if !push_succeeded {
        bail!(
            "Error: failed to merge patch {patch_id} to {base_branch} after {MAX_MERGE_ATTEMPTS} attempts.\n\n\
             The base branch was updated by concurrent pushes between each merge and push attempt.\n\
             Please retry: metis patches merge {patch_id}"
        );
    }

    // Clean up the temporary squash-merge branch (best-effort).
    if let Err(err) = git_delete_local_branch(&repo_root, &merge_branch) {
        eprintln!("Warning: failed to clean up temporary branch '{merge_branch}': {err}");
    }

    // 9. Update the patch status to Merged.
    let mut merged_patch = patch.clone();
    merged_patch.status = PatchStatus::Merged;
    client
        .update_patch(&patch_id, &UpsertPatchRequest::new(merged_patch))
        .await
        .with_context(|| format!("failed to update patch '{patch_id}' status to Merged"))?;

    println!("Patch '{patch_id}' merged successfully onto '{base_branch}'.");
    Ok(())
}

pub async fn resolve_service_repo_name(
    client: &dyn MetisClientInterface,
    job_id: Option<&TaskId>,
) -> Result<Option<RepoName>> {
    let job_id = job_id.ok_or_else(|| {
        anyhow!("service repo name must be resolved from a job; provide --job or set METIS_ID")
    })?;
    let job = client
        .get_job(job_id)
        .await
        .with_context(|| format!("failed to fetch job '{job_id}' to resolve service repo"))?;

    if let BundleSpec::ServiceRepository { name, .. } = job.task.context {
        return Ok(Some(name));
    }

    Ok(None)
}

pub async fn create_patch_artifact_from_repo(
    client: &dyn MetisClientInterface,
    repo_root: &Path,
    diff: String,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    is_automatic_backup: bool,
    force: bool,
    service_repo_name: RepoName,
    base_ref: &str,
) -> Result<UpsertPatchResponse> {
    let title = title.trim().to_string();
    let description = description.trim().to_string();
    if title.is_empty() {
        bail!("Patch title must not be empty.");
    }
    if description.is_empty() {
        bail!("Patch description must not be empty.");
    }
    if diff.trim().is_empty() {
        bail!("Patch diff must not be empty.");
    }

    let creator = resolve_creator_username(client).await?;

    // Resolve branch name, base branch, and commit range SHAs.
    let branch_name = current_branch(repo_root)?;
    let commit_range = git_resolve_commit_range_from_merge_base(repo_root, base_ref)
        .ok()
        .map(|(base_oid, head_oid)| metis_common::patches::CommitRange::new(base_oid, head_oid));

    let patch_payload = Patch::new(
        title.clone(),
        description.clone(),
        diff,
        PatchStatus::Open,
        is_automatic_backup,
        job_id.clone(),
        creator,
        Vec::new(),
        service_repo_name.clone(),
        None,
        false,
        Some(branch_name.clone()),
        commit_range,
        Some(
            base_ref
                .strip_prefix("origin/")
                .unwrap_or(base_ref)
                .to_string(),
        ),
    );

    let github_token = client.get_github_token().await.ok();
    push_branch(repo_root, &branch_name, github_token.as_deref(), force)?;

    let upsert_request = UpsertPatchRequest::new(patch_payload.clone());

    let response = client
        .create_patch(&upsert_request)
        .await
        .context("failed to create patch")?;

    Ok(response)
}

async fn resolve_creator_username(client: &dyn MetisClientInterface) -> Result<Username> {
    let response = client
        .whoami()
        .await
        .context("failed to resolve authenticated actor")?;
    match response.actor {
        ActorIdentity::User { username } => Ok(username),
        ActorIdentity::Task { creator, .. } | ActorIdentity::Issue { creator, .. } => Ok(creator),
        other => bail!("unexpected actor identity: {other:?}"),
    }
}

fn git_repository_root() -> Result<PathBuf> {
    git::repository_root(None)
}

async fn apply_patch_record(client: &dyn MetisClientInterface, id: PatchId) -> Result<()> {
    let patch_record = client
        .get_patch(&id)
        .await
        .with_context(|| format!("failed to fetch patch '{id}'"))?;
    let repo_root = git_repository_root()?;

    apply_patch_to_repo(&patch_record.patch, &repo_root)?;
    Ok(())
}

fn apply_patch_to_repo(patch: &Patch, git_root: &Path) -> Result<()> {
    println!(
        "Applying patch '{}' to current git repository...\n",
        patch.title
    );

    apply_patch(git_root, &patch.diff).context("failed to apply patch to repository")?;

    println!("Patch applied successfully.");
    Ok(())
}

async fn review_patch(
    client: &dyn MetisClientInterface,
    id: PatchId,
    author: String,
    contents: String,
    approve: bool,
    request_changes: bool,
) -> Result<()> {
    let author = author.trim().to_string();
    if author.is_empty() {
        bail!("Author must not be empty.");
    }
    let contents = contents.trim().to_string();
    if contents.is_empty() {
        bail!("Review contents must not be empty.");
    }

    let mut record = client
        .get_patch(&id)
        .await
        .with_context(|| format!("failed to fetch patch '{id}'"))?;

    record
        .patch
        .reviews
        .push(Review::new(contents, approve, author, Some(Utc::now())));

    if request_changes {
        record.patch.status = PatchStatus::ChangesRequested;
    }

    let response = client
        .update_patch(&id, &UpsertPatchRequest::new(record.patch))
        .await
        .with_context(|| format!("failed to update patch '{id}' with review"))?;

    println!("{}", response.patch_id);
    Ok(())
}

async fn changelog_patch(
    client: &dyn MetisClientInterface,
    id: PatchId,
    output_format: ResolvedOutputFormat,
    limit: usize,
) -> Result<()> {
    let response = client
        .list_patch_versions(&id)
        .await
        .with_context(|| format!("failed to fetch versions for patch '{id}'"))?;
    let versions: Vec<Versioned<Patch>> = response
        .versions
        .into_iter()
        .map(|record| {
            Versioned::new(
                record.patch,
                record.version,
                record.timestamp,
                record.creation_time,
            )
        })
        .collect();
    let entries = activity_log_for_patch_versions(id, &versions);
    let mut summaries = summarize_activity_log(&entries)?;
    summaries.reverse();
    summaries.truncate(limit);

    let mut buffer = Vec::new();
    match output_format {
        ResolvedOutputFormat::Pretty => {
            write_changelog_pretty(&summaries, &mut buffer)?;
        }
        ResolvedOutputFormat::Jsonl => {
            for entry in &summaries {
                serde_json::to_writer(&mut buffer, entry)?;
                buffer.write_all(b"\n")?;
            }
        }
    }
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use crate::command::output::ResolvedOutputFormat;
    use crate::git::{
        commit_changes as git_commit_changes, configure_repo as git_configure_repo,
        resolve_head_oid as git_resolve_head_oid, stage_all_changes as git_stage_all_changes,
    };
    use crate::test_utils::ids::{issue_id, patch_id, task_id};
    use anyhow::{anyhow, Context};
    use git2::Repository;
    use httpmock::{prelude::*, Mock};
    use metis_common::{
        issues::{Issue, IssueStatus, IssueType, IssueVersionRecord, UpsertIssueResponse},
        jobs::{BundleSpec, JobVersionRecord, Task},
        patches::{
            CommitRange, CreatePatchAssetResponse, GitOid, ListPatchVersionsResponse,
            ListPatchesResponse, Patch, PatchVersionRecord, Review, UpsertPatchResponse,
        },
        task_status::Status,
        users::Username,
        whoami::{ActorIdentity, WhoAmIResponse},
        RepoName,
    };
    use reqwest::Client as HttpClient;
    use std::{fs, str::FromStr};

    const TEST_METIS_TOKEN: &str = "u-test-user:test-metis-token";

    fn sample_diff() -> String {
        "--- a/file.txt\n+++ b/file.txt\n@@\n-old\n+new\n".to_string()
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn metis_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
            .expect("failed to create metis client")
    }

    fn mock_get_job(server: &MockServer, job: JobVersionRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/jobs/{}", job.job_id.as_ref()));
            then.status(200).json_body_obj(&job);
        })
    }

    fn mock_create_patch(
        server: &MockServer,
        expected_request: UpsertPatchRequest,
        response: UpsertPatchResponse,
    ) -> Mock {
        server.mock(move |when, then| {
            when.method(POST)
                .path("/v1/patches")
                .json_body_obj(&expected_request);
            then.status(200).json_body_obj(&response);
        })
    }

    fn mock_get_patch(server: &MockServer, patch: PatchVersionRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{}", patch.patch_id.as_ref()));
            then.status(200).json_body_obj(&patch);
        })
    }

    fn mock_get_github_token_failure(server: &MockServer) -> Mock {
        server.mock(move |when, then| {
            when.method(GET).path("/v1/github/token");
            then.status(401);
        })
    }

    fn mock_whoami(server: &MockServer) -> Mock {
        let response = WhoAmIResponse::new(ActorIdentity::User {
            username: Username::from("test-user"),
        });
        server.mock(move |when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&response);
        })
    }

    fn mock_update_patch(
        server: &MockServer,
        patch_id: PatchId,
        expected_request: UpsertPatchRequest,
        response: UpsertPatchResponse,
    ) -> Mock {
        server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/patches/{}", patch_id.as_ref()))
                .json_body_obj(&expected_request);
            then.status(200).json_body_obj(&response);
        })
    }

    fn mock_get_issue(server: &MockServer, issue: IssueVersionRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{}", issue.issue_id.as_ref()));
            then.status(200).json_body_obj(&issue);
        })
    }

    fn mock_update_issue(
        server: &MockServer,
        issue_id: IssueId,
        response: UpsertIssueResponse,
    ) -> Mock {
        server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{}", issue_id.as_ref()));
            then.status(200).json_body_obj(&response);
        })
    }

    fn sample_issue_record(issue_id: &IssueId, patches: Vec<PatchId>) -> IssueVersionRecord {
        IssueVersionRecord::new(
            issue_id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test Title".to_string(),
                "test issue".to_string(),
                Username::from("test-creator"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                patches,
                false,
            ),
            None,
            Utc::now(),
            Vec::new(),
        )
    }

    fn initialize_repo_with_changes(
    ) -> Result<(tempfile::TempDir, std::path::PathBuf, GitOid, GitOid)> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test repo")?;
        let repo_path = tempdir.path().join("repo");
        let bare_path = tempdir.path().join("remote.git");
        fs::create_dir_all(&repo_path)?;

        // Create a bare remote so push_branch succeeds in tests.
        Repository::init_bare(&bare_path).context("failed to init bare remote for test")?;
        let repo = Repository::init(&repo_path).context("failed to init git repo for test")?;
        // Ensure the default branch is "main" regardless of system git config.
        repo.set_head("refs/heads/main")
            .context("failed to set HEAD to main")?;
        git_configure_repo(&repo_path, "Test User", "test@example.com")?;
        let remote_url = bare_path
            .to_str()
            .context("bare path not utf-8")?
            .to_string();
        repo.remote("origin", &remote_url)
            .context("failed to set remote origin")?;

        fs::write(repo_path.join("README.md"), "initial content\n")
            .context("failed to write initial README.md")?;
        git_stage_all_changes(&repo_path)?;
        git_commit_changes(&repo_path, "initial commit")?;
        let base_commit = git_resolve_head_oid(&repo_path)?
            .ok_or_else(|| anyhow!("failed to resolve initial commit"))?;

        // Push the initial commit as origin/main so merge-base resolution works.
        push_branch(&repo_path, "main", None, false)?;

        fs::write(repo_path.join("README.md"), "updated content\n")
            .context("failed to update README.md")?;
        fs::write(repo_path.join("notes.txt"), "new note content\n")
            .context("failed to write notes.txt")?;
        git_stage_all_changes(&repo_path)?;
        git_commit_changes(&repo_path, "second commit")?;
        let head_commit = git_resolve_head_oid(&repo_path)?
            .ok_or_else(|| anyhow!("failed to resolve head commit"))?;

        Ok((tempdir, repo_path, base_commit, head_commit))
    }

    #[tokio::test]
    async fn list_patches_sets_patch_filter_and_query() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/patches")
                .query_param("q", "login");
            then.status(200)
                .json_body_obj(&ListPatchesResponse::new(Vec::new()));
        });

        list_patches(
            &client,
            None,
            Some("login".to_string()),
            false,
            ResolvedOutputFormat::Jsonl,
        )
        .await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn list_patches_emits_no_output_for_empty_results() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/patches");
            then.status(200)
                .json_body_obj(&ListPatchesResponse::new(Vec::new()));
        });

        let mut output = Vec::new();
        list_patches_with_writer(
            &client,
            None,
            None,
            false,
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .await?;

        assert!(output.is_empty());
        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_asset_writes_pretty_output() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let patch_id = patch_id("p-asset-output");
        let asset_url = "https://github.com/dourolabs/metis/assets/123";
        let path = format!("/v1/patches/{}/assets", patch_id.as_ref());

        let mock = server.mock(|when, then| {
            when.method(POST)
                .path(path.as_str())
                .query_param("name", "asset.txt")
                .body("asset-bytes");
            then.status(200)
                .json_body_obj(&CreatePatchAssetResponse::new(asset_url.to_string()));
        });

        let tempdir = tempfile::tempdir()?;
        let file_path = tempdir.path().join("asset.txt");
        fs::write(&file_path, "asset-bytes")?;

        let mut output = Vec::new();
        create_patch_asset_with_writer(
            &client,
            &patch_id,
            &file_path,
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .await?;

        mock.assert();
        assert_eq!(
            String::from_utf8(output)?,
            format!("Uploaded asset for patch {patch_id}: {asset_url}\n")
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_generates_diff_from_repo_changes() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-diff");
        let issue_id = issue_id("i-diff");
        let branch_name = current_branch(&repo_path)?;
        let job_record = JobVersionRecord::new(
            job_id.clone(),
            0,
            Utc::now(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                Username::from("test-creator"),
                None,
                None,
                Default::default(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        let job_id_clone = job_id.clone();
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            patch_title.clone(),
            patch_description.clone(),
            expected_diff.clone(),
            PatchStatus::Open,
            false,
            Some(job_id_clone.clone()),
            Username::from("test-user"),
            Vec::new(),
            sample_repo_name(),
            None,
            false,
            Some(branch_name.to_string()),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-1"), 0);
        let patch_record =
            PatchVersionRecord::new(patch_id("p-1"), 0, Utc::now(), patch, None, Utc::now());
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        let issue_record = sample_issue_record(&issue_id, Vec::new());
        let issue_update_response = UpsertIssueResponse::new(issue_id.clone(), 1);
        let get_issue_mock = mock_get_issue(&server, issue_record);
        let update_issue_mock = mock_update_issue(&server, issue_id.clone(), issue_update_response);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            Some(job_id),
            issue_id.clone(),
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();
        get_issue_mock.assert();
        update_issue_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_sets_created_by_from_job_id() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;

        let job_id = task_id("t-job-1234");
        let job_record = JobVersionRecord::new(
            job_id.clone(),
            0,
            Utc::now(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                Username::from("test-creator"),
                None,
                None,
                Default::default(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        );

        let title = "patch with job title".to_string();
        let job_id_opt = Some(job_id.clone());
        let description = "patch with job id".to_string();
        let issue_id = issue_id("i-job-1234");
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            title.clone(),
            description.clone(),
            expected_diff,
            PatchStatus::Open,
            false,
            job_id_opt.clone(),
            Username::from("test-user"),
            Vec::new(),
            sample_repo_name(),
            None,
            false,
            Some(branch_name.to_string()),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-2"), 0);
        let patch_record =
            PatchVersionRecord::new(patch_id("p-2"), 0, Utc::now(), patch, None, Utc::now());
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        let issue_record = sample_issue_record(&issue_id, Vec::new());
        let issue_update_response = UpsertIssueResponse::new(issue_id.clone(), 1);
        let get_issue_mock = mock_get_issue(&server, issue_record);
        let update_issue_mock = mock_update_issue(&server, issue_id.clone(), issue_update_response);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id_opt.clone(),
            issue_id.clone(),
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();
        get_issue_mock.assert();
        update_issue_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_errors_without_job_id() -> Result<()> {
        let (_tempdir, repo_path, _base_commit, _head_commit) = initialize_repo_with_changes()?;
        let server = MockServer::start();
        let client = metis_client(&server);
        let issue_id = issue_id("i-missing-job");
        let result = create_patch(
            &client,
            "missing job".to_string(),
            "patch without job id".to_string(),
            None,
            issue_id,
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("provide --job or set METIS_ID"),
            "error should mention missing job id: {error}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_artifact_marks_automatic_backup_when_requested() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let diff = git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let job_id = task_id("t-job-automatic");
        let expected_patch = Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            diff.clone(),
            PatchStatus::Open,
            true,
            Some(job_id.clone()),
            Username::from("test-user"),
            Vec::new(),
            sample_repo_name(),
            None,
            false,
            Some(branch_name),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let expected_request = UpsertPatchRequest::new(expected_patch);
        let patch_response = UpsertPatchResponse::new(patch_id("p-automatic"), 0);
        let server = MockServer::start();
        let client = metis_client(&server);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        mock_get_github_token_failure(&server);
        mock_whoami(&server);
        let _ = create_patch_artifact_from_repo(
            &client,
            &repo_path,
            diff.clone(),
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(job_id.clone()),
            true,
            false,
            sample_repo_name(),
            "origin/main",
        )
        .await?;

        patch_mock.assert();
        assert_eq!(patch_mock.hits(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_uses_service_repo_name_from_job() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let job_id = task_id("t-job-service");
        let job_record = JobVersionRecord::new(
            job_id.clone(),
            0,
            Utc::now(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: RepoName::from_str("dourolabs/api")?,
                    rev: None,
                },
                None,
                Username::from("test-creator"),
                None,
                None,
                Default::default(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let issue_id = issue_id("i-service");
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Username::from("test-user"),
            Vec::new(),
            RepoName::from_str("dourolabs/api")?,
            None,
            false,
            Some(branch_name.to_string()),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-service"), 0);
        let patch_record = PatchVersionRecord::new(
            patch_id("p-service"),
            0,
            Utc::now(),
            patch,
            None,
            Utc::now(),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        let issue_record = sample_issue_record(&issue_id, Vec::new());
        let issue_update_response = UpsertIssueResponse::new(issue_id.clone(), 1);
        let get_issue_mock = mock_get_issue(&server, issue_record);
        let update_issue_mock = mock_update_issue(&server, issue_id.clone(), issue_update_response);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);

        create_patch(
            &client,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(job_id.clone()),
            issue_id.clone(),
            false,
            false,
            "origin/main",
            Some(repo_path.as_path()),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();
        get_issue_mock.assert();
        update_issue_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_links_patch_to_issue() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let job_id = task_id("t-job-link");
        let issue_id = issue_id("i-link");
        let job_record = JobVersionRecord::new(
            job_id.clone(),
            0,
            Utc::now(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                Username::from("test-creator"),
                None,
                None,
                Default::default(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            "link test".to_string(),
            "link description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Username::from("test-user"),
            Vec::new(),
            sample_repo_name(),
            None,
            false,
            Some(branch_name.to_string()),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let new_patch_id = patch_id("p-link");
        let patch_response = UpsertPatchResponse::new(new_patch_id.clone(), 0);
        let patch_record =
            PatchVersionRecord::new(new_patch_id.clone(), 0, Utc::now(), patch, None, Utc::now());
        let server = MockServer::start();
        let client = metis_client(&server);
        mock_get_job(&server, job_record);
        mock_create_patch(&server, expected_request, patch_response);
        mock_get_patch(&server, patch_record);
        let issue_record = sample_issue_record(&issue_id, Vec::new());
        let issue_update_response = UpsertIssueResponse::new(issue_id.clone(), 1);
        mock_get_issue(&server, issue_record);

        // Verify the update_issue call receives the patch id in the issue's patches field.
        let update_issue_mock = server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/v1/issues/{}", issue_id.as_ref()))
                .json_body_partial(r#"{"issue": {"patches": ["p-link"]}}"#);
            then.status(200).json_body_obj(&issue_update_response);
        });
        mock_get_github_token_failure(&server);
        mock_whoami(&server);

        create_patch(
            &client,
            "link test".to_string(),
            "link description".to_string(),
            Some(job_id),
            issue_id,
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await?;

        update_issue_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_service_repo_name_requires_job_id() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);

        let error = resolve_service_repo_name(&client, None).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("service repo name must be resolved from a job"),
            "error should explain that a job id is required"
        );
        Ok(())
    }

    #[tokio::test]
    async fn resolve_service_repo_name_returns_none_for_non_service_job() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_id = task_id("t-job-non-service");
        let job_record = JobVersionRecord::new(
            job_id.clone(),
            0,
            Utc::now(),
            Task::new(
                "0".to_string(),
                BundleSpec::GitRepository {
                    url: "https://github.com/dourolabs/example".to_string(),
                    rev: "main".to_string(),
                },
                None,
                Username::from("test-creator"),
                None,
                None,
                Default::default(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        );
        let job_mock = mock_get_job(&server, job_record.clone());

        let repo_name = resolve_service_repo_name(&client, Some(&job_id)).await?;
        assert!(
            repo_name.is_none(),
            "non-service jobs should not resolve to a service repository name"
        );
        job_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn review_patch_appends_review() -> Result<()> {
        let existing_submitted_at = Utc::now();
        let existing_review = Review::new(
            "needs work".to_string(),
            false,
            "bob".to_string(),
            Some(existing_submitted_at),
        );
        let review_patch_id = patch_id("p-review");
        let patch_record = PatchVersionRecord::new(
            review_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "reviewed patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![existing_review.clone()],
                sample_repo_name(),
                None,
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let get_mock = mock_get_patch(&server, patch_record.clone());
        let patch_id_for_mock = review_patch_id.clone();
        let update_mock = server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/patches/{}", patch_id_for_mock.as_ref()))
                .json_body_partial(
                    r#"{
                        "patch": {
                            "title": "reviewed patch",
                            "description": "description",
                            "reviews": [
                                {"contents": "needs work", "is_approved": false, "author": "bob"},
                                {"contents": "looks good now", "is_approved": true, "author": "alice"}
                            ]
                        }
                    }"#,
                )
                .body_contains("submitted_at");
            then.status(200)
                .json_body_obj(&UpsertPatchResponse::new(patch_id("p-123"), 0));
        });

        review_patch(
            &client,
            review_patch_id.clone(),
            "alice".to_string(),
            "looks good now".to_string(),
            true,
            false,
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn review_patch_request_changes_sets_status() -> Result<()> {
        let review_patch_id = patch_id("p-review-rc");
        let patch_record = PatchVersionRecord::new(
            review_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "patch needing changes".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![],
                sample_repo_name(),
                None,
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let get_mock = mock_get_patch(&server, patch_record.clone());
        let patch_id_for_mock = review_patch_id.clone();
        let update_mock = server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/patches/{}", patch_id_for_mock.as_ref()))
                .json_body_partial(
                    r#"{
                        "patch": {
                            "status": "ChangesRequested",
                            "reviews": [
                                {"contents": "needs work", "is_approved": false, "author": "alice"}
                            ]
                        }
                    }"#,
                );
            then.status(200)
                .json_body_obj(&UpsertPatchResponse::new(review_patch_id.clone(), 1));
        });

        review_patch(
            &client,
            patch_id("p-review-rc"),
            "alice".to_string(),
            "needs work".to_string(),
            false,
            true,
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn review_patch_approve_does_not_change_status() -> Result<()> {
        let review_patch_id = patch_id("p-review-approve");
        let patch_record = PatchVersionRecord::new(
            review_patch_id.clone(),
            0,
            Utc::now(),
            Patch::new(
                "patch to approve".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![],
                sample_repo_name(),
                None,
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let get_mock = mock_get_patch(&server, patch_record.clone());
        let patch_id_for_mock = review_patch_id.clone();
        let update_mock = server.mock(move |when, then| {
            when.method(PUT)
                .path(format!("/v1/patches/{}", patch_id_for_mock.as_ref()))
                .json_body_partial(
                    r#"{
                        "patch": {
                            "status": "Open",
                            "reviews": [
                                {"contents": "lgtm", "is_approved": true, "author": "alice"}
                            ]
                        }
                    }"#,
                );
            then.status(200)
                .json_body_obj(&UpsertPatchResponse::new(review_patch_id.clone(), 1));
        });

        review_patch(
            &client,
            patch_id("p-review-approve"),
            "alice".to_string(),
            "lgtm".to_string(),
            true,
            false,
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[test]
    fn review_approve_and_request_changes_conflict() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Cli {
            #[command(subcommand)]
            command: PatchesCommand,
        }

        let result = Cli::try_parse_from([
            "cli",
            "review",
            "p-conflict",
            "--author",
            "alice",
            "--contents",
            "msg",
            "--approve",
            "--request-changes",
        ]);
        assert!(
            result.is_err(),
            "expected clap conflict error when both --approve and --request-changes are provided"
        );
    }

    #[tokio::test]
    async fn update_patch_modifies_requested_fields() -> Result<()> {
        // Use a non-git temp dir to isolate from the real project repo.
        let tempdir = tempfile::tempdir()?;
        let patch_record = PatchVersionRecord::new(
            patch_id("p-update"),
            0,
            Utc::now(),
            Patch::new(
                "Initial title".to_string(),
                "Initial description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![Review::new(
                    "looks ok".to_string(),
                    false,
                    "sam".to_string(),
                    None,
                )],
                sample_repo_name(),
                None,
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
        );
        let expected_request = UpsertPatchRequest::new(Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            sample_diff(),
            PatchStatus::Closed,
            false,
            None,
            Username::from("test-creator"),
            vec![Review::new(
                "looks ok".to_string(),
                false,
                "sam".to_string(),
                None,
            )],
            sample_repo_name(),
            None,
            false,
            None,
            None,
            None,
        ));
        let server = MockServer::start();
        let client = metis_client(&server);
        let get_mock = mock_get_patch(&server, patch_record.clone());
        let update_mock = mock_update_patch(
            &server,
            patch_id("p-update"),
            expected_request,
            UpsertPatchResponse::new(patch_id("p-update"), 0),
        );

        update_patch_inner(
            &client,
            patch_id("p-update"),
            Some("Updated title".to_string()),
            Some("Updated description".to_string()),
            Some(PatchStatus::Closed),
            false,
            "origin/main",
            Some(tempdir.path()),
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_rejects_empty_updates() {
        // Use a non-git temp dir to test the "no fields + no git repo" error path.
        let tempdir = tempfile::tempdir().unwrap();
        let server = MockServer::start();
        let client = metis_client(&server);
        let result = update_patch_inner(
            &client,
            patch_id("p-empty"),
            None,
            None,
            None,
            false,
            "origin/main",
            Some(tempdir.path()),
        )
        .await;

        assert!(result.is_err(), "expected update to reject empty payload");
    }

    #[tokio::test]
    async fn merge_patch_rejects_without_approved_review() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let merge_patch_id = patch_id("p-merge-no-review");
        let patch_record = PatchVersionRecord::new(
            merge_patch_id.clone(),
            1,
            Utc::now(),
            Patch::new(
                "test patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![],
                sample_repo_name(),
                None,
                false,
                Some("feature-branch".to_string()),
                None,
                Some("main".to_string()),
            ),
            None,
            Utc::now(),
        );
        let versions_response = ListPatchVersionsResponse::new(vec![patch_record.clone()]);

        let get_mock = mock_get_patch(&server, patch_record);
        let versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{}/versions", merge_patch_id.as_ref()));
            then.status(200).json_body_obj(&versions_response);
        });

        let result = merge_patch(&client, merge_patch_id, None).await;

        get_mock.assert();
        versions_mock.assert();
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("does not have an approved review"),
            "expected review error, got: {error}"
        );
        assert!(
            error.contains("end your session"),
            "expected agent-friendly instruction, got: {error}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn merge_patch_rejects_with_stale_review() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let merge_patch_id = patch_id("p-merge-stale");
        let now = Utc::now();

        // Version 1: original commit range with an approval
        let range_v1 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
        ));
        // Version 2: updated commit range (review is now stale)
        let range_v2 = Some(CommitRange::new(
            GitOid::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            GitOid::from_str("cccccccccccccccccccccccccccccccccccccccc").unwrap(),
        ));

        let approval = Review::new(
            "LGTM".to_string(),
            true,
            "reviewer".to_string(),
            Some(now - chrono::Duration::hours(2)),
        );

        let patch_v2 = Patch::new(
            "test patch".to_string(),
            "description".to_string(),
            sample_diff(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![approval],
            sample_repo_name(),
            None,
            false,
            Some("feature-branch".to_string()),
            range_v2.clone(),
            Some("main".to_string()),
        );

        let version1 = PatchVersionRecord::new(
            merge_patch_id.clone(),
            1,
            now - chrono::Duration::hours(3),
            Patch::new(
                "test patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                Username::from("test-creator"),
                vec![],
                sample_repo_name(),
                None,
                false,
                Some("feature-branch".to_string()),
                range_v1,
                Some("main".to_string()),
            ),
            None,
            now - chrono::Duration::hours(3),
        );
        let version2 = PatchVersionRecord::new(
            merge_patch_id.clone(),
            2,
            now - chrono::Duration::hours(1),
            patch_v2.clone(),
            None,
            now - chrono::Duration::hours(1),
        );

        let current_record = PatchVersionRecord::new(
            merge_patch_id.clone(),
            2,
            now - chrono::Duration::hours(1),
            patch_v2,
            None,
            now - chrono::Duration::hours(1),
        );

        let versions_response = ListPatchVersionsResponse::new(vec![version1, version2]);

        let get_mock = mock_get_patch(&server, current_record);
        let versions_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{}/versions", merge_patch_id.as_ref()));
            then.status(200).json_body_obj(&versions_response);
        });

        let result = merge_patch(&client, merge_patch_id, None).await;

        get_mock.assert();
        versions_mock.assert();
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("does not have an approved review"),
            "expected stale review error, got: {error}"
        );

        Ok(())
    }
}
