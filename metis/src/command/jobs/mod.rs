use crate::{
    client::MetisClientInterface, command::output::CommandContext, worker_commands::CodexCommands,
};
use anyhow::Result;
use clap::Subcommand;
use metis_common::{
    constants::{ENV_METIS_ISSUE_ID, ENV_OPENAI_API_KEY},
    IssueId, MetisId, TaskId,
};
use std::path::PathBuf;

pub mod create;
pub mod kill;
pub mod list;
pub mod logs;
pub mod worker_run;

pub(crate) use crate::command::output::format_runtime;
pub use list::DEFAULT_JOB_LIMIT;

#[derive(Subcommand)]
pub enum JobsCommand {
    /// Create a new orchestration worker.
    Create {
        /// Wait for the job to complete and stream its logs.
        #[arg(long = "wait")]
        wait: bool,

        /// Service repo name (preferred) or git URL to use as the job context.
        #[arg(long = "repo", value_name = "REPO")]
        repo: Option<String>,

        /// Revision to use with --repo (defaults to 'main' for service repos and git URLs).
        #[arg(long = "rev", value_name = "REV", requires = "repo")]
        rev: Option<String>,

        /// Override the worker Docker image for this task.
        #[arg(long = "image", value_name = "IMAGE")]
        image: Option<String>,

        /// Override or set job variable (format: KEY=VALUE). Can be repeated.
        #[arg(long = "var", value_name = "KEY=VALUE")]
        var: Vec<String>,

        /// Issue to associate with the job (defaults to METIS_ISSUE_ID).
        #[arg(long = "issue-id", value_name = "ISSUE_ID", env = ENV_METIS_ISSUE_ID)]
        issue_id: Option<IssueId>,

        /// Prompt to execute, captured as trailing varargs.
        #[arg(
            value_name = "PROMPT",
            trailing_var_arg = true,
            num_args = 1..
        )]
        prompt: Vec<String>,
    },
    /// List all Metis jobs in the configured namespace.
    List {
        /// Number of jobs to display (most recent first).
        #[arg(
            short = 'n',
            long = "limit",
            value_name = "COUNT",
            default_value_t = DEFAULT_JOB_LIMIT,
        )]
        limit: usize,
        /// Filter jobs that were spawned from a specific issue.
        #[arg(long = "from", value_name = "ISSUE_ID")]
        spawned_from: Option<IssueId>,
    },
    /// Show logs for an existing Metis job.
    Logs {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`, or an IssueId to stream the most recent job spawned from that issue.
        #[arg(value_name = "ID")]
        id: MetisId,

        /// Stream logs if the job is still running.
        #[arg(short = 'w', long = "watch")]
        watch: bool,
    },
    /// Terminate a running Metis job.
    Kill {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`.
        #[arg(value_name = "JOB_ID")]
        job: TaskId,
    },
    /// Retrieve a job's context locally and run it via Codex.
    WorkerRun {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`.
        #[arg(value_name = "JOB_ID")]
        job: TaskId,
        /// Destination directory where the context will be extracted/copied.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// API key to pass to Codex (defaults to OPENAI_API_KEY).
        #[arg(long = "openai-api-key", value_name = "KEY", env = ENV_OPENAI_API_KEY)]
        openai_api_key: Option<String>,

        #[arg(long = "issue-id", value_name = "ISSUE_ID", env = ENV_METIS_ISSUE_ID)]
        issue_id: Option<IssueId>,
    },
}

pub async fn run(
    client: &dyn MetisClientInterface,
    command: JobsCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        JobsCommand::Create {
            wait,
            repo,
            rev,
            image,
            var,
            prompt,
            issue_id,
        } => {
            create::run(
                client, wait, repo, rev, image, var, prompt, issue_id, context,
            )
            .await?
        }
        JobsCommand::List {
            limit,
            spawned_from,
        } => list::run(client, limit, spawned_from, context).await?,
        JobsCommand::Logs { id, watch } => logs::run(client, id, watch, context).await?,
        JobsCommand::Kill { job } => kill::run(client, job, context).await?,
        JobsCommand::WorkerRun {
            job,
            path,
            openai_api_key,
            issue_id,
        } => {
            let commands = CodexCommands;
            worker_run::run(
                client,
                job,
                path,
                openai_api_key,
                issue_id,
                &commands,
                context,
            )
            .await?
        }
    }

    Ok(())
}
