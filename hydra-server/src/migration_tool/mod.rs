//! One-off Rust migration helpers for the sessions-orthogonality redesign
//! (`/designs/sessions-orthogonality-redesign.md` §3.5). Backs the
//! `hydra-migrate-sessions` binary.
//!
//! See `events.rs` for the `migrate-events` pass which partitions
//! `conversation_events_v2` user/assistant message rows by the active session
//! at write time and writes them to `session_events*` (design §3.5 step 3).
//!
//! The original `migrate-state` pass (`state.rs`) was removed once Phase E
//! step 19 dropped its source table / column.

pub mod events;

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
