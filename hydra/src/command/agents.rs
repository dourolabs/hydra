use crate::{
    client::HydraClientInterface,
    command::output::{render_agent_records, CommandContext},
};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use hydra_common::agents::{AgentRecord, UpsertAgentRequest};
use std::io;

#[derive(Debug, Subcommand)]
pub enum AgentsCommand {
    /// List configured agents.
    List,
    /// Get details of an agent including its prompt text.
    Get {
        /// Agent name.
        #[arg(value_name = "NAME")]
        name: String,
    },
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

    /// Path to a local file containing the agent prompt.
    #[arg(long = "prompt-file", value_name = "PATH")]
    pub prompt_file: String,

    /// Maximum retries for this agent.
    #[arg(long = "max-tries", value_name = "MAX_TRIES", default_value_t = 3)]
    pub max_tries: i32,

    /// Maximum simultaneous tasks for this agent.
    #[arg(
        long = "max-simultaneous",
        value_name = "MAX_SIMULTANEOUS",
        default_value_t = i32::MAX
    )]
    pub max_simultaneous: i32,

    /// Mark this agent as the assignment agent (at most one allowed).
    #[arg(long = "is-assignment-agent")]
    pub is_assignment_agent: bool,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateAgentArgs {
    /// Agent name to update.
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Path to a local file containing the updated agent prompt.
    #[arg(long = "prompt-file", value_name = "PATH")]
    pub prompt_file: Option<String>,

    /// Updated max retries for the agent.
    #[arg(long = "max-tries", value_name = "MAX_TRIES")]
    pub max_tries: Option<i32>,

    /// Updated max simultaneous tasks for the agent.
    #[arg(long = "max-simultaneous", value_name = "MAX_SIMULTANEOUS")]
    pub max_simultaneous: Option<i32>,

    /// Mark this agent as the assignment agent (at most one allowed).
    #[arg(long = "is-assignment-agent")]
    pub is_assignment_agent: bool,

    /// Remove the assignment agent designation from this agent.
    #[arg(
        long = "no-is-assignment-agent",
        conflicts_with = "is_assignment_agent"
    )]
    pub no_is_assignment_agent: bool,
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: AgentsCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        AgentsCommand::List => {
            let agents = fetch_agents(client).await?;
            render_agent_records(context.output_format, &agents, &mut stdout)?;
        }
        AgentsCommand::Get { name } => {
            let agent = get_agent(client, &name).await?;
            render_agent_records(context.output_format, &[agent], &mut stdout)?;
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

async fn fetch_agents(client: &dyn HydraClientInterface) -> Result<Vec<AgentRecord>> {
    let response = client
        .list_agents()
        .await
        .context("failed to list agents")?;
    Ok(response.agents)
}

async fn get_agent(client: &dyn HydraClientInterface, name: &str) -> Result<AgentRecord> {
    let name = normalize_non_empty(name, "agent name")?;
    let response = client
        .get_agent(&name)
        .await
        .context("failed to get agent")?;
    Ok(response.agent)
}

fn read_prompt_file(path: &str) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read prompt file: {path}"))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        bail!("prompt file is empty: {path}");
    }
    Ok(trimmed.to_string())
}

async fn create_agent(
    client: &dyn HydraClientInterface,
    args: CreateAgentArgs,
) -> Result<AgentRecord> {
    let name = normalize_non_empty(&args.name, "agent name")?;
    let prompt = read_prompt_file(&args.prompt_file)?;

    let mut request = UpsertAgentRequest::new(name, prompt, args.max_tries, args.max_simultaneous);
    request.is_assignment_agent = args.is_assignment_agent;

    let response = client
        .create_agent(&request)
        .await
        .context("failed to create agent")?;
    Ok(response.agent)
}

async fn update_agent(
    client: &dyn HydraClientInterface,
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

    if let Some(prompt_file) = &args.prompt_file {
        request.prompt = read_prompt_file(prompt_file)?;
    }
    if let Some(max_tries) = args.max_tries {
        request.max_tries = max_tries;
    }
    if let Some(max_simultaneous) = args.max_simultaneous {
        request.max_simultaneous = max_simultaneous;
    }
    if args.is_assignment_agent {
        request.is_assignment_agent = true;
    } else if args.no_is_assignment_agent {
        request.is_assignment_agent = false;
    }

    let response = client
        .update_agent(&name, &request)
        .await
        .context("failed to update agent")?;
    Ok(response.agent)
}

async fn delete_agent(client: &dyn HydraClientInterface, name: &str) -> Result<AgentRecord> {
    let name = normalize_non_empty(name, "agent name")?;
    let response = client
        .delete_agent(&name)
        .await
        .context("failed to delete agent")?;
    Ok(response.agent)
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
        client::HydraClient,
        command::output::{render_agent_records, ResolvedOutputFormat},
    };
    use httpmock::prelude::*;
    use hydra_common::agents::{
        AgentRecord, AgentResponse, DeleteAgentResponse, ListAgentsResponse,
    };
    use reqwest::Client as HttpClient;
    use serde_json::json;
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn write_prompt_file(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[tokio::test]
    async fn list_agents_fetches_agents_and_prints_jsonl() -> Result<()> {
        let server = MockServer::start();
        let list_agents_response = ListAgentsResponse::new(vec![
            AgentRecord::new("alpha", "", "", 3, i32::MAX, false),
            AgentRecord::new("beta", "", "", 3, i32::MAX, false),
        ]);

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents");
            then.status(200).json_body_obj(&list_agents_response);
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

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
        let agents = vec![AgentRecord::new(
            "alpha",
            "prompt",
            "/agents/alpha/prompt.md",
            2,
            5,
            true,
        )];
        let mut output = Vec::new();

        render_agent_records(ResolvedOutputFormat::Pretty, &agents, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("alpha"));
        assert!(output.contains("/agents/alpha/prompt.md"));
        assert!(output.contains("max_tries: 2"));
        assert!(output.contains("max_simultaneous: 5"));
        assert!(output.contains("is_assignment_agent: true"));

        Ok(())
    }

    #[tokio::test]
    async fn get_agent_returns_details() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let response = AgentResponse::new(AgentRecord::new(
            "swe",
            "do software engineering",
            "/agents/swe/prompt.md",
            3,
            i32::MAX,
            false,
        ));
        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/swe");
            then.status(200).json_body_obj(&response);
        });

        let agent = get_agent(&client, "swe").await?;
        mock.assert();

        assert_eq!(agent.name, "swe");
        assert_eq!(agent.prompt, "do software engineering");

        Ok(())
    }

    #[tokio::test]
    async fn create_agent_sends_request_with_prompt_file() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let prompt_file = write_prompt_file("draft this");

        let args = CreateAgentArgs {
            name: "writer".to_string(),
            prompt_file: prompt_file.path().to_str().unwrap().to_string(),
            max_tries: 2,
            max_simultaneous: 4,
            is_assignment_agent: false,
        };
        let response =
            AgentResponse::new(AgentRecord::new("writer", "draft this", "", 2, 4, false));
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/agents").json_body(json!({
                "name": "writer",
                "prompt": "draft this",
                "prompt_path": "",
                "max_tries": 2,
                "max_simultaneous": 4,
                "is_assignment_agent": false
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
    async fn create_agent_with_assignment_flag() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let prompt_file = write_prompt_file("assign issues");

        let args = CreateAgentArgs {
            name: "pm".to_string(),
            prompt_file: prompt_file.path().to_str().unwrap().to_string(),
            max_tries: 3,
            max_simultaneous: i32::MAX,
            is_assignment_agent: true,
        };
        let response = AgentResponse::new(AgentRecord::new(
            "pm",
            "assign issues",
            "",
            3,
            i32::MAX,
            true,
        ));
        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/agents").json_body(json!({
                "name": "pm",
                "prompt": "assign issues",
                "prompt_path": "",
                "max_tries": 3,
                "max_simultaneous": 2147483647i64,
                "is_assignment_agent": true
            }));
            then.status(200).json_body_obj(&response);
        });

        let agent = create_agent(&client, args).await?;
        mock.assert();

        assert_eq!(agent.name, "pm");
        assert!(agent.is_assignment_agent);

        Ok(())
    }

    #[tokio::test]
    async fn update_agent_merges_missing_fields_from_existing() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let existing =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, false));
        let updated = AgentResponse::new(AgentRecord::new("writer", "revised", "", 3, 10, false));

        let prompt_file = write_prompt_file("revised");

        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/writer");
            then.status(200).json_body_obj(&existing);
        });
        let put_mock = server.mock(|when, then| {
            when.method(PUT).path("/v1/agents/writer").json_body(json!({
                "name": "writer",
                "prompt": "revised",
                "prompt_path": "",
                "max_tries": 3,
                "max_simultaneous": 10,
                "is_assignment_agent": false
            }));
            then.status(200).json_body_obj(&updated);
        });

        let args = UpdateAgentArgs {
            name: " writer ".to_string(),
            prompt_file: Some(prompt_file.path().to_str().unwrap().to_string()),
            max_tries: None,
            max_simultaneous: Some(10),
            is_assignment_agent: false,
            no_is_assignment_agent: false,
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
    async fn update_agent_sets_is_assignment_agent_true() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let existing =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, false));
        let updated =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, true));

        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/writer");
            then.status(200).json_body_obj(&existing);
        });
        let put_mock = server.mock(|when, then| {
            when.method(PUT).path("/v1/agents/writer").json_body(json!({
                "name": "writer",
                "prompt": "draft",
                "prompt_path": "",
                "max_tries": 3,
                "max_simultaneous": 2147483647,
                "is_assignment_agent": true
            }));
            then.status(200).json_body_obj(&updated);
        });

        let args = UpdateAgentArgs {
            name: "writer".to_string(),
            prompt_file: None,
            max_tries: None,
            max_simultaneous: None,
            is_assignment_agent: true,
            no_is_assignment_agent: false,
        };

        let response = update_agent(&client, args).await?;
        get_mock.assert();
        put_mock.assert();
        assert!(response.is_assignment_agent);

        Ok(())
    }

    #[tokio::test]
    async fn update_agent_unsets_is_assignment_agent() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let existing =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, true));
        let updated =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, false));

        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/writer");
            then.status(200).json_body_obj(&existing);
        });
        let put_mock = server.mock(|when, then| {
            when.method(PUT).path("/v1/agents/writer").json_body(json!({
                "name": "writer",
                "prompt": "draft",
                "prompt_path": "",
                "max_tries": 3,
                "max_simultaneous": 2147483647,
                "is_assignment_agent": false
            }));
            then.status(200).json_body_obj(&updated);
        });

        let args = UpdateAgentArgs {
            name: "writer".to_string(),
            prompt_file: None,
            max_tries: None,
            max_simultaneous: None,
            is_assignment_agent: false,
            no_is_assignment_agent: true,
        };

        let response = update_agent(&client, args).await?;
        get_mock.assert();
        put_mock.assert();
        assert!(!response.is_assignment_agent);

        Ok(())
    }

    #[tokio::test]
    async fn update_agent_preserves_is_assignment_agent_when_neither_flag() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let existing =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, true));
        let updated =
            AgentResponse::new(AgentRecord::new("writer", "draft", "", 3, i32::MAX, true));

        let get_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents/writer");
            then.status(200).json_body_obj(&existing);
        });
        let put_mock = server.mock(|when, then| {
            when.method(PUT).path("/v1/agents/writer").json_body(json!({
                "name": "writer",
                "prompt": "draft",
                "prompt_path": "",
                "max_tries": 3,
                "max_simultaneous": 2147483647,
                "is_assignment_agent": true
            }));
            then.status(200).json_body_obj(&updated);
        });

        let args = UpdateAgentArgs {
            name: "writer".to_string(),
            prompt_file: None,
            max_tries: None,
            max_simultaneous: None,
            is_assignment_agent: false,
            no_is_assignment_agent: false,
        };

        let response = update_agent(&client, args).await?;
        get_mock.assert();
        put_mock.assert();
        assert!(response.is_assignment_agent);

        Ok(())
    }

    #[test]
    fn update_agent_rejects_both_assignment_flags() {
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct Cli {
            #[command(flatten)]
            args: UpdateAgentArgs,
        }

        let result = Cli::try_parse_from([
            "cli",
            "writer",
            "--is-assignment-agent",
            "--no-is-assignment-agent",
        ]);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("cannot be used with"),
            "expected conflict error, got: {err}"
        );
    }

    #[tokio::test]
    async fn delete_agent_trims_name() -> Result<()> {
        let server = MockServer::start();
        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;
        let deleted = AgentRecord::new("writer", "", "", 3, i32::MAX, false);
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
    async fn read_prompt_file_rejects_empty() {
        let f = write_prompt_file("   ");
        let err = read_prompt_file(f.path().to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("prompt file is empty"));
    }

    #[tokio::test]
    async fn read_prompt_file_rejects_missing() {
        let err = read_prompt_file("/nonexistent/path.md").unwrap_err();
        assert!(err.to_string().contains("failed to read prompt file"));
    }
}
