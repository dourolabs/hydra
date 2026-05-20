use std::io::Write;

use anyhow::Result;
use hydra_common::agents::AgentRecord;

use super::Render;

pub struct AgentRecords<'a>(pub &'a [AgentRecord]);

impl Render for AgentRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for agent in self.0 {
            serde_json::to_writer(&mut *writer, agent)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No agents configured.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, agent) in self.0.iter().enumerate() {
            write_agent_details(agent, writer)?;
            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}

fn write_agent_details<W: Write>(agent: &AgentRecord, writer: &mut W) -> Result<()> {
    writeln!(writer, "- {}", agent.name)?;
    if !agent.prompt_path.is_empty() {
        writeln!(writer, "  prompt_path: {}", agent.prompt_path)?;
    }
    if !agent.prompt.is_empty() {
        writeln!(writer, "  prompt: {}", agent.prompt)?;
    }
    if let Some(mcp_config_path) = &agent.mcp_config_path {
        writeln!(writer, "  mcp_config_path: {mcp_config_path}")?;
    }
    writeln!(writer, "  max_tries: {}", agent.max_tries)?;
    writeln!(writer, "  max_simultaneous: {}", agent.max_simultaneous)?;
    writeln!(
        writer,
        "  is_assignment_agent: {}",
        agent.is_assignment_agent
    )?;
    writeln!(
        writer,
        "  is_default_conversation_agent: {}",
        agent.is_default_conversation_agent
    )?;
    if !agent.secrets.is_empty() {
        writeln!(writer, "  secrets: {}", agent.secrets.join(", "))?;
    }
    Ok(())
}
