//! In-Rust migration trait, registry, and shared interleave helper.
//!
//! Each store impl's `run_migrations(pool, up_to)` walks the sqlx `Migrator`
//! and the registry returned by [`rust_migrations`] in a single interleaved
//! sequence, so the events backfill (and any future Rust migration) runs at
//! its declared SQL version rather than as a separate post-SQL step. See
//! `/designs/migration-testing-redesign.md` §5 and §6.
//!
//! ## Adding a new Rust migration
//!
//! 1. Define a struct in a sibling module (see [`events`] for the template)
//!    and `impl RustMigration for YourMigration`. The implementation must be
//!    idempotent — re-running on already-migrated data must be a no-op (the
//!    server's startup hook re-invokes the full registry on every boot).
//! 2. Append `&YOUR_MIGRATION` to the slice in [`rust_migrations`]. Entries
//!    must be sorted by `version()` ascending — the function debug-asserts
//!    this on every call to catch a forgotten sort.
//!
//! ## Migration-author guardrail [[migrations]]
//!
//! SQLite migrations that reorder columns must NOT
//! `INSERT INTO new_table SELECT * FROM old_table` — `SELECT *`'s column
//! order is unstable across schema changes and silently corrupts data.
//! Out of scope for this module (we don't write SQL migrations here), but
//! Rust-migration authors who reach for SQL helpers should keep the rule in
//! mind.

pub mod events;

use anyhow::{Context, Result};
use sqlx::SqlitePool;

#[cfg(feature = "postgres")]
use sqlx::PgPool;

/// Backend connection used by Rust migrations. Hides the sqlite/postgres
/// split from each `RustMigration::run` implementation.
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

/// A Rust-implemented migration step that interleaves with sqlx SQL
/// migrations. See the module-level docs for how to add one.
#[async_trait::async_trait]
pub trait RustMigration: Send + Sync {
    /// The sqlx migration version this Rust step must run *after*. The
    /// interleave loop in each store's `run_migrations` invokes this step
    /// the moment SQL migrations reach this version, before any
    /// higher-versioned SQL migration runs.
    fn version(&self) -> u64;

    /// Short identifier for logs.
    fn name(&self) -> &'static str;

    /// Apply the migration. MUST be idempotent — server startup re-invokes
    /// the full registry on every boot, and integration tests re-run the
    /// full sequence many times against the same database.
    async fn run(&self, backend: &Backend) -> Result<()>;
}

static EVENTS_MIGRATION: events::EventsMigration = events::EventsMigration;

/// The static registry of Rust migrations to interleave with sqlx SQL
/// migrations. Order is by `version()` ascending; a debug assertion catches
/// a forgotten sort at first call.
pub fn rust_migrations() -> &'static [&'static dyn RustMigration] {
    const ALL: &[&'static dyn RustMigration] = &[&EVENTS_MIGRATION];
    debug_assert!(
        ALL.windows(2).all(|w| w[0].version() <= w[1].version()),
        "rust_migrations() must be sorted by version() ascending — see store/migrations/mod.rs"
    );
    ALL
}

/// One step in the interleaved SQL+Rust migration plan. Each store's
/// `run_migrations` executes the steps in order: SQL steps via its sqlx
/// `Migrator`, Rust steps via the trait method.
pub enum MigrationStep<'a> {
    Sql(&'a sqlx::migrate::Migration),
    Rust(&'static dyn RustMigration),
}

/// Build the interleaved SQL+Rust migration plan up to and including
/// `up_to` (or unbounded when `None`). SQL versions come from `migrator` in
/// declaration order, skipping any down-migrations. After each SQL step at
/// version `v`, any Rust migrations whose `version() <= v` (and not yet
/// emitted) are appended. Rust migrations with no matching SQL version
/// drain at the end.
pub fn plan_migrations<'a>(
    migrator: &'a sqlx::migrate::Migrator,
    rusts: &[&'static dyn RustMigration],
    up_to: Option<u64>,
) -> Vec<MigrationStep<'a>> {
    let target = up_to.unwrap_or(u64::MAX);
    let mut steps = Vec::new();
    let mut next_rust = 0usize;

    for migration in migrator.iter() {
        if migration.migration_type.is_down_migration() {
            continue;
        }
        if migration.version < 0 || (migration.version as u64) > target {
            break;
        }
        let sql_version = migration.version as u64;
        steps.push(MigrationStep::Sql(migration));
        while next_rust < rusts.len() && rusts[next_rust].version() <= sql_version {
            steps.push(MigrationStep::Rust(rusts[next_rust]));
            next_rust += 1;
        }
    }

    while next_rust < rusts.len() && rusts[next_rust].version() <= target {
        steps.push(MigrationStep::Rust(rusts[next_rust]));
        next_rust += 1;
    }

    steps
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    #[test]
    fn registry_is_sorted_today() {
        let regs = rust_migrations();
        assert!(
            regs.windows(2).all(|w| w[0].version() <= w[1].version()),
            "rust_migrations() must be sorted by version() ascending"
        );
    }
}
