use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use clap::{builder::NonEmptyStringValueParser, Subcommand};
use metis_common::{
    artifacts::{
        Artifact, ArtifactKind, ArtifactRecord, Review, SearchArtifactsQuery, UpsertArtifactRequest,
    },
    constants::ENV_METIS_ID,
    MetisId,
};

use crate::{client::MetisClientInterface, command::worker_run::create_patch_from_repo, constants};
use tempfile::NamedTempFile;

/// ANSI color codes
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

#[derive(Subcommand, Debug)]
pub enum PatchesCommand {
    /// List or search patch artifacts.
    List {
        /// Patch artifact id to retrieve.
        #[arg(long = "id", value_name = "PATCH_ID")]
        id: Option<MetisId>,

        /// Query string to filter patch artifacts.
        #[arg(long = "query", value_name = "QUERY")]
        query: Option<String>,

        /// Pretty-print the matching patch diffs with color coding.
        #[arg(long = "pretty")]
        pretty: bool,
    },

    /// Create a patch artifact from the current git repository.
    Create {
        /// Title for the patch artifact.
        #[arg(long = "title", value_name = "TITLE", required = true)]
        title: String,

        /// Description for the patch artifact.
        #[arg(long = "description", value_name = "DESCRIPTION", required = true)]
        description: String,

        /// Associate the patch with a Metis job.
        #[arg(
            long = "job",
            value_name = "METIS_ID",
            env = "METIS_ID",
            value_parser = NonEmptyStringValueParser::new()
        )]
        job: Option<MetisId>,

        /// Create a GitHub pull request with the patch contents.
        #[arg(long = "github")]
        github: bool,
    },

    /// Apply a patch artifact to the current git repository.
    Apply {
        /// Patch artifact id to apply.
        #[arg(value_name = "PATCH_ID")]
        id: MetisId,
    },

    /// Add a review to an existing patch artifact.
    Review {
        /// Patch artifact id to review.
        #[arg(value_name = "PATCH_ID")]
        id: MetisId,

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
}

pub async fn run(client: &dyn MetisClientInterface, command: PatchesCommand) -> Result<()> {
    match command {
        PatchesCommand::List { id, query, pretty } => list_patches(client, id, query, pretty).await,
        PatchesCommand::Create {
            title,
            description,
            job,
            github,
        } => create_patch(client, title, description, job, github).await,
        PatchesCommand::Apply { id } => apply_patch_artifact(client, id).await,
        PatchesCommand::Review {
            id,
            author,
            contents,
            approve,
        } => review_patch(client, id, author, contents, approve).await,
    }
}

async fn list_patches(
    client: &dyn MetisClientInterface,
    id: Option<MetisId>,
    query: Option<String>,
    pretty: bool,
) -> Result<()> {
    if let Some(id) = id {
        if query.is_some() {
            bail!("--id and --query cannot be combined");
        }

        let artifact = client
            .get_artifact(&id)
            .await
            .with_context(|| format!("failed to fetch patch artifact '{id}'"))?;
        ensure_patch(&artifact, &id)?;
        if pretty {
            print_patch_artifact(&artifact)?;
        } else {
            println!("{}", serde_json::to_string(&artifact)?);
        }
        return Ok(());
    }

    let response = client
        .list_artifacts(&SearchArtifactsQuery {
            artifact_type: Some(ArtifactKind::Patch),
            issue_type: None,
            status: None,
            assignee: None,
            q: query,
        })
        .await
        .context("failed to search for patch artifacts")?;

    if response.artifacts.is_empty() {
        eprintln!("No patch artifacts found.");
        return Ok(());
    }

    for artifact in response.artifacts {
        if pretty {
            print_patch_artifact(&artifact)?;
        } else {
            println!("{}", serde_json::to_string(&artifact)?);
        }
    }

    Ok(())
}

async fn create_patch(
    client: &dyn MetisClientInterface,
    title: String,
    description: String,
    job_id: Option<MetisId>,
    create_github_pr: bool,
) -> Result<()> {
    let title = title.trim().to_string();
    let description = description.trim().to_string();
    if title.is_empty() {
        bail!("Patch title must not be empty.");
    }
    if description.is_empty() {
        bail!("Patch description must not be empty.");
    }

    let job_id = job_id
        .or_else(|| env::var(ENV_METIS_ID).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let repo_root = git_repository_root()?;
    let patch = create_patch_from_repo(&repo_root)?;
    if patch.trim().is_empty() {
        bail!("No changes detected. Make edits before creating a patch artifact.");
    }

    let response = client
        .create_artifact(&UpsertArtifactRequest {
            artifact: Artifact::Patch {
                title: title.clone(),
                diff: patch,
                description: description.clone(),
                reviews: Vec::new(),
            },
            job_id: job_id.clone(),
        })
        .await
        .context("failed to create patch artifact")?;

    if create_github_pr {
        create_github_pull_request(&repo_root, &title, &description, job_id.as_deref())?;
    }

    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "artifact_id": response.artifact_id,
            "type": "patch"
        }))?
    );

    Ok(())
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

fn ensure_patch(record: &ArtifactRecord, id: &str) -> Result<()> {
    match &record.artifact {
        Artifact::Patch { .. } => Ok(()),
        _ => bail!("artifact '{id}' is not a patch"),
    }
}

fn extract_patch_title<'a>(record: &'a ArtifactRecord, id: &str) -> Result<&'a str> {
    match &record.artifact {
        Artifact::Patch { title, .. } => Ok(title),
        _ => bail!("artifact '{id}' is not a patch"),
    }
}

fn extract_patch_diff<'a>(record: &'a ArtifactRecord, id: &str) -> Result<&'a str> {
    match &record.artifact {
        Artifact::Patch { diff, .. } => Ok(diff),
        _ => bail!("artifact '{id}' is not a patch"),
    }
}

fn extract_patch_description<'a>(record: &'a ArtifactRecord, id: &str) -> Result<&'a str> {
    match &record.artifact {
        Artifact::Patch { description, .. } => Ok(description),
        _ => bail!("artifact '{id}' is not a patch"),
    }
}

fn print_patch_artifact(record: &ArtifactRecord) -> Result<()> {
    let diff = extract_patch_diff(record, &record.id)?;
    let title = extract_patch_title(record, &record.id)?;
    let description = extract_patch_description(record, &record.id)?;
    println!("Patch {}: {}", record.id, title);
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

async fn apply_patch_artifact(client: &dyn MetisClientInterface, id: MetisId) -> Result<()> {
    let artifact = client
        .get_artifact(&id)
        .await
        .with_context(|| format!("failed to fetch patch artifact '{id}'"))?;
    let diff = extract_patch_diff(&artifact, &id)?;
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
    id: MetisId,
    author: String,
    contents: String,
    approve: bool,
) -> Result<()> {
    let trimmed_id = id.trim();
    if trimmed_id.is_empty() {
        bail!("Patch id must not be empty.");
    }
    let patch_id = trimmed_id.to_string();

    let author = author.trim().to_string();
    if author.is_empty() {
        bail!("Author must not be empty.");
    }
    let contents = contents.trim().to_string();
    if contents.is_empty() {
        bail!("Review contents must not be empty.");
    }

    let artifact = client
        .get_artifact(&patch_id)
        .await
        .with_context(|| format!("failed to fetch patch artifact '{patch_id}'"))?;

    let updated_patch = match artifact.artifact {
        Artifact::Patch {
            title,
            description,
            diff,
            mut reviews,
        } => {
            reviews.push(Review {
                contents,
                is_approved: approve,
                author,
            });
            Artifact::Patch {
                title,
                description,
                diff,
                reviews,
            }
        }
        _ => bail!("artifact '{patch_id}' is not a patch"),
    };

    let response = client
        .update_artifact(
            &patch_id,
            &UpsertArtifactRequest {
                artifact: updated_patch,
                job_id: None,
            },
        )
        .await
        .with_context(|| format!("failed to update patch '{patch_id}' with review"))?;

    println!("{}", response.artifact_id);
    Ok(())
}

fn create_github_pull_request(
    repo_root: &Path,
    title: &str,
    description: &str,
    job_id: Option<&str>,
) -> Result<()> {
    let branch_name = ensure_feature_branch(repo_root, job_id)?;
    stage_changes_for_pr(repo_root)?;
    ensure_staged_changes(repo_root)?;
    commit_changes(repo_root, title)?;
    push_branch(repo_root, &branch_name)?;
    open_pull_request(repo_root, title, description, &branch_name)?;
    Ok(())
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

fn open_pull_request(repo_root: &Path, title: &str, description: &str, branch: &str) -> Result<()> {
    let mut body_file = NamedTempFile::new().context("failed to create temporary PR body file")?;
    writeln!(body_file, "{description}").context("failed to write pull request description")?;

    let status = Command::new("gh")
        .args(["pr", "create"])
        .args(["--head", branch])
        .args(["--title", title])
        .args(["--body-file"])
        .arg(body_file.path())
        .current_dir(repo_root)
        .status()
        .context("failed to create GitHub pull request")?;

    if status.success() {
        return Ok(());
    }

    bail!("failed to open GitHub pull request for branch '{branch}'");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{client::MockMetisClient, constants};
    use anyhow::anyhow;
    use metis_common::artifacts::{
        Artifact, ArtifactRecord, IssueStatus, IssueType, ListArtifactsResponse, Review,
        UpsertArtifactResponse,
    };
    use std::{env, fs, path::PathBuf, process::Command};

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

    struct WorkingDirGuard {
        original_dir: PathBuf,
    }

    impl WorkingDirGuard {
        fn change_to(path: &Path) -> Result<Self> {
            let original_dir =
                env::current_dir().context("failed to capture current working directory")?;
            env::set_current_dir(path)
                .with_context(|| format!("failed to change to {}", path.display()))?;
            Ok(Self { original_dir })
        }
    }

    impl Drop for WorkingDirGuard {
        fn drop(&mut self) {
            if let Err(error) = env::set_current_dir(&self.original_dir) {
                eprintln!("failed to restore working directory: {error}");
            }
        }
    }

    #[tokio::test]
    async fn list_patches_sets_patch_filter_and_query() -> Result<()> {
        let client = MockMetisClient::default();
        client.push_list_artifacts_response(ListArtifactsResponse { artifacts: vec![] });

        list_patches(&client, None, Some("login".to_string()), false).await?;

        let queries = client.recorded_list_artifacts_queries();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].artifact_type, Some(ArtifactKind::Patch));
        assert_eq!(queries[0].q.as_deref(), Some("login"));
        Ok(())
    }

    #[tokio::test]
    async fn list_patches_errors_when_id_is_not_patch() {
        let client = MockMetisClient::default();
        client.push_get_artifact_response(ArtifactRecord {
            id: "artifact-1".to_string(),
            artifact: Artifact::Issue {
                issue_type: IssueType::Task,
                description: "not a patch".to_string(),
                status: IssueStatus::Open,
                assignee: None,
                dependencies: vec![],
            },
            is_ready: None,
        });

        let err = list_patches(&client, Some("artifact-1".to_string()), None, false)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("is not a patch"),
            "expected patch type error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_patch_generates_diff_from_repo_changes() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let _working_dir_guard = WorkingDirGuard::change_to(&repo_path)?;

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-1".to_string(),
        });
        let patch_title = "custom patch title".to_string();
        let patch_description = "custom patch description".to_string();
        create_patch(
            &client,
            patch_title.clone(),
            patch_description.clone(),
            None,
            false,
        )
        .await?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        let (generated_title, generated_patch, generated_description) = match &request.artifact {
            Artifact::Patch {
                title,
                diff,
                description,
                ..
            } => (title, diff, description),
            other => panic!("expected patch artifact, got {other:?}"),
        };
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
        let _working_dir_guard = WorkingDirGuard::change_to(&repo_path)?;

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-2".to_string(),
        });

        let title = "patch with job title".to_string();
        let job_id = Some("job-1234".to_string());
        let description = "patch with job id".to_string();

        create_patch(
            &client,
            title.clone(),
            description.clone(),
            job_id.clone(),
            false,
        )
        .await?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        assert_eq!(
            request.job_id, job_id,
            "job id should be forwarded to the artifact request"
        );

        match &request.artifact {
            Artifact::Patch {
                title: recorded_title,
                description: recorded_description,
                ..
            } => {
                assert_eq!(recorded_title, &title);
                assert_eq!(recorded_description, &description);
            }
            other => panic!("expected patch artifact, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_reads_job_id_from_environment() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let _working_dir_guard = WorkingDirGuard::change_to(&repo_path)?;
        let _guard = EnvVarGuard::set(ENV_METIS_ID, "job-from-env");

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-3".to_string(),
        });

        let title = "patch with env title".to_string();
        let description = "patch with env job id".to_string();

        create_patch(&client, title.clone(), description.clone(), None, false).await?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        assert_eq!(
            request.job_id.as_deref(),
            Some("job-from-env"),
            "job id should default from the METIS_ID environment variable"
        );

        match &request.artifact {
            Artifact::Patch {
                title: recorded_title,
                description: recorded_description,
                ..
            } => {
                assert_eq!(recorded_title, &title);
                assert_eq!(recorded_description, &description);
            }
            other => panic!("expected patch artifact, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn review_patch_appends_review() -> Result<()> {
        let client = MockMetisClient::default();
        let existing_review = Review {
            contents: "needs work".to_string(),
            is_approved: false,
            author: "bob".to_string(),
        };
        client.push_get_artifact_response(ArtifactRecord {
            id: "patch-123".to_string(),
            artifact: Artifact::Patch {
                title: "reviewed patch".to_string(),
                description: "description".to_string(),
                diff: "diff --git a/file b/file\n+example".to_string(),
                reviews: vec![existing_review.clone()],
            },
            is_ready: None,
        });
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-123".to_string(),
        });

        review_patch(
            &client,
            "patch-123".to_string(),
            "alice".to_string(),
            "looks good now".to_string(),
            true,
        )
        .await?;

        assert_eq!(
            client.recorded_get_artifact_requests(),
            vec!["patch-123".to_string()]
        );
        let updates = client.recorded_artifact_upserts();
        assert_eq!(updates.len(), 1, "expected one artifact update");

        let (artifact_id, request) = &updates[0];
        assert_eq!(artifact_id.as_deref(), Some("patch-123"));
        match &request.artifact {
            Artifact::Patch { reviews, .. } => {
                assert_eq!(reviews.len(), 2);
                assert_eq!(reviews[0], existing_review);
                assert_eq!(
                    reviews[1],
                    Review {
                        contents: "looks good now".to_string(),
                        is_approved: true,
                        author: "alice".to_string(),
                    }
                );
            }
            other => panic!("expected patch artifact, got {other:?}"),
        }

        Ok(())
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
