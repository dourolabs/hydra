use std::{io::Write, path::Path, path::PathBuf};

use crate::output_writer::write_stdout;

use super::utils::resolve_username;
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use hydra_common::{
    activity_log_for_patch_versions,
    api::v1::merge_check::{
        BlockedAtLayer, EligiblePrincipal, MergeBlockedError, MergeBlockedReason,
        MergeCheckResponse, SuggestedAction,
    },
    constants::ENV_HYDRA_ISSUE_ID,
    issues::IssueId,
    patches::{
        Patch, PatchStatus, PatchSummaryRecord, PatchVersionRecord, Review, SearchPatchesQuery,
        UpsertPatchRequest, UpsertPatchResponse,
    },
    repositories::SearchRepositoriesQuery,
    PatchId, RelativeVersionNumber, RepoName, Versioned,
};
use serde::Serialize;

use crate::git;
use crate::git::{
    apply_patch, current_branch, delete_local_branch as git_delete_local_branch,
    diff_commit_range as git_diff_commit_range, fetch_remote as git_fetch_remote,
    has_uncommitted_changes as git_has_uncommitted_changes, push_branch, push_to_ref,
    resolve_commit_range_from_merge_base as git_resolve_commit_range_from_merge_base,
    resolve_ref_oid as git_resolve_ref_oid, squash_merge_onto as git_squash_merge_onto, PushError,
};
use crate::{
    client::HydraClient,
    command::{
        output::{render, CommandContext, PatchRecords, PatchSummaryRecords, ResolvedOutputFormat},
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

        /// Filter patches by exact target repository name (e.g., dourolabs/hydra).
        #[arg(long = "repo-name", value_name = "REPO_NAME")]
        repo_name: Option<String>,

        /// Filter patches by creator username (case-insensitive).
        #[arg(long = "creator", value_name = "CREATOR")]
        creator: Option<String>,
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

        /// Associate the merge-request issue with an existing issue id.
        #[arg(
            long = "issue-id",
            value_name = "ISSUE_ID",
            env = ENV_HYDRA_ISSUE_ID
        )]
        issue_id: IssueId,

        /// Override the service repository for the patch (e.g., dourolabs/hydra). When omitted, the repo is discovered from the configured git remote.
        #[arg(long = "service-repo", value_name = "ORG/REPO")]
        service_repo: Option<RepoName>,

        /// Git remote to read the remote URL from when discovering the service repository (defaults to origin).
        #[arg(long = "remote", value_name = "NAME", default_value = "origin")]
        remote: String,

        /// Allow creating a patch even when the working directory has uncommitted changes.
        #[arg(long = "allow-uncommitted")]
        allow_uncommitted: bool,

        /// Force push the branch to the remote.
        #[arg(long = "force")]
        force: bool,

        /// Base ref for computing the commit range (defaults to origin/main).
        #[arg(long = "base-ref", value_name = "BASE_REF")]
        base_ref: Option<String>,
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

        /// Associate the patch update with an existing issue id.
        #[arg(
            long = "issue-id",
            value_name = "ISSUE_ID",
            env = ENV_HYDRA_ISSUE_ID
        )]
        issue_id: Option<IssueId>,

        /// Override the service repository for the patch (e.g., dourolabs/hydra). When omitted, the repo is discovered from the configured git remote.
        #[arg(long = "service-repo", value_name = "ORG/REPO")]
        service_repo: Option<RepoName>,

        /// Git remote to read the remote URL from when discovering the service repository (defaults to origin).
        #[arg(long = "remote", value_name = "NAME", default_value = "origin")]
        remote: String,

        /// Base ref for computing the commit range (defaults to origin/main).
        #[arg(long = "base-ref", value_name = "BASE_REF")]
        base_ref: Option<String>,
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
    client: &HydraClient,
    command: PatchesCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        PatchesCommand::List {
            id,
            query,
            include_deleted,
            repo_name,
            creator,
        } => {
            list_patches(
                client,
                ListPatchesArgs {
                    id,
                    query,
                    include_deleted,
                    repo_name,
                    creator,
                    output_format: context.output_format,
                },
            )
            .await
        }
        PatchesCommand::Get { id, version } => {
            get_patch_by_version(client, &id, version, context.output_format).await
        }
        PatchesCommand::Create {
            title,
            description,
            issue_id,
            service_repo,
            remote,
            allow_uncommitted,
            force,
            base_ref,
        } => {
            let base_ref = resolve_base_ref(client, base_ref, Some(&issue_id)).await?;
            let patch = create_patch(
                client,
                title,
                description,
                service_repo,
                &remote,
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
            issue_id,
            service_repo: _service_repo,
            remote: _remote,
            base_ref,
        } => {
            let base_ref = resolve_base_ref(client, base_ref, issue_id.as_ref()).await?;
            let patch =
                update_patch(client, id, title, description, status, force, &base_ref).await?;
            write_patch_output(context.output_format, &patch)?;
            Ok(())
        }
        PatchesCommand::Merge { patch_id, base } => {
            merge_patch(client, patch_id, base, context.output_format).await
        }
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
    client: &HydraClient,
    command: PatchAssetsCommand,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    match command {
        PatchAssetsCommand::Create(args) => create_patch_asset(client, args, output_format).await,
    }
}

async fn create_patch_asset(
    client: &HydraClient,
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
    write_stdout(&buffer)?;
    Ok(())
}

async fn create_patch_asset_with_writer(
    client: &HydraClient,
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

#[derive(Debug)]
struct ListPatchesArgs {
    id: Option<PatchId>,
    query: Option<String>,
    include_deleted: bool,
    repo_name: Option<String>,
    creator: Option<String>,
    output_format: ResolvedOutputFormat,
}

async fn list_patches(client: &HydraClient, args: ListPatchesArgs) -> Result<()> {
    let mut buffer = Vec::new();
    list_patches_with_writer(client, args, &mut buffer).await?;
    write_stdout(&buffer)?;
    Ok(())
}

async fn list_patches_with_writer(
    client: &HydraClient,
    args: ListPatchesArgs,
    writer: &mut impl Write,
) -> Result<()> {
    let ListPatchesArgs {
        id,
        query,
        include_deleted,
        repo_name,
        creator,
        output_format,
    } = args;

    if let Some(id) = id {
        if query.is_some() {
            bail!("--id and --query cannot be combined");
        }

        let patch = client
            .get_patch(&id)
            .await
            .with_context(|| format!("failed to fetch patch '{id}'"))?;
        render(PatchRecords(&[patch]), output_format, writer)?;
        return Ok(());
    }

    let patches = fetch_patches(client, query, include_deleted, repo_name, creator).await?;

    render(PatchSummaryRecords(&patches), output_format, writer)?;

    Ok(())
}

async fn get_patch_by_version(
    client: &HydraClient,
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
    let mut buffer = Vec::new();
    render(PatchRecords(&[patch]), output_format, &mut buffer)?;
    write_stdout(&buffer)?;
    Ok(())
}

async fn fetch_patches(
    client: &HydraClient,
    query: Option<String>,
    include_deleted: bool,
    repo_name: Option<String>,
    creator: Option<String>,
) -> Result<Vec<PatchSummaryRecord>> {
    let include_deleted_opt = if include_deleted { Some(true) } else { None };
    let mut search_query = SearchPatchesQuery::new(query, include_deleted_opt, vec![], None);
    search_query.repo_name = repo_name;
    search_query.creator = creator;
    let response = client
        .list_patches(&search_query)
        .await
        .context("failed to search for patches")?;
    Ok(response.patches)
}

/// Resolve the effective base ref for a patch.
///
/// When `base_ref` is explicitly provided by the caller, it is returned as-is.
/// Otherwise, if an `issue_id` is given, the issue's `session_settings.branch`
/// is used (prefixed with `origin/`). Falls back to `"origin/main"`.
async fn resolve_base_ref(
    client: &HydraClient,
    base_ref: Option<String>,
    issue_id: Option<&IssueId>,
) -> Result<String> {
    if let Some(base_ref) = base_ref {
        return Ok(base_ref);
    }

    if let Some(issue_id) = issue_id {
        match client.get_issue(issue_id, false).await {
            Ok(issue_record) => {
                if let Some(branch) = &issue_record.issue.session_settings.branch {
                    return Ok(format!("origin/{branch}"));
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to fetch issue '{issue_id}' for base-ref resolution: {e}"
                );
            }
        }
    }

    Ok("origin/main".to_string())
}

#[allow(clippy::too_many_arguments)]
async fn create_patch(
    client: &HydraClient,
    title: String,
    description: String,
    service_repo_override: Option<RepoName>,
    remote_name: &str,
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

    let service_repo_name = resolve_patch_service_repo(
        client,
        service_repo_override.as_ref(),
        &repo_root,
        remote_name,
    )
    .await?;

    let is_automatic_backup = false;
    let response = create_patch_artifact_from_repo(
        client,
        &repo_root,
        diff,
        title,
        description,
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

    Ok(patch)
}

fn write_patch_output(
    output_format: ResolvedOutputFormat,
    patch: &PatchVersionRecord,
) -> Result<()> {
    let mut buffer = Vec::new();
    render(
        PatchRecords(std::slice::from_ref(patch)),
        output_format,
        &mut buffer,
    )?;
    write_stdout(&buffer)?;
    Ok(())
}

async fn update_patch(
    client: &HydraClient,
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
    client: &HydraClient,
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
            Some(hydra_common::patches::CommitRange::new(base_oid, head_oid));

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
        Vec::new(),
    ))
}

/// Convert a server-side `MergeBlockedError` body into the user-facing error
/// for `hydra patches merge`. The JSON body (when the resolved output format
/// is `Jsonl`) is printed to stdout and the human summary (when `Pretty`) is
/// printed to stderr as a side effect; the returned `anyhow::Error` carries a
/// short summary so the top-level CLI exits non-zero.
fn handle_merge_blocked(
    body: MergeBlockedError,
    output_format: ResolvedOutputFormat,
) -> anyhow::Error {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    handle_merge_blocked_with_writers(body, output_format, &mut stdout, &mut stderr)
}

fn handle_merge_blocked_with_writers(
    body: MergeBlockedError,
    output_format: ResolvedOutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> anyhow::Error {
    let patch_id = body.patch_id.clone();
    let layer = body.blocked_at_layer;
    match output_format {
        ResolvedOutputFormat::Jsonl => match serde_json::to_string(&body) {
            Ok(rendered) => {
                if let Err(err) = writeln!(stdout, "{rendered}") {
                    return anyhow!(
                        "failed to write merge_blocked JSON for patch '{patch_id}': {err}"
                    );
                }
            }
            Err(err) => {
                return anyhow!(
                    "failed to serialise merge_blocked body for patch '{patch_id}': {err}"
                );
            }
        },
        ResolvedOutputFormat::Pretty => {
            if let Err(err) = writeln!(stderr, "{}", render_merge_blocked_human(&body)) {
                return anyhow!(
                    "failed to write merge_blocked summary for patch '{patch_id}': {err}"
                );
            }
        }
    }
    anyhow!(
        "merge blocked at layer '{layer}' for patch '{patch_id}'",
        layer = blocked_at_layer_str(layer),
    )
}

fn blocked_at_layer_str(layer: BlockedAtLayer) -> &'static str {
    match layer {
        BlockedAtLayer::Reviews => "reviews",
        BlockedAtLayer::Mergers => "mergers",
    }
}

fn render_eligible_principal(principal: &EligiblePrincipal) -> String {
    match principal {
        EligiblePrincipal::User { username } => username.clone(),
        EligiblePrincipal::Dynamic {
            reference,
            resolved_to,
        } => {
            let raw = serde_json::to_value(reference)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "<unknown>".to_string());
            match resolved_to {
                Some(resolved) => format!("{raw} (resolved: {resolved})"),
                None => format!("{raw} (resolved: <unresolved>)"),
            }
        }
    }
}

fn render_principals(principals: &[EligiblePrincipal]) -> String {
    if principals.is_empty() {
        return "<none>".to_string();
    }
    principals
        .iter()
        .map(render_eligible_principal)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_assignees(names: &[String]) -> String {
    if names.is_empty() {
        return "<none>".to_string();
    }
    names.join(", ")
}

/// Render a `MergeBlockedError` body for human consumption on stderr. Keep
/// the format stable enough for tests / SWE scripts to look for landmarks
/// (`merge blocked`, the patch id, the layer name, suggested assignees).
fn render_merge_blocked_human(body: &MergeBlockedError) -> String {
    let mut out = String::new();
    let patch_id = &body.patch_id;
    match body.blocked_at_layer {
        BlockedAtLayer::Reviews => {
            out.push_str(&format!(
                "error: merge blocked — patch {patch_id} needs approval\n"
            ));
            for reason in &body.reasons {
                if let MergeBlockedReason::MissingApprovals {
                    group_index,
                    label,
                    eligible_principals,
                    current_approvals,
                    needed,
                    suggested_action,
                } = reason
                {
                    let label_disp = label
                        .clone()
                        .unwrap_or_else(|| format!("group {group_index}"));
                    out.push('\n');
                    out.push_str(&format!(
                        "  Group \"{label_disp}\" ({current} of {needed} approval{plural} needed):\n",
                        current = current_approvals.len(),
                        plural = if *needed == 1 { "" } else { "s" },
                    ));
                    out.push_str(&format!(
                        "    eligible: {}\n",
                        render_principals(eligible_principals)
                    ));
                    out.push_str(&format!(
                        "    current approvals: {}\n",
                        render_assignees(current_approvals)
                    ));
                    if let SuggestedAction::FileReviewRequest {
                        assign_to_one_of,
                        title_hint,
                    } = suggested_action
                    {
                        out.push_str(&format!(
                            "    suggested: file a review-request issue assigned to one of [{}]\n",
                            render_assignees(assign_to_one_of),
                        ));
                        if let Some(first) = assign_to_one_of.first() {
                            out.push_str(&format!(
                                "               (e.g. hydra issues create --title \"{title_hint}\" --assignee {first} --type review-request)\n",
                            ));
                        }
                    }
                }
            }
        }
        BlockedAtLayer::Mergers => {
            out.push_str(&format!(
                "error: merge blocked — caller is not authorized to merge patch {patch_id}\n"
            ));
            for reason in &body.reasons {
                if let MergeBlockedReason::NotInMergers {
                    actor,
                    allowed_mergers,
                    suggested_action,
                } = reason
                {
                    out.push('\n');
                    out.push_str(&format!("  Acting actor: {actor}\n"));
                    out.push_str(&format!(
                        "  Allowed mergers: {}\n",
                        render_principals(allowed_mergers)
                    ));
                    if let SuggestedAction::FileMergeRequest { assign_to_one_of } = suggested_action
                    {
                        out.push_str(&format!(
                            "  Suggested: file a merge-request issue assigned to one of [{}]\n",
                            render_assignees(assign_to_one_of),
                        ));
                        if let Some(first) = assign_to_one_of.first() {
                            out.push_str(&format!(
                                "             (e.g. hydra issues create --title \"Merge {patch_id}\" --assignee {first} --type merge-request)\n",
                            ));
                        }
                    }
                }
            }
        }
    }
    out.push_str("\nRun with --output-format jsonl to get the machine-readable payload.");
    out
}

async fn merge_patch(
    client: &HydraClient,
    patch_id: PatchId,
    base_override: Option<String>,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    // 1. Fetch the patch and its version history.
    let patch_record = client
        .get_patch(&patch_id)
        .await
        .with_context(|| format!("failed to fetch patch '{patch_id}'"))?;
    let patch = &patch_record.patch;

    // 2. Server-side preflight. Runs the same `merge_authorization` restriction
    //    as the write path in read-only mode, so the CLI never starts a git
    //    push it cannot finish. A blocked response carries the same structured
    //    `MergeBlockedError` body the SWE agent already understands.
    match client.merge_check(&patch_id).await {
        Ok(MergeCheckResponse::Ok(_)) => {}
        Ok(MergeCheckResponse::Blocked(body)) => {
            return Err(handle_merge_blocked(body, output_format));
        }
        Err(err) => {
            return Err(err.context(format!(
                "merge_check preflight failed for patch '{patch_id}'; refusing to merge"
            )));
        }
    }

    // 3. If the patch is linked to a GitHub PR, merge via the GitHub API.
    if let Some(github_pr) = &patch.github {
        let github_token = client
            .get_github_token()
            .await
            .context("GitHub token required to merge via GitHub API")?;
        let octocrab_client = hydra_common::github::build_octocrab_client(&github_token)?;

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
                 3. Try merging again: hydra patches merge {patch_id}\n\n\
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
                .list_repositories(&SearchRepositoriesQuery::new(None, None))
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
             Please commit or stash your changes before running 'hydra patches merge', then retry."
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
    let merge_branch = format!("hydra-squash-merge/{}", patch_id.as_ref());
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
                 3. Update the patch: hydra patches update {patch_id}\n\
                 4. Try merging again: hydra patches merge {patch_id}\n\n\
                 Underlying error: {err}"
            );
        }

        // Record the base branch OID for compare-and-swap push semantics.
        // This prevents concurrent pushes from silently overwriting each other.
        let expected_old_oid = git_resolve_ref_oid(&repo_root, &onto_ref).ok();

        // Push the squash-merged branch to the base branch on origin.
        match push_to_ref(
            &repo_root,
            &merge_branch,
            &base_branch,
            github_token.as_deref(),
            false,
            expected_old_oid,
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
             Please retry: hydra patches merge {patch_id}"
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

/// Resolve the service repository name for a patch.
///
/// Priority:
///   1. `--service-repo <org/repo>` always wins.
///   2. Otherwise, discover the enclosing git repository at `pwd`, read its
///      `remote_name` remote URL, and query `GET /v1/repositories?remote_url=...`
///      to map it to a registered service repository. The server normalizes
///      URLs (`Repository::normalize_remote_url`); the CLI sends the raw URL.
pub async fn resolve_patch_service_repo(
    client: &HydraClient,
    service_repo_override: Option<&RepoName>,
    pwd: &Path,
    remote_name: &str,
) -> Result<RepoName> {
    if let Some(name) = service_repo_override {
        return Ok(name.clone());
    }

    let repo = git2::Repository::discover(pwd)
        .map_err(|_| anyhow!("call from inside a git repo, or pass --service-repo <org/repo>"))?;

    let workdir_display = repo
        .workdir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| pwd.display().to_string());

    let remote = repo.find_remote(remote_name).map_err(|_| {
        anyhow!(
            "git repository at '{workdir_display}' has no remote '{remote_name}'; \
             pass --service-repo <org/repo>"
        )
    })?;

    let url = remote.url().ok_or_else(|| {
        anyhow!(
            "git repository at '{workdir_display}' has no remote '{remote_name}'; \
             pass --service-repo <org/repo>"
        )
    })?;

    let query = SearchRepositoriesQuery::new(None, Some(url.to_string()));
    let response = client
        .list_repositories(&query)
        .await
        .with_context(|| format!("failed to list repositories for remote '{url}'"))?;

    match response.repositories.len() {
        0 => bail!(
            "remote '{url}' is not a registered service repository; \
             register it with 'hydra repos create' or pass --service-repo <org/repo>"
        ),
        1 => Ok(response
            .repositories
            .into_iter()
            .next()
            .expect("exactly one repository in the response")
            .name),
        _ => {
            let names: Vec<String> = response
                .repositories
                .iter()
                .map(|r| r.name.to_string())
                .collect();
            bail!(
                "remote '{url}' matches multiple service repositories ({names}); \
                 pass --service-repo <org/repo> to disambiguate",
                names = names.join(", ")
            )
        }
    }
}

pub async fn create_patch_artifact_from_repo(
    client: &HydraClient,
    repo_root: &Path,
    diff: String,
    title: String,
    description: String,
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

    let creator = resolve_username(client).await?;

    // Resolve branch name, base branch, and commit range SHAs.
    let branch_name = current_branch(repo_root)?;
    let commit_range = git_resolve_commit_range_from_merge_base(repo_root, base_ref)
        .ok()
        .map(|(base_oid, head_oid)| hydra_common::patches::CommitRange::new(base_oid, head_oid));

    let patch_payload = Patch::new(
        title.clone(),
        description.clone(),
        diff,
        PatchStatus::Open,
        is_automatic_backup,
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

fn git_repository_root() -> Result<PathBuf> {
    git::repository_root(None)
}

async fn apply_patch_record(client: &HydraClient, id: PatchId) -> Result<()> {
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
    client: &HydraClient,
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
    client: &HydraClient,
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
    write_stdout(&buffer)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::HydraClient;
    use crate::command::output::ResolvedOutputFormat;
    use crate::git::{
        commit_changes as git_commit_changes, configure_repo as git_configure_repo,
        resolve_head_oid as git_resolve_head_oid, stage_all_changes as git_stage_all_changes,
    };
    use crate::test_utils::ids::{issue_id, patch_id};
    use anyhow::{anyhow, Context};
    use git2::Repository;
    use httpmock::{prelude::*, Mock};
    use hydra_common::{
        issues::{Issue, IssueStatus, IssueType, IssueVersionRecord, SessionSettings},
        patches::{
            CommitRange, CreatePatchAssetResponse, GitOid, ListPatchesResponse, Patch,
            PatchVersionRecord, Review, UpsertPatchResponse,
        },
        users::Username,
        whoami::{ActorIdentity, WhoAmIResponse},
        RepoName,
    };
    use reqwest::Client as HttpClient;
    use std::{fs, str::FromStr};

    const TEST_HYDRA_TOKEN: &str = "u-test-user:test-hydra-token";

    fn sample_diff() -> String {
        "--- a/file.txt\n+++ b/file.txt\n@@\n-old\n+new\n".to_string()
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn hydra_client(server: &MockServer) -> HydraClient {
        HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())
            .expect("failed to create hydra client")
    }

    /// Mock `GET /v1/repositories?remote_url=<remote_url>` returning the given
    /// list of `(RepoName, remote_url)` matches. The CLI's pwd-based resolver
    /// uses this endpoint to map a discovered remote URL to a service repository.
    fn mock_list_repositories_for_remote_url(
        server: &MockServer,
        remote_url: String,
        matches: Vec<(RepoName, String)>,
    ) -> Mock {
        use hydra_common::api::v1::repositories::{
            ListRepositoriesResponse, Repository, RepositoryRecord,
        };
        let response = ListRepositoriesResponse::new(
            matches
                .into_iter()
                .map(|(name, url)| RepositoryRecord::new(name, Repository::new(url, None, None)))
                .collect(),
        );
        server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/repositories")
                .query_param("remote_url", remote_url.as_str());
            then.status(200).json_body_obj(&response);
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

    fn mock_merge_check_blocked(
        server: &MockServer,
        patch_id: PatchId,
        body: hydra_common::api::v1::merge_check::MergeBlockedError,
    ) -> Mock {
        server.mock(move |when, then| {
            when.method(POST)
                .path(format!("/v1/patches/{}/merge_check", patch_id.as_ref()));
            then.status(422).json_body_obj(&body);
        })
    }

    fn mock_merge_check_status(server: &MockServer, patch_id: PatchId, status: u16) -> Mock {
        server.mock(move |when, then| {
            when.method(POST)
                .path(format!("/v1/patches/{}/merge_check", patch_id.as_ref()));
            then.status(status);
        })
    }

    fn sample_reviews_blocked(
        patch_id: &PatchId,
    ) -> hydra_common::api::v1::merge_check::MergeBlockedError {
        use hydra_common::api::v1::merge_check::{
            BlockedAtLayer, EligiblePrincipal, MergeBlockedCode, MergeBlockedError,
            MergeBlockedReason, SuggestedAction,
        };
        MergeBlockedError {
            code: MergeBlockedCode::MergeBlocked,
            patch_id: patch_id.clone(),
            blocked_at_layer: BlockedAtLayer::Reviews,
            reasons: vec![MergeBlockedReason::MissingApprovals {
                group_index: 0,
                label: Some("code-review".to_string()),
                eligible_principals: vec![
                    EligiblePrincipal::User {
                        username: "reviewer".to_string(),
                    },
                    EligiblePrincipal::User {
                        username: "jayantk".to_string(),
                    },
                ],
                current_approvals: vec![],
                needed: 1,
                suggested_action: SuggestedAction::FileReviewRequest {
                    assign_to_one_of: vec!["reviewer".to_string(), "jayantk".to_string()],
                    title_hint: format!("Review {patch_id} (code-review)"),
                },
            }],
        }
    }

    fn sample_mergers_blocked(
        patch_id: &PatchId,
    ) -> hydra_common::api::v1::merge_check::MergeBlockedError {
        use hydra_common::api::v1::merge_check::{
            BlockedAtLayer, EligiblePrincipal, MergeBlockedCode, MergeBlockedError,
            MergeBlockedReason, SuggestedAction,
        };
        use hydra_common::api::v1::repositories::DynamicRef;
        MergeBlockedError {
            code: MergeBlockedCode::MergeBlocked,
            patch_id: patch_id.clone(),
            blocked_at_layer: BlockedAtLayer::Mergers,
            reasons: vec![MergeBlockedReason::NotInMergers {
                actor: "swe-session-abcd".to_string(),
                allowed_mergers: vec![EligiblePrincipal::Dynamic {
                    reference: DynamicRef::PatchAuthor,
                    resolved_to: Some("jayantk".to_string()),
                }],
                suggested_action: SuggestedAction::FileMergeRequest {
                    assign_to_one_of: vec!["jayantk".to_string()],
                },
            }],
        }
    }

    fn mock_get_issue(server: &MockServer, issue: IssueVersionRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{}", issue.issue_id.as_ref()));
            then.status(200).json_body_obj(&issue);
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
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        )
    }

    /// Read the URL of a repository's `origin` remote. Used by the resolver
    /// tests so the mock can match on the same URL the CLI sends.
    fn origin_remote_url(repo_path: &std::path::Path) -> Result<String> {
        let repo = Repository::open(repo_path).context("failed to open repo")?;
        let remote = repo
            .find_remote("origin")
            .context("failed to find origin remote")?;
        Ok(remote
            .url()
            .ok_or_else(|| anyhow!("origin remote has no UTF-8 url"))?
            .to_string())
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
        let client = hydra_client(&server);
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/patches")
                .query_param("q", "login");
            then.status(200)
                .json_body_obj(&ListPatchesResponse::new(Vec::new()));
        });

        list_patches(
            &client,
            ListPatchesArgs {
                id: None,
                query: Some("login".to_string()),
                include_deleted: false,
                repo_name: None,
                creator: None,
                output_format: ResolvedOutputFormat::Jsonl,
            },
        )
        .await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn list_patches_sends_repo_name_and_creator_filters() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/patches")
                .query_param("repo_name", "dourolabs/hydra")
                .query_param("creator", "alice");
            then.status(200)
                .json_body_obj(&ListPatchesResponse::new(Vec::new()));
        });

        list_patches(
            &client,
            ListPatchesArgs {
                id: None,
                query: None,
                include_deleted: false,
                repo_name: Some("dourolabs/hydra".to_string()),
                creator: Some("alice".to_string()),
                output_format: ResolvedOutputFormat::Jsonl,
            },
        )
        .await?;

        mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn list_patches_emits_no_output_for_empty_results() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/patches");
            then.status(200)
                .json_body_obj(&ListPatchesResponse::new(Vec::new()));
        });

        let mut output = Vec::new();
        list_patches_with_writer(
            &client,
            ListPatchesArgs {
                id: None,
                query: None,
                include_deleted: false,
                repo_name: None,
                creator: None,
                output_format: ResolvedOutputFormat::Jsonl,
            },
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
        let client = hydra_client(&server);
        let patch_id = patch_id("p-asset-output");
        let asset_url = "https://github.com/dourolabs/hydra/assets/123";
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
        let branch_name = current_branch(&repo_path)?;
        let remote_url = origin_remote_url(&repo_path)?;
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            patch_title.clone(),
            patch_description.clone(),
            expected_diff.clone(),
            PatchStatus::Open,
            false,
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
        let patch_record = PatchVersionRecord::new(
            patch_id("p-1"),
            0,
            Utc::now(),
            patch,
            None,
            Utc::now(),
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
        let repos_mock = mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![(sample_repo_name(), remote_url)],
        );
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            None,
            "origin",
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await?;

        repos_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_errors_when_not_in_git_repo() -> Result<()> {
        // Use a temp dir that is NOT a git repo and pass `repo_root=None` so
        // the resolver discovers from pwd. We change pwd to the temp dir for
        // the duration of the test; this also exercises the
        // "git_repository_root" failure path in create_patch.
        let tempdir = tempfile::tempdir()?;
        let server = MockServer::start();
        let client = hydra_client(&server);

        // The resolver is what we want to exercise here, so call it directly
        // to keep the test independent of create_patch's other preflight
        // (which itself fails first if pwd is not a git repo).
        let error = resolve_patch_service_repo(&client, None, tempdir.path(), "origin")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("call from inside a git repo, or pass --service-repo <org/repo>"),
            "error should mention not being in a git repo: {error}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn resolve_patch_service_repo_errors_when_remote_missing() -> Result<()> {
        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let server = MockServer::start();
        let client = hydra_client(&server);
        let error = resolve_patch_service_repo(&client, None, &repo_path, "upstream")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("has no remote 'upstream'"),
            "error should mention missing remote: {error}"
        );
        assert!(
            error.contains("pass --service-repo <org/repo>"),
            "error should mention the override flag: {error}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn resolve_patch_service_repo_errors_when_no_matches() -> Result<()> {
        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let remote_url = origin_remote_url(&repo_path)?;
        let server = MockServer::start();
        let client = hydra_client(&server);
        let repos_mock =
            mock_list_repositories_for_remote_url(&server, remote_url.clone(), Vec::new());

        let error = resolve_patch_service_repo(&client, None, &repo_path, "origin")
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("is not a registered service repository"),
            "error should mention unregistered remote: {error}"
        );
        assert!(
            error.contains("hydra repos create"),
            "error should suggest 'hydra repos create': {error}"
        );
        repos_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_patch_service_repo_errors_when_multiple_matches() -> Result<()> {
        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let remote_url = origin_remote_url(&repo_path)?;
        let server = MockServer::start();
        let client = hydra_client(&server);
        let first = RepoName::from_str("dourolabs/hydra")?;
        let second = RepoName::from_str("dourolabs/hydra-fork")?;
        let repos_mock = mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![
                (first.clone(), remote_url.clone()),
                (second.clone(), remote_url.clone()),
            ],
        );

        let error = resolve_patch_service_repo(&client, None, &repo_path, "origin")
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("matches multiple service repositories"),
            "error should mention multi-match: {error}"
        );
        assert!(
            error.contains(&first.to_string()) && error.contains(&second.to_string()),
            "error should list both candidates: {error}"
        );
        assert!(
            error.contains("--service-repo <org/repo> to disambiguate"),
            "error should mention --service-repo to disambiguate: {error}"
        );
        repos_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_patch_service_repo_returns_single_match() -> Result<()> {
        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let remote_url = origin_remote_url(&repo_path)?;
        let expected = sample_repo_name();
        let server = MockServer::start();
        let client = hydra_client(&server);
        let repos_mock = mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![(expected.clone(), remote_url)],
        );

        let resolved = resolve_patch_service_repo(&client, None, &repo_path, "origin").await?;
        assert_eq!(resolved, expected);
        repos_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_patch_service_repo_override_wins_without_calling_server() -> Result<()> {
        // The override must short-circuit before any pwd / network access, so
        // that even when pwd would discover a *different* repo, the override
        // is what comes out. Point the test at a non-git directory and a mock
        // server with no expectations; if we reach either path, the test
        // explodes loudly.
        let tempdir = tempfile::tempdir()?;
        let server = MockServer::start();
        let client = hydra_client(&server);
        let override_repo = RepoName::from_str("dourolabs/override-wins")?;

        let resolved =
            resolve_patch_service_repo(&client, Some(&override_repo), tempdir.path(), "origin")
                .await?;

        assert_eq!(resolved, override_repo);
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_artifact_marks_automatic_backup_when_requested() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let diff = git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let expected_patch = Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            diff.clone(),
            PatchStatus::Open,
            true,
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
        let client = hydra_client(&server);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        mock_get_github_token_failure(&server);
        mock_whoami(&server);
        let _ = create_patch_artifact_from_repo(
            &client,
            &repo_path,
            diff.clone(),
            "backup patch".to_string(),
            "backup description".to_string(),
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
    async fn create_patch_service_repo_override_short_circuits_discovery() -> Result<()> {
        // --service-repo wins even when the discovered remote would resolve
        // to a *different* repository. We assert this by:
        //   (1) registering a different repo in mock_list_repositories
        //   (2) passing --service-repo with a third, override-only name
        //   (3) confirming the patch is created with the override name AND
        //       that the repositories endpoint is never queried.
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let remote_url = origin_remote_url(&repo_path)?;
        let override_repo = RepoName::from_str("dourolabs/api")?;
        let expected_diff =
            git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch = Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Username::from("test-user"),
            Vec::new(),
            override_repo.clone(),
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
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
        // Register a different repo at the remote URL — the override must
        // win without ever consulting this mock.
        let pwd_repo = RepoName::from_str("dourolabs/discovered-by-pwd")?;
        let repos_mock = mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![(pwd_repo, remote_url)],
        );
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);

        create_patch(
            &client,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(override_repo),
            "origin",
            false,
            false,
            "origin/main",
            Some(repo_path.as_path()),
        )
        .await?;

        patch_mock.assert();
        get_patch_mock.assert();
        // The override path must not have queried the repositories endpoint.
        assert_eq!(
            repos_mock.hits(),
            0,
            "repositories endpoint must not be called when --service-repo is provided"
        );
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
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
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
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
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
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
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
            Vec::new(),
        );
        let expected_request = UpsertPatchRequest::new(Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            sample_diff(),
            PatchStatus::Closed,
            false,
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
        let client = hydra_client(&server);
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
        let client = hydra_client(&server);
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
    async fn merge_patch_preflight_blocked_human_short_circuits() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let merge_patch_id = patch_id("p-merge-blocked");
        let patch_record = PatchVersionRecord::new(
            merge_patch_id.clone(),
            1,
            Utc::now(),
            Patch::new(
                "blocked patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
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
            Vec::new(),
        );

        let get_mock = mock_get_patch(&server, patch_record);
        let blocked_body = sample_reviews_blocked(&merge_patch_id);
        let preflight_mock =
            mock_merge_check_blocked(&server, merge_patch_id.clone(), blocked_body);

        // If the preflight short-circuits properly, the versions endpoint
        // (next step in `merge_patch`) is never queried — so a strict mock
        // with `exactly(0)` would also work. We omit the negative mock to
        // keep this test focused; the error assertion below proves we
        // never reached the review check.
        let result = merge_patch(
            &client,
            merge_patch_id.clone(),
            None,
            ResolvedOutputFormat::Pretty,
        )
        .await;

        get_mock.assert();
        preflight_mock.assert();
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("merge blocked"),
            "expected 'merge blocked' summary, got: {error}"
        );
        assert!(
            error.contains(merge_patch_id.as_ref()),
            "expected the patch id in the error, got: {error}"
        );
        assert!(
            error.contains("reviews"),
            "expected the blocked layer name in the error, got: {error}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn merge_patch_preflight_network_error_short_circuits() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let merge_patch_id = patch_id("p-merge-5xx");
        let patch_record = PatchVersionRecord::new(
            merge_patch_id.clone(),
            1,
            Utc::now(),
            Patch::new(
                "5xx patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
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
            Vec::new(),
        );

        let get_mock = mock_get_patch(&server, patch_record);
        let preflight_mock = mock_merge_check_status(&server, merge_patch_id.clone(), 500);

        let result = merge_patch(
            &client,
            merge_patch_id.clone(),
            None,
            ResolvedOutputFormat::Pretty,
        )
        .await;

        get_mock.assert();
        preflight_mock.assert();
        let error = result.unwrap_err();
        let chain = format!("{error:#}");
        assert!(
            chain.contains("merge_check"),
            "expected preflight failure context in error chain, got: {chain}"
        );

        Ok(())
    }

    #[test]
    fn handle_merge_blocked_human_writes_summary_to_stderr() {
        let pid = patch_id("p-render-reviews");
        let body = sample_reviews_blocked(&pid);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let err = handle_merge_blocked_with_writers(
            body,
            ResolvedOutputFormat::Pretty,
            &mut stdout,
            &mut stderr,
        );

        assert!(
            stdout.is_empty(),
            "stdout must be empty in the human path, got: {}",
            String::from_utf8_lossy(&stdout)
        );

        let stderr_text = String::from_utf8(stderr).unwrap();
        assert!(
            stderr_text.contains("merge blocked"),
            "stderr must include 'merge blocked', got: {stderr_text}"
        );
        assert!(
            stderr_text.contains(pid.as_ref()),
            "stderr must include patch id, got: {stderr_text}"
        );
        assert!(
            stderr_text.contains("code-review"),
            "stderr must include the group label, got: {stderr_text}"
        );
        assert!(
            stderr_text.contains("reviewer"),
            "stderr must list eligible reviewer, got: {stderr_text}"
        );
        assert!(
            stderr_text.contains("jayantk"),
            "stderr must show the resolved dynamic ref, got: {stderr_text}"
        );
        assert!(
            stderr_text.contains("Run with --output-format jsonl"),
            "stderr must include the --output-format jsonl hint, got: {stderr_text}"
        );

        let err_text = err.to_string();
        assert!(
            err_text.contains("reviews"),
            "returned error must name the blocked layer, got: {err_text}"
        );
    }

    #[test]
    fn handle_merge_blocked_json_writes_verbatim_to_stdout() {
        let pid = patch_id("p-render-json");
        let body = sample_reviews_blocked(&pid);
        let expected = serde_json::to_string(&body).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let _err = handle_merge_blocked_with_writers(
            body,
            ResolvedOutputFormat::Jsonl,
            &mut stdout,
            &mut stderr,
        );

        assert!(
            stderr.is_empty(),
            "stderr must be empty in the jsonl path, got: {}",
            String::from_utf8_lossy(&stderr)
        );

        let stdout_text = String::from_utf8(stdout).unwrap();
        // Exactly one trailing newline; the line itself is the verbatim JSON.
        assert_eq!(stdout_text, format!("{expected}\n"));
    }

    #[test]
    fn handle_merge_blocked_human_renders_mergers_layer() {
        let pid = patch_id("p-render-mergers");
        let body = sample_mergers_blocked(&pid);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let _err = handle_merge_blocked_with_writers(
            body,
            ResolvedOutputFormat::Pretty,
            &mut stdout,
            &mut stderr,
        );

        let stderr_text = String::from_utf8(stderr).unwrap();
        assert!(stderr_text.contains("not authorized to merge"));
        assert!(stderr_text.contains(pid.as_ref()));
        assert!(stderr_text.contains("swe-session-abcd"));
        assert!(stderr_text.contains("jayantk"));
        assert!(stderr_text.contains("merge-request"));
    }

    #[test]
    fn merge_clap_definition_rejects_json_flag() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct Cli {
            #[command(subcommand)]
            command: PatchesCommand,
        }

        // `--json` was removed in favour of the global `--output-format`.
        let parsed = Cli::try_parse_from(["cli", "merge", "p-some", "--json"]);
        assert!(
            parsed.is_err(),
            "--json must no longer be accepted by the merge subcommand"
        );

        // The merge subcommand still parses without the (now removed) flag.
        let parsed = Cli::try_parse_from(["cli", "merge", "p-some"])
            .expect("merge must still parse without --json");
        assert!(matches!(parsed.command, PatchesCommand::Merge { .. }));
    }

    #[tokio::test]
    async fn resolve_base_ref_uses_issue_branch_when_no_explicit_base_ref() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let id = issue_id("i-branch");

        let issue_record = IssueVersionRecord::new(
            id.clone(),
            0,
            Utc::now(),
            Issue::new(
                IssueType::Task,
                "Test".to_string(),
                "test".to_string(),
                Username::from("creator"),
                String::new(),
                IssueStatus::Open,
                None,
                Some({
                    let mut ss = SessionSettings::default();
                    ss.branch = Some("feature-branch".to_string());
                    ss
                }),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
                None,
                None,
                None,
            ),
            None,
            Utc::now(),
            Vec::new(),
        );
        let get_issue_mock = mock_get_issue(&server, issue_record);

        let result = resolve_base_ref(&client, None, Some(&id)).await?;
        assert_eq!(result, "origin/feature-branch");
        get_issue_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_base_ref_prefers_explicit_base_ref() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let id = issue_id("i-explicit");

        let result =
            resolve_base_ref(&client, Some("origin/custom".to_string()), Some(&id)).await?;
        assert_eq!(result, "origin/custom");
        Ok(())
    }

    #[tokio::test]
    async fn resolve_base_ref_defaults_to_origin_main() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);
        let id = issue_id("i-default");

        // Issue has no branch set.
        let issue_record = sample_issue_record(&id, Vec::new());
        let get_issue_mock = mock_get_issue(&server, issue_record);

        let result = resolve_base_ref(&client, None, Some(&id)).await?;
        assert_eq!(result, "origin/main");
        get_issue_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn resolve_base_ref_defaults_to_origin_main_without_issue() -> Result<()> {
        let server = MockServer::start();
        let client = hydra_client(&server);

        let result = resolve_base_ref(&client, None, None).await?;
        assert_eq!(result, "origin/main");
        Ok(())
    }

    // ---- Regression tests for silent EXIT 0 in `hydra patches create`.
    //
    // Background (issue i-hhnekayg): when the server connection dropped
    // mid-response, reqwest's hyper transport could surface a BrokenPipe
    // somewhere in the anyhow error chain. The old `is_broken_pipe` walked
    // the entire chain for ANY io::ErrorKind::BrokenPipe and exited 0
    // silently, causing real submission failures to look like clean
    // successes. These tests pin down the contract:
    //
    //   (a) success path: Ok and a real patch record is returned.
    //   (b) server 5xx:  Err with a descriptive message, NOT tagged as
    //                    a stdout BrokenPipe.
    //   (c) connection dropped mid-response: Err, NOT tagged as a stdout
    //                                        BrokenPipe.

    #[tokio::test]
    async fn create_patch_returns_patch_record_on_success() -> Result<()> {
        // (a) Happy path — succeeds with a populated PatchVersionRecord.
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let branch_name = current_branch(&repo_path)?;
        let remote_url = origin_remote_url(&repo_path)?;
        let diff = git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let patch_payload = Patch::new(
            "happy".to_string(),
            "happy desc".to_string(),
            diff,
            PatchStatus::Open,
            false,
            Username::from("test-user"),
            Vec::new(),
            sample_repo_name(),
            None,
            false,
            Some(branch_name),
            Some(CommitRange::new(base_commit, head_commit)),
            Some("main".to_string()),
        );
        let response = UpsertPatchResponse::new(patch_id("p-happy"), 0);
        let patch_record = PatchVersionRecord::new(
            patch_id("p-happy"),
            0,
            Utc::now(),
            patch_payload.clone(),
            None,
            Utc::now(),
            Vec::new(),
        );
        let server = MockServer::start();
        let client = hydra_client(&server);
        mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![(sample_repo_name(), remote_url)],
        );
        mock_create_patch(&server, UpsertPatchRequest::new(patch_payload), response);
        mock_get_patch(&server, patch_record);
        mock_get_github_token_failure(&server);
        mock_whoami(&server);

        let result = create_patch(
            &client,
            "happy".to_string(),
            "happy desc".to_string(),
            None,
            "origin",
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await?;

        assert_eq!(result.patch_id, patch_id("p-happy"));
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_returns_descriptive_error_on_server_5xx() -> Result<()> {
        // (b) When the server returns 5xx the CLI must propagate a
        // descriptive error AND must NOT classify it as a stdout
        // BrokenPipe (which would silently exit 0).
        use crate::cli::is_broken_pipe;

        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let remote_url = origin_remote_url(&repo_path)?;
        let server = MockServer::start();
        let client = hydra_client(&server);
        mock_list_repositories_for_remote_url(
            &server,
            remote_url.clone(),
            vec![(sample_repo_name(), remote_url)],
        );
        mock_get_github_token_failure(&server);
        mock_whoami(&server);
        let _patch_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/patches");
            then.status(500).body("upstream is on fire");
        });

        let result = create_patch(
            &client,
            "fivexx".to_string(),
            "fivexx desc".to_string(),
            None,
            "origin",
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await;

        let err = result.expect_err("server 5xx must surface as an Err");
        let chain_text = format!("{err:#}");
        assert!(
            chain_text.contains("500") || chain_text.contains("upstream is on fire"),
            "5xx error message should be descriptive, got: {chain_text}"
        );
        assert!(
            !is_broken_pipe(&err),
            "server 5xx error must not be misclassified as a stdout BrokenPipe (would exit 0 silently): {chain_text}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_error_not_treated_as_pipe_close_when_server_drops_connection(
    ) -> Result<()> {
        // (c) When the server accepts the TCP connection but drops it
        // without responding, reqwest's hyper transport can surface a
        // BrokenPipe (or related) IO error deep in the error chain.
        // The CLI's `is_broken_pipe` MUST NOT classify that as the
        // stdout-pipe-close case, or `hydra patches create` will exit 0
        // silently and the agent will never know its patch wasn't saved.
        //
        // (This is the exact failure mode reported in production: session
        // s-kkouueui's `hydra patches create` invocations all exited 0
        // with empty stdout/stderr and no Patch row persisted, leading
        // the agent to fall back to `gh pr create`.)
        use crate::cli::is_broken_pipe;
        use hydra_common::api::v1::repositories::{
            ListRepositoriesResponse, Repository, RepositoryRecord,
        };
        use std::net::SocketAddr;
        use tokio::net::TcpListener;

        let (_tempdir, repo_path, _base, _head) = initialize_repo_with_changes()?;
        let remote_url = origin_remote_url(&repo_path)?;
        let repositories_response = ListRepositoriesResponse::new(vec![RepositoryRecord::new(
            sample_repo_name(),
            Repository::new(remote_url, None, None),
        )]);
        let repositories_json = serde_json::to_string(&repositories_response)?;

        // First, run the pre-create RPCs (list_repositories, github_token,
        // whoami) against a normal mock server so they succeed; then aim
        // `client.create_patch` at a separate URL that drops connections.
        //
        // We do that by completing all the pre-flight calls with the mock,
        // then swapping the client's base URL to a connection-dropping
        // listener for the actual `POST /v1/patches`. The simplest shape
        // is to do the whole thing via a single client whose base URL
        // points at the dropping listener, but that breaks pre-flight.
        // Instead, we mount BOTH endpoints on the same TCP listener:
        // - reads a request line ("GET /v1/...." -> respond 200 with the
        //   appropriate JSON body)
        // - on POST /v1/patches, simply close the connection.

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_addr: SocketAddr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(value) => value,
                    Err(_) => return,
                };
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Read up to the end of the request headers (\r\n\r\n) or 16 KiB.
                let mut buf = vec![0u8; 16 * 1024];
                let mut filled = 0;
                loop {
                    match socket.read(&mut buf[filled..]).await {
                        Ok(0) => break,
                        Ok(n) => {
                            filled += n;
                            if buf[..filled].windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                            if filled == buf.len() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let request = std::str::from_utf8(&buf[..filled]).unwrap_or("");
                let first_line = request.lines().next().unwrap_or("");
                let is_post_patches =
                    first_line.starts_with("POST /v1/patches ") && !first_line.contains("/assets");
                if is_post_patches {
                    // Drop the connection without responding.
                    let _ = socket.shutdown().await;
                    drop(socket);
                    continue;
                }
                let (body, content_type): (String, &str) = if first_line
                    .contains("/v1/repositories")
                {
                    (repositories_json.clone(), "application/json")
                } else if first_line.contains("/v1/whoami") {
                    (
                        serde_json::to_string(&WhoAmIResponse::new(ActorIdentity::User {
                            username: Username::from("test-user"),
                        }))
                        .unwrap(),
                        "application/json",
                    )
                } else if first_line.contains("/v1/github/token") {
                    // Return 401 so the client falls back without a token.
                    let response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    continue;
                } else {
                    // Default 404 for anything unexpected.
                    let response =
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = socket.write_all(response.as_bytes()).await;
                    continue;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let base_url = format!("http://{local_addr}");
        let client = HydraClient::with_http_client(&base_url, TEST_HYDRA_TOKEN, HttpClient::new())
            .expect("failed to create hydra client");

        let result = create_patch(
            &client,
            "drop".to_string(),
            "drop desc".to_string(),
            None,
            "origin",
            false,
            false,
            "origin/main",
            Some(&repo_path),
        )
        .await;

        let err = result.expect_err("dropped connection must surface as an Err");
        assert!(
            !is_broken_pipe(&err),
            "transport BrokenPipe from dropped server connection must NOT be classified as a stdout pipe close (would exit 0 silently). Error chain: {err:#}"
        );

        // Replay the legacy chain-walking logic to demonstrate the bug it
        // produced. If reqwest/hyper happen to surface an io::Error with
        // BrokenPipe anywhere in the chain (which they often do when a
        // server drops the connection mid-write on Linux), the OLD
        // `is_broken_pipe` would have returned true and the CLI would have
        // exited 0 silently. The NEW implementation deliberately ignores
        // raw io::Error(BrokenPipe) — only its tagged `StdoutBrokenPipe`
        // sentinel triggers the silent-exit path.
        let _legacy_would_fire = err.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .map(|io_err| io_err.kind() == std::io::ErrorKind::BrokenPipe)
                .unwrap_or(false)
        });
        // We deliberately do NOT assert on `_legacy_would_fire` because the
        // OS-level error kind depends on timing and platform — but the
        // production failure showed it firing in real life.
        Ok(())
    }
}
