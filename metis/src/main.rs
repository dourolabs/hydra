#![allow(clippy::too_many_arguments)]

mod client;
mod command;
mod config;
mod constants;
mod exec;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use client::MetisClient;
use config::AppConfig;
use metis_common::constants::ENV_METIS_SERVER_URL;
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

        /// Branch or commit to use as the starting point for the job.
        #[arg(long = "from", value_name = "REV")]
        from: Option<String>,

        /// Git repository URL to clone when providing --from.
        #[arg(long = "repo-url", value_name = "URL")]
        repo_url: Option<String>,

        /// Named GitHub repository configured on the server to use as context.
        #[arg(
            long = "service-repo",
            value_name = "NAME",
            conflicts_with_all = ["context_dir", "repo_url", "encode_directory", "encode_git_bundle", "from"]
        )]
        service_repo: Option<String>,

        /// Optional revision to use for --service-repo.
        #[arg(
            long = "service-repo-rev",
            value_name = "REV",
            requires = "service_repo",
            conflicts_with_all = ["context_dir", "repo_url", "encode_directory", "encode_git_bundle", "from"]
        )]
        service_repo_rev: Option<String>,

        /// Override the worker Docker image for this task.
        #[arg(long = "image", value_name = "IMAGE")]
        image: Option<String>,

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

        /// Override or set job variable (format: KEY=VALUE). Can be repeated.
        #[arg(long = "var", value_name = "KEY=VALUE")]
        var: Vec<String>,

        /// Rhai program to execute. Can be a file path or an inline script.
        #[arg(long = "program", value_name = "PROGRAM", required = true)]
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
        id: String,

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
    /// Retrieve a job's context and extract/copy it to a directory, then submit the job output.
    WorkerRun {
        /// Job identifier returned by `metis spawn` or `metis jobs`.
        #[arg(value_name = "JOB_ID")]
        job: String,
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
            from,
            repo_url,
            service_repo,
            service_repo_rev,
            image,
            context_dir,
            encode_directory,
            encode_git_bundle,
            after,
            var,
            program,
            prompt,
        } => {
            command::spawn::run(
                &client,
                wait,
                from,
                repo_url,
                service_repo,
                service_repo_rev,
                image,
                context_dir,
                encode_directory,
                encode_git_bundle,
                after,
                var,
                program,
                prompt,
            )
            .await?
        }
        Commands::Jobs { limit } => command::jobs::run(&client, limit).await?,
        Commands::Logs { id, watch } => command::logs::run(&client, id, watch).await?,
        Commands::Kill { job } => command::kill::run(&client, job).await?,
        Commands::Patch { job, apply } => command::patch::run(&client, job, apply).await?,

        Commands::WorkerRun { job, path } => command::worker_run::run(&client, job, path).await?,
        Commands::Run { script } => command::run::run(script).await?,
    }

    Ok(())
}
