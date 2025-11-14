mod command;
mod config;
mod kube;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::AppConfig;
use std::path::PathBuf;

/// Top-level CLI options for the metis tool.
#[derive(Parser)]
#[command(
    name = "metis",
    version,
    about = "Utility CLI for AI orchestrator prototypes"
)]
struct Cli {
    /// Path to the CLI configuration file.
    #[arg(long, value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

/// Available subcommands for the CLI.
#[derive(Subcommand)]
enum Commands {
    /// Spawn a new orchestration worker.
    Spawn {
        /// Optional label to attach to the spawned worker.
        #[arg(short, long, value_name = "LABEL")]
        label: Option<String>,

        /// Wait for the job to complete and stream its logs.
        #[arg(short = 'w', long = "wait")]
        wait: bool,
    },
    /// List all Metis jobs in the configured namespace.
    Jobs,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
    let app_config = AppConfig::load(&config_path)?;

    match cli.command {
        Commands::Spawn { label, wait } => command::spawn::run(&app_config, label, wait).await?,
        Commands::Jobs => command::jobs::run(&app_config).await?,
    }

    Ok(())
}
