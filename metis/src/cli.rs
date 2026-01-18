use crate::{
    client::{MetisClient, MetisClientInterface},
    command,
    config::{self, AppConfig},
    constants,
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use metis_common::constants::ENV_METIS_SERVER_URL;
use std::{env, ffi::OsString, path::PathBuf};

/// Top-level CLI options for the metis tool.
#[derive(Parser)]
#[command(
    name = "metis",
    version,
    about = "Utility CLI for AI orchestrator prototypes"
)]
pub struct Cli {
    /// Path to the CLI configuration file.
    #[arg(long, value_name = "FILE", global = true)]
    pub config: Option<PathBuf>,

    /// Override the Metis server URL (also via METIS_SERVER_URL).
    #[arg(long = "server-url", value_name = "URL", env = ENV_METIS_SERVER_URL, global = true)]
    pub server_url: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for the CLI.
#[derive(Subcommand)]
pub enum Commands {
    /// Manage jobs.
    Jobs {
        #[command(subcommand)]
        command: command::jobs::JobsCommand,
    },
    /// List available agents.
    Agents {
        /// Pretty-print the agents instead of emitting JSONL.
        #[arg(long)]
        pretty: bool,
    },
    /// Manage patches.
    Patches {
        #[command(subcommand)]
        command: command::patches::PatchesCommand,
    },
    /// Launch a live dashboard for jobs, issues, and patches.
    Dashboard {
        /// Only show a dedicated panel for open issues assigned to this user.
        #[arg(long = "username", value_name = "USERNAME", env = "METIS_USER")]
        username: Option<String>,
    },
    /// List or create issues.
    Issues {
        #[command(subcommand)]
        command: command::issues::IssueCommands,
    },
    /// Manage service repositories.
    Repos {
        #[command(subcommand)]
        command: command::repos::ReposCommand,
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

/// Execute the metis CLI using the provided CLI arguments and client.
pub async fn run_with_client_and_config<I, T>(
    args: I,
    client: &dyn MetisClientInterface,
    app_config: &AppConfig,
) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let cli = Cli::parse_from(&args);
    dispatch(cli, client, app_config).await
}

/// Execute the metis CLI using injected arguments and a default client constructed from config.
pub async fn run_with_args<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    let app_config = load_app_config(&cli)?;
    let client = MetisClient::from_config(&app_config)?;

    dispatch(cli, &client, &app_config).await
}

/// Execute the metis CLI using the current process arguments.
pub async fn run() -> Result<()> {
    run_with_args(env::args_os()).await
}

async fn dispatch(
    cli: Cli,
    client: &dyn MetisClientInterface,
    app_config: &AppConfig,
) -> Result<()> {
    match cli.command {
        Commands::Jobs { command } => command::jobs::run(client, command).await?,
        Commands::Agents { pretty } => command::agents::run(client, pretty).await?,
        Commands::Patches { command } => command::patches::run(client, command).await?,
        Commands::Dashboard { username } => command::dashboard::run(client, username).await?,
        Commands::Issues { command } => command::issues::run(client, command).await?,
        Commands::Repos { command } => command::repos::run(client, command).await?,
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(app_config, prompt, model, full_auto).await?,
    }

    Ok(())
}

fn load_app_config(cli: &Cli) -> Result<AppConfig> {
    if let Some(url) = cli
        .server_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return Ok(AppConfig {
            server: config::ServerSection {
                url: url.to_string(),
            },
        });
    }

    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_CONFIG_FILE));
    AppConfig::load(&config_path)
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
                command: JobsCommand::WorkerRun { job, path, .. },
            } => {
                assert_eq!(job, task_id(job_id));
                assert_eq!(path, PathBuf::from("/tmp/output"));
            }
            _ => panic!("expected jobs worker-run subcommand to parse"),
        }
    }

    #[test]
    fn agents_parses_as_top_level_command() {
        let cli = Cli::parse_from(["metis", "agents", "--pretty"]);

        match cli.command {
            Commands::Agents { pretty } => assert!(pretty),
            _ => panic!("expected agents subcommand to parse"),
        }
    }
}
