use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use metis::{
    client::{MetisClient, MetisClientInterface},
    command,
    config::{self, AppConfig},
    constants,
};
use metis_common::constants::ENV_METIS_SERVER_URL;

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

    /// Override the Metis server URL (also via METIS_SERVER_URL).
    #[arg(
        long = "server-url",
        value_name = "URL",
        env = ENV_METIS_SERVER_URL,
        global = true
    )]
    server_url: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let app_config = load_app_config(&cli)?;
    let client = MetisClient::from_config(&app_config)?;

    dispatch(cli, &client, &app_config).await
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
