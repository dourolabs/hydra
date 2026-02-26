use crate::{
    client::MetisClientInterface,
    command::output::{render_versioned_messages, CommandContext},
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Subcommand;
use metis_common::{
    actor_ref::ActorId,
    api::v1::messages::{
        SearchMessagesQuery, SendMessageRequest, SendMessageResponse, WaitMessagesQuery,
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

        /// Mark the message as already read.
        #[arg(long = "read")]
        is_read: bool,
    },
    /// List recent messages.
    List {
        /// Filter by sender (e.g. "u-alice" or "a-i-abc").
        #[arg(long, value_name = "SENDER")]
        sender: Option<String>,

        /// Filter by recipient (e.g. "u-alice" or "a-i-abc"). Defaults to the current actor.
        #[arg(long, value_name = "RECIPIENT")]
        recipient: Option<String>,

        /// Only show messages after this timestamp (RFC 3339).
        #[arg(long, value_name = "TIMESTAMP")]
        after: Option<DateTime<Utc>>,

        /// Maximum number of messages to return.
        #[arg(long, value_name = "LIMIT", default_value_t = 50)]
        limit: u32,
    },
    /// Block until a new message arrives (long-poll).
    Wait {
        /// Filter by sender (e.g. "u-alice" or "a-i-abc").
        #[arg(long, value_name = "SENDER")]
        sender: Option<String>,

        /// Filter by recipient (e.g. "u-alice" or "a-i-abc").
        #[arg(long, value_name = "RECIPIENT")]
        recipient: Option<String>,

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
        MessagesCommand::Send {
            recipient,
            body,
            is_read,
        } => {
            let response = send_message(client, recipient, body, is_read).await?;
            render_send_response(context, &response, &mut stdout)?;
        }
        MessagesCommand::List {
            sender,
            recipient,
            after,
            limit,
        } => {
            let recipient = match recipient {
                Some(r) => Some(r),
                None => Some(client.current_actor_id().await?.to_string()),
            };
            let mut query = SearchMessagesQuery::default();
            query.sender = sender;
            query.recipient = recipient;
            query.after = after;
            query.limit = Some(limit);
            let response = client
                .list_messages(&query)
                .await
                .context("failed to list messages")?;
            render_versioned_messages(context.output_format, &response.messages, &mut stdout)?;
        }
        MessagesCommand::Wait {
            sender,
            recipient,
            timeout,
        } => {
            let mut query = WaitMessagesQuery::default();
            query.sender = sender;
            query.recipient = recipient;
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
            if let Some(ref sender) = response.message.sender {
                writeln!(writer, "  sender: {sender}")?;
            } else {
                writeln!(writer, "  sender: system")?;
            }
            writeln!(writer, "  recipient: {}", response.message.recipient)?;
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
    is_read: bool,
) -> Result<SendMessageResponse> {
    let mut request = SendMessageRequest::new(recipient, body);
    request.is_read = is_read;
    client
        .send_message(&request)
        .await
        .context("failed to send message")
}
