use std::{io::Write, path::Path, path::PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use metis_common::{
    constants::{ENV_METIS_ID, ENV_METIS_ISSUE_ID},
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueId, IssueRecord, IssueStatus, IssueType,
        UpsertIssueRequest,
    },
    jobs::BundleSpec,
    merge_queues::MergeQueue,
    patches::{
        Patch, PatchRecord, PatchStatus, Review, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    PatchId, RepoName, TaskId,
};
use serde::Serialize;

use crate::git;
use crate::git::{
    apply_patch, current_branch, diff_commit_range as git_diff_commit_range,
    has_uncommitted_changes as git_has_uncommitted_changes, push_branch,
};
use crate::{
    client::MetisClientInterface,
    command::output::{
        render_issue_records, render_patch_records, CommandContext, ResolvedOutputFormat,
    },
};
#[derive(Subcommand, Debug)]
pub enum PatchesCommand {
    /// List or search patches.
    List {
        /// Patch id to retrieve.
        #[arg(long = "id", value_name = "PATCH_ID")]
        id: Option<PatchId>,

        /// Query string to filter patches.
        #[arg(long = "query", value_name = "QUERY")]
        query: Option<String>,
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

        /// Create a GitHub pull request with the patch contents.
        #[arg(long = "github")]
        github: bool,

        /// Assign the merge-request issue to a user and automatically create it.
        #[arg(long = "assignee", value_name = "ASSIGNEE")]
        assignee: Option<String>,

        /// Associate the merge-request issue with an existing issue id.
        #[arg(
            long = "issue-id",
            value_name = "ISSUE_ID",
            env = ENV_METIS_ISSUE_ID
        )]
        issue_id: IssueId,

        /// Commit range to include in the patch (e.g., base..HEAD). Defaults to metis/<issue-id>/base..HEAD.
        #[arg(long = "range", value_name = "COMMIT_RANGE")]
        commit_range: Option<String>,

        /// Allow creating a patch even when the working directory has uncommitted changes.
        #[arg(long = "allow-uncommitted")]
        allow_uncommitted: bool,

        /// Force push the branch to GitHub when using --github.
        #[arg(long = "force")]
        force: bool,
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
    },

    /// Inspect or enqueue merge queue entries for a repository branch.
    Merge {
        /// Repository to target, e.g. dourolabs/api.
        #[arg(long = "repo", value_name = "REPO", required = true)]
        repo: RepoName,

        /// Branch name for the merge queue.
        #[arg(long = "branch", value_name = "BRANCH", required = true)]
        branch: String,

        /// Patch id to enqueue onto the merge queue. Omit to only fetch the queue.
        #[arg(long = "patch-id", value_name = "PATCH_ID")]
        patch_id: Option<PatchId>,
    },
    /// Manage patch assets.
    Assets {
        #[command(subcommand)]
        command: PatchAssetsCommand,
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
        PatchesCommand::List { id, query } => {
            list_patches(client, id, query, context.output_format).await
        }
        PatchesCommand::Create {
            title,
            description,
            job,
            github,
            assignee,
            issue_id,
            commit_range,
            allow_uncommitted,
            force,
        } => {
            let created = create_patch(
                client,
                title,
                description,
                job,
                github,
                assignee,
                issue_id,
                commit_range,
                allow_uncommitted,
                force,
                None,
            )
            .await?;
            write_patch_output(
                context.output_format,
                &created.patch,
                created.merge_request_issue,
            )?;
            Ok(())
        }
        PatchesCommand::Apply { id } => apply_patch_record(client, id).await,
        PatchesCommand::Review {
            id,
            author,
            contents,
            approve,
        } => review_patch(client, id, author, contents, approve).await,
        PatchesCommand::Update {
            id,
            title,
            description,
            status,
        } => {
            let patch = update_patch(client, id, title, description, status).await?;
            write_patch_output(context.output_format, &patch, None)?;
            Ok(())
        }
        PatchesCommand::Merge {
            repo,
            branch,
            patch_id,
        } => merge_queue(client, repo, branch, patch_id, context.output_format).await,
        PatchesCommand::Assets { command } => {
            patch_assets(client, command, context.output_format).await
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
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let mut buffer = Vec::new();
    list_patches_with_writer(client, id, query, output_format, &mut buffer).await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn list_patches_with_writer(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
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

    let patches = fetch_patches(client, query).await?;

    render_patch_records(output_format, &patches, writer)?;

    Ok(())
}

async fn fetch_patches(
    client: &dyn MetisClientInterface,
    query: Option<String>,
) -> Result<Vec<PatchRecord>> {
    let response = client
        .list_patches(&SearchPatchesQuery::new(query))
        .await
        .context("failed to search for patches")?;
    Ok(response.patches)
}

#[derive(Debug)]
struct CreatedPatch {
    patch: PatchRecord,
    merge_request_issue: Option<IssueRecord>,
}

async fn create_patch(
    client: &dyn MetisClientInterface,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    create_github_pr: bool,
    assignee: Option<String>,
    issue_id: IssueId,
    commit_range: Option<String>,
    allow_uncommitted: bool,
    force: bool,
    repo_root: Option<&Path>,
) -> Result<CreatedPatch> {
    let repo_root = match repo_root {
        Some(path) => path.to_path_buf(),
        None => git_repository_root()?,
    };

    if !allow_uncommitted && git_has_uncommitted_changes(&repo_root)? {
        bail!("Working directory has uncommitted changes. Commit them before creating a patch or re-run with --allow-uncommitted.");
    }

    let commit_range = resolve_commit_range(commit_range, &issue_id)?;
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
    let issue_record = client
        .get_issue(&issue_id)
        .await
        .with_context(|| format!("failed to fetch issue '{issue_id}' to inspect patches"))?;
    let existing_patch_id = (issue_record.issue.issue_type == IssueType::MergeRequest)
        .then(|| issue_record.issue.patches.last().cloned())
        .flatten();

    if let Some(existing_patch_id) = existing_patch_id {
        let existing_patch = client
            .get_patch(&existing_patch_id)
            .await
            .with_context(|| format!("failed to fetch patch '{existing_patch_id}'"))?;
        if matches!(
            existing_patch.patch.status,
            PatchStatus::Open | PatchStatus::ChangesRequested
        ) {
            let mut updated_patch = existing_patch.patch;
            updated_patch.title = title.clone();
            updated_patch.description = description.clone();
            updated_patch.diff = diff.clone();

            let response = client
                .update_patch(
                    &existing_patch_id,
                    &UpsertPatchRequest::new(updated_patch.clone()),
                )
                .await
                .with_context(|| format!("failed to update patch '{existing_patch_id}'"))?;

            let patch = PatchRecord::new(response.patch_id, updated_patch);
            return Ok(CreatedPatch {
                patch,
                merge_request_issue: None,
            });
        }
    }
    let github_token = if create_github_pr {
        Some(
            client
                .get_github_token()
                .await
                .context("failed to fetch GitHub token")?,
        )
    } else {
        None
    };
    let is_automatic_backup = false;
    let patch_title = title.clone();
    let patch_description = description.clone();
    let response = create_patch_artifact_from_repo(
        client,
        &repo_root,
        diff,
        patch_title,
        patch_description,
        job_id.clone(),
        create_github_pr,
        github_token.as_deref(),
        is_automatic_backup,
        force,
        service_repo_name,
    )
    .await?;

    let patch = client
        .get_patch(&response.patch_id)
        .await
        .with_context(|| format!("failed to fetch patch '{}'", response.patch_id))?;

    let merge_request_issue = if let Some(assignee) = assignee {
        Some(
            create_merge_request_issue(
                client,
                response.patch_id.clone(),
                assignee,
                issue_id,
                title,
                description,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(CreatedPatch {
        patch,
        merge_request_issue,
    })
}

fn write_patch_output(
    output_format: ResolvedOutputFormat,
    patch: &PatchRecord,
    merge_request_issue: Option<IssueRecord>,
) -> Result<()> {
    let mut buffer = Vec::new();
    render_patch_records(output_format, std::slice::from_ref(patch), &mut buffer)?;
    if let Some(issue) = merge_request_issue {
        render_issue_records(output_format, std::slice::from_ref(&issue), &mut buffer)?;
    }
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

fn resolve_commit_range(commit_range: Option<String>, issue_id: &IssueId) -> Result<String> {
    if let Some(range) = commit_range {
        let trimmed = range.trim();
        if trimmed.is_empty() {
            bail!("commit range must not be empty");
        }
        return Ok(trimmed.to_string());
    }

    Ok(format!("metis/{}/base..HEAD", issue_id.as_ref()))
}

async fn update_patch(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    title: Option<String>,
    description: Option<String>,
    status: Option<PatchStatus>,
) -> Result<PatchRecord> {
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

    let no_changes = title.is_none() && description.is_none() && status.is_none();
    if no_changes {
        bail!("At least one field must be provided to update.");
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

    let response = client
        .update_patch(&patch_id, &UpsertPatchRequest::new(updated_patch.clone()))
        .await
        .with_context(|| format!("failed to update patch '{patch_id}'"))?;

    Ok(PatchRecord::new(response.patch_id, updated_patch))
}

async fn merge_queue(
    client: &dyn MetisClientInterface,
    repo: RepoName,
    branch: String,
    patch_id: Option<PatchId>,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let mut buffer = Vec::new();
    merge_queue_with_writer(client, repo, branch, patch_id, output_format, &mut buffer).await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn merge_queue_with_writer(
    client: &dyn MetisClientInterface,
    repo: RepoName,
    branch: String,
    patch_id: Option<PatchId>,
    output_format: ResolvedOutputFormat,
    writer: &mut impl Write,
) -> Result<()> {
    let queue = match patch_id {
        Some(patch_id) => client
            .enqueue_merge_patch(&repo, &branch, &patch_id)
            .await
            .with_context(|| {
                format!(
                    "failed to enqueue patch '{patch_id}' onto merge queue for '{repo}:{branch}'"
                )
            })?,
        None => client
            .get_merge_queue(&repo, &branch)
            .await
            .with_context(|| format!("failed to fetch merge queue for '{repo}:{branch}'"))?,
    };

    match output_format {
        ResolvedOutputFormat::Pretty => print_merge_queue_pretty(&queue, &repo, &branch, writer)?,
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, &queue)?;
            writeln!(writer)?;
        }
    }

    Ok(())
}

fn print_merge_queue_pretty(
    queue: &MergeQueue,
    repo: &RepoName,
    branch: &str,
    writer: &mut impl Write,
) -> Result<()> {
    writeln!(writer, "Merge queue for {repo}:{branch}")?;
    if queue.patches.is_empty() {
        writeln!(writer, "- <empty>")?;
    } else {
        for patch_id in &queue.patches {
            writeln!(writer, "- {patch_id}")?;
        }
    }
    Ok(())
}

#[doc(hidden)]
pub async fn create_merge_request_issue(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    assignee: String,
    parent_issue_id: IssueId,
    patch_title: String,
    patch_description: String,
) -> Result<IssueRecord> {
    let assignee = assignee.trim().to_string();
    if assignee.is_empty() {
        bail!("Assignee must not be empty.");
    }

    let dependencies = vec![IssueDependency::new(
        IssueDependencyType::ChildOf,
        parent_issue_id.clone(),
    )];

    let summary = patch_title.trim();
    let title = if summary.is_empty() {
        patch_description
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .unwrap_or("Patch review")
            .to_string()
    } else {
        summary.to_string()
    };

    let description = format!("Review patch {}: {title}", patch_id.as_ref());
    let parent_issue = client.get_issue(&parent_issue_id).await.with_context(|| {
        format!(
            "failed to fetch parent issue '{parent_issue_id}' to determine merge-request creator"
        )
    })?;
    let creator = parent_issue.issue.creator;
    let job_settings = parent_issue.issue.job_settings.clone();
    let issue = Issue::new(
        IssueType::MergeRequest,
        description,
        creator,
        String::new(),
        IssueStatus::Open,
        Some(assignee),
        Some(job_settings),
        Vec::new(),
        dependencies,
        vec![patch_id],
    );

    let response = client
        .create_issue(&UpsertIssueRequest::new(issue.clone(), None))
        .await
        .context("failed to create merge-request issue")?;

    Ok(IssueRecord::new(response.issue_id, issue))
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
    create_github_pr: bool,
    github_token: Option<&str>,
    is_automatic_backup: bool,
    force: bool,
    service_repo_name: RepoName,
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

    let patch_payload = Patch::new(
        title.clone(),
        description.clone(),
        diff,
        PatchStatus::Open,
        is_automatic_backup,
        job_id.clone(),
        Vec::new(),
        service_repo_name.clone(),
        None,
    );
    let mut upsert_request = UpsertPatchRequest::new(patch_payload.clone());

    if create_github_pr {
        let github_token = github_token
            .ok_or_else(|| anyhow!("Creator GitHub token is required to push a GitHub branch"))?;
        let branch_name = current_branch(repo_root)?;
        if !branch_name.starts_with("metis/") {
            bail!(
                "Cannot push to GitHub: current branch '{branch_name}' does not have the required 'metis/' prefix. \
                Please checkout a branch named 'metis/<issue-id>/...' before creating a patch with --github."
            );
        }
        push_branch(repo_root, &branch_name, Some(github_token), force)?;
        upsert_request = upsert_request.with_sync_github_branch(&branch_name);
    }

    let response = client
        .create_patch(&upsert_request)
        .await
        .context("failed to create patch")?;

    Ok(response)
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

    let response = client
        .update_patch(&id, &UpsertPatchRequest::new(record.patch))
        .await
        .with_context(|| format!("failed to update patch '{id}' with review"))?;

    println!("{}", response.patch_id);
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
        issues::{
            Issue, IssueDependency, IssueDependencyType, IssueRecord, IssueStatus, IssueType,
            UpsertIssueRequest, UpsertIssueResponse,
        },
        jobs::{BundleSpec, JobRecord, Task},
        merge_queues::{EnqueueMergePatchRequest, MergeQueue},
        patches::{
            CreatePatchAssetResponse, GitOid, ListPatchesResponse, Patch, PatchRecord, Review,
            UpsertPatchResponse,
        },
        task_status::TaskStatusLog,
        users::Username,
        RepoName,
    };
    use reqwest::Client as HttpClient;
    use std::{fs, path::Path, str::FromStr};

    const TEST_METIS_TOKEN: &str = "test-metis-token";

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

    fn mock_get_job(server: &MockServer, job: JobRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/jobs/{}", job.id.as_ref()));
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

    fn mock_get_issue(server: &MockServer, issue_record: IssueRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/issues/{}", issue_record.id.as_ref()));
            then.status(200).json_body_obj(&issue_record);
        })
    }

    fn mock_get_patch(server: &MockServer, patch: PatchRecord) -> Mock {
        server.mock(move |when, then| {
            when.method(GET)
                .path(format!("/v1/patches/{}", patch.id.as_ref()));
            then.status(200).json_body_obj(&patch);
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

    fn initialize_repo_with_changes(
    ) -> Result<(tempfile::TempDir, std::path::PathBuf, GitOid, GitOid)> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test repo")?;
        let repo_path = tempdir.path().to_path_buf();
        let repo = Repository::init(&repo_path).context("failed to init git repo for test")?;
        git_configure_repo(&repo_path, "Test User", "test@example.com")?;
        repo.remote("origin", "https://github.com/dourolabs/example.git")
            .context("failed to set remote origin")?;

        fs::write(repo_path.join("README.md"), "initial content\n")
            .context("failed to write initial README.md")?;
        git_stage_all_changes(&repo_path)?;
        git_commit_changes(&repo_path, "initial commit")?;
        let base_commit = git_resolve_head_oid(&repo_path)?
            .ok_or_else(|| anyhow!("failed to resolve initial commit"))?;

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

    fn create_branch_at(repo_path: &Path, branch: &str, target: GitOid) -> Result<()> {
        let repo = Repository::open(repo_path)?;
        let commit = repo
            .find_commit(target.into())
            .with_context(|| format!("failed to resolve commit {target} for branch '{branch}'"))?;
        repo.branch(branch, &commit, true)
            .with_context(|| format!("failed to create branch '{branch}'"))?;
        Ok(())
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
        let (_tempdir, repo_path, base_commit, _head_commit) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-diff");
        let issue_id = issue_id("i-diff");
        let base_branch = format!("metis/{}/base", issue_id.as_ref());
        create_branch_at(&repo_path, &base_branch, base_commit)?;
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        let job_id_clone = job_id.clone();
        let expected_diff = git_diff_commit_range(&repo_path, &format!("{base_branch}..HEAD"))?;
        let patch = Patch::new(
            patch_title.clone(),
            patch_description.clone(),
            expected_diff.clone(),
            PatchStatus::Open,
            false,
            Some(job_id_clone.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-1"));
        let patch_record = PatchRecord::new(patch_id("p-1"), patch);
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::Task,
                "issue for patch diff".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let issue_mock = mock_get_issue(&server, issue_record);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            Some(job_id),
            false,
            None,
            issue_id.clone(),
            None,
            false,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        issue_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_sets_created_by_from_job_id() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;

        let job_id = task_id("t-job-1234");
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );

        let title = "patch with job title".to_string();
        let job_id_opt = Some(job_id.clone());
        let description = "patch with job id".to_string();
        let issue_id = issue_id("i-job-1234");
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let expected_diff = git_diff_commit_range(&repo_path, &commit_range.clone().unwrap())?;
        let patch = Patch::new(
            title.clone(),
            description.clone(),
            expected_diff,
            PatchStatus::Open,
            false,
            job_id_opt.clone(),
            Vec::new(),
            sample_repo_name(),
            None,
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-2"));
        let patch_record = PatchRecord::new(patch_id("p-2"), patch);
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::Task,
                "issue with job patch".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let issue_mock = mock_get_issue(&server, issue_record);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id_opt.clone(),
            false,
            None,
            issue_id.clone(),
            commit_range,
            false,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        issue_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_errors_without_job_id() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let server = MockServer::start();
        let client = metis_client(&server);
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let issue_id = issue_id("i-missing-job");
        let result = create_patch(
            &client,
            "missing job".to_string(),
            "patch without job id".to_string(),
            None,
            false,
            None,
            issue_id,
            commit_range,
            false,
            false,
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
    async fn create_patch_requires_creator_github_token_when_creating_pr() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let server = MockServer::start();
        let client = metis_client(&server);
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let issue_id = issue_id("i-gh-token");
        let job_id = task_id("t-job-gh-token");
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let job_mock = mock_get_job(&server, job_record);
        let expected_diff = git_diff_commit_range(&repo_path, &commit_range.clone().unwrap())?;
        let expected_patch_request = UpsertPatchRequest::new(Patch::new(
            "pr title".to_string(),
            "pr description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        ));
        let patch_response = UpsertPatchResponse::new(patch_id("p-gh-token"));
        let _patch_mock = mock_create_patch(&server, expected_patch_request, patch_response);
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::Task,
                "issue requiring github token".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        let issue_mock = mock_get_issue(&server, issue_record);

        let result = create_patch(
            &client,
            "pr title".to_string(),
            "pr description".to_string(),
            Some(job_id),
            true,
            None,
            issue_id,
            commit_range,
            false,
            false,
            Some(&repo_path),
        )
        .await;

        let error = result.unwrap_err().to_string();

        assert!(
            error.contains("Creator GitHub token is required to create a GitHub pull request")
                || error.contains("failed to create GitHub pull request")
                || error.contains("failed to push branch")
                || error.contains("GitHub token")
                || error.contains("github"),
            "error should reference GitHub token or PR creation: {error}"
        );
        job_mock.assert();
        issue_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_creates_merge_request_issue_when_assignee_provided() -> Result<()> {
        let (_tempdir, repo_path, base_commit, _) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-merge");
        let parent_issue = issue_id("i-parent");
        let base_branch = format!("metis/{}/base", parent_issue.as_ref());
        create_branch_at(&repo_path, &base_branch, base_commit)?;
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let created_patch_id = patch_id("p-merge");
        let expected_diff = git_diff_commit_range(&repo_path, &format!("{base_branch}..HEAD"))?;
        let patch = Patch::new(
            "custom patch title".to_string(),
            "custom patch description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        );
        let expected_patch_request = UpsertPatchRequest::new(patch.clone());
        let parent_issue_record = IssueRecord::new(
            parent_issue.clone(),
            Issue::new(
                IssueType::Task,
                "parent issue".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                Some("owner-a".to_string()),
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        let issue_request = UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                format!(
                    "Review patch {}: custom patch title",
                    created_patch_id.as_ref()
                ),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                Some("owner-a".to_string()),
                None,
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent_issue.clone(),
                )],
                vec![created_patch_id.clone()],
            ),
            None,
        );
        let patch_response = UpsertPatchResponse::new(created_patch_id.clone());
        let patch_record = PatchRecord::new(created_patch_id.clone(), patch);
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_patch_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);
        let parent_issue_mock = mock_get_issue(&server, parent_issue_record);
        let issue_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/issues")
                .json_body_obj(&issue_request);
            then.status(200)
                .json_body_obj(&UpsertIssueResponse::new(issue_id("i-merge")));
        });

        create_patch(
            &client,
            "custom patch title".to_string(),
            "custom patch description".to_string(),
            Some(job_id),
            false,
            Some("owner-a".to_string()),
            parent_issue.clone(),
            None,
            false,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();
        parent_issue_mock.assert_hits(2);
        issue_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_updates_merge_request_patch_when_present() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-review");
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let issue_id = issue_id("i-review");
        let patch_id = patch_id("p-review");
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let expected_diff = git_diff_commit_range(&repo_path, &commit_range.clone().unwrap())?;
        let reviews = vec![Review::new(
            "needs adjustments".to_string(),
            false,
            "reviewer".to_string(),
            None,
        )];
        let existing_patch = Patch::new(
            "old title".to_string(),
            "old description".to_string(),
            sample_diff(),
            PatchStatus::ChangesRequested,
            false,
            Some(job_id.clone()),
            reviews.clone(),
            sample_repo_name(),
            None,
        );
        let patch_record = PatchRecord::new(patch_id.clone(), existing_patch);
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::MergeRequest,
                "review patch".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                Some("agent-a".to_string()),
                None,
                Vec::new(),
                Vec::new(),
                vec![patch_id.clone()],
            ),
        );
        let updated_patch = Patch::new(
            "updated title".to_string(),
            "updated description".to_string(),
            expected_diff.clone(),
            PatchStatus::ChangesRequested,
            false,
            Some(job_id.clone()),
            reviews,
            sample_repo_name(),
            None,
        );
        let expected_request = UpsertPatchRequest::new(updated_patch);
        let patch_response = UpsertPatchResponse::new(patch_id.clone());
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record);
        let issue_mock = mock_get_issue(&server, issue_record);
        let patch_mock = mock_get_patch(&server, patch_record);
        let update_mock =
            mock_update_patch(&server, patch_id.clone(), expected_request, patch_response);

        create_patch(
            &client,
            "updated title".to_string(),
            "updated description".to_string(),
            Some(job_id),
            false,
            None,
            issue_id,
            commit_range,
            false,
            false,
            Some(repo_path.as_path()),
        )
        .await?;

        job_mock.assert();
        issue_mock.assert();
        patch_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_artifact_marks_automatic_backup_when_requested() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let diff = git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let job_id = task_id("t-job-automatic");
        let expected_request = UpsertPatchRequest::new(Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            diff.clone(),
            PatchStatus::Open,
            true,
            Some(job_id.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        ));
        let patch_response = UpsertPatchResponse::new(patch_id("p-automatic"));
        let server = MockServer::start();
        let client = metis_client(&server);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let _ = create_patch_artifact_from_repo(
            &client,
            &repo_path,
            diff.clone(),
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(job_id.clone()),
            false,
            None,
            true,
            false,
            sample_repo_name(),
        )
        .await?;

        patch_mock.assert();
        assert_eq!(patch_mock.hits(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_uses_service_repo_name_from_job() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-service");
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::ServiceRepository {
                    name: RepoName::from_str("dourolabs/api")?,
                    rev: None,
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let issue_id = issue_id("i-service");
        let expected_diff = git_diff_commit_range(&repo_path, &commit_range.clone().unwrap())?;
        let patch = Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            RepoName::from_str("dourolabs/api")?,
            None,
        );
        let expected_request = UpsertPatchRequest::new(patch.clone());
        let patch_response = UpsertPatchResponse::new(patch_id("p-service"));
        let patch_record = PatchRecord::new(patch_id("p-service"), patch);
        let issue_record = IssueRecord::new(
            issue_id.clone(),
            Issue::new(
                IssueType::Task,
                "issue with service repo".to_string(),
                Username::from("creator-a"),
                String::new(),
                IssueStatus::Open,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        );
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let issue_mock = mock_get_issue(&server, issue_record);
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        let get_patch_mock = mock_get_patch(&server, patch_record);

        create_patch(
            &client,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(job_id.clone()),
            false,
            None,
            issue_id.clone(),
            commit_range,
            false,
            false,
            Some(repo_path.as_path()),
        )
        .await?;

        job_mock.assert();
        issue_mock.assert();
        patch_mock.assert();
        get_patch_mock.assert();
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
        let job_record = JobRecord::new(
            job_id.clone(),
            Task::new(
                "0".to_string(),
                BundleSpec::GitRepository {
                    url: "https://github.com/dourolabs/example".to_string(),
                    rev: "main".to_string(),
                },
                None,
                None,
                None,
                Default::default(),
                None,
                None,
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
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
        let patch_record = PatchRecord::new(
            review_patch_id.clone(),
            Patch::new(
                "reviewed patch".to_string(),
                "description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                vec![existing_review.clone()],
                sample_repo_name(),
                None,
            ),
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
                .json_body_obj(&UpsertPatchResponse::new(patch_id("p-123")));
        });

        review_patch(
            &client,
            review_patch_id.clone(),
            "alice".to_string(),
            "looks good now".to_string(),
            true,
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_modifies_requested_fields() -> Result<()> {
        let patch_record = PatchRecord::new(
            patch_id("p-update"),
            Patch::new(
                "Initial title".to_string(),
                "Initial description".to_string(),
                sample_diff(),
                PatchStatus::Open,
                false,
                None,
                vec![Review::new(
                    "looks ok".to_string(),
                    false,
                    "sam".to_string(),
                    None,
                )],
                sample_repo_name(),
                None,
            ),
        );
        let expected_request = UpsertPatchRequest::new(Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            sample_diff(),
            PatchStatus::Closed,
            false,
            None,
            vec![Review::new(
                "looks ok".to_string(),
                false,
                "sam".to_string(),
                None,
            )],
            sample_repo_name(),
            None,
        ));
        let server = MockServer::start();
        let client = metis_client(&server);
        let get_mock = mock_get_patch(&server, patch_record.clone());
        let update_mock = mock_update_patch(
            &server,
            patch_id("p-update"),
            expected_request,
            UpsertPatchResponse::new(patch_id("p-update")),
        );

        update_patch(
            &client,
            patch_id("p-update"),
            Some("Updated title".to_string()),
            Some("Updated description".to_string()),
            Some(PatchStatus::Closed),
        )
        .await?;

        get_mock.assert();
        update_mock.assert();

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_rejects_empty_updates() {
        let server = MockServer::start();
        let client = metis_client(&server);
        let result = update_patch(&client, patch_id("p-empty"), None, None, None).await;

        assert!(result.is_err(), "expected update to reject empty payload");
    }

    #[tokio::test]
    async fn merge_queue_fetches_queue_and_writes_json() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let repo = sample_repo_name();
        let branch = "main".to_string();
        let queued_patch = patch_id("p-queue-001");
        let merge_queue = MergeQueue::new(vec![queued_patch.clone()]);
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/merge-queues/dourolabs/example/main/patches");
            then.status(200).json_body_obj(&merge_queue);
        });

        let mut output = Vec::new();
        merge_queue_with_writer(
            &client,
            repo.clone(),
            branch.clone(),
            None,
            ResolvedOutputFormat::Jsonl,
            &mut output,
        )
        .await?;

        mock.assert();
        assert_eq!(
            String::from_utf8(output)?,
            format!("{}\n", serde_json::to_string(&merge_queue)?)
        );

        Ok(())
    }

    #[tokio::test]
    async fn merge_queue_enqueues_patch_and_pretty_prints() -> Result<()> {
        let server = MockServer::start();
        let client = metis_client(&server);
        let repo = sample_repo_name();
        let branch = "feature".to_string();
        let patch = patch_id("p-queue-002");
        let merge_queue = MergeQueue::new(vec![patch.clone()]);
        let enqueue_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/merge-queues/dourolabs/example/feature/patches")
                .json_body_obj(&EnqueueMergePatchRequest::new(patch.clone()));
            then.status(200).json_body_obj(&merge_queue);
        });

        let mut output = Vec::new();
        merge_queue_with_writer(
            &client,
            repo.clone(),
            branch.clone(),
            Some(patch.clone()),
            ResolvedOutputFormat::Pretty,
            &mut output,
        )
        .await?;

        enqueue_mock.assert();
        assert_eq!(
            String::from_utf8(output)?,
            format!("Merge queue for {repo}:{branch}\n- {patch}\n")
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_with_github_rejects_non_metis_branch() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let diff = git_diff_commit_range(&repo_path, &format!("{base_commit}..{head_commit}"))?;
        let job_id = task_id("t-job-branch-check");
        let server = MockServer::start();
        let client = metis_client(&server);

        let result = create_patch_artifact_from_repo(
            &client,
            &repo_path,
            diff,
            "test patch".to_string(),
            "test description".to_string(),
            Some(job_id),
            true,
            Some("test-token"),
            false,
            false,
            sample_repo_name(),
        )
        .await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("does not have the required 'metis/' prefix"),
            "error should mention metis/ prefix requirement: {error}"
        );
        assert!(
            error.contains("main") || error.contains("master"),
            "error should reference the current branch name: {error}"
        );

        Ok(())
    }
}
