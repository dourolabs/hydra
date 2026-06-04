use std::io::Write;

use anyhow::Result;
use clap::ValueEnum;
use hydra_common::whoami::ActorIdentity;

use crate::client::HydraClientInterface;

mod agents;
mod conversations;
mod documents;
mod issues;
mod patches;
mod projects;
mod repositories;
mod sessions;
mod triggers;
mod users;

pub use agents::AgentRecords;
pub use conversations::{ConversationSummaryRecords, ConversationView};
pub use documents::{DocumentRecordsView, DocumentSummaryRecords};
pub use issues::{IssueRecords, IssueSummaryRecords};
pub use patches::{PatchRecords, PatchSummaryRecords};
pub use projects::{ProjectRecords, ProjectStatuses};
pub use repositories::RepositoryRecords;
pub use sessions::{SessionRecords, SessionSummaryRecords};
pub use triggers::TriggerRecords;
pub use users::UserRecords;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Auto,
    Jsonl,
    Pretty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedOutputFormat {
    Jsonl,
    Pretty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandContext {
    pub output_format: ResolvedOutputFormat,
}

impl CommandContext {
    pub fn new(output_format: ResolvedOutputFormat) -> Self {
        Self { output_format }
    }
}

pub trait Render {
    fn render_jsonl<W: Write>(&self, writer: &mut W) -> Result<()>;
    fn render_pretty<W: Write>(&self, writer: &mut W) -> Result<()>;
}

pub fn render<R: Render, W: Write>(
    value: R,
    format: ResolvedOutputFormat,
    writer: &mut W,
) -> Result<()> {
    match format {
        ResolvedOutputFormat::Jsonl => value.render_jsonl(writer),
        ResolvedOutputFormat::Pretty => value.render_pretty(writer),
    }
}

pub async fn resolve_output_format(
    client: &dyn HydraClientInterface,
    output_format: OutputFormat,
) -> Result<ResolvedOutputFormat> {
    match output_format {
        OutputFormat::Auto => resolve_auto_output_format(client).await,
        OutputFormat::Jsonl => Ok(ResolvedOutputFormat::Jsonl),
        OutputFormat::Pretty => Ok(ResolvedOutputFormat::Pretty),
    }
}

async fn resolve_auto_output_format(
    client: &dyn HydraClientInterface,
) -> Result<ResolvedOutputFormat> {
    let whoami = client.whoami().await?;
    Ok(match whoami.actor {
        ActorIdentity::User { .. } => ResolvedOutputFormat::Pretty,
        _ => ResolvedOutputFormat::Jsonl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{HydraClient, HydraClientTimeouts};
    use httpmock::prelude::*;
    use hydra_common::{whoami::WhoAmIResponse, SessionId};
    use std::str::FromStr;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    #[tokio::test]
    async fn resolve_output_format_auto_prefers_pretty_for_users() {
        let server = MockServer::start();
        let client = HydraClient::new(
            server.base_url(),
            TEST_HYDRA_TOKEN,
            &HydraClientTimeouts::default(),
        )
        .expect("client");
        let whoami = WhoAmIResponse::new(ActorIdentity::User {
            username: "user".into(),
        });

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami);
        });

        let resolved = resolve_output_format(&client, OutputFormat::Auto)
            .await
            .expect("resolve output format");

        mock.assert();
        assert_eq!(resolved, ResolvedOutputFormat::Pretty);
    }

    #[tokio::test]
    async fn resolve_output_format_auto_prefers_jsonl_for_tasks() {
        let server = MockServer::start();
        let client = HydraClient::new(
            server.base_url(),
            TEST_HYDRA_TOKEN,
            &HydraClientTimeouts::default(),
        )
        .expect("client");
        let whoami = WhoAmIResponse::new(ActorIdentity::Adhoc {
            session_id: SessionId::from_str("s-task").expect("task id"),
            creator: "test-creator".into(),
        });

        let mock = server.mock(|when, then| {
            when.method(GET).path("/v1/whoami");
            then.status(200).json_body_obj(&whoami);
        });

        let resolved = resolve_output_format(&client, OutputFormat::Auto)
            .await
            .expect("resolve output format");

        mock.assert();
        assert_eq!(resolved, ResolvedOutputFormat::Jsonl);
    }
}
