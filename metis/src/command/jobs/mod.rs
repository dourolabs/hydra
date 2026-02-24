use crate::{
    client::MetisClientInterface,
    command::{
        changelog::{summarize_activity_log, write_changelog_pretty},
        output::{render_job_records, CommandContext, ResolvedOutputFormat},
    },
    worker_commands::ModelAwareCommands,
};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use metis_common::{
    activity_log_for_job_versions,
    constants::{
        ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_METIS_ISSUE_ID, ENV_OPENAI_API_KEY,
    },
    jobs::Task,
    IssueId, MetisId, RelativeVersionNumber, TaskId, Versioned,
};
use std::{
    io::{self, Write},
    path::PathBuf,
};

pub mod create;
pub mod kill;
pub mod list;
pub mod logs;
pub mod worker_run;

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
    /// List all Metis jobs in the configured namespace. Returns summary records with a truncated prompt; use `get` for full details.
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
    /// Get the full details of a single job by ID. Returns the complete job record including the full prompt, context, and configuration.
    Get {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`.
        #[arg(value_name = "JOB_ID")]
        id: TaskId,

        /// Retrieve a specific version (positive = exact version, negative = offset from latest).
        #[arg(long)]
        version: Option<i64>,
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
    /// Show changelog for a job (most recent first).
    Changelog {
        /// Job identifier returned by `metis jobs create` or `metis jobs list`.
        #[arg(value_name = "JOB_ID")]
        id: TaskId,

        /// Maximum number of changelog entries to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Retrieve a job's context locally and run it via Codex or Claude.
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
        /// API key to pass to Claude (defaults to ANTHROPIC_API_KEY).
        #[arg(
            long = "anthropic-api-key",
            value_name = "KEY",
            env = ENV_ANTHROPIC_API_KEY
        )]
        anthropic_api_key: Option<String>,
        /// OAuth token to pass to Claude Code (defaults to CLAUDE_CODE_OAUTH_TOKEN).
        #[arg(
            long = "claude-code-oauth-token",
            value_name = "TOKEN",
            env = ENV_CLAUDE_CODE_OAUTH_TOKEN
        )]
        claude_code_oauth_token: Option<String>,

        #[arg(long = "issue-id", value_name = "ISSUE_ID", env = ENV_METIS_ISSUE_ID)]
        issue_id: Option<IssueId>,

        /// Use a temporary directory as the working destination instead of PATH.
        #[arg(long = "tempdir")]
        tempdir: bool,
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
        JobsCommand::Get { id, version } => get_job(client, &id, version, context).await?,
        JobsCommand::Changelog { id, limit } => {
            changelog_job(client, id, context.output_format, limit).await?
        }
        JobsCommand::Logs { id, watch } => logs::run(client, id, watch, context).await?,
        JobsCommand::Kill { job } => kill::run(client, job, context).await?,
        JobsCommand::WorkerRun {
            job,
            path,
            openai_api_key,
            anthropic_api_key,
            claude_code_oauth_token,
            issue_id,
            tempdir,
        } => {
            let commands = ModelAwareCommands::default();
            worker_run::run(
                client,
                job,
                path,
                openai_api_key,
                anthropic_api_key,
                claude_code_oauth_token,
                issue_id,
                tempdir,
                &commands,
                context,
            )
            .await?
        }
    }

    Ok(())
}

async fn get_job(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    version: Option<i64>,
    context: &CommandContext,
) -> Result<()> {
    let job = match version {
        Some(0) => {
            bail!("--version 0 is not valid; use a positive version number or a negative offset")
        }
        Some(v) => client
            .get_job_version(job_id, RelativeVersionNumber::new(v))
            .await
            .with_context(|| format!("failed to fetch version {v} of job '{job_id}'"))?,
        None => client
            .get_job(job_id)
            .await
            .with_context(|| format!("failed to fetch job '{job_id}'"))?,
    };
    render_job_records(context.output_format, &[job], &mut std::io::stdout())?;
    Ok(())
}

async fn changelog_job(
    client: &dyn MetisClientInterface,
    id: TaskId,
    output_format: ResolvedOutputFormat,
    limit: usize,
) -> Result<()> {
    let response = client
        .list_job_versions(&id)
        .await
        .with_context(|| format!("failed to fetch versions for job '{id}'"))?;
    let versions: Vec<Versioned<Task>> = response
        .versions
        .into_iter()
        .map(|record| Versioned::new(record.task, record.version, record.timestamp))
        .collect();
    let entries = activity_log_for_job_versions(id, &versions);
    let mut summaries = summarize_activity_log(&entries)?;
    summaries.reverse();
    summaries.truncate(limit);

    let mut buffer = Vec::new();
    match output_format {
        ResolvedOutputFormat::Pretty => {
            write_changelog_pretty(&summaries, &mut buffer)?;
        }
        ResolvedOutputFormat::Jsonl => {
            for entry in &summaries {
                serde_json::to_writer(&mut buffer, entry)?;
                buffer.write_all(b"\n")?;
            }
        }
    }
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    Ok(())
}
