//! `hydra-migrate-sessions` — one-off data-migration tool for the
//! sessions-orthogonality redesign
//! (`/designs/sessions-orthogonality-redesign.md`).
//!
//! # Subcommands
//!
//! * `migrate-state` — copy `conversation_session_state` rows into the new
//!   `session_state` storage, keyed on the producing session id
//!   (design §3.5 step 4).
//! * `migrate-events` — copy `conversation_events_v2` user/assistant
//!   message rows into `session_events_v2`, partitioned by the active
//!   session at write time (design §3.5 step 3 + §6 step 8). Supports
//!   `--up-to <TIMESTAMP>` for clean cut-over with the dual-write path
//!   (PR-1, `i-aankjvnz`): only source rows `created_at < T` are migrated,
//!   leaving newer rows for the dual-write path.
//!
//! # Run order & prerequisites
//!
//! 1. Phase B migrations must have run on the target DB so the new
//!    `session_events` / `session_events_v2` and
//!    `session_state` / `session_state_v2` tables exist.
//! 2. `migrate-state` must run BEFORE the
//!    `conversation_session_state` source table / column is dropped in
//!    Phase E step 19.
//!
//! Recommended sequence on a deployed db:
//!
//! ```text
//! # 1. Inspect + apply the state-blob migration.
//! hydra-migrate-sessions migrate-state --dsn <DSN> --dry-run
//! hydra-migrate-sessions migrate-state --dsn <DSN>
//!
//! # 2. Inspect + apply the historical-events migration. Pass --up-to
//! #    with the dual-write cut-over timestamp to leave anything newer
//! #    to the worker dual-write path (PR-1, i-aankjvnz).
//! hydra-migrate-sessions migrate-events --dsn <DSN> --up-to <T> --dry-run
//! hydra-migrate-sessions migrate-events --dsn <DSN> --up-to <T>
//! ```
//!
//! # Expected runtime
//!
//! `migrate-state` runs one `SELECT … WHERE conversation_id = ?` + one
//! existence check + (at most) one INSERT per conversation that has a
//! state blob — a few milliseconds per row on a warm postgres. For the
//! 10⁴–10⁵ rows we expect in prod the whole pass should finish in well
//! under a minute. For larger datasets, run it during a maintenance
//! window so the dual-write traffic from Phase C step 7 doesn't race
//! the cut-over.
//!
//! # Rollback / safety
//!
//! The tool only writes to the *new* `session_state` / `session_state_v2`
//! tables and never touches the source columns. The writes use
//! `INSERT … ON CONFLICT DO NOTHING`, so re-running is a no-op. If a
//! pass is interrupted, re-running picks up the remaining rows without
//! reprocessing the ones already migrated.
//!
//! # Output format
//!
//! All subcommands emit one JSON object per processed row to stdout
//! (JSON Lines):
//!
//! ```jsonl
//! {"conversation_id":"c-abc…","producing_session_id":"s-def…","byte_len":12345,"action":"would-write"}
//! ```
//!
//! `action` is one of `"would-write"` / `"would-skip"` (dry-run) or
//! `"wrote"` / `"skipped"` (live).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use hydra_server::migration_tool::{Backend, events, state};

#[derive(Debug, Parser)]
#[command(
    name = "hydra-migrate-sessions",
    about = "One-off data-migration tool for the sessions-orthogonality redesign (\
             designs/sessions-orthogonality-redesign.md)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Copy `conversation_session_state` rows into `session_state`, keyed on
    /// the producing session id. Re-running is a no-op.
    MigrateState(MigrateStateArgs),
    /// Partition historical `conversation_events_v2` user/assistant message
    /// rows by the active session at write time and write them to
    /// `session_events*`. Re-running is a no-op (sessions that already have
    /// rows are skipped).
    MigrateEvents(MigrateEventsArgs),
}

#[derive(Debug, clap::Args)]
struct MigrateStateArgs {
    /// Database DSN. Either `sqlite:<path>` (e.g. `sqlite:./hydra.db?mode=rwc`,
    /// `sqlite::memory:`) or `postgres(ql)://USER:PASSWORD@HOST/DB`. The
    /// `postgres` variant requires the binary to be built with
    /// `--features postgres`.
    #[arg(long, env = "DATABASE_URL")]
    dsn: String,

    /// Print the migration plan as JSON Lines without writing anything. The
    /// dry-run output is the live-run plan minus the actual writes — same
    /// `producing_session_id` and `byte_len`, only the `action` field
    /// differs (`would-*` vs the live `wrote` / `skipped`).
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, clap::Args)]
struct MigrateEventsArgs {
    /// Database DSN. Same scheme rules as `migrate-state`: `sqlite:<path>`
    /// or `postgres(ql)://USER:PASSWORD@HOST/DB`.
    #[arg(long, env = "DATABASE_URL")]
    dsn: String,

    /// Print the migration plan as JSON Lines without writing anything.
    /// The dry-run output matches the live-run plan exactly except for the
    /// `action` field (`would-*` vs `wrote`/`skipped`).
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Cut-over timestamp (ISO-8601). Only source rows whose `created_at`
    /// is strictly less than this value are migrated; anything `>= T` is
    /// left for the worker dual-write path (PR-1, `i-aankjvnz`). Pass the
    /// time you enabled dual-writes in production. Omit to migrate every
    /// historical row (only safe if dual-writes are NOT yet running).
    #[arg(long, value_name = "TIMESTAMP")]
    up_to: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::MigrateState(args) => run_migrate_state(args).await,
        Command::MigrateEvents(args) => run_migrate_events(args).await,
    }
}

async fn run_migrate_state(args: MigrateStateArgs) -> Result<()> {
    let backend = Backend::connect(&args.dsn).await?;
    let plan = state::run(&backend, args.dry_run).await?;
    for entry in &plan {
        state::emit_jsonl(entry)?;
    }
    eprintln!(
        "migrate-state {}: {} conversation_session_state row(s) processed",
        if args.dry_run { "dry-run" } else { "complete" },
        plan.len(),
    );
    Ok(())
}

async fn run_migrate_events(args: MigrateEventsArgs) -> Result<()> {
    let up_to = match args.up_to.as_deref() {
        None => None,
        Some(s) => Some(
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .with_context(|| format!("parsing --up-to value '{s}' as ISO-8601"))?,
        ),
    };
    let backend = Backend::connect(&args.dsn).await?;
    let plan = events::run(&backend, args.dry_run, up_to).await?;
    for entry in &plan {
        events::emit_jsonl(entry)?;
    }
    eprintln!(
        "migrate-events {}: {} conversation_events row(s) processed{}",
        if args.dry_run { "dry-run" } else { "complete" },
        plan.len(),
        match up_to {
            Some(t) => format!(" (--up-to {})", t.to_rfc3339()),
            None => String::new(),
        },
    );
    Ok(())
}
