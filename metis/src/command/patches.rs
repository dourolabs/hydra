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
    use crate::client::MockMetisClient;
    use metis_common::artifacts::{Artifact, ArtifactRecord, ListArtifactsResponse};

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
}
