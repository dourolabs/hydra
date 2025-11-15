mod client;
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

        /// Branch or commit to use as the starting point for the job.
        #[arg(long = "from", value_name = "REV")]
        from: Option<String>,

        /// Prompt to execute, captured as trailing varargs.
        #[arg(
            value_name = "PROMPT",
            trailing_var_arg = true,
            num_args = 1..
        )]
        prompt: Vec<String>,
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
    /// Retrieve the recorded output for a completed job.
    Output {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
    let app_config = AppConfig::load(&config_path)?;

    match cli.command {
        Commands::Spawn { wait, from, prompt } => {
            command::spawn::run(&app_config, wait, from, prompt).await?
        }
        Commands::Jobs => command::jobs::run(&app_config).await?,
        Commands::Logs { job, watch } => command::logs::run(&app_config, job, watch).await?,
        Commands::Output { job } => command::output::run(&app_config, job).await?,
    }

    Ok(())
}
