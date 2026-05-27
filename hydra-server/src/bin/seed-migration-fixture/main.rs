//! `seed-migration-fixture` — regeneration tool for the migration
//! baseline fixture (`hydra-server/tests/fixtures/migration_baseline.sql`).
//!
//! See `/designs/pre-prod-deploy-test-plan.md` §5. Run this once per
//! release from a fresh checkout of the release tag against a dedicated
//! empty Postgres:
//!
//! ```text
//! cargo run -p hydra-server --features postgres --bin seed-migration-fixture -- \
//!     --database-url postgres://... --force
//! ```
//!
//! Steps the tool runs:
//!
//! 1. Resets the `metis` schema on the target DB (`DROP SCHEMA IF EXISTS
//!    metis CASCADE; CREATE SCHEMA metis;`). Without `--force`, refuses to
//!    drop a populated DB (any pre-existing `metis.*` table with at least
//!    one row).
//! 2. Runs `MIGRATOR.run(&pool)` to apply every migration on the checkout
//!    to HEAD. Captures the maximum applied version (the **pin**).
//! 3. Invokes [`seed::seed_baseline`] (sibling module) for the
//!    deterministic, store-based catalogue of rows. The catalogue size
//!    scales with the `--scale` flag.
//! 4. Shells out to `pg_dump --data-only --inserts --column-inserts
//!    --schema=metis $DATABASE_URL` and captures stdout.
//! 5. Computes a sha256 over `hydra-server/migrations/` (filenames sorted
//!    asc; concatenate `filename + contents` for each file) and writes
//!    `--out` as:
//!
//!    ```text
//!    -- baseline-version: <N>
//!    -- migrations-hash: <hex>
//!
//!    <pg_dump output>
//!    ```
//!
//! 6. Refuses to overwrite an existing `--out` whose first two lines are
//!    not in the expected `-- baseline-version: <N>` / `-- migrations-hash:
//!    <hex>` form AND whose recorded hash differs from what the tool would
//!    emit now. Override with `--force` (catches accidental runs against
//!    the wrong checkout).

mod seed;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hydra_server::store::postgres_v2::PostgresStoreV2;
use sha2::{Digest, Sha256};
use sqlx::Row;
use sqlx::postgres::{PgPoolOptions, PgRow};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

use crate::seed::SeedConfig;

/// Directory containing the Postgres migrations bundled into this binary.
/// Resolved at build time via `CARGO_MANIFEST_DIR` so the tool works
/// regardless of caller cwd.
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

/// Default `--out` path, relative to the workspace root (where release
/// engineers invoke `cargo run`).
const DEFAULT_OUT: &str = "hydra-server/tests/fixtures/migration_baseline.sql";

#[derive(Debug, Parser)]
#[command(
    name = "seed-migration-fixture",
    about = "Regenerate hydra-server/tests/fixtures/migration_baseline.sql. \
             Designed to be run once per release from a fresh checkout of \
             the release tag against a dedicated empty Postgres."
)]
struct Cli {
    /// Postgres DSN. Falls back to `DATABASE_URL`.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Path the generated fixture is written to.
    #[arg(long, default_value = DEFAULT_OUT)]
    out: PathBuf,

    /// Bypass safety guards: drop a populated `metis` schema and
    /// overwrite an `--out` whose recorded `-- migrations-hash:` does
    /// not match the current tree.
    #[arg(long, default_value_t = false)]
    force: bool,

    /// Per-shape multiplier for the deterministic seed catalogue. `1`
    /// (the default) matches the prior raw-SQL seed's coverage (one
    /// row per assignee shape, per Review.author shape, etc.); larger
    /// values fan everything out by that factor for migration-stress
    /// testing without changing the kinds of rows produced.
    #[arg(long, default_value_t = 1)]
    scale: usize,
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
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    // Compute the migrations hash up-front: we use it both to validate the
    // pre-existing --out (if any) and to write it into the new header.
    let migrations_hash = hash_migrations_tree(Path::new(MIGRATIONS_DIR))
        .with_context(|| format!("hash migrations at {MIGRATIONS_DIR}"))?;
    info!(hash = %migrations_hash, "computed migrations-hash");

    if cli.out.exists() {
        check_existing_out(&cli.out, &migrations_hash, cli.force)?;
    }

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&cli.database_url)
        .await
        .context("connect to DATABASE_URL")?;

    ensure_db_empty_or_force(&pool, cli.force).await?;

    // Reset the schema even when --force: the migrator expects to run
    // against an empty `metis`, and partial state from a previous run
    // would otherwise produce a non-deterministic dump.
    sqlx::query("DROP SCHEMA IF EXISTS metis CASCADE; CREATE SCHEMA metis;")
        .execute(&pool)
        .await
        .context("reset metis schema")?;
    info!("reset metis schema");

    hydra_server::store::postgres_v2::run_migrations(&pool, None)
        .await
        .context("run postgres migrations")?;
    info!("applied migrations to HEAD");

    let baseline_version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .context("read max migration version from _sqlx_migrations")?;
    info!(baseline_version, "captured baseline pin");

    let store = PostgresStoreV2::new(pool.clone());
    seed::seed_baseline(&store, SeedConfig::for_scale(cli.scale))
        .await
        .context("run seed_baseline")?;
    info!(scale = cli.scale, "seeded baseline rows");

    pool.close().await;

    let dump = run_pg_dump(&cli.database_url).context("pg_dump")?;

    let body = format!(
        "-- baseline-version: {baseline_version}\n\
         -- migrations-hash: {migrations_hash}\n\
         \n\
         {dump}"
    );

    if let Some(parent) = cli.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create parent dir for {}", cli.out.display()))?;
        }
    }
    fs::write(&cli.out, body).with_context(|| format!("write fixture to {}", cli.out.display()))?;
    info!(path = %cli.out.display(), "wrote migration baseline fixture");

    eprintln!(
        "seed-migration-fixture: wrote {} (baseline-version {}, migrations-hash {})",
        cli.out.display(),
        baseline_version,
        migrations_hash
    );

    Ok(())
}

/// Check that any pre-existing fixture at `path` is safe to overwrite.
/// Refuses unless the first two lines parse to the expected
/// `-- baseline-version` / `-- migrations-hash` form **and** the recorded
/// hash matches `current_hash`. `--force` bypasses both checks.
fn check_existing_out(path: &Path, current_hash: &str, force: bool) -> Result<()> {
    let body =
        fs::read_to_string(path).with_context(|| format!("read existing {}", path.display()))?;

    let mut lines = body.lines();
    let line1 = lines.next().unwrap_or("");
    let line2 = lines.next().unwrap_or("");

    let header_ok =
        line1.starts_with("-- baseline-version:") && line2.starts_with("-- migrations-hash:");
    let recorded_hash = line2
        .strip_prefix("-- migrations-hash:")
        .map(|s| s.trim())
        .unwrap_or("");
    let hash_matches = header_ok && recorded_hash == current_hash;

    if hash_matches {
        info!("existing fixture migrations-hash matches; overwriting");
        return Ok(());
    }

    if force {
        info!(
            recorded = recorded_hash,
            current = current_hash,
            "existing fixture migrations-hash mismatch — overwriting under --force"
        );
        return Ok(());
    }

    if !header_ok {
        bail!(
            "refusing to overwrite {}: first two lines are not in `-- baseline-version: <N>` / `-- migrations-hash: <hex>` form. Pass --force to override.",
            path.display(),
        );
    }
    bail!(
        "refusing to overwrite {}: recorded migrations-hash `{}` does not match current tree hash `{}`. \
         The fixture appears to come from a different checkout. Pass --force to override.",
        path.display(),
        recorded_hash,
        current_hash,
    );
}

/// Reject populated databases unless `--force`. "Populated" = any
/// pre-existing table in the `metis` schema contains at least one row.
async fn ensure_db_empty_or_force(pool: &sqlx::PgPool, force: bool) -> Result<()> {
    let tables: Vec<PgRow> = sqlx::query(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'metis'",
    )
    .fetch_all(pool)
    .await
    .context("list metis tables")?;

    for row in tables {
        let table: String = row.try_get("table_name").context("decode table_name")?;
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM metis.{} LIMIT 1",
            quote_ident(&table)
        ))
        .fetch_one(pool)
        .await
        .with_context(|| format!("count metis.{table}"))?;
        if count > 0 {
            if force {
                info!(
                    %table,
                    %count,
                    "metis schema is populated; proceeding under --force"
                );
                return Ok(());
            }
            bail!(
                "refusing to operate on populated DB: metis.{table} contains rows. \
                 Pass --force to drop the metis schema anyway."
            );
        }
    }
    Ok(())
}

/// Quote a Postgres identifier (defensive — `information_schema.tables`
/// already returns valid identifiers, but a `"` could exist in pathological
/// fixtures and we forward the name into dynamic SQL).
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// Shell out to `pg_dump --data-only --inserts --column-inserts
/// --schema=metis <dsn>` and return stdout.
fn run_pg_dump(dsn: &str) -> Result<String> {
    let out = Command::new("pg_dump")
        .args([
            "--data-only",
            "--inserts",
            "--column-inserts",
            "--schema=metis",
            dsn,
        ])
        .output()
        .context("spawn pg_dump (is it on PATH?)")?;
    if !out.status.success() {
        bail!(
            "pg_dump failed (status {:?}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8(out.stdout).context("pg_dump output not utf-8")
}

/// Stable sha256 of the migrations tree: sort entries by filename, then
/// concatenate `filename + "\0" + contents` and hash. The null byte
/// prevents adjacent file boundaries from colliding.
fn hash_migrations_tree(dir: &Path) -> Result<String> {
    let mut entries: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .with_context(|| format!("read_dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (name, path) in &entries {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        hasher.update(name.as_bytes());
        hasher.update([0u8]);
        hasher.update(&bytes);
        hasher.update([0u8]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn hash_migrations_tree_is_stable_and_sensitive() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("001_a.sql"), b"CREATE TABLE a();").unwrap();
        fs::write(tmp.path().join("002_b.sql"), b"CREATE TABLE b();").unwrap();
        let h1 = hash_migrations_tree(tmp.path()).unwrap();
        let h2 = hash_migrations_tree(tmp.path()).unwrap();
        assert_eq!(h1, h2, "hash must be deterministic");

        // Editing a file changes the hash.
        fs::write(tmp.path().join("001_a.sql"), b"CREATE TABLE a2();").unwrap();
        let h3 = hash_migrations_tree(tmp.path()).unwrap();
        assert_ne!(h1, h3, "edits must change the hash");

        // Renaming a file changes the hash (filename is part of the hash).
        fs::rename(
            tmp.path().join("001_a.sql"),
            tmp.path().join("001_a_renamed.sql"),
        )
        .unwrap();
        let h4 = hash_migrations_tree(tmp.path()).unwrap();
        assert_ne!(h3, h4, "renames must change the hash");
    }

    #[test]
    fn check_existing_out_accepts_matching_hash() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("baseline.sql");
        fs::write(
            &path,
            "-- baseline-version: 20260101000000\n-- migrations-hash: abc123\n\nINSERT ...\n",
        )
        .unwrap();
        check_existing_out(&path, "abc123", false).unwrap();
    }

    #[test]
    fn check_existing_out_rejects_mismatched_hash_without_force() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("baseline.sql");
        fs::write(
            &path,
            "-- baseline-version: 20260101000000\n-- migrations-hash: stale\n\nINSERT ...\n",
        )
        .unwrap();
        let err = check_existing_out(&path, "fresh", false).unwrap_err();
        assert!(format!("{err}").contains("stale"));
    }

    #[test]
    fn check_existing_out_accepts_mismatched_hash_with_force() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("baseline.sql");
        fs::write(
            &path,
            "-- baseline-version: 20260101000000\n-- migrations-hash: stale\n\nINSERT ...\n",
        )
        .unwrap();
        check_existing_out(&path, "fresh", true).unwrap();
    }

    #[test]
    fn check_existing_out_rejects_bad_header_without_force() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("baseline.sql");
        fs::write(&path, "INSERT INTO ...\n").unwrap();
        let err = check_existing_out(&path, "any", false).unwrap_err();
        assert!(format!("{err}").contains("baseline-version"));
    }
}
