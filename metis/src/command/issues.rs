use crate::client::MetisClientInterface;
use anyhow::{anyhow, bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    artifacts::{
        Artifact, ArtifactKind, ArtifactRecord, SearchArtifactsQuery, UpsertArtifactRequest,
    },
    MetisId,
};
use std::io::{self, Write};

#[derive(Debug, Subcommand)]
pub enum IssueCommands {
    /// List Metis issues (artifacts of type Issue).
    List {
        /// Filter by issue ID.
        #[arg(long, value_name = "ISSUE_ID", conflicts_with = "query")]
        id: Option<MetisId>,

        /// Search by query string.
        #[arg(long, value_name = "QUERY")]
        query: Option<String>,
    },
    /// Create a new issue artifact.
    Create {
        /// Description for the issue.
        #[arg(value_name = "DESCRIPTION")]
        description: String,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: IssueCommands) -> Result<()> {
    match command {
        IssueCommands::List { id, query } => {
            let artifacts = fetch_issues(client, id, query).await?;
            let mut stdout = io::stdout().lock();
            print_artifacts_jsonl(&artifacts, &mut stdout)?;
            Ok(())
        }
        IssueCommands::Create { description } => create_issue(client, description).await,
    }
}

async fn fetch_issues(
    client: &dyn MetisClientInterface,
    id: Option<MetisId>,
    query: Option<String>,
) -> Result<Vec<ArtifactRecord>> {
    if let Some(id) = id {
        let issue_id = id.trim();
        if issue_id.is_empty() {
            bail!("Issue ID must not be empty.");
        }
        let issue_id: MetisId = issue_id.to_string();

        let artifact = client
            .get_artifact(&issue_id)
            .await
            .with_context(|| format!("failed to fetch artifact '{issue_id}'"))?;

        ensure_issue(&artifact)?;
        return Ok(vec![artifact]);
    }

    let trimmed_query = query.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    let artifacts = client
        .list_artifacts(&SearchArtifactsQuery {
            artifact_type: Some(ArtifactKind::Issue),
            q: trimmed_query,
        })
        .await
        .context("failed to list issues")?
        .artifacts;

    for artifact in &artifacts {
        ensure_issue(artifact)?;
    }

    Ok(artifacts)
}

async fn create_issue(client: &dyn MetisClientInterface, description: String) -> Result<()> {
    let description = description.trim();
    if description.is_empty() {
        bail!("Issue description must not be empty.");
    }

    let request = UpsertArtifactRequest {
        artifact: Artifact::Issue {
            description: description.to_string(),
        },
        job_id: None,
    };

    let response = client
        .create_artifact(&request)
        .await
        .context("failed to create issue")?;

    println!("{}", response.artifact_id);
    Ok(())
}

fn ensure_issue(record: &ArtifactRecord) -> Result<()> {
    match record.artifact {
        Artifact::Issue { .. } => Ok(()),
        _ => Err(anyhow!("artifact '{}' is not an issue", record.id)),
    }
}

fn print_artifacts_jsonl(artifacts: &[ArtifactRecord], writer: &mut impl Write) -> Result<()> {
    for artifact in artifacts {
        serde_json::to_writer(&mut *writer, artifact)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use metis_common::artifacts::{
        ListArtifactsResponse, UpsertArtifactRequest, UpsertArtifactResponse,
    };

    #[tokio::test]
    async fn list_issues_filters_by_query_and_prints_jsonl() {
        let client = MockMetisClient::default();
        client.push_list_artifacts_response(ListArtifactsResponse {
            artifacts: vec![
                ArtifactRecord {
                    id: "issue-1".into(),
                    artifact: Artifact::Issue {
                        description: "First issue".into(),
                    },
                },
                ArtifactRecord {
                    id: "issue-2".into(),
                    artifact: Artifact::Issue {
                        description: "Second issue".into(),
                    },
                },
            ],
        });

        let artifacts = fetch_issues(&client, None, Some("bug".into()))
            .await
            .unwrap();

        assert_eq!(
            client.recorded_list_artifacts_queries(),
            vec![SearchArtifactsQuery {
                artifact_type: Some(ArtifactKind::Issue),
                q: Some("bug".into()),
            }]
        );

        let mut output = Vec::new();
        print_artifacts_jsonl(&artifacts, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"id\":\"issue-1\""));
        assert!(output.contains("\"id\":\"issue-2\""));
    }

    #[tokio::test]
    async fn list_issues_by_id_returns_single_issue() {
        let client = MockMetisClient::default();
        client.push_get_artifact_response(ArtifactRecord {
            id: "issue-123".into(),
            artifact: Artifact::Issue {
                description: "Edge case bug".into(),
            },
        });

        let artifacts = fetch_issues(&client, Some("issue-123".into()), None)
            .await
            .unwrap();

        assert_eq!(client.recorded_get_artifact_requests(), vec!["issue-123"]);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "issue-123");
    }

    #[tokio::test]
    async fn create_issue_submits_issue_artifact() {
        let client = MockMetisClient::default();
        client.push_upsert_artifact_response(UpsertArtifactResponse {
            artifact_id: "issue-456".into(),
        });

        create_issue(&client, "New issue description".into())
            .await
            .unwrap();

        assert_eq!(
            client.recorded_artifact_upserts(),
            vec![(
                None,
                UpsertArtifactRequest {
                    artifact: Artifact::Issue {
                        description: "New issue description".into(),
                    },
                    job_id: None,
                }
            )]
        );
    }

    #[tokio::test]
    async fn create_issue_requires_description() {
        let client = MockMetisClient::default();
        assert!(create_issue(&client, "   ".into()).await.is_err());
    }
}
