mod server;

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use metis::{
    client::{MetisClient, MetisClientInterface, MetisClientUnauthenticated},
    command::{
        self,
        output::{resolve_output_format, CommandContext, OutputFormat},
    },
    config::{self, AppConfig},
    constants, github_device_flow,
};
use metis_common::constants::{ENV_BROWSER, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN};

use crate::server::ServerCommand;

#[derive(Parser)]
#[command(
    name = "metis",
    version,
    about = "Metis single-player: CLI + local server management"
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

    /// Auth token value (also via env var).
    #[arg(
        long = "token",
        env = ENV_METIS_TOKEN,
        value_name = "TOKEN",
        global = true
    )]
    token: Option<String>,

    /// Browser command for opening links (defaults to $BROWSER).
    #[arg(long = "browser", value_name = "COMMAND", env = ENV_BROWSER, global = true)]
    browser: Option<String>,

    /// Output format (auto, jsonl, or pretty).
    #[arg(
        long = "output-format",
        value_enum,
        default_value = "auto",
        global = true
    )]
    output_format: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the local metis server.
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
    /// Manage jobs.
    Jobs {
        #[command(subcommand)]
        command: command::jobs::JobsCommand,
    },
    /// Manage agents.
    Agents {
        #[command(subcommand)]
        command: command::agents::AgentsCommand,
    },
    /// Manage patches.
    Patches {
        #[command(subcommand)]
        command: command::patches::PatchesCommand,
    },
    /// Manage markdown documents.
    Documents {
        #[command(subcommand)]
        command: command::documents::DocumentsCommand,
    },
    /// Manage build caches.
    Caches {
        #[command(subcommand)]
        command: command::caches::CachesCommand,
    },
    /// Launch a live dashboard for jobs, issues, and patches.
    Dashboard,
    /// List or create issues.
    Issues {
        #[command(subcommand)]
        command: command::issues::IssueCommands,
    },
    /// Send, list, or wait for messages.
    Messages {
        #[command(subcommand)]
        command: command::messages::MessagesCommand,
    },
    /// Manage notifications.
    Notifications {
        #[command(subcommand)]
        command: command::notifications::NotificationsCommand,
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Server commands run synchronously (before the tokio runtime) because
    // `server start` uses fork() which must happen before any threads exist.
    if let Some(Commands::Server { command }) = cli.command {
        return server::run(command);
    }

    // Build the tokio runtime manually.
    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> Result<()> {
    let config_path = resolve_config_path(&cli);
    let app_config = load_app_config(&config_path)?;
    let server_url = resolve_server_url(&cli, &app_config)?;
    let unauth_client = MetisClientUnauthenticated::new(&server_url)?;
    let client =
        resolve_client(&cli, &app_config, &unauth_client, &config_path, &server_url).await?;
    let output_format = resolve_output_format(&client, cli.output_format).await?;
    let context = CommandContext::new(output_format);

    let result = dispatch(cli, &client, &server_url, &context).await;
    if let Err(ref err) = result {
        if is_broken_pipe(err) {
            std::process::exit(0);
        }
    }
    result
}

/// Check whether any error in the `anyhow` chain is an `io::ErrorKind::BrokenPipe`.
fn is_broken_pipe(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            if io_err.kind() == ErrorKind::BrokenPipe {
                return true;
            }
        }
    }
    false
}

async fn resolve_client(
    cli: &Cli,
    app_config: &AppConfig,
    unauth_client: &MetisClientUnauthenticated,
    config_path: &Path,
    server_url: &str,
) -> Result<MetisClient> {
    if let Some(token) = cli
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return MetisClient::new(server_url, token.to_string());
    }

    if let Some(token) = app_config.auth_token_for_url(server_url)? {
        return MetisClient::new(server_url, token.to_string());
    }

    github_device_flow::login_with_github_device_flow(unauth_client, config_path, server_url).await
}

async fn dispatch(
    cli: Cli,
    client: &dyn MetisClientInterface,
    server_url: &str,
    context: &CommandContext,
) -> Result<()> {
    match resolve_command(cli.command) {
        // Server is handled in main() before the tokio runtime.
        Commands::Server { .. } => unreachable!("server command handled before async dispatch"),
        Commands::Jobs { command } => command::jobs::run(client, command, context).await?,
        Commands::Agents { command } => command::agents::run(client, command, context).await?,
        Commands::Patches { command } => command::patches::run(client, command, context).await?,
        Commands::Documents { command } => {
            command::documents::run(client, command, context).await?
        }
        Commands::Caches { command } => command::caches::run(command, context).await?,
        Commands::Dashboard => {
            command::dashboard::run(client, server_url, cli.browser.as_deref(), context).await?
        }
        Commands::Issues { command } => command::issues::run(client, command, context).await?,
        Commands::Messages { command } => command::messages::run(client, command, context).await?,
        Commands::Notifications { command } => {
            command::notifications::run(client, command, context).await?
        }
        Commands::Repos { command } => command::repos::run(client, command, context).await?,
        Commands::Users { command } => command::users::run(client, command).await?,
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(server_url, prompt, model, full_auto, context).await?,
    }

    Ok(())
}

fn resolve_command(command: Option<Commands>) -> Commands {
    command.unwrap_or(Commands::Dashboard)
}

fn resolve_config_path(cli: &Cli) -> PathBuf {
    cli.config
        .clone()
        .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_CONFIG_FILE))
}

fn load_app_config(config_path: &Path) -> Result<AppConfig> {
    let resolved_path = config::expand_path(config_path);
    if !resolved_path.exists() {
        config::create_default_config(&resolved_path)?;
    }

    AppConfig::load(&resolved_path)
}

fn resolve_server_url(cli: &Cli, app_config: &AppConfig) -> Result<String> {
    if let Some(url) = cli
        .server_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return Ok(url.to_string());
    }

    Ok(app_config.default_server()?.url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parse_server_init_subcommand() {
        let cli = Cli::try_parse_from(["metis", "server", "init"]).expect("parse");
        match cli.command {
            Some(Commands::Server {
                command: ServerCommand::Init,
            }) => {}
            _ => panic!("expected Server Init"),
        }
    }

    #[test]
    fn parse_issues_subcommand() {
        let cli = Cli::try_parse_from(["metis", "issues", "list"]).expect("parse");
        match cli.command {
            Some(Commands::Issues { .. }) => {}
            _ => panic!("expected Issues"),
        }
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

    #[test]
    fn parse_server_logs_with_options() {
        let cli = Cli::try_parse_from(["metis", "server", "logs", "-n", "100", "--follow"])
            .expect("parse");
        match cli.command {
            Some(Commands::Server {
                command: ServerCommand::Logs { lines, follow },
            }) => {
                assert_eq!(lines, 100);
                assert!(follow);
            }
            _ => panic!("expected Server Logs"),
        }
    }
}
