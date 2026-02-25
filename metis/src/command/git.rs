use anyhow::{Context, Result};
use clap::Subcommand;

use crate::{
    client::MetisClientInterface,
    git::{checkout_branch, fetch_remote},
};

#[derive(Debug, Subcommand)]
pub enum GitCommand {
    /// Switch the working tree to the specified branch.
    ///
    /// Fetches the latest remote state then checks out the target branch,
    /// creating a local tracking branch from origin if needed.
    Checkout {
        /// Branch name to check out.
        #[arg(value_name = "BRANCH")]
        branch: String,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: GitCommand) -> Result<()> {
    match command {
        GitCommand::Checkout { branch } => checkout(client, &branch).await,
    }
}

async fn checkout(client: &dyn MetisClientInterface, branch: &str) -> Result<()> {
    let repo_root = std::env::current_dir().context("failed to determine current directory")?;
    let github_token = client.get_github_token().await.ok();

    fetch_remote(&repo_root, github_token.as_deref())
        .context("failed to fetch remote before checkout")?;
    checkout_branch(&repo_root, branch)
        .with_context(|| format!("failed to check out branch '{branch}'"))?;

    Ok(())
}
