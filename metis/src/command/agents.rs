use crate::client::MetisClientInterface;
use anyhow::Result;
use metis_common::agents::AgentRecord;
use std::io::{self, Write};

pub async fn run(client: &dyn MetisClientInterface, pretty: bool) -> Result<()> {
    let agents = fetch_agents(client).await?;
    let mut stdout = io::stdout().lock();
    if pretty {
        print_agents_pretty(&agents, &mut stdout)?;
    } else {
        print_agents_jsonl(&agents, &mut stdout)?;
    }
    Ok(())
}

async fn fetch_agents(client: &dyn MetisClientInterface) -> Result<Vec<AgentRecord>> {
    let response = client.list_agents().await?;
    Ok(response.agents)
}

fn print_agents_jsonl(agents: &[AgentRecord], writer: &mut impl Write) -> Result<()> {
    for agent in agents {
        serde_json::to_writer(&mut *writer, agent)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn print_agents_pretty(agents: &[AgentRecord], writer: &mut impl Write) -> Result<()> {
    if agents.is_empty() {
        writeln!(writer, "No agents configured.")?;
        writer.flush()?;
        return Ok(());
    }

    writeln!(writer, "Available agents:")?;
    for agent in agents {
        writeln!(
            writer,
            "  - {}: {} (max_tries={}, max_simultaneous={})",
            agent.name, agent.prompt, agent.max_tries, agent.max_simultaneous
        )?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use httpmock::prelude::*;
    use metis_common::agents::{AgentRecord, ListAgentsResponse};
    use reqwest::Client as HttpClient;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    #[tokio::test]
    async fn list_agents_fetches_agents_and_prints_jsonl() -> Result<()> {
        let server = MockServer::start();
        let list_agents_response =
            ListAgentsResponse::new(vec![AgentRecord::new("alpha"), AgentRecord::new("beta")]);

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/agents");
            then.status(200).json_body_obj(&list_agents_response);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let agents = fetch_agents(&client).await?;
        mock.assert();

        let mut output = Vec::new();
        print_agents_jsonl(&agents, &mut output)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("\"name\":\"alpha\""));
        assert!(output.contains("\"name\":\"beta\""));

        Ok(())
    }
}
