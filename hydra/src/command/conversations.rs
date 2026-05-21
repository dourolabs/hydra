use anyhow::{Context, Result};
use clap::Subcommand;
use hydra_common::{
    api::v1::conversations::{
        ConversationStatus, CreateConversationRequest, SearchConversationsQuery,
        UpdateConversationRequest,
    },
    constants::ENV_HYDRA_CONVERSATION_ID,
    ConversationId,
};

use crate::{client::HydraClientInterface, output_writer::write_stdout};

use super::output::{render, CommandContext, ConversationSummaryRecords, ConversationView};

#[derive(Subcommand)]
pub enum ConversationsCommand {
    /// List conversations.
    List {
        /// Filter by status (active, idle, or closed).
        #[arg(long, value_name = "STATUS")]
        status: Option<ConversationStatusArg>,

        /// Filter by creator username.
        #[arg(long, value_name = "CREATOR")]
        creator: Option<String>,

        /// Free-text search across title, agent name, and ID.
        #[arg(long = "query", short = 'q', value_name = "QUERY")]
        query: Option<String>,

        /// Include soft-deleted conversations.
        #[arg(long)]
        include_deleted: bool,

        /// Maximum number of conversations to return.
        #[arg(short = 'n', long, value_name = "COUNT", default_value_t = 20)]
        limit: u32,
    },
    /// Get conversation details and full chat transcript.
    Get {
        /// Conversation identifier (defaults to HYDRA_CONVERSATION_ID).
        #[arg(value_name = "CONVERSATION_ID", env = ENV_HYDRA_CONVERSATION_ID)]
        id: ConversationId,
    },
    /// Create a new conversation.
    Create {
        /// Initial message to send. If omitted, the conversation starts with no events.
        #[arg(long, value_name = "MESSAGE")]
        message: Option<String>,

        /// Agent name for the conversation.
        #[arg(long = "agent", value_name = "AGENT")]
        agent_name: Option<String>,
    },
    /// Update a conversation's title.
    Update {
        /// Conversation identifier (defaults to HYDRA_CONVERSATION_ID).
        #[arg(value_name = "CONVERSATION_ID", env = ENV_HYDRA_CONVERSATION_ID)]
        id: ConversationId,

        /// New title for the conversation.
        #[arg(long, value_name = "TITLE")]
        title: String,
    },
    /// Soft-delete a conversation.
    Delete {
        /// Conversation identifier (defaults to HYDRA_CONVERSATION_ID).
        #[arg(value_name = "CONVERSATION_ID", env = ENV_HYDRA_CONVERSATION_ID)]
        id: ConversationId,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ConversationStatusArg {
    Active,
    Idle,
    Closed,
}

impl From<ConversationStatusArg> for ConversationStatus {
    fn from(arg: ConversationStatusArg) -> Self {
        match arg {
            ConversationStatusArg::Active => ConversationStatus::Active,
            ConversationStatusArg::Idle => ConversationStatus::Idle,
            ConversationStatusArg::Closed => ConversationStatus::Closed,
        }
    }
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: ConversationsCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        ConversationsCommand::List {
            status,
            creator,
            query,
            include_deleted,
            limit,
        } => {
            let search_query = SearchConversationsQuery {
                q: query,
                status: status.map(Into::into),
                creator,
                include_deleted: if include_deleted { Some(true) } else { None },
                limit: Some(limit),
                cursor: None,
            };

            let conversations = client
                .list_conversations(&search_query)
                .await
                .context("failed to list conversations")?;

            let mut buffer = Vec::new();
            render(
                ConversationSummaryRecords(&conversations),
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
        }
        ConversationsCommand::Get { id } => {
            let conversation = client
                .get_conversation(&id)
                .await
                .with_context(|| format!("failed to fetch conversation '{id}'"))?;
            let events = client
                .get_conversation_events(&id)
                .await
                .with_context(|| format!("failed to fetch events for conversation '{id}'"))?;

            let mut buffer = Vec::new();
            render(
                ConversationView {
                    conversation: &conversation,
                    events: &events,
                },
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
        }
        ConversationsCommand::Create {
            message,
            agent_name,
        } => {
            let request = CreateConversationRequest {
                message,
                agent_name,
                session_settings: None,
            };
            let conversation = client
                .create_conversation(&request)
                .await
                .context("failed to create conversation")?;

            let mut buffer = Vec::new();
            render(
                ConversationView {
                    conversation: &conversation,
                    events: &[],
                },
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
        }
        ConversationsCommand::Update { id, title } => {
            let request = UpdateConversationRequest { title: Some(title) };
            let conversation = client
                .update_conversation(&id, &request)
                .await
                .with_context(|| format!("failed to update conversation '{id}'"))?;

            let mut buffer = Vec::new();
            render(
                ConversationView {
                    conversation: &conversation,
                    events: &[],
                },
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
        }
        ConversationsCommand::Delete { id } => {
            let conversation = client
                .delete_conversation(&id)
                .await
                .with_context(|| format!("failed to delete conversation '{id}'"))?;

            let mut buffer = Vec::new();
            render(
                ConversationView {
                    conversation: &conversation,
                    events: &[],
                },
                context.output_format,
                &mut buffer,
            )?;
            write_stdout(&buffer)?;
        }
    }
    Ok(())
}
