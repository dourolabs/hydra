//! One-off Rust migration helpers for the sessions-orthogonality redesign
//! (`/designs/sessions-orthogonality-redesign.md` §3.5). Backs the
//! `hydra-migrate-sessions` binary.
//!
//! See `state.rs` for the `migrate-state` pass which copies
//! `conversation_session_state` (postgres `metis.conversation_session_state`,
//! sqlite `conversations.session_state` column) into `session_state` keyed on
//! the producing session id. The follow-up `migrate-events` pass lands
//! separately and will reuse the [`Backend`] / [`PlanEntry`] scaffolding here.

pub mod state;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

#[cfg(feature = "postgres")]
use sqlx::PgPool;

/// Backend connection used by the migration tool. Hides the sqlite/postgres
/// split from each subcommand entry point.
pub enum Backend {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(PgPool),
}

impl Backend {
    /// Connect to a backend selected by DSN scheme:
    /// * `sqlite:<path>` (or `sqlite::memory:`) → [`Backend::Sqlite`]
    /// * `postgres://…` / `postgresql://…` → [`Backend::Postgres`] (requires
    ///   the `postgres` cargo feature)
    pub async fn connect(dsn: &str) -> Result<Self> {
        if dsn.starts_with("sqlite:") {
            let pool = SqlitePool::connect(dsn)
                .await
                .with_context(|| format!("failed to connect to sqlite DSN '{dsn}'"))?;
            return Ok(Backend::Sqlite(pool));
        }

        if dsn.starts_with("postgres://") || dsn.starts_with("postgresql://") {
            #[cfg(feature = "postgres")]
            {
                let pool = PgPool::connect(dsn)
                    .await
                    .with_context(|| format!("failed to connect to postgres DSN '{dsn}'"))?;
                return Ok(Backend::Postgres(pool));
            }
            #[cfg(not(feature = "postgres"))]
            {
                anyhow::bail!(
                    "postgres DSN passed but binary was built without the 'postgres' feature. \
                     Rebuild with `--features postgres`."
                );
            }
        }

        anyhow::bail!("unrecognized DSN '{dsn}': expected a 'sqlite:' or 'postgres(ql)://' scheme",);
    }
}

/// Status of a single conversation-state row in the migration plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanAction {
    /// Dry-run: a write that *would* happen if we re-ran without `--dry-run`.
    WouldWrite,
    /// Dry-run: a row already present in `session_state`; would be skipped.
    WouldSkip,
    /// Live run: row was inserted into `session_state`.
    Wrote,
    /// Live run: row already existed in `session_state` and was left alone.
    Skipped,
}

/// One row of the migrate-state plan. Serialized as JSON Lines on stdout.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanEntry {
    pub conversation_id: String,
    pub producing_session_id: String,
    pub byte_len: usize,
    pub action: PlanAction,
}
