use std::{io::Write, path::Path, path::PathBuf, process::Command};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::Subcommand;
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_METIS_ID, ENV_METIS_ISSUE_ID},
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueId, IssueStatus, IssueType,
        UpsertIssueRequest,
    },
    jobs::BundleSpec,
    merge_queues::MergeQueue,
    patches::{
        GithubPr, Patch, PatchRecord, PatchStatus, Review, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    users::{User, Username},
    PatchId, RepoName, TaskId,
};
use octocrab::Octocrab;
use serde::Deserialize;

use crate::client::MetisClientInterface;
use crate::git;
use crate::git::{
    apply_patch, branch_exists, checkout_new_branch, current_branch,
    diff_commit_range as git_diff_commit_range,
    has_uncommitted_changes as git_has_uncommitted_changes, push_branch,
};
use tempfile::NamedTempFile;

/// ANSI color codes
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

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

        /// Pretty-print the matching patch details.
        #[arg(long = "pretty")]
        pretty: bool,
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

        /// GitHub token to use when creating pull requests.
        #[arg(long = "github-token", value_name = "TOKEN", env = ENV_GH_TOKEN)]
        github_token: Option<String>,

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

        /// Pretty-print the merge queue instead of emitting JSON.
        #[arg(long = "pretty")]
        pretty: bool,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: PatchesCommand) -> Result<()> {
    match command {
        PatchesCommand::List { id, query, pretty } => list_patches(client, id, query, pretty).await,
        PatchesCommand::Create {
            title,
            description,
            job,
            github,
            github_token,
            assignee,
            issue_id,
            commit_range,
            allow_uncommitted,
        } => {
            create_patch(
                client,
                title,
                description,
                job,
                github,
                github_token,
                assignee,
                issue_id,
                commit_range,
                allow_uncommitted,
                None,
            )
            .await
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
        } => update_patch(client, id, title, description, status).await,
        PatchesCommand::Merge {
            repo,
            branch,
            patch_id,
            pretty,
        } => merge_queue(client, repo, branch, patch_id, pretty).await,
    }
}

async fn list_patches(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
    pretty: bool,
) -> Result<()> {
    let mut buffer = Vec::new();
    list_patches_with_writer(client, id, query, pretty, &mut buffer).await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn list_patches_with_writer(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
    pretty: bool,
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
        if pretty {
            print_patches_pretty(&[patch], writer)?;
        } else {
            print_patches_jsonl(&[patch], writer)?;
        }
        return Ok(());
    }

    let patches = fetch_patches(client, query).await?;

    if pretty {
        print_patches_pretty(&patches, writer)?;
    } else {
        print_patches_jsonl(&patches, writer)?;
    }

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

async fn create_patch(
    client: &dyn MetisClientInterface,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    create_github_pr: bool,
    github_token: Option<String>,
    assignee: Option<String>,
    issue_id: IssueId,
    commit_range: Option<String>,
    allow_uncommitted: bool,
    repo_root: Option<&Path>,
) -> Result<()> {
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

    let github_token = if create_github_pr {
        Some(
            github_token
                .as_deref()
                .ok_or_else(|| {
                    anyhow!(
                        "{ENV_GH_TOKEN} must be provided via --github-token or environment when using --github"
                    )
                })?,
        )
    } else {
        None
    };
    let service_repo_name = resolve_service_repo_name(client, job_id.as_ref()).await?;
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
        github_token,
        is_automatic_backup,
        service_repo_name,
    )
    .await?;

    let mut merge_request_issue_id = None;
    if let Some(assignee) = assignee {
        let merge_issue_id = create_merge_request_issue(
            client,
            response.patch_id.clone(),
            assignee,
            issue_id,
            title,
            description,
        )
        .await?;
        merge_request_issue_id = Some(merge_issue_id);
    }

    let mut output = serde_json::json!({
        "patch_id": response.patch_id,
        "type": "patch"
    });

    if let Some(issue_id) = merge_request_issue_id {
        if let Some(object) = output.as_object_mut() {
            object.insert(
                "merge_request_issue_id".to_string(),
                serde_json::json!(issue_id),
            );
        }
    }

    println!("{}", serde_json::to_string(&output)?);

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
) -> Result<()> {
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
        .update_patch(&patch_id, &UpsertPatchRequest::new(updated_patch))
        .await
        .with_context(|| format!("failed to update patch '{patch_id}'"))?;

    println!("{}", response.patch_id);

    Ok(())
}

async fn merge_queue(
    client: &dyn MetisClientInterface,
    repo: RepoName,
    branch: String,
    patch_id: Option<PatchId>,
    pretty: bool,
) -> Result<()> {
    let mut buffer = Vec::new();
    merge_queue_with_writer(client, repo, branch, patch_id, pretty, &mut buffer).await?;
    std::io::stdout().write_all(&buffer)?;
    std::io::stdout().flush()?;
    Ok(())
}

async fn merge_queue_with_writer(
    client: &dyn MetisClientInterface,
    repo: RepoName,
    branch: String,
    patch_id: Option<PatchId>,
    pretty: bool,
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

    if pretty {
        print_merge_queue_pretty(&queue, &repo, &branch, writer)?;
    } else {
        serde_json::to_writer(&mut *writer, &queue)?;
        writeln!(writer)?;
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

async fn create_merge_request_issue(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    assignee: String,
    parent_issue_id: IssueId,
    patch_title: String,
    patch_description: String,
) -> Result<IssueId> {
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
    let creator = if parent_issue
        .issue
        .creator
        .username
        .as_ref()
        .trim()
        .is_empty()
    {
        User::new(Username::from("unknown"), String::new())
    } else {
        parent_issue.issue.creator
    };

    let response = client
        .create_issue(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::MergeRequest,
                description,
                creator,
                String::new(),
                IssueStatus::Open,
                Some(assignee),
                None,
                Vec::new(),
                dependencies,
                vec![patch_id],
            ),
            None,
        ))
        .await
        .context("failed to create merge-request issue")?;

    Ok(response.issue_id)
}

pub async fn resolve_service_repo_name(
    client: &dyn MetisClientInterface,
    job_id: Option<&TaskId>,
) -> Result<RepoName> {
    let job_id = job_id.ok_or_else(|| {
        anyhow!("service repo name must be resolved from a job; provide --job or set METIS_ID")
    })?;
    let job = client
        .get_job(job_id)
        .await
        .with_context(|| format!("failed to fetch job '{job_id}' to resolve service repo"))?;

    if let BundleSpec::ServiceRepository { name, .. } = job.task.context {
        return Ok(name);
    }

    bail!("job '{job_id}' does not reference a service repository")
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

    let mut patch_payload = Patch::new(
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
    let response = client
        .create_patch(&UpsertPatchRequest::new(patch_payload.clone()))
        .await
        .context("failed to create patch")?;

    if create_github_pr {
        let github_pr = create_github_pull_request(
            repo_root,
            &title,
            &description,
            github_token,
            job_id.as_ref().map(|id| id.as_ref()),
        )
        .await?;
        patch_payload.github = Some(github_pr);
        client
            .update_patch(&response.patch_id, &UpsertPatchRequest::new(patch_payload))
            .await
            .context("failed to update patch with GitHub metadata")?;
    }

    Ok(response)
}

fn git_repository_root() -> Result<PathBuf> {
    git::repository_root(None)
}

fn extract_patch_title(record: &PatchRecord) -> &str {
    &record.patch.title
}

fn extract_patch_status(record: &PatchRecord) -> PatchStatus {
    record.patch.status
}

fn extract_patch_description(record: &PatchRecord) -> &str {
    &record.patch.description
}

fn format_patch_status(status: PatchStatus) -> &'static str {
    match status {
        PatchStatus::Open => "open",
        PatchStatus::Closed => "closed",
        PatchStatus::Merged => "merged",
        _ => "unknown",
    }
}

fn print_patches_jsonl(patches: &[PatchRecord], writer: &mut impl Write) -> Result<()> {
    for patch in patches {
        serde_json::to_writer(&mut *writer, patch)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn print_patches_pretty(patches: &[PatchRecord], writer: &mut impl Write) -> Result<()> {
    for patch in patches {
        write_patch_record_pretty(patch, writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_patch_record_pretty(record: &PatchRecord, writer: &mut impl Write) -> Result<()> {
    let title = extract_patch_title(record);
    let status = extract_patch_status(record);
    let description = extract_patch_description(record);
    writeln!(
        writer,
        "Patch {} [{}]: {}",
        record.id,
        format_patch_status(status),
        title
    )?;
    writeln!(
        writer,
        "Repository: {}",
        record.patch.service_repo_name.as_str()
    )?;
    if !description.trim().is_empty() {
        writeln!(writer, "{description}")?;
    }
    if record.patch.diff.trim().is_empty() {
        writeln!(writer, "[no diff available]")?;
    } else {
        writeln!(writer)?;
        pretty_print_patch(&record.patch.diff, writer)?;
    }
    writeln!(writer)?;
    Ok(())
}

/// Pretty-print a patch with color coding (green for additions, red for deletions).
fn pretty_print_patch(patch: &str, writer: &mut impl Write) -> Result<()> {
    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            writeln!(writer, "{GREEN}{line}{RESET}")?;
        } else if line.starts_with('-') && !line.starts_with("---") {
            writeln!(writer, "{RED}{line}{RESET}")?;
        } else {
            writeln!(writer, "{line}")?;
        }
    }
    Ok(())
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

async fn create_github_pull_request(
    repo_root: &Path,
    title: &str,
    description: &str,
    github_token: Option<&str>,
    job_id: Option<&str>,
) -> Result<GithubPr> {
    let github_token = github_token
        .ok_or_else(|| anyhow!("{ENV_GH_TOKEN} is required when creating a GitHub pull request"))?;
    let branch_name = ensure_feature_branch(repo_root, job_id)?;
    push_branch(repo_root, &branch_name, Some(github_token))?;
    let pr_metadata =
        open_pull_request(repo_root, title, description, &branch_name, github_token).await?;
    let (owner, repo) = parse_pr_repository(&pr_metadata.url)
        .ok_or_else(|| anyhow!("failed to parse GitHub PR URL '{}'", pr_metadata.url))?;
    Ok(GithubPr::new(
        owner,
        repo,
        pr_metadata.number,
        pr_metadata.head_ref_name,
        pr_metadata.base_ref_name,
        Some(pr_metadata.url),
        None,
    ))
}

fn ensure_feature_branch(repo_root: &Path, job_id: Option<&str>) -> Result<String> {
    let current_branch = current_branch(repo_root)?;
    if !should_create_new_branch(&current_branch) {
        return Ok(current_branch);
    }

    let sanitized_job = sanitize_branch_segment(job_id.unwrap_or("patch"));
    let mut candidate = if sanitized_job.is_empty() {
        "metis-patch".to_string()
    } else {
        format!("metis-{sanitized_job}")
    };
    let mut suffix = 0;
    while branch_exists(repo_root, &candidate)? {
        suffix += 1;
        candidate = format!("{candidate}-{suffix}");
    }

    checkout_new_branch(repo_root, &candidate)?;
    Ok(candidate)
}

fn should_create_new_branch(branch: &str) -> bool {
    matches!(branch, "HEAD" | "main" | "master")
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

#[derive(Deserialize)]
struct GhPrCreateResponse {
    url: String,
    number: u64,
    #[serde(rename = "headRefName", default)]
    head_ref_name: Option<String>,
    #[serde(rename = "baseRefName", default)]
    base_ref_name: Option<String>,
}

fn parse_pr_repository(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
    let mut segments = without_scheme.split('/');
    let owner = segments.next()?;
    let repo = segments.next()?;
    let pr_segment = segments.next()?;
    if pr_segment != "pull" {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn parse_pr_number(url: &str) -> Option<u64> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
    let mut segments = without_scheme.split('/');
    let _owner = segments.next()?;
    let _repo = segments.next()?;
    let pr_segment = segments.next()?;
    if pr_segment != "pull" {
        return None;
    }
    let pr_number_str = segments.next()?;
    pr_number_str.parse().ok()
}

async fn open_pull_request(
    repo_root: &Path,
    title: &str,
    description: &str,
    branch: &str,
    github_token: &str,
) -> Result<GhPrCreateResponse> {
    let mut body_file = NamedTempFile::new().context("failed to create temporary PR body file")?;
    writeln!(body_file, "{description}").context("failed to write pull request description")?;

    // Create the PR (gh pr create doesn't support --json)
    let create_output = Command::new("gh")
        .args(["pr", "create"])
        .args(["--head", branch])
        .args(["--title", title])
        .args(["--body-file"])
        .arg(body_file.path())
        .current_dir(repo_root)
        .output()
        .context("failed to create GitHub pull request")?;

    if !create_output.status.success() {
        let stdout = String::from_utf8_lossy(&create_output.stdout);
        let stderr = String::from_utf8_lossy(&create_output.stderr);
        bail!("failed to open GitHub pull request for branch '{branch}': {stderr}{stdout}");
    }

    // Extract PR URL from output (gh pr create outputs the URL to stdout)
    let stdout = String::from_utf8_lossy(&create_output.stdout);
    let pr_url = stdout
        .lines()
        .find(|line| line.contains("github.com") && line.contains("/pull/"))
        .ok_or_else(|| anyhow!("failed to extract PR URL from gh pr create output: {stdout}"))?;

    // Use octocrab to get structured PR metadata
    let crab = Octocrab::builder()
        .personal_token(github_token.to_string())
        .build()
        .context("failed to create octocrab client")?;

    let (owner, repo) = parse_pr_repository(pr_url.trim())
        .ok_or_else(|| anyhow!("failed to parse repository from PR URL: {pr_url}"))?;
    let pr_number = parse_pr_number(pr_url.trim())
        .ok_or_else(|| anyhow!("failed to parse PR number from URL: {pr_url}"))?;

    let pr = crab
        .pulls(owner, repo)
        .get(pr_number)
        .await
        .context("failed to fetch GitHub pull request metadata")?;

    Ok(GhPrCreateResponse {
        url: pr
            .html_url
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| pr_url.trim().to_string()),
        number: pr.number,
        head_ref_name: Some(pr.head.ref_field.clone()),
        base_ref_name: Some(pr.base.ref_field.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
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
        patches::{GitOid, ListPatchesResponse, Patch, PatchRecord, Review, UpsertPatchResponse},
        task_status::TaskStatusLog,
        users::{User, Username},
        RepoName,
    };
    use reqwest::Client as HttpClient;
    use std::{fs, path::Path, str::FromStr};

    fn sample_diff() -> String {
        "--- a/file.txt\n+++ b/file.txt\n@@\n-old\n+new\n".to_string()
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn metis_client(server: &MockServer) -> MetisClient {
        MetisClient::with_http_client(server.base_url(), HttpClient::new())
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

        list_patches(&client, None, Some("login".to_string()), false).await?;

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
        list_patches_with_writer(&client, None, None, false, &mut output).await?;

        assert!(output.is_empty());
        mock.assert();
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
                Default::default(),
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        let job_id_clone = job_id.clone();
        let expected_diff = git_diff_commit_range(&repo_path, &format!("{base_branch}..HEAD"))?;
        let expected_request = UpsertPatchRequest::new(Patch::new(
            patch_title.clone(),
            patch_description.clone(),
            expected_diff.clone(),
            PatchStatus::Open,
            false,
            Some(job_id_clone.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        ));
        let patch_response = UpsertPatchResponse::new(patch_id("p-1"));
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            Some(job_id),
            false,
            None,
            None,
            issue_id.clone(),
            None,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();

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
                Default::default(),
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
        let expected_request = UpsertPatchRequest::new(Patch::new(
            title.clone(),
            description.clone(),
            expected_diff,
            PatchStatus::Open,
            false,
            job_id_opt.clone(),
            Vec::new(),
            sample_repo_name(),
            None,
        ));
        let patch_response = UpsertPatchResponse::new(patch_id("p-2"));
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id_opt.clone(),
            false,
            None,
            None,
            issue_id.clone(),
            commit_range,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();

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
            None,
            issue_id,
            commit_range,
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
    async fn create_patch_requires_github_token_when_creating_pr() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let server = MockServer::start();
        let client = metis_client(&server);
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let issue_id = issue_id("i-gh-token");

        let result = create_patch(
            &client,
            "pr title".to_string(),
            "pr description".to_string(),
            Some(task_id("t-job-gh-token")),
            true,
            None,
            None,
            issue_id,
            commit_range,
            false,
            Some(&repo_path),
        )
        .await;

        let error = result.unwrap_err().to_string();
        assert!(
            error.contains(ENV_GH_TOKEN),
            "error should reference missing GitHub token: {error}"
        );

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
                Default::default(),
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let created_patch_id = patch_id("p-merge");
        let expected_diff = git_diff_commit_range(&repo_path, &format!("{base_branch}..HEAD"))?;
        let expected_patch_request = UpsertPatchRequest::new(Patch::new(
            "custom patch title".to_string(),
            "custom patch description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            sample_repo_name(),
            None,
        ));
        let parent_issue_record = IssueRecord::new(
            parent_issue.clone(),
            Issue::new(
                IssueType::Task,
                "parent issue".to_string(),
                User::new(Username::from("creator-a"), String::new()),
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
                User::new(Username::from("creator-a"), String::new()),
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
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_patch_request, patch_response.clone());
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
            None,
            Some("owner-a".to_string()),
            parent_issue.clone(),
            None,
            false,
            Some(&repo_path),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
        parent_issue_mock.assert();
        issue_mock.assert();

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
                Default::default(),
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let commit_range = Some(format!("{base_commit}..{head_commit}"));
        let issue_id = issue_id("i-service");
        let expected_diff = git_diff_commit_range(&repo_path, &commit_range.clone().unwrap())?;
        let expected_request = UpsertPatchRequest::new(Patch::new(
            "backup patch".to_string(),
            "backup description".to_string(),
            expected_diff,
            PatchStatus::Open,
            false,
            Some(job_id.clone()),
            Vec::new(),
            RepoName::from_str("dourolabs/api")?,
            None,
        ));
        let patch_response = UpsertPatchResponse::new(patch_id("p-service"));
        let server = MockServer::start();
        let client = metis_client(&server);
        let job_mock = mock_get_job(&server, job_record.clone());
        let patch_mock = mock_create_patch(&server, expected_request, patch_response.clone());

        create_patch(
            &client,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(job_id.clone()),
            false,
            None,
            None,
            issue_id.clone(),
            commit_range,
            false,
            Some(repo_path.as_path()),
        )
        .await?;

        job_mock.assert();
        patch_mock.assert();
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
    async fn resolve_service_repo_name_errors_for_non_service_job() -> Result<()> {
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
                Default::default(),
            ),
            None,
            TaskStatusLog::from_events(Vec::new()),
        );
        let job_mock = mock_get_job(&server, job_record.clone());

        let error = resolve_service_repo_name(&client, Some(&job_id))
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not reference a service repository"),
            "error should indicate missing service repository context"
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
            false,
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
            true,
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

    #[test]
    fn ensure_feature_branch_uses_job_id_when_on_main() -> Result<()> {
        let (_tempdir, repo_path, _, _) = initialize_repo_with_changes()?;
        let branch = ensure_feature_branch(&repo_path, Some("Job 123"))?;

        assert_eq!(branch, "metis-job-123");
        assert_eq!(current_branch(&repo_path)?, branch);

        Ok(())
    }
}
