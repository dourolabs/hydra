mod command;
mod config;

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
        /// Wait for the job to complete and stream its logs.
        #[arg(short = 'w', long = "wait")]
        wait: bool,
    },
    /// List all Metis jobs in the configured namespace.
    Jobs,
    /// Show logs for an existing Metis job.
    Logs {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,

        /// Stream logs if the job is still running.
        #[arg(short = 'w', long = "watch")]
        watch: bool,
    },
    /// Delete completed or failed Metis jobs.
    Cleanup,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
    let app_config = AppConfig::load(&config_path)?;

    match cli.command {
        Commands::Spawn { wait } => command::spawn::run(&app_config, wait).await?,
        Commands::Jobs => command::jobs::run(&app_config).await?,
        Commands::Logs { job, watch } => command::logs::run(&app_config, job, watch).await?,
        Commands::Cleanup => command::cleanup::run(&app_config).await?,
    }

    Ok(())
}
