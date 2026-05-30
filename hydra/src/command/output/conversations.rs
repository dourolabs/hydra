use std::io::Write;

use anyhow::Result;
use hydra_common::api::v1::conversations::{
    Conversation as ApiConversation, ConversationSummary as ApiConversationSummary,
};

use super::Render;

pub struct ConversationView<'a> {
    pub conversation: &'a ApiConversation,
}

pub struct ConversationSummaryRecords<'a>(pub &'a [ApiConversationSummary]);

impl Render for ConversationView<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        serde_json::to_writer(&mut *writer, self.conversation)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        writeln!(writer, "Conversation {}", self.conversation.conversation_id)?;
        writeln!(
            writer,
            "Title: {}",
            self.conversation.title.as_deref().unwrap_or("-")
        )?;
        writeln!(
            writer,
            "Agent: {}",
            self.conversation
                .agent_name
                .as_ref()
                .map(|n| n.as_str())
                .unwrap_or("-")
        )?;
        writeln!(writer, "Status: {:?}", self.conversation.status)?;
        writeln!(writer, "Creator: {}", self.conversation.creator)?;
        writeln!(writer, "Created: {}", self.conversation.created_at)?;
        writeln!(writer, "Updated: {}", self.conversation.updated_at)?;
        writer.flush()?;
        Ok(())
    }
}

impl Render for ConversationSummaryRecords<'_> {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()> {
        for conversation in self.0 {
            serde_json::to_writer(&mut *writer, conversation)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
        Ok(())
    }

    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.0.is_empty() {
            writeln!(writer, "No conversations found.")?;
            writer.flush()?;
            return Ok(());
        }

        for (index, conversation) in self.0.iter().enumerate() {
            writeln!(
                writer,
                "Conversation {} ({:?})",
                conversation.conversation_id, conversation.status
            )?;
            writeln!(
                writer,
                "  Title: {}",
                conversation.title.as_deref().unwrap_or("-")
            )?;
            writeln!(
                writer,
                "  Agent: {}",
                conversation
                    .agent_name
                    .as_ref()
                    .map(|n| n.as_str())
                    .unwrap_or("-")
            )?;
            writeln!(writer, "  Creator: {}", conversation.creator)?;
            writeln!(writer, "  Events: {}", conversation.event_count)?;
            if let Some(ref preview) = conversation.last_event_preview {
                writeln!(writer, "  Last: {preview}")?;
            }
            writeln!(writer, "  Created: {}", conversation.created_at)?;
            writeln!(writer, "  Updated: {}", conversation.updated_at)?;

            if index + 1 < self.0.len() {
                writeln!(writer)?;
            }
        }
        writer.flush()?;
        Ok(())
    }
}
