use crate::client::MetisClientInterface;
use anyhow::Result;
use clap::{ArgGroup, Args, Subcommand};
use metis_common::{IssueId, TaskId};

pub mod create;
pub mod kill;
pub mod list;
pub mod logs;

pub(crate) use list::format_runtime;
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

        /// Revision to use with --repo (optional for service repos, required for URLs).
        #[arg(long = "rev", value_name = "REV", requires = "repo")]
        rev: Option<String>,

        /// Override the worker Docker image for this task.
        #[arg(long = "image", value_name = "IMAGE")]
        image: Option<String>,

        /// Override or set job variable (format: KEY=VALUE). Can be repeated.
        #[arg(long = "var", value_name = "KEY=VALUE")]
        var: Vec<String>,

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
        #[command(flatten)]
        job: JobIdArg,

        /// Stream logs if the job is still running.
        #[arg(short = 'w', long = "watch")]
        watch: bool,
    },
    /// Terminate a running Metis job.
    Kill {
        #[command(flatten)]
        job: JobIdArg,
    },
}

pub async fn run(client: &dyn MetisClientInterface, command: JobsCommand) -> Result<()> {
    match command {
        JobsCommand::Create {
            wait,
            repo,
            rev,
            image,
            var,
            prompt,
        } => create::run(client, wait, repo, rev, image, var, prompt).await?,
        JobsCommand::List {
            limit,
            spawned_from,
        } => list::run(client, limit, spawned_from).await?,
        JobsCommand::Logs { job, watch } => logs::run(client, job.into_task_id(), watch).await?,
        JobsCommand::Kill { job } => kill::run(client, job.into_task_id()).await?,
    }

    Ok(())
}

#[derive(Args, Clone, Debug)]
#[command(
    group(
        ArgGroup::new("job_id_source")
            .required(true)
            .args(&["job_id", "job_id_positional"])
    )
)]
pub struct JobIdArg {
    /// Job identifier returned by `metis jobs create` or `metis jobs list`.
    #[arg(
        long = "job-id",
        value_name = "JOB_ID",
        alias = "job",
        group = "job_id_source"
    )]
    job_id: Option<TaskId>,

    /// (deprecated) Job identifier passed positionally; prefer --job-id.
    #[arg(value_name = "JOB_ID", hide = true, group = "job_id_source")]
    job_id_positional: Option<TaskId>,
}

impl JobIdArg {
    pub fn into_task_id(self) -> TaskId {
        self.job_id
            .or(self.job_id_positional)
            .expect("job_id_source requires a job id")
    }
}
