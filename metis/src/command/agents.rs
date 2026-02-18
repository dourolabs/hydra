use crate::{
    client::MetisClientInterface,
    command::output::{render_agent_records, CommandContext},
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use metis_common::agents::{AgentRecord, UpsertAgentRequest};
use std::io;

#[derive(Debug, Subcommand)]
pub enum AgentsCommand {
    /// List configured agents.
    List,
    /// Create a new agent.
    Create(CreateAgentArgs),
    /// Update an existing agent.
    Update(UpdateAgentArgs),
    /// Delete an agent.
    Delete {
        /// Agent name to delete.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Clone, Args)]
pub struct CreateAgentArgs {
    /// Agent name (must be unique).
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Prompt the agent will execute.
    #[arg(value_name = "PROMPT")]
    pub prompt: String,

    /// Maximum retries for this agent.
    #[arg(long = "max-tries", value_name = "MAX_TRIES", default_value_t = 3)]
    pub max_tries: u32,

    /// Maximum simultaneous tasks for this agent.
    #[arg(
        long = "max-simultaneous",
        value_name = "MAX_SIMULTANEOUS",
        default_value_t = u32::MAX
    )]
    pub max_simultaneous: u32,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateAgentArgs {
    /// Agent name to update.
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Updated prompt for the agent.
    #[arg(long = "prompt", value_name = "PROMPT")]
    pub prompt: Option<String>,

    /// Updated max retries for the agent.
    #[arg(long = "max-tries", value_name = "MAX_TRIES")]
    pub max_tries: Option<u32>,

    /// Updated max simultaneous tasks for the agent.
    #[arg(long = "max-simultaneous", value_name = "MAX_SIMULTANEOUS")]
    pub max_simultaneous: Option<u32>,
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: AgentsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        AgentsCommand::List => {
            let agents = fetch_agents(client).await?;
            render_agent_records(context.output_format, &agents, &mut stdout)?;
        }
        AgentsCommand::Create(args) => {
            let agent = create_agent(client, args).await?;
            render_agent_records(context.output_format, &[agent], &mut stdout)?;
        }
        AgentsCommand::Update(args) => {
            let agent = update_agent(client, args).await?;
            render_agent_records(context.output_format, &[agent], &mut stdout)?;
        }
        AgentsCommand::Delete { name } => {
            let deleted = delete_agent(client, &name).await?;
            render_agent_records(context.output_format, &[deleted], &mut stdout)?;
        }
    }

    Ok(())
}

async fn fetch_agents(client: &dyn MetisClientInterface) -> Result<Vec<AgentRecord>> {
    let response = client
        .list_agents()
        .await
        .context("failed to list agents")?;
    Ok(response.agents)
}

async fn create_agent(
    client: &dyn MetisClientInterface,
    args: CreateAgentArgs,
) -> Result<AgentRecord> {
    let request = build_upsert_request(
        &args.name,
        &args.prompt,
        args.max_tries,
        args.max_simultaneous,
    )?;
    let response = client
        .create_agent(&request)
        .await
        .context("failed to create agent")?;
    Ok(response.agent)
}

async fn update_agent(
    client: &dyn MetisClientInterface,
    args: UpdateAgentArgs,
) -> Result<AgentRecord> {
    let name = normalize_non_empty(&args.name, "agent name")?;
    let existing = client
        .get_agent(&name)
        .await
        .context("failed to fetch agent")?
        .agent;

    let mut request = UpsertAgentRequest::from(existing);
    request.name = name.clone();

    if let Some(prompt) = args.prompt {
        request.prompt = normalize_non_empty(&prompt, "prompt")?;
    }
    if let Some(max_tries) = args.max_tries {
        request.max_tries = max_tries;
    }
    if let Some(max_simultaneous) = args.max_simultaneous {
        request.max_simultaneous = max_simultaneous;
    }

    let response = client
        .update_agent(&name, &request)
        .await
        .context("failed to update agent")?;
    Ok(response.agent)
}

async fn delete_agent(client: &dyn MetisClientInterface, name: &str) -> Result<AgentRecord> {
    let name = normalize_non_empty(name, "agent name")?;
    let response = client
        .delete_agent(&name)
        .await
        .context("failed to delete agent")?;
    Ok(response.agent)
}

fn build_upsert_request(
    name: &str,
    prompt: &str,
    max_tries: u32,
    max_simultaneous: u32,
) -> Result<UpsertAgentRequest> {
    let request = UpsertAgentRequest::new(
        normalize_non_empty(name, "agent name")?,
        normalize_non_empty(prompt, "prompt")?,
        max_tries,
        max_simultaneous,
    );

    Ok(request)
}

fn normalize_non_empty(value: &str, field: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("{field} must not be empty");
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MetisClient,
        command::output::{render_agent_records, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use metis_common::agents::{
        AgentRecord, AgentResponse, DeleteAgentResponse, ListAgentsResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::json;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    #[tokio::test]
    async fn list_agents_fetches_agents_and_prints_jsonl() -> Result<()> {
        let server = MockServer::start();
        let list_agents_response = ListAgentsResponse::new(vec![
            AgentRecord::new("alpha", "", 3, u32::MAX),
            AgentRecord::new("beta", "", 3, u32::MAX),
        ]);

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents");
            then.status(200).json_body_obj(&list_agents_response);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let agents = fetch_agents(&client).await?;
        mock.assert();

        let mut output = Vec::new();
        render_agent_records(ResolvedOutputFormat::Jsonl, &agents, &mut output)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("\"name\":\"alpha\""));
        assert!(output.contains("\"name\":\"beta\""));

        Ok(())
    }

    #[tokio::test]
    async fn list_agents_prints_pretty_format() -> Result<()> {
        let agents = vec![AgentRecord::new("alpha", "prompt", 2, 5)];
        let mut output = Vec::new();

        render_agent_records(ResolvedOutputFormat::Pretty, &agents, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("alpha"));
        assert!(output.contains("prompt"));
        assert!(output.contains("max_tries: 2"));
        assert!(output.contains("max_simultaneous: 5"));

        Ok(())
    }

    #[tokio::test]
    async fn create_agent_sends_request() -> Result<()> {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let args = CreateAgentArgs {
            name: "writer".to_string(),
            prompt: "draft this".to_string(),
            max_tries: 2,
            max_simultaneous: 4,
        };
        let response = AgentResponse::new(AgentRecord::new("writer", "draft this", 2, 4));
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/agents").json_body(json!({
                "name": "writer",
                "prompt": "draft this",
                "max_tries": 2,
                "max_simultaneous": 4
            }));
            then.status(200).json_body_obj(&response);
        });

        let agent = create_agent(&client, args).await?;
        mock.assert();

        assert_eq!(agent.name, "writer");
        assert_eq!(agent.max_tries, 2);
        assert_eq!(agent.max_simultaneous, 4);

        Ok(())
    }

    #[tokio::test]
    async fn update_agent_merges_missing_fields_from_existing() -> Result<()> {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let existing = AgentResponse::new(AgentRecord::new("writer", "draft", 3, u32::MAX));
        let updated = AgentResponse::new(AgentRecord::new("writer", "revised", 3, 10));

        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/writer");
            then.status(200).json_body_obj(&existing);
        });
        let put_mock = server.mock(|when, then| {
            when.method(PUT).path("/v1/agents/writer").json_body(json!({
                "name": "writer",
                "prompt": "revised",
                "max_tries": 3,
                "max_simultaneous": 10
            }));
            then.status(200).json_body_obj(&updated);
        });

        let args = UpdateAgentArgs {
            name: " writer ".to_string(),
            prompt: Some("revised".to_string()),
            max_tries: None,
            max_simultaneous: Some(10),
        };

        let response = update_agent(&client, args).await?;
        get_mock.assert();
        put_mock.assert();
        assert_eq!(response.prompt, "revised");
        assert_eq!(response.max_simultaneous, 10);
        assert_eq!(response.max_tries, 3);

        Ok(())
    }

    #[tokio::test]
    async fn delete_agent_trims_name() -> Result<()> {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let deleted = AgentRecord::new("writer", "", 3, u32::MAX);
        let mock = server.mock(|when, then| {
            when.method(DELETE).path("/v1/agents/writer");
            then.status(200)
                .json_body_obj(&DeleteAgentResponse::new(deleted.clone()));
        });

        let response = delete_agent(&client, "  writer ").await?;
        mock.assert();
        assert_eq!(response.name, "writer");

        Ok(())
    }

    #[tokio::test]
    async fn normalize_agent_name_rejects_empty() {
        let error = build_upsert_request("", "prompt", 1, 1).unwrap_err();
        assert!(error.to_string().contains("agent name must not be empty"));
    }
}
