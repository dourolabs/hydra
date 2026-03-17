use crate::{
    client::HydraClientInterface,
    command::{
        output::{render_session_records, CommandContext, ResolvedOutputFormat},
        utils::changelog::{summarize_activity_log, write_changelog_pretty},
    },
    worker_commands::ModelAwareCommands,
};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use hydra_common::{
    activity_log_for_session_versions, constants::ENV_HYDRA_ISSUE_ID, sessions::Session, HydraId,
    IssueId, RelativeVersionNumber, SessionId, Versioned,
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

pub use list::DEFAULT_SESSION_LIMIT;

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new orchestration worker.
    Create {
        /// Wait for the session to complete and stream its logs.
        #[arg(long = "wait")]
        wait: bool,

        /// Service repo name (preferred) or git URL to use as the session context.
        #[arg(long = "repo", value_name = "REPO")]
        repo: Option<String>,

        /// Revision to use with --repo (defaults to 'main' for service repos and git URLs).
        #[arg(long = "rev", value_name = "REV", requires = "repo")]
        rev: Option<String>,

        /// Override the worker Docker image for this session.
        #[arg(long = "image", value_name = "IMAGE")]
        image: Option<String>,

        /// Override or set session variable (format: KEY=VALUE). Can be repeated.
        #[arg(long = "var", value_name = "KEY=VALUE")]
        var: Vec<String>,

        /// Issue to associate with the session (defaults to HYDRA_ISSUE_ID).
        #[arg(long = "issue-id", value_name = "ISSUE_ID", env = ENV_HYDRA_ISSUE_ID)]
        issue_id: Option<IssueId>,

        /// Prompt to execute, captured as trailing varargs.
        #[arg(
            value_name = "PROMPT",
            trailing_var_arg = true,
            num_args = 1..
        )]
        prompt: Vec<String>,
    },
    /// List all Hydra sessions in the configured namespace. Returns summary records with a truncated prompt; use `get` for full details.
    List {
        /// Number of sessions to display (most recent first).
        #[arg(
            short = 'n',
            long = "limit",
            value_name = "COUNT",
            default_value_t = DEFAULT_SESSION_LIMIT,
        )]
        limit: usize,
        /// Filter sessions that were spawned from a specific issue.
        #[arg(long = "from", value_name = "ISSUE_ID")]
        spawned_from: Option<IssueId>,
    },
    /// Get the full details of a single session by ID. Returns the complete session record including the full prompt, context, and configuration.
    Get {
        /// Session identifier returned by `hydra sessions create` or `hydra sessions list`.
        #[arg(value_name = "SESSION_ID")]
        id: SessionId,

        /// Retrieve a specific version (positive = exact version, negative = offset from latest).
        #[arg(long)]
        version: Option<i64>,
    },
    /// Show logs for an existing Hydra session.
    Logs {
        /// Session identifier returned by `hydra sessions create` or `hydra sessions list`, or an IssueId to stream the most recent session spawned from that issue.
        #[arg(value_name = "ID")]
        id: HydraId,

        /// Stream logs if the session is still running.
        #[arg(short = 'w', long = "watch")]
        watch: bool,
    },
    /// Terminate a running Hydra session.
    Kill {
        /// Session identifier returned by `hydra sessions create` or `hydra sessions list`.
        #[arg(value_name = "SESSION_ID")]
        session: SessionId,
    },
    /// Show changelog for a session (most recent first).
    Changelog {
        /// Session identifier returned by `hydra sessions create` or `hydra sessions list`.
        #[arg(value_name = "SESSION_ID")]
        id: SessionId,

        /// Maximum number of changelog entries to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Retrieve a session's context locally and run it via Codex or Claude.
    WorkerRun {
        /// Session identifier returned by `hydra sessions create` or `hydra sessions list`.
        #[arg(value_name = "SESSION_ID")]
        session: SessionId,
        /// Destination directory where the context will be extracted/copied.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        #[arg(long = "issue-id", value_name = "ISSUE_ID", env = ENV_HYDRA_ISSUE_ID)]
        issue_id: Option<IssueId>,

        /// Use a temporary directory as the working destination instead of PATH.
        #[arg(long = "tempdir")]
        tempdir: bool,
    },
}

pub async fn run(
    client: &dyn HydraClientInterface,
    command: SessionsCommand,
    context: &CommandContext,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
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
        SessionsCommand::List {
            limit,
            spawned_from,
        } => list::run(client, limit, spawned_from, context).await?,
        SessionsCommand::Get { id, version } => get_session(client, &id, version, context).await?,
        SessionsCommand::Changelog { id, limit } => {
            changelog_session(client, id, context.output_format, limit).await?
        }
        SessionsCommand::Logs { id, watch } => logs::run(client, id, watch, context).await?,
        SessionsCommand::Kill { session } => kill::run(client, session, context).await?,
        SessionsCommand::WorkerRun {
            session,
            path,
            issue_id,
            tempdir,
        } => {
            let commands = ModelAwareCommands::default();
            worker_run::run(client, session, path, issue_id, tempdir, &commands, context).await?
        }
    }

    Ok(())
}

async fn get_session(
    client: &dyn HydraClientInterface,
    session_id: &SessionId,
    version: Option<i64>,
    context: &CommandContext,
) -> Result<()> {
    let session = match version {
        Some(0) => {
            bail!("--version 0 is not valid; use a positive version number or a negative offset")
        }
        Some(v) => client
            .get_session_version(session_id, RelativeVersionNumber::new(v))
            .await
            .with_context(|| format!("failed to fetch version {v} of session '{session_id}'"))?,
        None => client
            .get_session(session_id)
            .await
            .with_context(|| format!("failed to fetch session '{session_id}'"))?,
    };
    render_session_records(context.output_format, &[session], &mut std::io::stdout())?;
    Ok(())
}

async fn changelog_session(
    client: &dyn HydraClientInterface,
    id: SessionId,
    output_format: ResolvedOutputFormat,
    limit: usize,
) -> Result<()> {
    let response = client
        .list_session_versions(&id)
        .await
        .with_context(|| format!("failed to fetch versions for session '{id}'"))?;
    let versions: Vec<Versioned<Session>> = response
        .versions
        .into_iter()
        .map(|record| {
            Versioned::new(
                record.session,
                record.version,
                record.timestamp,
                record.timestamp,
            )
        })
        .collect();
    let entries = activity_log_for_session_versions(id, &versions);
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
