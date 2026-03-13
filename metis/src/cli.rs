use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{
    client::{MetisClient, MetisClientInterface, MetisClientUnauthenticated},
    command::{
        self,
        output::{resolve_output_format, CommandContext, OutputFormat},
    },
    config::{self, empty_app_config, AppConfig},
    constants, github_device_flow,
};
use metis_common::constants::{ENV_BROWSER, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN};

#[derive(Parser)]
#[command(name = "metis", version)]
pub struct Cli {
    /// Path to the CLI configuration file.
    #[arg(long, value_name = "FILE", global = true)]
    pub config: Option<PathBuf>,

    /// Override the Metis server URL (also via METIS_SERVER_URL).
    #[arg(
        long = "server-url",
        value_name = "URL",
        env = ENV_METIS_SERVER_URL,
        global = true
    )]
    pub server_url: Option<String>,

    /// Auth token value (also via env var).
    #[arg(
        long = "token",
        env = ENV_METIS_TOKEN,
        value_name = "TOKEN",
        global = true
    )]
    pub token: Option<String>,

    /// Browser command for opening links (defaults to $BROWSER).
    #[arg(long = "browser", value_name = "COMMAND", env = ENV_BROWSER, global = true)]
    pub browser: Option<String>,

    /// Output format (auto, jsonl, or pretty).
    #[arg(
        long = "output-format",
        value_enum,
        default_value = "auto",
        global = true
    )]
    pub output_format: OutputFormat,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
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

/// Check whether both server URL and token are available via CLI flags or env vars.
fn has_env_credentials(cli: &Cli) -> bool {
    let has_url = cli
        .server_url
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let has_token = cli
        .token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    has_url && has_token
}

/// Resolve the app config: if env vars provide both server URL and token,
/// skip config file loading entirely; otherwise load from the config file.
pub fn resolve_app_config(cli: &Cli) -> Result<(AppConfig, PathBuf)> {
    let config_path = resolve_config_path(cli);
    if has_env_credentials(cli) {
        Ok((empty_app_config(), config_path))
    } else {
        let app_config = load_app_config(&config_path)?;
        Ok((app_config, config_path))
    }
}

/// Run the CLI with the given parsed arguments.
pub async fn run(cli: Cli) -> Result<()> {
    let (app_config, config_path) = resolve_app_config(&cli)?;
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
/// We walk the full chain because serialization layers (e.g. `serde_json::Error`)
/// may wrap the underlying I/O error.
pub fn is_broken_pipe(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(io_err) = cause.downcast_ref::<std::io::Error>() {
            if io_err.kind() == ErrorKind::BrokenPipe {
                return true;
            }
        }
    }
    false
}

pub async fn resolve_client(
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

pub async fn dispatch(
    cli: Cli,
    client: &dyn MetisClientInterface,
    server_url: &str,
    context: &CommandContext,
) -> Result<()> {
    match resolve_command(cli.command) {
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

pub fn resolve_command(command: Option<Commands>) -> Commands {
    command.unwrap_or(Commands::Dashboard)
}

pub fn resolve_config_path(cli: &Cli) -> PathBuf {
    cli.config
        .clone()
        .unwrap_or_else(|| PathBuf::from(constants::DEFAULT_CONFIG_FILE))
}

pub fn load_app_config(config_path: &Path) -> Result<AppConfig> {
    let resolved_path = config::expand_path(config_path);
    if !resolved_path.exists() {
        anyhow::bail!(
            "No configuration file found at '{}'. Run 'metis server init' or create a config file with your server URL.",
            resolved_path.display()
        );
    }

    AppConfig::load(&resolved_path)
}

pub fn resolve_server_url(cli: &Cli, app_config: &AppConfig) -> Result<String> {
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
    use crate::command::agents::AgentsCommand;
    use clap::Parser;
    use tempfile::tempdir;

    fn base_cli() -> Cli {
        Cli {
            config: None,
            server_url: None,
            token: None,
            browser: None,
            output_format: OutputFormat::Auto,
            command: Some(Commands::Agents {
                command: AgentsCommand::List,
            }),
        }
    }

    #[test]
    fn resolve_server_url_prefers_cli_override() {
        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            ..base_cli()
        };

        let config = empty_app_config();
        let server_url = resolve_server_url(&cli, &config).expect("resolve server url");
        assert_eq!(server_url, "http://localhost:9000");
    }

    #[test]
    fn load_config_missing_errors_with_helpful_message() {
        let temp = tempdir().expect("tempdir");
        let missing_path = temp.path().join("missing.toml");
        let err = load_app_config(&missing_path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("No configuration file found"),
            "expected helpful error, got: {msg}"
        );
        assert!(
            msg.contains("metis server init"),
            "expected init hint, got: {msg}"
        );
    }

    #[test]
    fn load_config_present_without_server_url_uses_config() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "[[servers]]\nurl = \"http://127.0.0.1:8080\"\ndefault = true\n",
        )
        .expect("write config");

        let config = load_app_config(&config_path).expect("config should load from file");
        let server_url = config.default_server().expect("default server");
        assert_eq!(server_url.url, "http://127.0.0.1:8080");
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
    fn is_broken_pipe_detects_direct_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe closed");
        let err: anyhow::Error = io_err.into();
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn is_broken_pipe_detects_wrapped_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe closed");
        let err = anyhow::Error::new(io_err).context("writing output");
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn is_broken_pipe_returns_false_for_other_errors() {
        let err = anyhow::anyhow!("some other error");
        assert!(!is_broken_pipe(&err));
    }

    #[test]
    fn is_broken_pipe_returns_false_for_other_io_errors() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err: anyhow::Error = io_err.into();
        assert!(!is_broken_pipe(&err));
    }

    #[test]
    fn resolve_app_config_uses_empty_config_when_env_vars_present() {
        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            token: Some("test-token".to_string()),
            ..base_cli()
        };

        let (config, _) = resolve_app_config(&cli).expect("resolve app config");
        assert!(
            config.servers.is_empty(),
            "expected empty config when env vars are set"
        );
    }

    #[test]
    fn resolve_app_config_skips_config_file_when_env_vars_present() {
        // Even with a nonexistent config path, env vars should bypass config loading.
        let cli = Cli {
            config: Some(PathBuf::from("/nonexistent/config.toml")),
            server_url: Some("http://localhost:9000".to_string()),
            token: Some("test-token".to_string()),
            ..base_cli()
        };

        let (config, _) = resolve_app_config(&cli).expect("should not fail on missing config");
        assert!(config.servers.is_empty());
    }

    #[test]
    fn resolve_app_config_falls_back_to_config_file_without_env_vars() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "[[servers]]\nurl = \"http://127.0.0.1:8080\"\ndefault = true\n",
        )
        .expect("write config");

        let cli = Cli {
            config: Some(config_path),
            ..base_cli()
        };

        let (config, _) = resolve_app_config(&cli).expect("resolve app config");
        let server = config.default_server().expect("default server");
        assert_eq!(server.url, "http://127.0.0.1:8080");
    }

    #[test]
    fn resolve_app_config_errors_when_no_env_vars_and_no_config_file() {
        let cli = Cli {
            config: Some(PathBuf::from("/nonexistent/config.toml")),
            ..base_cli()
        };

        let err = resolve_app_config(&cli).unwrap_err();
        assert!(
            err.to_string().contains("No configuration file found"),
            "expected config file error, got: {err}"
        );
    }

    #[test]
    fn has_env_credentials_requires_both_url_and_token() {
        // Only URL set
        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            ..base_cli()
        };
        assert!(!has_env_credentials(&cli));

        // Only token set
        let cli = Cli {
            token: Some("test-token".to_string()),
            ..base_cli()
        };
        assert!(!has_env_credentials(&cli));

        // Both set
        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            token: Some("test-token".to_string()),
            ..base_cli()
        };
        assert!(has_env_credentials(&cli));

        // Neither set
        assert!(!has_env_credentials(&base_cli()));
    }

    #[test]
    fn has_env_credentials_ignores_empty_strings() {
        let cli = Cli {
            server_url: Some("".to_string()),
            token: Some("test-token".to_string()),
            ..base_cli()
        };
        assert!(!has_env_credentials(&cli));

        let cli = Cli {
            server_url: Some("http://localhost:9000".to_string()),
            token: Some("  ".to_string()),
            ..base_cli()
        };
        assert!(!has_env_credentials(&cli));
    }
}
