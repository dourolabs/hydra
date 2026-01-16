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
        writeln!(writer, "  - {}", agent.name)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use metis_common::agents::{AgentRecord, ListAgentsResponse};

    #[tokio::test]
    async fn list_agents_fetches_agents_and_prints_jsonl() {
        let client = MockMetisClient::default();
        client.push_list_agents_response(ListAgentsResponse {
            agents: vec![
                AgentRecord {
                    name: "alpha".into(),
                },
                AgentRecord {
                    name: "beta".into(),
                },
            ],
        });

        let agents = fetch_agents(&client).await.unwrap();
        assert_eq!(client.recorded_list_agents_calls(), 1);

        let mut output = Vec::new();
        print_agents_jsonl(&agents, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\"name\":\"alpha\""));
        assert!(output.contains("\"name\":\"beta\""));
    }
}
