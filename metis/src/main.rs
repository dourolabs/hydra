#![allow(clippy::too_many_arguments)]

mod client;
mod command;
mod config;
mod constants;
mod exec;
mod util;

#[cfg(test)]
mod test_utils;

use anyhow::Result;
use clap::{Parser, Subcommand};
use client::MetisClient;
use config::AppConfig;
use metis_common::{constants::ENV_METIS_SERVER_URL, TaskId};
use std::env;
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
    /// Manage jobs.
    Jobs {
        #[command(subcommand)]
        command: command::jobs::JobsCommand,
    },
    /// Manage patches.
    Patches {
        #[command(subcommand)]
        command: command::patches::PatchesCommand,
    },
    /// Launch a live dashboard for jobs, issues, and patches.
    Dashboard {
        /// Only show a dedicated panel for open issues assigned to this user.
        #[arg(long = "username", value_name = "USERNAME")]
        username: Option<String>,
    },
    /// List or create issues.
    Issues {
        #[command(subcommand)]
        command: command::issues::IssueCommands,
    },
    /// [Deprecated] Alias for `metis jobs worker-run`.
    WorkerRun {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`.
        #[arg(value_name = "JOB_ID")]
        job: TaskId,
        /// Destination directory where the context will be extracted/copied.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Chat with a Codex agent that can call the metis CLI.
    Chat {
        /// Run a single-turn conversation by forwarding this prompt to Codex non-interactively.
        #[arg(long = "prompt", value_name = "PROMPT")]
        prompt: Option<String>,

        /// Optional Codex model override (e.g. gpt-4o).
        #[arg(long = "model", value_name = "MODEL")]
        model: Option<String>,

        /// Allow the agent to run commands without prompting (maps to Codex --full-auto).
        #[arg(long = "full-auto")]
        full_auto: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let app_config = match env::var(ENV_METIS_SERVER_URL) {
        Ok(url) if !url.trim().is_empty() => AppConfig {
            server: config::ServerSection { url },
        },
        _ => {
            let config_path = cli
                .config
                .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_CONFIG_FILE));
            AppConfig::load(&config_path)?
        }
    };
    let client = MetisClient::from_config(&app_config)?;

    match cli.command {
        Commands::Jobs { command } => command::jobs::run(&client, command).await?,
        Commands::Patches { command } => command::patches::run(&client, command).await?,
        Commands::Dashboard { username } => command::dashboard::run(&client, username).await?,
        Commands::Issues { command } => command::issues::run(&client, command).await?,

        Commands::WorkerRun { job, path } => {
            eprintln!("Deprecated: use `metis jobs worker-run` instead of the top-level `worker-run` command.");
            command::worker_run::run(&client, job, path).await?
        }
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(&app_config, prompt, model, full_auto).await?,
    }

    Ok(())
}

#[cfg(test)]
mod cli_routing_tests {
    use super::{Cli, Commands};
    use crate::command::jobs::JobsCommand;
    use crate::test_utils::ids::task_id;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn jobs_worker_run_parses_under_jobs_namespace() {
        let job_id = "t-routingaaaa";
        let cli = Cli::parse_from(["metis", "jobs", "worker-run", job_id, "/tmp/output"]);

        match cli.command {
            Commands::Jobs {
                command: JobsCommand::WorkerRun { job, path },
            } => {
                assert_eq!(job, task_id(job_id));
                assert_eq!(path, PathBuf::from("/tmp/output"));
            }
            _ => panic!("expected jobs worker-run subcommand to parse"),
        }
    }

    #[test]
    fn top_level_worker_run_remains_supported_as_alias() {
        let job_id = "t-aliasabcd";
        let cli = Cli::parse_from(["metis", "worker-run", job_id, "/tmp/alias"]);

        match cli.command {
            Commands::WorkerRun { job, path } => {
                assert_eq!(job, task_id(job_id));
                assert_eq!(path, PathBuf::from("/tmp/alias"));
            }
            _ => panic!("expected top-level worker-run alias to parse"),
        }
    }
}
