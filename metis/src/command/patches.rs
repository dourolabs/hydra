use std::{path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    artifacts::{
        Artifact, ArtifactKind, ArtifactRecord, SearchArtifactsQuery, UpsertArtifactRequest,
    },
    MetisId,
};

use crate::{client::MetisClientInterface, command::worker_run::create_patch_from_repo};

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
    },

    /// Create a patch artifact from the current git repository.
    Create,
}

pub async fn run(client: &dyn MetisClientInterface, command: PatchesCommand) -> Result<()> {
    match command {
        PatchesCommand::List { id, query } => list_patches(client, id, query).await,
        PatchesCommand::Create => create_patch(client).await,
    }
}

async fn list_patches(
    client: &dyn MetisClientInterface,
    id: Option<MetisId>,
    query: Option<String>,
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
        println!("{}", serde_json::to_string(&artifact)?);
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
        println!("{}", serde_json::to_string(&artifact)?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use crate::{client::MockMetisClient, constants};
    use metis_common::artifacts::{
        Artifact, ArtifactRecord, ListArtifactsResponse, UpsertArtifactResponse,
    };
    use std::{env, fs, process::Command};

    #[tokio::test]
    async fn list_patches_sets_patch_filter_and_query() -> Result<()> {
        let client = MockMetisClient::default();
        client.push_list_artifacts_response(ListArtifactsResponse { artifacts: vec![] });

        list_patches(&client, None, Some("login".to_string())).await?;

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

        let err = list_patches(&client, Some("artifact-1".to_string()), None)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("is not a patch"),
            "expected patch type error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn create_patch_generates_diff_from_repo_changes() -> Result<()> {
        let tempdir = tempfile::tempdir().context("failed to create tempdir for test repo")?;
        let repo_path = tempdir.path();
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

        let original_dir = env::current_dir().context("failed to capture current dir")?;
        env::set_current_dir(repo_path).context("failed to change to repo dir")?;

        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "patch-1".to_string(),
        });
        create_patch(&client).await?;

        env::set_current_dir(original_dir).context("failed to restore current dir")?;

        let requests = client.recorded_artifact_upserts();
        assert_eq!(requests.len(), 1, "expected one artifact upsert");

        let (_, request) = &requests[0];
        let generated_patch = match &request.artifact {
            Artifact::Patch { diff } => diff,
            other => panic!("expected patch artifact, got {other:?}"),
        };

        let add_status = Command::new("git")
            .args([
                "add",
                "-A",
                "--",
                ".",
                &format!(":!{}/**", constants::METIS_DIR),
            ])
            .current_dir(repo_path)
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
            .current_dir(repo_path)
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
}
