//! `hydra-migrate-sessions` — one-off data-migration tool for the
//! sessions-orthogonality redesign
//! (`/designs/sessions-orthogonality-redesign.md`).
//!
//! # Subcommands
//!
//! * `migrate-events` — copy `conversation_events_v2` user/assistant
//!   message rows into `session_events_v2`, partitioned by the active
//!   session at write time (design §3.5 step 3 + §6 step 8). Supports
//!   `--up-to <TIMESTAMP>` for clean cut-over with the dual-write path
//!   (PR-1, `i-aankjvnz`): only source rows `created_at < T` are migrated,
//!   leaving newer rows for the dual-write path.
//!
//! The original `migrate-state` subcommand was retired once Phase E step 19
//! dropped its source table / column.
//!
//! # Run order & prerequisites
//!
//! Phase B migrations must have run on the target DB so the new
//! `session_events` / `session_events_v2` table exists.
//!
//! Recommended sequence on a deployed db:
//!
//! ```text
//! # Inspect + apply the historical-events migration. Pass --up-to
//! # with the dual-write cut-over timestamp to leave anything newer
//! # to the worker dual-write path (PR-1, i-aankjvnz).
//! hydra-migrate-sessions migrate-events --dsn <DSN> --up-to <T> --dry-run
//! hydra-migrate-sessions migrate-events --dsn <DSN> --up-to <T>
//! ```
//!
//! # Rollback / safety
//!
//! The tool only writes to the *new* `session_events*` tables and never
//! touches the source tables. The writes use `INSERT … ON CONFLICT DO
//! NOTHING`, so re-running is a no-op. If a pass is interrupted, re-running
//! picks up the remaining rows without reprocessing the ones already
//! migrated.
//!
//! # Output format
//!
//! All subcommands emit one JSON object per processed row to stdout
//! (JSON Lines).

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use hydra_common::time::HydraTime;
use hydra_server::migration_tool::{Backend, events};

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
    /// Partition historical `conversation_events_v2` user/assistant message
    /// rows by the active session at write time and write them to
    /// `session_events*`. Re-running is a no-op (sessions that already have
    /// rows are skipped).
    MigrateEvents(MigrateEventsArgs),
}

#[derive(Debug, clap::Args)]
struct MigrateEventsArgs {
    /// Database DSN. Supports `sqlite:<path>` or `postgres(ql)://USER:PASSWORD@HOST/DB`.
    #[arg(long, env = "DATABASE_URL")]
    dsn: String,

    /// Print the migration plan as JSON Lines without writing anything.
    /// The dry-run output matches the live-run plan exactly except for the
    /// `action` field (`would-*` vs `wrote`/`skipped`).
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Cut-over timestamp. Only source rows whose `created_at` is strictly
    /// less than this value are migrated; anything `>= T` is left for the
    /// worker dual-write path (PR-1, `i-aankjvnz`). Pass the time you
    /// enabled dual-writes in production. Omit to migrate every historical
    /// row (only safe if dual-writes are NOT yet running).
    ///
    /// Accepts the same forms as `hydra graph log --since/--until`:
    /// an RFC 3339 absolute timestamp (e.g. `2026-05-15T13:00:00Z`),
    /// a relative duration against `now` (e.g. `-30m`, `-1h`, `-7d`),
    /// or the literal `now`.
    #[arg(long, value_name = "TIMESTAMP")]
    up_to: Option<HydraTime>,
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
        Command::MigrateEvents(args) => run_migrate_events(args).await,
    }
}

async fn run_migrate_events(args: MigrateEventsArgs) -> Result<()> {
    let up_to: Option<DateTime<Utc>> = args.up_to.map(HydraTime::into_inner);
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
