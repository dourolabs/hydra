use crate::{
    client::{MetisClient, MetisClientInterface},
    command,
    config::{self, AppConfig},
    constants,
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use metis_common::constants::ENV_METIS_SERVER_URL;
use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

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
        #[arg(long = "username", value_name = "USERNAME")]
        username: Option<String>,
    },
    /// List or create issues.
    Issues {
        #[command(subcommand)]
        command: command::issues::IssueCommands,
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
/// Environment variables from `env` (if provided) are set before parsing CLI arguments
/// so they can be used to fill in values for CLI command structs with `env` attributes.
pub async fn run_with_client_and_config<I, T>(
    args: I,
    client: &dyn MetisClientInterface,
    app_config: &AppConfig,
) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_client_and_config_and_env(args, client, app_config, None, None).await
}

/// Global mutex to guard environment variable modifications during CLI parsing.
/// This prevents race conditions when multiple CLI parsing operations run concurrently,
/// which is especially important during testing.
static ENV_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

/// Execute the metis CLI using the provided CLI arguments, client, and environment variables.
/// Environment variables from `env` are set before parsing CLI arguments so they can be used
/// to fill in values for CLI command structs with `env` attributes.
/// If `working_dir` is provided, the current directory is changed before dispatch and restored after.
pub async fn run_with_client_and_config_and_env<I, T>(
    args: I,
    client: &dyn MetisClientInterface,
    app_config: &AppConfig,
    env: Option<&HashMap<String, String>>,
    working_dir: Option<&Path>,
) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // Collect args before acquiring lock to minimize lock duration
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();

    // Save original working directory if we need to change it
    let original_dir = if working_dir.is_some() {
        Some(env::current_dir().context("failed to get current directory")?)
    } else {
        None
    };

    // Acquire guard to serialize environment variable modifications, parsing, and restoration
    // This prevents race conditions during concurrent CLI parsing operations in tests
    let guard = ENV_GUARD.get_or_init(|| Mutex::new(()));
    let cli = {
        let _lock = guard
            .lock()
            .expect("env guard mutex should not be poisoned");

        // Save existing env var values and set new ones if provided
        let mut saved_vars: HashMap<String, Option<String>> = HashMap::new();
        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                let old_value = env::var(key).ok();
                saved_vars.insert(key.clone(), old_value);
                env::set_var(key, value);
            }
        }

        // Parse CLI with environment variables set (still holding the lock)
        let cli = Cli::parse_from(&args);

        // Restore original env var values (still holding the lock)
        for (key, old_value) in saved_vars {
            match old_value {
                Some(val) => env::set_var(key, val),
                None => env::remove_var(key),
            }
        }
        // Lock is released here when _lock is dropped

        cli
    };

    // Change working directory if specified
    if let Some(dir) = working_dir {
        env::set_current_dir(dir)
            .with_context(|| format!("failed to change directory to {dir:?}"))?;
    }

    // Dispatch command and restore working directory after
    let result = dispatch(cli, client, app_config).await;

    // Restore original working directory if we changed it
    if let Some(original) = original_dir {
        if let Err(e) = env::set_current_dir(&original) {
            eprintln!("warning: failed to restore working directory to {original:?}: {e}");
        }
    }

    result
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
