use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::Subcommand;
use metis_common::{
    constants::{ENV_GH_TOKEN, ENV_METIS_BASE_COMMIT, ENV_METIS_ID, ENV_METIS_ISSUE_ID},
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueId, IssueStatus, IssueType,
        UpsertIssueRequest,
    },
    jobs::BundleSpec,
    merge_queues::MergeQueue,
    patches::{
        GitOid, GithubPr, Patch, PatchCommitRange, PatchRecord, PatchStatus, Review,
        SearchPatchesQuery, UpsertPatchRequest, UpsertPatchResponse,
    },
    PatchId, RepoName, TaskId,
};
use octocrab::Octocrab;
use serde::Deserialize;

use crate::client::MetisClientInterface;
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

        /// Base commit for the patch range.
        #[arg(
            long = "base",
            value_name = "OID",
            required = true,
            env = ENV_METIS_BASE_COMMIT
        )]
        base: GitOid,

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
        issue_id: Option<IssueId>,
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

        /// Updated base commit for the patch.
        #[arg(long = "base", value_name = "OID")]
        base: Option<GitOid>,

        /// Updated head commit for the patch.
        #[arg(long = "head", value_name = "OID")]
        head: Option<GitOid>,
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
            base,
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
                base,
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
            base,
            head,
        } => update_patch(client, id, title, description, status, base, head).await,
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
    let mut stdout = std::io::stdout().lock();
    list_patches_with_writer(client, id, query, pretty, &mut stdout).await
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
        .list_patches(&SearchPatchesQuery { q: query })
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
    issue_id: Option<IssueId>,
    base: GitOid,
    repo_root: Option<&Path>,
) -> Result<()> {
    let repo_root = match repo_root {
        Some(path) => path.to_path_buf(),
        None => git_repository_root()?,
    };
    ensure_clean_worktree(&repo_root)?;
    let head = resolve_head_commit(&repo_root)?;
    ensure_base_is_ancestor(&repo_root, &base, &head)?;
    let commit_range = PatchCommitRange { base, head };
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
        commit_range,
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

async fn update_patch(
    client: &dyn MetisClientInterface,
    patch_id: PatchId,
    title: Option<String>,
    description: Option<String>,
    status: Option<PatchStatus>,
    base: Option<GitOid>,
    head: Option<GitOid>,
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

    let has_partial_range = base.is_some() ^ head.is_some();
    if has_partial_range {
        bail!("--base and --head must be provided together.");
    }

    let no_changes = title.is_none() && description.is_none() && status.is_none() && base.is_none();
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
    if let (Some(base), Some(head)) = (base, head) {
        updated_patch.commit_range = PatchCommitRange { base, head };
    }

    let response = client
        .update_patch(
            &patch_id,
            &UpsertPatchRequest {
                patch: updated_patch,
                job_id: None,
            },
        )
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
    let mut stdout = std::io::stdout().lock();
    merge_queue_with_writer(client, repo, branch, patch_id, pretty, &mut stdout).await
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
    parent_issue_id: Option<IssueId>,
    patch_title: String,
    patch_description: String,
) -> Result<IssueId> {
    let assignee = assignee.trim().to_string();
    if assignee.is_empty() {
        bail!("Assignee must not be empty.");
    }

    let mut dependencies = Vec::new();
    if let Some(issue_id) = parent_issue_id {
        dependencies.push(IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id,
        });
    }

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

    let response = client
        .create_issue(&UpsertIssueRequest {
            issue: Issue {
                issue_type: IssueType::MergeRequest,
                description,
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some(assignee),
                dependencies,
                patches: vec![patch_id],
            },
            job_id: None,
        })
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
    commit_range: PatchCommitRange,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    create_github_pr: bool,
    github_token: Option<&str>,
    is_automatic_backup: bool,
    service_repo_name: RepoName,
) -> Result<UpsertPatchResponse> {
    if !is_automatic_backup {
        ensure_clean_worktree(repo_root)?;
    }
    ensure_base_is_ancestor(repo_root, &commit_range.base, &commit_range.head)?;
    let diff = git_diff_for_range(repo_root, &commit_range)?;

    let title = title.trim().to_string();
    let description = description.trim().to_string();
    if title.is_empty() {
        bail!("Patch title must not be empty.");
    }
    if description.is_empty() {
        bail!("Patch description must not be empty.");
    }

    let mut patch_payload = Patch {
        title: title.clone(),
        description: description.clone(),
        commit_range,
        diff,
        status: PatchStatus::Open,
        is_automatic_backup,
        reviews: Vec::new(),
        service_repo_name: service_repo_name.clone(),
        github: None,
    };
    let response = client
        .create_patch(&UpsertPatchRequest {
            patch: patch_payload.clone(),
            job_id: job_id.clone(),
        })
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
            .update_patch(
                &response.patch_id,
                &UpsertPatchRequest {
                    patch: patch_payload,
                    job_id: None,
                },
            )
            .await
            .context("failed to update patch with GitHub metadata")?;
    }

    Ok(response)
}

fn git_repository_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("failed to find git repository root")?;

    if !output.status.success() {
        bail!("Current directory is not inside a git repository.");
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        bail!("Failed to resolve git repository root.");
    }

    Ok(PathBuf::from(root))
}

fn resolve_head_commit(repo_root: &Path) -> Result<GitOid> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD^{commit}"])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve HEAD commit")?;
    if !output.status.success() {
        bail!("failed to resolve HEAD commit");
    }

    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    GitOid::from_str(&oid).context("failed to parse HEAD commit oid")
}

fn ensure_clean_worktree(repo_root: &Path) -> Result<()> {
    if repository_has_pending_changes(repo_root)? {
        bail!("Repository has uncommitted changes. Commit them before creating a patch.");
    }

    Ok(())
}

fn repository_has_pending_changes(repo_root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .context("failed to check repository status")?;

    if !output.status.success() {
        bail!("git status failed while validating repository cleanliness");
    }

    Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
}

fn ensure_base_is_ancestor(repo_root: &Path, base: &GitOid, head: &GitOid) -> Result<()> {
    let output = Command::new("git")
        .args([
            "merge-base",
            "--is-ancestor",
            &base.to_string(),
            &head.to_string(),
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to verify patch base relationship")?;

    if output.status.success() {
        return Ok(());
    }

    match output.status.code() {
        Some(1) => bail!("Patch base {base} is not an ancestor of head {head}"),
        _ => bail!(
            "failed to validate base ancestry: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

fn git_diff_for_range(repo_root: &Path, range: &PatchCommitRange) -> Result<String> {
    let diff_range = format_commit_range(range);
    let output = Command::new("git")
        .args(["diff", &diff_range])
        .current_dir(repo_root)
        .output()
        .context("failed to compute diff for commit range")?;
    if !output.status.success() {
        bail!(
            "git diff for range {diff_range} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
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

fn format_commit_range(range: &PatchCommitRange) -> String {
    format!("{}..{}", range.base, range.head)
}

fn format_patch_status(status: PatchStatus) -> &'static str {
    match status {
        PatchStatus::Open => "open",
        PatchStatus::Closed => "closed",
        PatchStatus::Merged => "merged",
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
    let repo_root = git_repository_root().ok();
    for patch in patches {
        write_patch_record_pretty(patch, repo_root.as_deref(), writer)?;
    }
    writer.flush()?;
    Ok(())
}

fn write_patch_record_pretty(
    record: &PatchRecord,
    repo_root: Option<&Path>,
    writer: &mut impl Write,
) -> Result<()> {
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
    writeln!(
        writer,
        "Commit range: {}",
        format_commit_range(&record.patch.commit_range)
    )?;
    if !description.trim().is_empty() {
        writeln!(writer, "{description}")?;
    }
    if let Some(root) = repo_root {
        match git_diff_for_range(root, &record.patch.commit_range) {
            Ok(diff) if !diff.trim().is_empty() => {
                writeln!(writer)?;
                pretty_print_patch(&diff, writer)?;
            }
            Ok(_) => writeln!(writer, "[no diff between commits]")?,
            Err(_) => writeln!(writer, "[diff unavailable locally]")?,
        }
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
    let range = format_commit_range(&patch.commit_range);
    println!("Applying commit range {range} to current git repository...\n");

    let output = Command::new("git")
        .arg("cherry-pick")
        .arg(&range)
        .current_dir(git_root)
        .output()
        .context("Failed to execute git cherry-pick")?;

    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("git cherry-pick stderr: {stderr}");
    }

    if !output.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("git cherry-pick stdout: {stdout}");
    }

    if output.status.success() {
        println!("Commit range applied successfully.");
        return Ok(());
    }

    let conflicted_files = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(git_root)
        .output()
        .context("Failed to check for merge conflicts after applying patch")?;
    let conflicts = String::from_utf8_lossy(&conflicted_files.stdout);

    if !conflicts.trim().is_empty() {
        bail!("Merge conflicts detected while applying patch; resolve these files and continue:\n{conflicts}");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "Failed to apply commit range {range}. Exit code: {}. Error: {}",
        output.status.code().unwrap_or(-1),
        stderr
    );
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

    record.patch.reviews.push(Review {
        contents,
        is_approved: approve,
        author,
        submitted_at: Some(Utc::now()),
    });

    let response = client
        .update_patch(
            &id,
            &UpsertPatchRequest {
                patch: record.patch,
                job_id: None,
            },
        )
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
    push_branch(repo_root, &branch_name)?;
    let pr_metadata =
        open_pull_request(repo_root, title, description, &branch_name, github_token).await?;
    let (owner, repo) = parse_pr_repository(&pr_metadata.url)
        .ok_or_else(|| anyhow!("failed to parse GitHub PR URL '{}'", pr_metadata.url))?;
    Ok(GithubPr {
        owner,
        repo,
        number: pr_metadata.number,
        head_ref: pr_metadata.head_ref_name,
        base_ref: pr_metadata.base_ref_name,
        url: Some(pr_metadata.url),
        ci: None,
    })
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

fn branch_exists(repo_root: &Path, branch: &str) -> Result<bool> {
    let status = Command::new("git")
        .args(["show-ref", "--verify", &format!("refs/heads/{branch}")])
        .current_dir(repo_root)
        .status()
        .context("failed to check for existing branch")?;

    Ok(status.success())
}

fn checkout_new_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["checkout", "-b", branch])
        .current_dir(repo_root)
        .status()
        .context("failed to create feature branch for GitHub PR")?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to create branch '{branch}'");
}

fn current_branch(repo_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve current branch")?;
    if !output.status.success() {
        bail!("git rev-parse --abbrev-ref failed");
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        bail!("unable to determine current branch");
    }

    Ok(branch)
}

fn push_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(repo_root)
        .status()
        .context("failed to push branch to origin for GitHub PR")?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to push branch '{branch}' to origin");
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
    use crate::{
        client::MockMetisClient,
        test_utils::ids::{issue_id, patch_id, task_id},
    };
    use anyhow::anyhow;
    use metis_common::{
        issues::{
            IssueDependency, IssueDependencyType, IssueStatus, IssueType, UpsertIssueResponse,
        },
        jobs::{BundleSpec, JobRecord, Task},
        merge_queues::MergeQueue,
        patches::{
            GitOid, ListPatchesResponse, Patch, PatchCommitRange, PatchRecord, Review,
            SearchPatchesQuery, UpsertPatchResponse,
        },
        task_status::TaskStatusLog,
        RepoName,
    };
    use std::{fs, process::Command, str::FromStr};

    fn sample_commit_range() -> PatchCommitRange {
        PatchCommitRange {
            base: GitOid::from_str("0000000000000000000000000000000000000001").unwrap(),
            head: GitOid::from_str("0000000000000000000000000000000000000002").unwrap(),
        }
    }

    fn sample_repo_name() -> RepoName {
        RepoName::from_str("dourolabs/example").unwrap()
    }

    fn initialize_repo_with_changes(
    ) -> Result<(tempfile::TempDir, std::path::PathBuf, GitOid, GitOid)> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test repo")?;
        let repo_path = tempdir.path().to_path_buf();
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
        Command::new("git")
            .args([
                "-C",
                repo_str,
                "remote",
                "add",
                "origin",
                "https://github.com/dourolabs/example.git",
            ])
            .status()
            .context("failed to set remote origin")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git remote add returned non-zero exit code"))?;

        fs::write(repo_path.join("README.md"), "initial content\n")
            .context("failed to write initial README.md")?;
        Command::new("git")
            .args(["-C", repo_str, "add", "README.md"])
            .status()
            .context("failed to add README.md to repo")?
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

        let base_commit = GitOid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["-C", repo_str, "rev-parse", "HEAD^{commit}"])
                    .output()
                    .context("failed to resolve initial commit")?
                    .stdout,
            )
            .trim(),
        )
        .context("failed to parse initial commit oid")?;

        fs::write(repo_path.join("README.md"), "updated content\n")
            .context("failed to update README.md")?;
        fs::write(repo_path.join("notes.txt"), "new note content\n")
            .context("failed to write notes.txt")?;
        Command::new("git")
            .args(["-C", repo_str, "add", "README.md", "notes.txt"])
            .status()
            .context("failed to stage modified files")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git add returned non-zero exit code"))?;
        Command::new("git")
            .args(["-C", repo_str, "commit", "-m", "apply updates"])
            .status()
            .context("failed to commit updated files")?
            .success()
            .then_some(())
            .ok_or_else(|| anyhow!("git commit returned non-zero exit code"))?;

        let head_commit = GitOid::from_str(
            String::from_utf8_lossy(
                &Command::new("git")
                    .args(["-C", repo_str, "rev-parse", "HEAD^{commit}"])
                    .output()
                    .context("failed to resolve updated commit")?
                    .stdout,
            )
            .trim(),
        )
        .context("failed to parse updated commit oid")?;

        Ok((tempdir, repo_path, base_commit, head_commit))
    }

    #[tokio::test]
    async fn list_patches_sets_patch_filter_and_query() -> Result<()> {
        let client = MockMetisClient::default();
        client.push_list_patches_response(ListPatchesResponse { patches: vec![] });

        list_patches(&client, None, Some("login".to_string()), false).await?;

        let queries = client.recorded_list_patch_queries();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].q.as_deref(), Some("login"));
        Ok(())
    }

    #[tokio::test]
    async fn list_patches_emits_no_output_for_empty_results() -> Result<()> {
        let client = MockMetisClient::default();
        client.push_list_patches_response(ListPatchesResponse { patches: vec![] });

        let mut output = Vec::new();
        list_patches_with_writer(&client, None, None, false, &mut output).await?;

        assert!(output.is_empty());
        assert_eq!(
            client.recorded_list_patch_queries(),
            vec![SearchPatchesQuery { q: None }]
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_generates_diff_from_repo_changes() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let job_id = task_id("t-job-diff");
        let client = MockMetisClient::default();
        client.push_get_job_response(JobRecord {
            id: job_id.clone(),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            },
            notes: None,
            status_log: TaskStatusLog { events: Vec::new() },
        });
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-1"),
        });
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            Some(job_id),
            false,
            None,
            None,
            None,
            base_commit,
            Some(&repo_path),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");

        let (_, request) = &requests[0];
        let patch = &request.patch;
        let generated_title = &patch.title;
        let generated_patch = git_diff_for_range(&repo_path, &patch.commit_range)?;
        let generated_description = &patch.description;
        assert!(
            !patch.is_automatic_backup,
            "manual patch creation should not be marked as an automatic backup"
        );
        assert_eq!(patch.commit_range.base, base_commit);
        assert_eq!(patch.commit_range.head, head_commit);
        assert_eq!(
            generated_title, &patch_title,
            "expected provided title to be applied"
        );
        assert_eq!(
            generated_description, &patch_description,
            "expected provided description to be applied"
        );

        let expected_output = Command::new("git")
            .args([
                "-C",
                repo_path
                    .to_str()
                    .ok_or_else(|| anyhow!("repo path contains invalid UTF-8"))?,
                "diff",
                &format!("{base_commit}..{head_commit}"),
            ])
            .output()
            .context("failed to capture expected diff")?;
        assert!(expected_output.status.success(), "git diff failed");
        let expected_patch = String::from_utf8_lossy(&expected_output.stdout).to_string();

        assert_eq!(*generated_patch, expected_patch);

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_uses_provided_job_id() -> Result<()> {
        let (_tempdir, repo_path, base_commit, _) = initialize_repo_with_changes()?;

        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-2"),
        });
        let job_id = task_id("t-job-1234");
        client.push_get_job_response(JobRecord {
            id: job_id.clone(),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            },
            notes: None,
            status_log: TaskStatusLog { events: Vec::new() },
        });

        let title = "patch with job title".to_string();
        let job_id = Some(job_id);
        let description = "patch with job id".to_string();

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id.clone(),
            false,
            None,
            None,
            None,
            base_commit,
            Some(&repo_path),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");

        let (_, request) = &requests[0];
        assert_eq!(
            request.job_id, job_id,
            "job id should be forwarded to the patch request"
        );

        assert_eq!(request.patch.title, title);
        assert_eq!(request.patch.description, description);

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_errors_without_job_id() -> Result<()> {
        let (_tempdir, repo_path, base_commit, _) = initialize_repo_with_changes()?;
        let client = MockMetisClient::default();
        let result = create_patch(
            &client,
            "missing job".to_string(),
            "patch without job id".to_string(),
            None,
            false,
            None,
            None,
            None,
            base_commit,
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
        let (_tempdir, repo_path, base_commit, _) = initialize_repo_with_changes()?;
        let client = MockMetisClient::default();

        let result = create_patch(
            &client,
            "pr title".to_string(),
            "pr description".to_string(),
            Some(task_id("t-job-gh-token")),
            true,
            None,
            None,
            None,
            base_commit,
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
        let client = MockMetisClient::default();
        client.push_get_job_response(JobRecord {
            id: job_id.clone(),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::ServiceRepository {
                    name: sample_repo_name(),
                    rev: None,
                },
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            },
            notes: None,
            status_log: TaskStatusLog { events: Vec::new() },
        });
        let created_patch_id = patch_id("p-merge");
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: created_patch_id.clone(),
        });
        client.push_upsert_issue_response(UpsertIssueResponse {
            issue_id: issue_id("i-merge"),
        });

        let title = "custom patch title".to_string();
        let description = "custom patch description".to_string();
        let parent_issue = issue_id("i-parent");

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            Some(job_id),
            false,
            None,
            Some("owner-a".to_string()),
            Some(parent_issue.clone()),
            base_commit,
            Some(&repo_path),
        )
        .await?;

        assert!(client.recorded_get_patch_requests().is_empty());

        let issue_requests = client.recorded_issue_upserts();
        assert_eq!(issue_requests.len(), 1);
        let (issue_id, request) = &issue_requests[0];
        assert!(issue_id.is_none());
        assert_eq!(request.issue.issue_type, IssueType::MergeRequest);
        assert_eq!(request.issue.status, IssueStatus::Open);
        assert_eq!(request.issue.assignee.as_deref(), Some("owner-a"));
        assert_eq!(
            request.issue.dependencies,
            vec![IssueDependency {
                dependency_type: IssueDependencyType::ChildOf,
                issue_id: parent_issue.clone()
            }]
        );
        assert_eq!(request.issue.patches, vec![created_patch_id.clone()]);
        assert!(
            request
                .issue
                .description
                .contains(created_patch_id.as_ref()),
            "description should link to patch id"
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_artifact_marks_automatic_backup_when_requested() -> Result<()> {
        let (_tempdir, repo_path, base_commit, head_commit) = initialize_repo_with_changes()?;
        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-automatic"),
        });
        let commit_range = PatchCommitRange {
            base: base_commit,
            head: head_commit,
        };

        let _ = create_patch_artifact_from_repo(
            &client,
            &repo_path,
            commit_range,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(task_id("t-job-automatic")),
            false,
            None,
            true,
            sample_repo_name(),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");
        let (_, request) = &requests[0];
        assert!(request.patch.is_automatic_backup);
        Ok(())
    }

    #[tokio::test]
    async fn create_patch_uses_service_repo_name_from_job() -> Result<()> {
        let (_tempdir, repo_path, base_commit, _) = initialize_repo_with_changes()?;
        let client = MockMetisClient::default();
        client.push_get_job_response(JobRecord {
            id: task_id("t-job-service"),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::ServiceRepository {
                    name: RepoName::from_str("dourolabs/api")?,
                    rev: None,
                },
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            },
            notes: None,
            status_log: TaskStatusLog { events: Vec::new() },
        });
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-service"),
        });

        create_patch(
            &client,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(task_id("t-job-service")),
            false,
            None,
            None,
            None,
            base_commit,
            Some(repo_path.as_path()),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");
        let (_, request) = &requests[0];
        assert_eq!(
            request.patch.service_repo_name.to_string(),
            "dourolabs/api".to_string(),
            "service repo name should be derived from the job context"
        );
        Ok(())
    }

    #[tokio::test]
    async fn resolve_service_repo_name_requires_job_id() -> Result<()> {
        let client = MockMetisClient::default();

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
        let client = MockMetisClient::default();
        let job_id = task_id("t-job-non-service");
        client.push_get_job_response(JobRecord {
            id: job_id.clone(),
            task: Task {
                prompt: "0".to_string(),
                context: BundleSpec::GitRepository {
                    url: "https://github.com/dourolabs/example".to_string(),
                    rev: "main".to_string(),
                },
                spawned_from: None,
                image: None,
                env_vars: Default::default(),
            },
            notes: None,
            status_log: TaskStatusLog { events: Vec::new() },
        });

        let error = resolve_service_repo_name(&client, Some(&job_id))
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not reference a service repository"),
            "error should indicate missing service repository context"
        );
        Ok(())
    }

    #[tokio::test]
    async fn review_patch_appends_review() -> Result<()> {
        let client = MockMetisClient::default();
        let existing_submitted_at = Utc::now();
        let existing_review = Review {
            contents: "needs work".to_string(),
            is_approved: false,
            author: "bob".to_string(),
            submitted_at: Some(existing_submitted_at),
        };
        client.push_get_patch_response(PatchRecord {
            id: patch_id("p-123"),
            patch: Patch {
                title: "reviewed patch".to_string(),
                description: "description".to_string(),
                commit_range: sample_commit_range(),
                diff: String::new(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: vec![existing_review.clone()],
                service_repo_name: sample_repo_name(),
                github: None,
            },
        });
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-123"),
        });

        review_patch(
            &client,
            patch_id("p-123"),
            "alice".to_string(),
            "looks good now".to_string(),
            true,
        )
        .await?;

        assert_eq!(
            client.recorded_get_patch_requests(),
            vec![patch_id("p-123")]
        );
        let updates = client.recorded_patch_upserts();
        assert_eq!(updates.len(), 1, "expected one patch update");

        let (patch_id_opt, request) = &updates[0];
        assert_eq!(patch_id_opt, &Some(patch_id("p-123")));
        let reviews = &request.patch.reviews;
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[0], existing_review);
        let new_review = &reviews[1];
        assert_eq!(new_review.contents, "looks good now");
        assert!(new_review.is_approved);
        assert_eq!(new_review.author, "alice");
        assert!(
            new_review.submitted_at.is_some(),
            "new reviews should include a timestamp"
        );

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_modifies_requested_fields() -> Result<()> {
        let client = MockMetisClient::default();
        let updated_range = PatchCommitRange {
            base: GitOid::from_str("0000000000000000000000000000000000000003").unwrap(),
            head: GitOid::from_str("0000000000000000000000000000000000000004").unwrap(),
        };
        client.push_get_patch_response(PatchRecord {
            id: patch_id("p-update"),
            patch: Patch {
                title: "Initial title".to_string(),
                description: "Initial description".to_string(),
                commit_range: sample_commit_range(),
                diff: String::new(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: vec![Review {
                    contents: "looks ok".to_string(),
                    is_approved: false,
                    author: "sam".to_string(),
                    submitted_at: None,
                }],
                service_repo_name: sample_repo_name(),
                github: None,
            },
        });
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-update"),
        });

        update_patch(
            &client,
            patch_id("p-update"),
            Some("Updated title".to_string()),
            Some("Updated description".to_string()),
            Some(PatchStatus::Closed),
            Some(updated_range.base),
            Some(updated_range.head),
        )
        .await?;

        assert_eq!(
            client.recorded_get_patch_requests(),
            vec![patch_id("p-update")]
        );
        assert_eq!(
            client.recorded_patch_upserts(),
            vec![(
                Some(patch_id("p-update")),
                UpsertPatchRequest {
                    patch: Patch {
                        title: "Updated title".to_string(),
                        description: "Updated description".to_string(),
                        commit_range: updated_range,
                        diff: String::new(),
                        status: PatchStatus::Closed,
                        is_automatic_backup: false,
                        reviews: vec![Review {
                            contents: "looks ok".to_string(),
                            is_approved: false,
                            author: "sam".to_string(),
                            submitted_at: None,
                        }],
                        service_repo_name: sample_repo_name(),
                        github: None,
                    },
                    job_id: None,
                }
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_requires_full_commit_range() -> Result<()> {
        let client = MockMetisClient::default();

        let result = update_patch(
            &client,
            patch_id("p-file"),
            None,
            None,
            Some(PatchStatus::Merged),
            Some(sample_commit_range().base),
            None,
        )
        .await;

        assert!(
            result.is_err(),
            "expected error when only one commit provided"
        );
        assert!(
            client.recorded_get_patch_requests().is_empty(),
            "patch should not be fetched when commit range is incomplete"
        );

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_rejects_empty_updates() {
        let client = MockMetisClient::default();
        let result = update_patch(&client, patch_id("p-empty"), None, None, None, None, None).await;

        assert!(result.is_err(), "expected update to reject empty payload");
        assert!(
            client.recorded_get_patch_requests().is_empty(),
            "patch should not be fetched when no fields provided"
        );
    }

    #[tokio::test]
    async fn merge_queue_fetches_queue_and_writes_json() -> Result<()> {
        let client = MockMetisClient::default();
        let repo = sample_repo_name();
        let branch = "main".to_string();
        let queued_patch = patch_id("p-queue-001");
        client.push_merge_queue_response(MergeQueue {
            patches: vec![queued_patch.clone()],
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

        assert_eq!(
            client.recorded_merge_queue_requests(),
            vec![(repo, branch.clone())]
        );
        assert_eq!(
            String::from_utf8(output)?,
            format!(
                "{}\n",
                serde_json::to_string(&MergeQueue {
                    patches: vec![queued_patch]
                })?
            )
        );

        Ok(())
    }

    #[tokio::test]
    async fn merge_queue_enqueues_patch_and_pretty_prints() -> Result<()> {
        let client = MockMetisClient::default();
        let repo = sample_repo_name();
        let branch = "feature".to_string();
        let patch = patch_id("p-queue-002");
        client.push_enqueue_merge_queue_response(MergeQueue {
            patches: vec![patch.clone()],
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

        assert_eq!(
            client.recorded_enqueue_merge_queue_requests(),
            vec![(repo.clone(), branch.clone(), patch.clone())]
        );
        assert!(
            client.recorded_merge_queue_requests().is_empty(),
            "enqueue should not call fetch endpoint"
        );
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
