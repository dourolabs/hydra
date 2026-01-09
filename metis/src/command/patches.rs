use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    artifacts::{
        Artifact, ArtifactKind, ArtifactRecord, SearchArtifactsQuery, UpsertArtifactRequest,
    },
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
    Create,

    /// Apply a patch artifact to the current git repository.
    Apply {
        /// Patch artifact id to apply.
        #[arg(value_name = "PATCH_ID")]
        id: MetisId,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: PatchesCommand) -> Result<()> {
    match command {
        PatchesCommand::List {
            id,
            query,
            pretty,
        } => list_patches(client, id, query, pretty).await,
        PatchesCommand::Create => create_patch(client).await,
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

async fn create_patch(client: &dyn MetisClientInterface) -> Result<()> {
    let repo_root = git_repository_root()?;
    let patch = create_patch_from_repo(&repo_root)?;
    if patch.trim().is_empty() {
        bail!("No changes detected. Make edits before creating a patch artifact.");
    }

    let response = client
        .create_artifact(&UpsertArtifactRequest {
            artifact: Artifact::Patch {
                diff: patch.clone(),
            },
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
        Artifact::Patch { diff } => Ok(diff),
        _ => bail!("artifact '{id}' is not a patch"),
    }
}

fn print_patch_artifact(record: &ArtifactRecord) -> Result<()> {
    let diff = extract_patch_diff(record, &record.id)?;
    println!("Patch {}:\n", record.id);
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

    let patch_file =
        NamedTempFile::new().context("Failed to create temporary file for patch")?;
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
    use crate::client::MockMetisClient;
    use metis_common::artifacts::{Artifact, ArtifactRecord, ListArtifactsResponse};

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
}
