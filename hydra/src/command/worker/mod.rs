use crate::client::HydraClientInterface;
use crate::command::output::CommandContext;
use anyhow::Result;
use clap::Subcommand;
use std::sync::Arc;

pub mod proxy;

#[derive(Subcommand)]
pub enum WorkerCommand {
    /// Manage proxy targets the worker advertises for the interactive dev
    /// preview (ports the platform's reverse proxy can forward user traffic
    /// to).
    Proxy {
        #[command(subcommand)]
        command: proxy::ProxyCommand,
    },
}

pub async fn run(
    client: Arc<dyn HydraClientInterface>,
    command: WorkerCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        WorkerCommand::Proxy { command } => proxy::run(client.as_ref(), command, context).await,
    }
}
