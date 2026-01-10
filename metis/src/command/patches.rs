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
        Artifact, ArtifactKind, ArtifactRecord, SearchArtifactsQuery, UpsertArtifactRequest,
    },
    constants::ENV_METIS_ID,
    MetisId,
};

use crate::{client::MetisClientInterface, command::worker_run::create_patch_from_repo};
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
    },

    /// Apply a patch artifact to the current git repository.
    Apply {
        /// Patch artifact id to apply.
        #[arg(value_name = "PATCH_ID")]
        id: MetisId,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: PatchesCommand) -> Result<()> {
    match command {
        PatchesCommand::List { id, query, pretty } => list_patches(client, id, query, pretty).await,
        PatchesCommand::Create { description, job } => create_patch(client, description, job).await,
        PatchesCommand::Apply { id } => apply_patch_artifact(client, id).await,
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
    description: String,
    job_id: Option<MetisId>,
) -> Result<()> {
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
                diff: patch,
                description,
            },
            job_id,
        })
        .await
        .context("failed to create patch artifact")?;

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
    let description = extract_patch_description(record, &record.id)?;
    println!("Patch {}: {}", record.id, description);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{client::MockMetisClient, constants};
    use anyhow::anyhow;
    use metis_common::artifacts::{
        Artifact, ArtifactRecord, ListArtifactsResponse, UpsertArtifactResponse,
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

    fn capture_current_dir() -> Result<PathBuf> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        env::set_current_dir(&manifest_dir).context("failed to ensure working directory exists")?;
        Ok(manifest_dir)
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
                description: "not a patch".to_string(),
            },
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
        let original_dir = capture_current_dir()?;
        env::set_current_dir(&repo_path).context("failed to change to repo dir")?;

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-1".to_string(),
        });
        let patch_description = "custom patch description".to_string();
        create_patch(&client, patch_description.clone(), None).await?;

        env::set_current_dir(original_dir).context("failed to restore current dir")?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        let (generated_patch, generated_description) = match &request.artifact {
            Artifact::Patch { diff, description } => (diff, description),
            other => panic!("expected patch artifact, got {other:?}"),
        };
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
        let original_dir = capture_current_dir()?;
        env::set_current_dir(&repo_path).context("failed to change to repo dir")?;

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-2".to_string(),
        });

        let job_id = Some("job-1234".to_string());
        let description = "patch with job id".to_string();

        create_patch(&client, description.clone(), job_id.clone()).await?;

        env::set_current_dir(original_dir).context("failed to restore current dir")?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        assert_eq!(
            request.job_id, job_id,
            "job id should be forwarded to the artifact request"
        );

        match &request.artifact {
            Artifact::Patch {
                description: recorded_description,
                ..
            } => assert_eq!(recorded_description, &description),
            other => panic!("expected patch artifact, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_reads_job_id_from_environment() -> Result<()> {
        let (_tempdir, repo_path) = initialize_repo_with_changes()?;
        let original_dir = capture_current_dir()?;
        env::set_current_dir(&repo_path).context("failed to change to repo dir")?;
        let _guard = EnvVarGuard::set(ENV_METIS_ID, "job-from-env");

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-3".to_string(),
        });

        let description = "patch with env job id".to_string();

        create_patch(&client, description.clone(), None).await?;

        env::set_current_dir(original_dir).context("failed to restore current dir")?;

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
                description: recorded_description,
                ..
            } => assert_eq!(recorded_description, &description),
            other => panic!("expected patch artifact, got {other:?}"),
        }

        Ok(())
    }
}
