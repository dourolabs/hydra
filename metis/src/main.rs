use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use metis::{
    auth,
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
    command: Option<Commands>,
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
    Dashboard,
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
    /// Manage users.
    Users {
        #[command(subcommand)]
        command: command::users::UsersCommand,
    },
    /// Log in with GitHub device flow.
    Login,
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
    let command = resolve_command(cli.command);
    if !matches!(command, Commands::Login) {
        auth::read_auth_token()?;
    }

    match command {
        Commands::Jobs { command } => command::jobs::run(client, command).await?,
        Commands::Agents { pretty } => command::agents::run(client, pretty).await?,
        Commands::Patches { command } => command::patches::run(client, command).await?,
        Commands::Dashboard => command::dashboard::run(client).await?,
        Commands::Issues { command } => command::issues::run(client, command).await?,
        Commands::Repos { command } => command::repos::run(client, command).await?,
        Commands::Users { command } => command::users::run(client, command).await?,
        Commands::Login => command::login::run(client).await?,
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(app_config, prompt, model, full_auto).await?,
    }

    Ok(())
}

fn resolve_command(command: Option<Commands>) -> Commands {
    command.unwrap_or(Commands::Dashboard)
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
    let resolved_path = config::expand_path(&config_path);
    if !resolved_path.exists() {
        return Err(anyhow!(
            "No server URL provided and configuration file '{}' was not found. Use --server-url or --config.",
            resolved_path.display()
        ));
    }

    AppConfig::load(&config_path)
}

#[cfg(test)]
mod tests {
    use super::{load_app_config, resolve_command, Cli, Commands};
    use crate::constants::DEFAULT_CONFIG_FILE;
    use clap::Parser;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn base_cli() -> Cli {
        Cli {
            config: None,
            server_url: None,
            command: Some(super::Commands::Agents { pretty: false }),
        }
    }

    #[test]
    fn load_config_missing_allows_server_url_override() {
        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            ..base_cli()
        };

        let config = load_app_config(&cli).expect("config should load from server url");
        assert_eq!(config.server.url, "http://localhost:9000");
    }

    #[test]
    fn load_config_missing_without_server_url_errors() {
        let temp = tempdir().expect("tempdir");
        let missing_path = temp.path().join("missing.toml");
        let cli = Cli {
            config: Some(missing_path),
            ..base_cli()
        };

        let err = load_app_config(&cli).expect_err("missing config should error");
        assert!(
            err.to_string().contains("No server URL provided"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_config_present_without_server_url_uses_config() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join(DEFAULT_CONFIG_FILE);
        std::fs::write(&config_path, "[server]\nurl = \"http://127.0.0.1:8080\"\n")
            .expect("write config");

        let cli = Cli {
            config: Some(PathBuf::from(&config_path)),
            ..base_cli()
        };
        let config = load_app_config(&cli).expect("config should load from file");
        assert_eq!(config.server.url, "http://127.0.0.1:8080");
    }

    #[test]
    fn parse_without_subcommand_defaults_to_dashboard() {
        let cli = Cli::try_parse_from(["metis"]).expect("parse");
        let command = resolve_command(cli.command);

        match command {
            Commands::Dashboard => {}
            _ => panic!("expected dashboard default"),
        }
    }
}
