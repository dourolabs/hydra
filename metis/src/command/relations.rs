use crate::{
    client::MetisClientInterface,
    command::output::{render_relations, CommandContext},
};
use anyhow::{Context, Result};
use clap::Subcommand;
use metis_common::api::v1::relations::ListRelationsRequest;
use metis_common::MetisId;
use std::io;

#[derive(Debug, Subcommand)]
pub enum RelationsCommand {
    /// List relations between objects.
    List {
        /// Filter by source object ID.
        #[arg(long, value_name = "ID")]
        source: Option<MetisId>,

        /// Filter by target object ID.
        #[arg(long, value_name = "ID")]
        target: Option<MetisId>,

        /// Show all relations where this object is source or target.
        #[arg(long, value_name = "ID")]
        object: Option<MetisId>,

        /// Filter by relation type (e.g. child-of, blocked-on, has-patch).
        #[arg(long, value_name = "TYPE")]
        rel_type: Option<String>,

        /// Follow transitive edges (requires --source or --target plus --rel-type).
        #[arg(long)]
        transitive: bool,
    },
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: RelationsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        RelationsCommand::List {
            source,
            target,
            object,
            rel_type,
            transitive,
        } => {
            let query = ListRelationsRequest {
                source_id: source,
                source_ids: None,
                target_id: target,
                target_ids: None,
                object_id: object,
                rel_type,
                transitive: if transitive { Some(true) } else { None },
            };
            let response = client
                .list_relations(&query)
                .await
                .context("failed to list relations")?;
            render_relations(context.output_format, &response, &mut stdout)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use crate::command::output::ResolvedOutputFormat;
    use httpmock::prelude::*;
    use metis_common::api::v1::relations::{ListRelationsResponse, RelationResponse};
    use reqwest::Client as HttpClient;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    #[tokio::test]
    async fn test_list_relations_dispatches_and_renders() {
        let server = MockServer::start();
        let api_response = ListRelationsResponse {
            relations: vec![
                RelationResponse {
                    source_id: "i-aaaaaa".parse().unwrap(),
                    target_id: "i-bbbbbb".parse().unwrap(),
                    rel_type: "child-of".to_string(),
                },
                RelationResponse {
                    source_id: "i-cccccc".parse().unwrap(),
                    target_id: "p-dddddd".parse().unwrap(),
                    rel_type: "has-patch".to_string(),
                },
            ],
        };

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/relations/")
                .query_param("source_id", "i-aaaaaa")
                .query_param("rel_type", "child-of");
            then.status(200).json_body_obj(&api_response);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .unwrap();
        let context = CommandContext {
            output_format: ResolvedOutputFormat::Jsonl,
        };
        let command = RelationsCommand::List {
            source: Some("i-aaaaaa".parse().unwrap()),
            target: None,
            object: None,
            rel_type: Some("child-of".to_string()),
            transitive: false,
        };

        run(&client, command, &context).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn test_list_relations_with_no_filters() {
        let server = MockServer::start();
        let api_response = ListRelationsResponse {
            relations: vec![],
        };

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/relations/");
            then.status(200).json_body_obj(&api_response);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .unwrap();
        let context = CommandContext {
            output_format: ResolvedOutputFormat::Pretty,
        };
        let command = RelationsCommand::List {
            source: None,
            target: None,
            object: None,
            rel_type: None,
            transitive: false,
        };

        run(&client, command, &context).await.unwrap();
        mock.assert();
    }
}
