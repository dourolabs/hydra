#![allow(clippy::too_many_arguments)]

mod client;
mod command;
mod config;

use anyhow::Result;
use clap::{Parser, Subcommand};
use client::MetisClient;
use config::AppConfig;
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
        #[arg(short = 'w', long = "wait")]
        wait: bool,

        /// Branch or commit to use as the starting point for the job.
        #[arg(long = "from", value_name = "REV")]
        from: Option<String>,

        /// Git repository URL to clone when providing --from.
        #[arg(long = "repo-url", value_name = "URL")]
        repo_url: Option<String>,

        /// Directory to upload as the job context (will be archived and base64 encoded).
        #[arg(long = "context-dir", value_name = "PATH")]
        context_dir: Option<PathBuf>,

        /// Force --context-dir to be encoded as a tar archive, even if it is a git repo.
        #[arg(long = "encode-directory", conflicts_with = "encode_git_bundle")]
        encode_directory: bool,

        /// Force --context-dir to be encoded as a git bundle.
        #[arg(long = "encode-git-bundle")]
        encode_git_bundle: bool,

        /// Create the job after the given Metis job ID (repeatable).
        #[arg(long = "after", value_name = "JOB_ID")]
        after: Vec<String>,

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
    /// Terminate a running Metis job.
    Kill {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,
    },
    /// Retrieve and display the patch for a completed job.
    Patch {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,

        /// Apply the patch to the current git repository using `git apply`.
        #[arg(short = 'a', long = "apply")]
        apply: bool,
    },
    /// Retrieve a job's context and extract/copy it to a directory.
    Context {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,
        /// Destination directory where the context will be extracted/copied.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Set the recorded output for a job.
    SetOutput {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,
        /// Path to a file containing the last agent message to store.
        #[arg(long = "last-message", value_name = "FILE")]
        last_message: PathBuf,
        /// Path to a file containing the patch text to store.
        #[arg(long = "patch", value_name = "FILE")]
        patch: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let app_config = match env::var("METIS_SERVER_URL") {
        Ok(url) if !url.trim().is_empty() => AppConfig {
            server: config::ServerSection { url },
        },
        _ => {
            let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
            AppConfig::load(&config_path)?
        }
    };
    let client = MetisClient::from_config(&app_config)?;

    match cli.command {
        Commands::Spawn {
            wait,
            from,
            repo_url,
            context_dir,
            encode_directory,
            encode_git_bundle,
            after,
            prompt,
        } => {
            command::spawn::run(
                &client,
                wait,
                from,
                repo_url,
                context_dir,
                encode_directory,
                encode_git_bundle,
                after,
                prompt,
            )
            .await?
        }
        Commands::Jobs => command::jobs::run(&client).await?,
        Commands::Logs { job, watch } => command::logs::run(&client, job, watch).await?,
        Commands::Kill { job } => command::kill::run(&client, job).await?,
        Commands::Patch { job, apply } => command::patch::run(&client, job, apply).await?,
        Commands::Context { job, path } => command::context::run(&client, job, path).await?,
        Commands::SetOutput {
            job,
            last_message,
            patch,
        } => command::set_output::run(&client, job, last_message, patch).await?,
    }

    Ok(())
}
