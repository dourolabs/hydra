use crate::{
    client::MetisClientInterface,
    command::output::{render_versioned_messages, CommandContext},
};
use anyhow::{Context, Result};
use clap::Subcommand;
use metis_common::{
    actor_ref::ActorId,
    api::v1::messages::{
        ListMessagesQuery, SendMessageRequest, SendMessageResponse, WaitMessagesQuery,
    },
};
use std::io::{self, Write};

#[derive(Debug, Subcommand)]
pub enum MessagesCommand {
    /// Send a message to a user or issue-agent.
    Send {
        /// Recipient: an issue ID (e.g. "i-abc") or a username (e.g. "alice").
        #[arg(value_name = "RECIPIENT")]
        recipient: ActorId,

        /// Message body.
        #[arg(value_name = "BODY")]
        body: String,
    },
    /// List recent messages.
    List {
        /// Filter by participant (issue ID or username).
        #[arg(long, value_name = "PARTICIPANT")]
        participant: Option<String>,

        /// Maximum number of messages to return.
        #[arg(long, value_name = "LIMIT", default_value_t = 50)]
        limit: u32,
    },
    /// Block until a new message arrives (long-poll).
    Wait {
        /// Filter by participant (issue ID or username).
        #[arg(long, value_name = "PARTICIPANT")]
        participant: Option<String>,

        /// Timeout in seconds.
        #[arg(long, value_name = "SECONDS", default_value_t = 30)]
        timeout: u32,
    },
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: MessagesCommand,
    context: &CommandContext,
) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match command {
        MessagesCommand::Send { recipient, body } => {
            let response = send_message(client, recipient, body).await?;
            render_send_response(context, &response, &mut stdout)?;
        }
        MessagesCommand::List { participant, limit } => {
            let mut query = ListMessagesQuery::default();
            query.participant = participant;
            query.limit = Some(limit);
            let response = client
                .list_messages(&query)
                .await
                .context("failed to list messages")?;
            render_versioned_messages(context.output_format, &response.messages, &mut stdout)?;
        }
        MessagesCommand::Wait {
            participant,
            timeout,
        } => {
            let mut query = WaitMessagesQuery::default();
            query.participant = participant;
            query.timeout = Some(timeout);
            let response = client
                .wait_for_message(&query)
                .await
                .context("failed to wait for messages")?;
            if response.messages.is_empty() {
                if matches!(
                    context.output_format,
                    crate::command::output::ResolvedOutputFormat::Pretty
                ) {
                    writeln!(stdout, "No new messages (timed out).")?;
                }
            } else {
                render_versioned_messages(context.output_format, &response.messages, &mut stdout)?;
            }
        }
    }
    Ok(())
}

fn render_send_response(
    context: &CommandContext,
    response: &SendMessageResponse,
    writer: &mut impl Write,
) -> Result<()> {
    use crate::command::output::ResolvedOutputFormat;
    match context.output_format {
        ResolvedOutputFormat::Jsonl => {
            serde_json::to_writer(&mut *writer, response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        ResolvedOutputFormat::Pretty => {
            writeln!(writer, "Message sent.")?;
            writeln!(writer, "  message_id: {}", response.message_id)?;
            writeln!(writer, "  version: {}", response.version)?;
            writeln!(writer, "  sender: {}", response.message.sender)?;
            writeln!(writer, "  timestamp: {}", response.timestamp)?;
            writer.flush()?;
        }
    }
    Ok(())
}

async fn send_message(
    client: &dyn MetisClientInterface,
    recipient: ActorId,
    body: String,
) -> Result<SendMessageResponse> {
    let request = SendMessageRequest::new(recipient, body);
    client
        .send_message(&request)
        .await
        .context("failed to send message")
}
