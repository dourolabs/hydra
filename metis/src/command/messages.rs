use crate::{
    client::MetisClientInterface,
    command::output::{render_versioned_messages, CommandContext},
};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    actor_ref::ActorId,
    api::v1::messages::{
        ListMessagesQuery, SendMessageRequest, SendMessageResponse, WaitMessagesQuery,
    },
    users::Username,
    IssueId,
};
use std::io::{self, Write};
use std::str::FromStr;

#[derive(Debug, Subcommand)]
pub enum MessagesCommand {
    /// Send a message to a user or issue-agent.
    Send {
        /// Recipient: an issue ID (e.g. "i-abc") or a username (e.g. "alice").
        #[arg(value_name = "RECIPIENT")]
        recipient: String,

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
            let response = send_message(client, &recipient, body).await?;
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

/// Parse a recipient string into an ActorId.
///
/// Shorthand rules:
/// - Strings starting with "i-" are parsed as issue IDs → ActorId::Issue
/// - Everything else is treated as a username → ActorId::Username
fn parse_recipient(raw: &str) -> Result<ActorId> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("recipient must not be empty");
    }

    if trimmed.starts_with("i-") {
        let issue_id = IssueId::from_str(trimmed)
            .map_err(|e| anyhow::anyhow!("invalid issue ID '{trimmed}': {e}"))?;
        return Ok(ActorId::Issue(issue_id));
    }

    Ok(ActorId::Username(Username::from(trimmed)))
}

async fn send_message(
    client: &dyn MetisClientInterface,
    recipient_raw: &str,
    body: String,
) -> Result<SendMessageResponse> {
    let recipient = parse_recipient(recipient_raw)?;
    let request = SendMessageRequest::new(recipient, body);
    client
        .send_message(&request)
        .await
        .context("failed to send message")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recipient_issue_id() {
        let actor = parse_recipient("i-abcdef").unwrap();
        match actor {
            ActorId::Issue(id) => assert_eq!(id.to_string(), "i-abcdef"),
            other => panic!("expected ActorId::Issue, got {other:?}"),
        }
    }

    #[test]
    fn parse_recipient_username() {
        let actor = parse_recipient("alice").unwrap();
        match actor {
            ActorId::Username(username) => assert_eq!(username.as_str(), "alice"),
            other => panic!("expected ActorId::Username, got {other:?}"),
        }
    }

    #[test]
    fn parse_recipient_empty_fails() {
        assert!(parse_recipient("").is_err());
        assert!(parse_recipient("  ").is_err());
    }

    #[test]
    fn parse_recipient_trims_whitespace() {
        let actor = parse_recipient("  bob  ").unwrap();
        match actor {
            ActorId::Username(username) => assert_eq!(username.as_str(), "bob"),
            other => panic!("expected ActorId::Username, got {other:?}"),
        }
    }
}
