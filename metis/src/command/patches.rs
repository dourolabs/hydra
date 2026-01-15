use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use clap::Subcommand;
use metis_common::{
    constants::{ENV_METIS_ID, ENV_METIS_ISSUE_ID},
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueId, IssueStatus, IssueType,
        UpsertIssueRequest,
    },
    patches::{
        GithubPr, Patch, PatchRecord, PatchStatus, Review, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    PatchId, TaskId,
};
use serde::Deserialize;

use crate::{client::MetisClientInterface, command::worker_run::create_patch_from_repo, constants};
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

        /// Pretty-print the matching patch diffs with color coding.
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
        #[arg(long = "job", value_name = "METIS_ID", env = "METIS_ID")]
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

        /// Updated diff contents for the patch.
        #[arg(long = "diff", value_name = "DIFF", conflicts_with = "diff_file")]
        diff: Option<String>,

        /// Path to a file containing the updated patch diff.
        #[arg(long = "diff-file", value_name = "PATH", conflicts_with = "diff")]
        diff_file: Option<PathBuf>,
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
            assignee,
            issue_id,
        } => {
            create_patch(
                client,
                title,
                description,
                job,
                github,
                assignee,
                issue_id,
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
            diff,
            diff_file,
        } => update_patch(client, id, title, description, status, diff, diff_file).await,
    }
}

async fn list_patches(
    client: &dyn MetisClientInterface,
    id: Option<PatchId>,
    query: Option<String>,
    pretty: bool,
) -> Result<()> {
    if let Some(id) = id {
        if query.is_some() {
            bail!("--id and --query cannot be combined");
        }

        let patch_record = client
            .get_patch(&id)
            .await
            .with_context(|| format!("failed to fetch patch '{id}'"))?;
        if pretty {
            print_patch_record(&patch_record)?;
        } else {
            println!("{}", serde_json::to_string(&patch_record)?);
        }
        return Ok(());
    }

    let response = client
        .list_patches(&SearchPatchesQuery { q: query })
        .await
        .context("failed to search for patches")?;

    if response.patches.is_empty() {
        eprintln!("No patches found.");
        return Ok(());
    }

    for patch_record in response.patches {
        if pretty {
            print_patch_record(&patch_record)?;
        } else {
            println!("{}", serde_json::to_string(&patch_record)?);
        }
    }

    Ok(())
}

async fn create_patch(
    client: &dyn MetisClientInterface,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    create_github_pr: bool,
    assignee: Option<String>,
    issue_id: Option<IssueId>,
    repo_root: Option<&Path>,
) -> Result<()> {
    let job_id = resolve_job_id(job_id)?;
    let issue_id = resolve_issue_id(issue_id)?;
    let repo_root = match repo_root {
        Some(path) => path.to_path_buf(),
        None => git_repository_root()?,
    };
    let is_automatic_backup = false;
    let patch_title = title.clone();
    let patch_description = description.clone();
    let response = create_patch_artifact_from_repo(
        client,
        &repo_root,
        patch_title,
        patch_description,
        job_id.clone(),
        create_github_pr,
        is_automatic_backup,
    )
    .await?
    .ok_or_else(|| anyhow!("No changes detected. Make edits before creating a patch artifact."))?;

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
    diff: Option<String>,
    diff_file: Option<PathBuf>,
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

    let diff = match (diff, diff_file) {
        (Some(inline), None) => {
            if inline.trim().is_empty() {
                bail!("Patch diff must not be empty.");
            }
            Some(inline)
        }
        (None, Some(path)) => {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read diff from '{}'", path.display()))?;
            if contents.trim().is_empty() {
                bail!("Patch diff must not be empty.");
            }
            Some(contents)
        }
        (None, None) => None,
        _ => unreachable!("clap should enforce diff/diff-file exclusivity"),
    };

    let no_changes = title.is_none() && description.is_none() && status.is_none() && diff.is_none();
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
    if let Some(diff) = diff {
        updated_patch.diff = diff;
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

fn resolve_job_id(job_id: Option<TaskId>) -> Result<Option<TaskId>> {
    if job_id.is_some() {
        return Ok(job_id);
    }

    let env_value = match env::var(ENV_METIS_ID) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let trimmed = env_value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let task_id = TaskId::from_str(trimmed)
        .with_context(|| format!("invalid {ENV_METIS_ID} value '{trimmed}'"))?;
    Ok(Some(task_id))
}

fn resolve_issue_id(issue_id: Option<IssueId>) -> Result<Option<IssueId>> {
    if issue_id.is_some() {
        return Ok(issue_id);
    }

    let env_value = match env::var(ENV_METIS_ISSUE_ID) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let trimmed = env_value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let issue_id = IssueId::from_str(trimmed)
        .with_context(|| format!("invalid {ENV_METIS_ISSUE_ID} value '{trimmed}'"))?;
    Ok(Some(issue_id))
}

pub async fn create_patch_artifact_from_repo(
    client: &dyn MetisClientInterface,
    repo_root: &Path,
    title: String,
    description: String,
    job_id: Option<TaskId>,
    create_github_pr: bool,
    is_automatic_backup: bool,
) -> Result<Option<UpsertPatchResponse>> {
    let patch = create_patch_from_repo(repo_root)?;
    if patch.trim().is_empty() {
        return Ok(None);
    }

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
        diff: patch,
        description: description.clone(),
        status: PatchStatus::Open,
        is_automatic_backup,
        reviews: Vec::new(),
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
            job_id.as_ref().map(|id| id.as_ref()),
        )?;
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

    Ok(Some(response))
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

fn extract_patch_title(record: &PatchRecord) -> &str {
    &record.patch.title
}

fn extract_patch_status(record: &PatchRecord) -> PatchStatus {
    record.patch.status
}

fn extract_patch_diff(record: &PatchRecord) -> &str {
    &record.patch.diff
}

fn extract_patch_description(record: &PatchRecord) -> &str {
    &record.patch.description
}

fn format_patch_status(status: PatchStatus) -> &'static str {
    match status {
        PatchStatus::Open => "open",
        PatchStatus::Closed => "closed",
        PatchStatus::Merged => "merged",
    }
}

fn print_patch_record(record: &PatchRecord) -> Result<()> {
    let diff = extract_patch_diff(record);
    let title = extract_patch_title(record);
    let status = extract_patch_status(record);
    let description = extract_patch_description(record);
    println!(
        "Patch {} [{}]: {}",
        record.id,
        format_patch_status(status),
        title
    );
    if !description.trim().is_empty() {
        println!("{description}");
    }
    println!();
    pretty_print_patch(diff);
    println!();
    Ok(())
}

/// Pretty-print a patch with color coding (green for additions, red for deletions).
fn pretty_print_patch(patch: &str) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    for line in patch.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            writeln!(handle, "{GREEN}{line}{RESET}").unwrap();
        } else if line.starts_with('-') && !line.starts_with("---") {
            writeln!(handle, "{RED}{line}{RESET}").unwrap();
        } else {
            writeln!(handle, "{line}").unwrap();
        }
    }
}

async fn apply_patch_record(client: &dyn MetisClientInterface, id: PatchId) -> Result<()> {
    let patch_record = client
        .get_patch(&id)
        .await
        .with_context(|| format!("failed to fetch patch '{id}'"))?;
    let diff = extract_patch_diff(&patch_record);
    let repo_root = git_repository_root()?;

    apply_patch_to_repo(diff, &repo_root)?;
    Ok(())
}

fn apply_patch_to_repo(patch: &str, git_root: &Path) -> Result<()> {
    if patch.trim().is_empty() {
        bail!("Patch is empty. Nothing to apply.");
    }

    println!("Applying patch to current git repository...\n");
    pretty_print_patch(patch);

    let patch_file = NamedTempFile::new().context("Failed to create temporary file for patch")?;
    fs::write(patch_file.path(), patch).context("Failed to write patch to temporary file")?;

    let output = Command::new("git")
        .arg("apply")
        .args(["--3way", "--index"])
        .arg(patch_file.path())
        .current_dir(git_root)
        .output()
        .context("Failed to execute git apply with 3-way merge")?;

    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("git apply stderr: {stderr}");
    }

    if !output.stdout.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("git apply stdout: {stdout}");
    }

    if output.status.success() {
        println!("Patch applied successfully.");
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
        "Failed to apply patch. Exit code: {}. Error: {}",
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

fn create_github_pull_request(
    repo_root: &Path,
    title: &str,
    description: &str,
    job_id: Option<&str>,
) -> Result<GithubPr> {
    let branch_name = ensure_feature_branch(repo_root, job_id)?;
    stage_changes_for_pr(repo_root)?;
    ensure_staged_changes(repo_root)?;
    commit_changes(repo_root, title)?;
    push_branch(repo_root, &branch_name)?;
    let pr_metadata = open_pull_request(repo_root, title, description, &branch_name)?;
    let (owner, repo) = parse_pr_repository(&pr_metadata.url)
        .ok_or_else(|| anyhow!("failed to parse GitHub PR URL '{}'", pr_metadata.url))?;
    Ok(GithubPr {
        owner,
        repo,
        number: pr_metadata.number,
        head_ref: pr_metadata.head_ref_name,
        base_ref: pr_metadata.base_ref_name,
        url: Some(pr_metadata.url),
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

fn stage_changes_for_pr(repo_root: &Path) -> Result<()> {
    let add_status = Command::new("git")
        .arg("add")
        .args(["-A", "--", "."])
        .current_dir(repo_root)
        .status()
        .context("failed to stage changes for GitHub PR")?;

    if !add_status.success() {
        bail!("failed to stage changes for GitHub PR");
    }

    let reset_status = Command::new("git")
        .args(["reset", "-q", "--", constants::METIS_DIR])
        .current_dir(repo_root)
        .status()
        .context("failed to exclude .metis directory from GitHub PR staging")?;

    if reset_status.success() {
        return Ok(());
    }

    bail!("failed to exclude .metis directory from GitHub PR staging");
}

fn ensure_staged_changes(repo_root: &Path) -> Result<()> {
    let status = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(repo_root)
        .status()
        .context("failed to check staged changes")?;

    match status.code() {
        Some(0) => bail!("No staged changes to commit for GitHub PR"),
        Some(1) => Ok(()),
        _ => bail!("failed to check staged changes before committing"),
    }
}

fn commit_changes(repo_root: &Path, title: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["commit", "-m", title])
        .current_dir(repo_root)
        .status()
        .context("failed to commit changes for GitHub PR")?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to commit changes for GitHub PR");
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

fn open_pull_request(
    repo_root: &Path,
    title: &str,
    description: &str,
    branch: &str,
) -> Result<GhPrCreateResponse> {
    let mut body_file = NamedTempFile::new().context("failed to create temporary PR body file")?;
    writeln!(body_file, "{description}").context("failed to write pull request description")?;

    let output = Command::new("gh")
        .args(["pr", "create"])
        .args(["--head", branch])
        .args(["--title", title])
        .args(["--body-file"])
        .arg(body_file.path())
        .args(["--json", "url,number,headRefName,baseRefName"])
        .current_dir(repo_root)
        .output()
        .context("failed to create GitHub pull request")?;

    if output.status.success() {
        let parsed = serde_json::from_slice::<GhPrCreateResponse>(&output.stdout)
            .context("failed to decode GitHub pull request metadata")?;
        return Ok(parsed);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("failed to open GitHub pull request for branch '{branch}': {stderr}{stdout}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MockMetisClient,
        constants,
        test_utils::ids::{issue_id, patch_id, task_id},
    };
    use anyhow::anyhow;
    use metis_common::issues::{
        IssueDependency, IssueDependencyType, IssueStatus, IssueType, UpsertIssueResponse,
    };
    use metis_common::patches::{
        ListPatchesResponse, Patch, PatchRecord, Review, UpsertPatchResponse,
    };
    use std::{env, fs, process::Command};

    fn initialize_repo_with_changes() -> Result<(tempfile::TempDir, std::path::PathBuf)> {
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

        fs::write(repo_path.join("README.md"), "updated content\n")
            .context("failed to update README.md")?;
        fs::write(repo_path.join("notes.txt"), "new note content\n")
            .context("failed to write notes.txt")?;
        let metis_internal = repo_path
            .join(constants::METIS_DIR)
            .join("should_be_ignored.txt");
        if let Some(parent) = metis_internal.parent() {
            fs::create_dir_all(parent).context("failed to create .metis directory")?;
        }
        fs::write(&metis_internal, "ignore me").context("failed to write .metis file")?;

        Ok((tempdir, repo_path))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
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
    async fn create_patch_generates_diff_from_repo_changes() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;

        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-1"),
        });
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            None,
            false,
            None,
            None,
            Some(&repo_path),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");

        let (_, request) = &requests[0];
        let patch = &request.patch;
        let generated_title = &patch.title;
        let generated_patch = &patch.diff;
        let generated_description = &patch.description;
        assert!(
            !patch.is_automatic_backup,
            "manual patch creation should not be marked as an automatic backup"
        );
        assert_eq!(
            generated_title, &patch_title,
            "expected provided title to be applied"
        );
        assert_eq!(
            generated_description, &patch_description,
            "expected provided description to be applied"
        );

        let add_status = Command::new("git")
            .args([
                "add",
                "-A",
                "--",
                ".",
                &format!(":!{}/**", constants::METIS_DIR),
            ])
            .current_dir(&repo_path)
            .status()
            .context("failed to stage changes for expected diff")?;
        assert!(
            add_status.success(),
            "git add for expected diff returned non-zero exit code"
        );

        let expected_output = Command::new("git")
            .args([
                "diff",
                "--cached",
                "--",
                ".",
                &format!(":!{}/**", constants::METIS_DIR),
            ])
            .current_dir(&repo_path)
            .output()
            .context("failed to capture expected diff")?;
        assert!(
            expected_output.status.success() || expected_output.status.code() == Some(1),
            "git diff failed with status {:?}",
            expected_output.status.code()
        );
        let expected_patch = String::from_utf8_lossy(&expected_output.stdout).to_string();

        assert_eq!(
            *generated_patch, expected_patch,
            "generated patch does not match repository changes"
        );
        assert!(
            !generated_patch.contains(constants::METIS_DIR),
            "patch should not include files under {}",
            constants::METIS_DIR
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_uses_provided_job_id() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;

        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-2"),
        });

        let title = "patch with job title".to_string();
        let job_id = Some(task_id("t-job-1234"));
        let description = "patch with job id".to_string();

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id.clone(),
            false,
            None,
            None,
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
    async fn create_patch_reads_job_id_from_environment() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let env_job_id = task_id("t-job-from-env");
        let env_job_value = env_job_id.to_string();
        let _guard = EnvVarGuard::set(ENV_METIS_ID, &env_job_value);

        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-3"),
        });

        let title = "patch with env title".to_string();
        let description = "patch with env job id".to_string();

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            None,
            false,
            None,
            None,
            Some(&repo_path),
        )
        .await?;

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");

        let (_, request) = &requests[0];
        assert_eq!(request.job_id, Some(env_job_id.clone()));

        assert_eq!(request.patch.title, title);
        assert_eq!(request.patch.description, description);

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_creates_merge_request_issue_when_assignee_provided() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;

        let client = MockMetisClient::default();
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
            None,
            false,
            Some("owner-a".to_string()),
            Some(parent_issue.clone()),
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
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let client = MockMetisClient::default();
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-automatic"),
        });

        create_patch_artifact_from_repo(
            &client,
            &repo_path,
            "backup patch".to_string(),
            "backup description".to_string(),
            Some(task_id("t-job-automatic")),
            false,
            true,
        )
        .await?
        .expect("patch should be created for repository changes");

        let requests = client.recorded_patch_upserts();
        assert_eq!(requests.len(), 1, "expected one patch upsert");
        let (_, request) = &requests[0];
        assert!(request.patch.is_automatic_backup);
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
                diff: "diff --git a/file b/file\n+example".to_string(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: vec![existing_review.clone()],
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
        client.push_get_patch_response(PatchRecord {
            id: patch_id("p-update"),
            patch: Patch {
                title: "Initial title".to_string(),
                description: "Initial description".to_string(),
                diff: "diff --git a/file b/file\n+old".to_string(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: vec![Review {
                    contents: "looks ok".to_string(),
                    is_approved: false,
                    author: "sam".to_string(),
                    submitted_at: None,
                }],
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
            Some("diff --git a/file b/file\n+new".to_string()),
            None,
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
                        diff: "diff --git a/file b/file\n+new".to_string(),
                        status: PatchStatus::Closed,
                        is_automatic_backup: false,
                        reviews: vec![Review {
                            contents: "looks ok".to_string(),
                            is_approved: false,
                            author: "sam".to_string(),
                            submitted_at: None,
                        }],
                        github: None,
                    },
                    job_id: None,
                }
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn update_patch_reads_diff_from_file() -> Result<()> {
        let client = MockMetisClient::default();
        client.push_get_patch_response(PatchRecord {
            id: patch_id("p-file"),
            patch: Patch {
                title: "Title".to_string(),
                description: "Description".to_string(),
                diff: "diff --git a/file b/file\n+old".to_string(),
                status: PatchStatus::Open,
                is_automatic_backup: false,
                reviews: vec![],
                github: None,
            },
        });
        client.push_upsert_patch_response(UpsertPatchResponse {
            patch_id: patch_id("p-file"),
        });

        let tempdir = tempfile::tempdir().context("failed to create tempdir for diff file")?;
        let diff_path = tempdir.path().join("patch.diff");
        let updated_diff = "diff --git a/file b/file\n+from file";
        fs::write(&diff_path, updated_diff).context("failed to write diff file")?;

        update_patch(
            &client,
            patch_id("p-file"),
            None,
            None,
            Some(PatchStatus::Merged),
            None,
            Some(diff_path.clone()),
        )
        .await?;

        let upserts = client.recorded_patch_upserts();
        assert_eq!(upserts.len(), 1, "expected one patch update");
        let (id, request) = &upserts[0];
        assert_eq!(id, &Some(patch_id("p-file")));
        assert_eq!(request.patch.diff, updated_diff);
        assert_eq!(request.patch.status, PatchStatus::Merged);

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

    #[test]
    fn stage_changes_for_pr_keeps_metis_directory() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let repo_str = repo_path
            .to_str()
            .ok_or_else(|| anyhow!("tempdir path contains invalid UTF-8"))?;

        fs::write(
            repo_path.join(".gitignore"),
            format!("{}/\n", constants::METIS_DIR),
        )
        .context("failed to write .gitignore for test repo")?;

        stage_changes_for_pr(&repo_path)?;

        assert!(
            repo_path.join(constants::METIS_DIR).exists(),
            ".metis directory should remain after staging PR changes"
        );

        let staged_output = Command::new("git")
            .args(["-C", repo_str, "diff", "--cached", "--name-only"])
            .output()
            .context("failed to read staged changes")?;
        let staged_paths = String::from_utf8_lossy(&staged_output.stdout);
        assert!(
            staged_paths.contains("README.md"),
            "expected README.md to be staged for PR creation: {staged_paths}"
        );
        assert!(
            !staged_paths
                .lines()
                .any(|line| line.starts_with(constants::METIS_DIR)),
            ".metis contents should not be staged for PR creation: {staged_paths}"
        );

        Ok(())
    }

    #[test]
    fn ensure_feature_branch_uses_job_id_when_on_main() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let branch = ensure_feature_branch(&repo_path, Some("Job 123"))?;

        assert_eq!(branch, "metis-job-123");
        assert_eq!(current_branch(&repo_path)?, branch);

        Ok(())
    }
}
