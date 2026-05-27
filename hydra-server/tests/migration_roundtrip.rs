//! Migration roundtrip integration test (PR-1 + PR-2 of the migration test
//! harness; see `/designs/pre-prod-deploy-test-plan.md`).
//!
//! Applies sqlx migrations to a baseline-shaped Postgres database, executes
//! the `migration_baseline.sql` fixture, rolls the remaining migrations
//! forward, runs the external `migrate-events` pass through the same library
//! entry point the server's startup migration uses, and asserts:
//!
//! 1. (§3.1) schema invariants — columns / tables added / dropped / tightened
//!    by this release's migrations.
//! 2. (§3.2) data-shape invariants — SQL-level read-back of the backfilled
//!    rows.
//! 3. (§3.3) store / domain-level smoke — high-level `Store` API reads of the
//!    migrated rows confirm the typed domain values deserialize as expected,
//!    plus a fresh CREATE → read-back cycle exercises the post-migration
//!    write paths.
//!
//! The §3.1 / §3.2 / §3.3 bodies cover migrations applied *after* the
//! baseline pin recorded in the fixture header. They are emptied each time
//! the release-cut engineer rolls the baseline forward (the migrations they
//! used to assert on become baked into the new baseline) and re-populated as
//! the next release cycle's migrations land.
//!
//! Gated behind the `postgres` Cargo feature to match the rest of
//! `hydra-server`'s postgres-specific code (`migration_tool/mod.rs:17`,
//! `ee/store/postgres_v2.rs`, etc.).

#![cfg(feature = "postgres")]

use anyhow::{Context, Result, bail};
use hydra_server::store::postgres_v2::MIGRATOR;
use sqlx::PgPool;
use sqlx::migrate::Migrate;
use std::collections::HashSet;

const FIXTURE_SQL: &str = include_str!("fixtures/migration_baseline.sql");

#[tokio::test]
#[ignore]
async fn migration_roundtrip() -> Result<()> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("migration_roundtrip: DATABASE_URL unset; skipping.");
        return Ok(());
    };

    let pool = PgPool::connect(&database_url)
        .await
        .with_context(|| format!("connect to {database_url}"))?;

    reset_database(&pool).await?;

    let baseline_pin = parse_baseline_pin(FIXTURE_SQL)?;
    run_migrations_up_to(&pool, baseline_pin)
        .await
        .with_context(|| format!("apply migrations up to baseline pin {baseline_pin}"))?;
    sqlx::raw_sql(FIXTURE_SQL)
        .execute(&pool)
        .await
        .context("load migration_baseline.sql fixture")?;

    MIGRATOR
        .run(&pool)
        .await
        .context("apply remaining sqlx migrations past the baseline pin")?;

    run_external_migrations(&pool)
        .await
        .context("run external migrations (migrate-events pass)")?;

    assert_schema_invariants(&pool).await?;
    assert_data_shape_invariants(&pool).await?;
    assert_store_level_smoke(&pool)
        .await
        .context("§3.3 store / domain-level smoke")?;

    Ok(())
}

/// External-migration hook. `events::run` is the exact library entry point
/// the server's startup migration in `hydra-server/src/lib.rs` invokes —
/// calling it here with the same `dry_run = false`, `up_to = None` arguments
/// mirrors the work the server performs on each boot. ORDER MATTERS:
/// `MIGRATOR.run` must complete first so `session_events_v2` exists; the
/// later §3.3 assertions then exercise the rows this pass moved.
async fn run_external_migrations(pool: &PgPool) -> Result<()> {
    use hydra_server::migration_tool::{Backend, events};

    let _ = events::run(
        &Backend::Postgres(pool.clone()),
        /* dry_run */ false,
        /* up_to */ None,
    )
    .await
    .context("migrate-events pass against the migrated baseline pool")?;
    Ok(())
}

/// Drop and recreate the `metis` schema, and drop the sqlx migration tracking
/// table so the next `MIGRATOR.run*` replays from scratch.
async fn reset_database(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(
        "DROP SCHEMA IF EXISTS metis CASCADE; \
         CREATE SCHEMA metis; \
         DROP TABLE IF EXISTS public._sqlx_migrations;",
    )
    .execute(pool)
    .await
    .context("reset metis schema and sqlx migration tracking table")?;
    Ok(())
}

/// sqlx 0.7.4's `Migrator` does not expose a `run_to` method. We mirror the
/// fallback described in `/designs/pre-prod-deploy-test-plan.md` §9: filter
/// `MIGRATOR.iter()` by version, then drive `Migrate::apply` directly.
async fn run_migrations_up_to(pool: &PgPool, target: u64) -> Result<()> {
    let target = i64::try_from(target).context("baseline pin overflows i64")?;
    let mut conn = pool
        .acquire()
        .await
        .context("acquire connection for baseline migration apply")?;
    let conn: &mut sqlx::PgConnection = &mut conn;

    conn.ensure_migrations_table()
        .await
        .context("ensure _sqlx_migrations table")?;
    if let Some(version) = conn.dirty_version().await? {
        bail!("database is in a dirty state at migration version {version}");
    }
    let applied: HashSet<i64> = conn
        .list_applied_migrations()
        .await?
        .into_iter()
        .map(|m| m.version)
        .collect();

    for migration in MIGRATOR.iter() {
        if migration.migration_type.is_down_migration() {
            continue;
        }
        if migration.version > target {
            break;
        }
        if applied.contains(&migration.version) {
            continue;
        }
        conn.apply(migration)
            .await
            .with_context(|| format!("apply migration {}", migration.version))?;
    }
    Ok(())
}

/// Parse the `-- baseline-version: <N>` SQL comment on the first line of the
/// fixture. The pin is consumed by `Migrator::run_to` to stop migrations at the
/// version representing the prior release.
fn parse_baseline_pin(text: &str) -> Result<u64> {
    let line = text
        .lines()
        .next()
        .context("fixture is empty; expected `-- baseline-version: <N>` on line 1")?;
    let suffix = line.strip_prefix("-- baseline-version:").with_context(|| {
        format!(
            "fixture line 1 must start with `-- baseline-version:`; got `{line}`. \
             See the regen tool in PR-3 (designs/pre-prod-deploy-test-plan.md §5)."
        )
    })?;
    let raw = suffix.trim();
    raw.parse::<u64>().with_context(|| {
        format!("`-- baseline-version:` value `{raw}` is not a u64 migration version")
    })
}

// ---------------------------------------------------------------------------
// §3.1 schema invariants
//
// Assertions for migrations applied *after* the baseline pin. Re-populate as
// each new release's migrations land; clear when the release-cut engineer
// rolls the baseline forward.
// ---------------------------------------------------------------------------

async fn assert_schema_invariants(_pool: &PgPool) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// §3.2 data-shape invariants (SQL-level)
//
// Assertions for migrations applied *after* the baseline pin. Re-populate as
// each new release's migrations land; clear when the release-cut engineer
// rolls the baseline forward.
// ---------------------------------------------------------------------------

async fn assert_data_shape_invariants(_pool: &PgPool) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// §3.3 store / domain-level smoke
//
// Assertions for migrations applied *after* the baseline pin. Re-populate as
// each new release's migrations land; clear when the release-cut engineer
// rolls the baseline forward.
// ---------------------------------------------------------------------------

async fn assert_store_level_smoke(_pool: &PgPool) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// parse_baseline_pin unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::parse_baseline_pin;

    #[test]
    fn parses_well_formed_header() {
        let s = "-- baseline-version: 20260601000000\nINSERT INTO ...\n";
        assert_eq!(parse_baseline_pin(s).unwrap(), 20_260_601_000_000);
    }

    #[test]
    fn errors_when_header_missing() {
        let s = "INSERT INTO ...\n";
        let err = parse_baseline_pin(s).unwrap_err().to_string();
        assert!(
            err.contains("baseline-version"),
            "error should mention the expected header; got: {err}"
        );
    }

    #[test]
    fn errors_on_empty_input() {
        let err = parse_baseline_pin("").unwrap_err().to_string();
        assert!(
            err.contains("empty"),
            "error should mention empty fixture; got: {err}"
        );
    }

    #[test]
    fn errors_on_malformed_version() {
        let s = "-- baseline-version: not-a-number\nINSERT INTO ...\n";
        let err = parse_baseline_pin(s).unwrap_err().to_string();
        assert!(
            err.contains("not-a-number"),
            "error should include the bad value; got: {err}"
        );
    }
}
