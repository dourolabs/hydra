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
    /// Spawn a new orchestration worker.
    Spawn {
        /// Wait for the job to complete and stream its logs.
        #[arg(long = "wait")]
        wait: bool,

        /// Service repo name (preferred) or git URL to use as the job context.
        #[arg(long = "repo", value_name = "REPO")]
        repo: Option<String>,

        /// Revision to use with --repo (optional for service repos, required for URLs).
        #[arg(long = "rev", value_name = "REV", requires = "repo")]
        rev: Option<String>,

        /// Override the worker Docker image for this task.
        #[arg(long = "image", value_name = "IMAGE")]
        image: Option<String>,

        /// Override or set job variable (format: KEY=VALUE). Can be repeated.
        #[arg(long = "var", value_name = "KEY=VALUE")]
        var: Vec<String>,

        /// Rhai program to execute. Can be a file path or an inline script.
        #[arg(
            long = "program",
            value_name = "PROGRAM",
            default_value = constants::DEFAULT_PROGRAM_PATH
        )]
        program: String,

        /// Prompt to execute, captured as trailing varargs.
        #[arg(
            value_name = "PROMPT",
            trailing_var_arg = true,
            num_args = 1..
        )]
        prompt: Vec<String>,
    },
    /// List all Metis jobs in the configured namespace.
    Jobs {
        /// Number of jobs to display (most recent first).
        #[arg(
            short = 'n',
            long = "limit",
            value_name = "COUNT",
            default_value_t = command::jobs::DEFAULT_JOB_LIMIT,
        )]
        limit: usize,
    },
    /// Show logs for an existing Metis job.
    Logs {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "ID")]
        id: TaskId,

        /// Stream logs if the job is still running.
        #[arg(short = 'w', long = "watch")]
        watch: bool,
    },
    /// Terminate a running Metis job.
    Kill {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: TaskId,
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
    /// Retrieve a job's context and extract/copy it to a directory, then submit the job output.
    WorkerRun {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: TaskId,
        /// Destination directory where the context will be extracted/copied.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Run a Rhai script.
    Run {
        /// Rhai script to execute. Can be a file path or a one-line script.
        #[arg(value_name = "SCRIPT_OR_FILE")]
        script: String,
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
        Commands::Spawn {
            wait,
            repo,
            rev,
            image,
            var,
            program,
            prompt,
        } => command::spawn::run(&client, wait, repo, rev, image, var, program, prompt).await?,
        Commands::Jobs { limit } => command::jobs::run(&client, limit).await?,
        Commands::Logs { id, watch } => command::logs::run(&client, id, watch).await?,
        Commands::Kill { job } => command::kill::run(&client, job).await?,
        Commands::Patches { command } => command::patches::run(&client, command).await?,
        Commands::Dashboard => command::dashboard::run(&client).await?,
        Commands::Issues { command } => command::issues::run(&client, command).await?,

        Commands::WorkerRun { job, path } => command::worker_run::run(&client, job, path).await?,
        Commands::Run { script } => command::run::run(script).await?,
        Commands::Chat {
            prompt,
            model,
            full_auto,
        } => command::chat::run(&app_config, prompt, model, full_auto).await?,
    }

    Ok(())
}
