use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
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

    /// Path to the auth token file (defaults to ~/.local/share/metis/auth-token).
    #[arg(
        long = "token-path",
        value_name = "PATH",
        global = true,
        default_value = constants::DEFAULT_AUTH_TOKEN_PATH
    )]
    token_path: String,

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
    let unauth_client = MetisClientUnauthenticated::from_config(&app_config)?;
    let token_path = config::expand_path(PathBuf::from(&cli.token_path));
    let client = resolve_client(&cli, &app_config, &unauth_client, &token_path).await?;
    let output_format = resolve_output_format(&client, cli.output_format).await?;
    let context = CommandContext::new(output_format);

    dispatch(cli, &client, &app_config, &context).await
}

async fn resolve_client(
    cli: &Cli,
    app_config: &AppConfig,
    unauth_client: &MetisClientUnauthenticated,
    token_path: &Path,
) -> Result<MetisClient> {
    if let Some(token) = cli
        .token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return MetisClient::from_config(app_config, token.to_string());
    }

    if let Some(token) = read_token_from_path(token_path)? {
        return MetisClient::from_config(app_config, token);
    }

    github_device_flow::login_with_github_device_flow(unauth_client, token_path).await
}

async fn dispatch(
    cli: Cli,
    client: &dyn MetisClientInterface,
    app_config: &AppConfig,
    context: &CommandContext,
) -> Result<()> {
    match resolve_command(cli.command) {
        Commands::Jobs { command } => command::jobs::run(client, command, context).await?,
        Commands::Agents { command } => command::agents::run(client, command, context).await?,
        Commands::Patches { command } => command::patches::run(client, command, context).await?,
        Commands::Dashboard => {
            command::dashboard::run(
                client,
                &app_config.server.url,
                cli.browser.as_deref(),
                context,
            )
            .await?
        }
        Commands::Issues { command } => command::issues::run(client, command, context).await?,
        Commands::Repos { command } => command::repos::run(client, command, context).await?,
        Commands::Login => (),
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(app_config, prompt, model, full_auto, context).await?,
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
        config::create_default_config(&resolved_path)?;
    }

    AppConfig::load(&config_path)
}

fn read_token_from_path(token_path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(token_path) {
        Ok(token) => {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(anyhow!(
            "failed to read auth token from {}: {err}",
            token_path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        load_app_config, read_token_from_path, resolve_command, Cli, Commands, OutputFormat,
    };
    use crate::constants::{DEFAULT_AUTH_TOKEN_PATH, DEFAULT_SERVER_URL};
    use clap::Parser;
    use metis::command::agents::AgentsCommand;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn base_cli() -> Cli {
        Cli {
            config: None,
            server_url: None,
            token_path: DEFAULT_AUTH_TOKEN_PATH.to_string(),
            token: None,
            browser: None,
            output_format: OutputFormat::Auto,
            command: Some(super::Commands::Agents {
                command: AgentsCommand::List,
            }),
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
    fn load_config_missing_without_server_url_creates_default() {
        let temp = tempdir().expect("tempdir");
        let missing_path = temp.path().join("missing.toml");
        let cli = Cli {
            config: Some(missing_path.clone()),
            ..base_cli()
        };

        let config = load_app_config(&cli).expect("default config should be created");
        assert_eq!(config.server.url, DEFAULT_SERVER_URL);

        let contents = fs::read_to_string(missing_path).expect("read default config");
        assert!(
            contents.contains(DEFAULT_SERVER_URL),
            "default config missing server url"
        );
    }

    #[test]
    fn load_config_present_without_server_url_uses_config() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
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
    fn read_token_from_path_returns_none_when_missing() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("missing-token");

        let token = read_token_from_path(&token_path).expect("read token");
        assert!(token.is_none());
    }

    #[test]
    fn read_token_from_path_returns_none_when_empty() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");
        fs::write(&token_path, "   \n").expect("write token");

        let token = read_token_from_path(&token_path).expect("read token");
        assert!(token.is_none());
    }

    #[test]
    fn read_token_from_path_trims_contents() {
        let temp = tempdir().expect("tempdir");
        let token_path = temp.path().join("auth-token");
        fs::write(&token_path, "  token-123 \n").expect("write token");

        let token = read_token_from_path(&token_path).expect("read token");
        assert_eq!(token, Some("token-123".to_string()));
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
