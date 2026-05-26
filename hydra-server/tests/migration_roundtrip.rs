//! Migration roundtrip integration test (PR-1 of the migration test harness;
//! see `/designs/pre-prod-deploy-test-plan.md`).
//!
//! Applies sqlx migrations to a baseline-shaped Postgres database, executes the
//! hand-curated `migration_baseline.sql` fixture, rolls the remaining migrations
//! forward, and asserts schema + data-shape invariants for this release's
//! migrations.
//!
//! Gated behind the `postgres` Cargo feature to match the rest of
//! `hydra-server`'s postgres-specific code (`migration_tool/mod.rs:17`,
//! `ee/store/postgres_v2.rs`, etc.). PR-2 will extend this with the
//! external-migration hook and the store/domain smoke; PR-5 will give it a
//! dedicated CI workflow.

#![cfg(feature = "postgres")]

use anyhow::{Context, Result, bail};
use hydra_server::store::postgres_v2::MIGRATOR;
use sqlx::migrate::Migrate;
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, HashSet};

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

    let _pre = capture_pre_rollforward_counts(&pool).await?;

    MIGRATOR
        .run(&pool)
        .await
        .context("apply remaining sqlx migrations past the baseline pin")?;

    assert_schema_invariants(&pool).await?;
    assert_data_shape_invariants(&pool).await?;

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

async fn capture_pre_rollforward_counts(pool: &PgPool) -> Result<BTreeMap<&'static str, i64>> {
    // Only tables that exist at the baseline pin (i.e. before this release's
    // additions); `session_events_v2` and `session_state_v2` show up later and
    // are not counted here.
    let tables: &[&str] = &[
        "issues_v2",
        "patches_v2",
        "tasks_v2",
        "conversations_v2",
        "conversation_events_v2",
        "object_relationships",
        "auth_tokens",
        "repositories_v2",
        "documents_v2",
    ];
    let mut out = BTreeMap::new();
    for &table in tables {
        let q = format!("SELECT COUNT(*) FROM metis.{table}");
        let row = sqlx::query(&q)
            .fetch_one(pool)
            .await
            .with_context(|| format!("count rows in {table}"))?;
        let n: i64 = row.get(0);
        out.insert(table, n);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// §3.1 schema invariants
// ---------------------------------------------------------------------------

async fn assert_schema_invariants(pool: &PgPool) -> Result<()> {
    // Columns added by this release's migrations.
    for (table, col) in [
        ("issues_v2", "assignee_principal"),
        ("auth_tokens", "session_id"),
        ("auth_tokens", "is_revoked"),
        ("tasks_v2", "mount_spec"),
        ("tasks_v2", "agent_config"),
        ("tasks_v2", "mode"),
        ("tasks_v2", "resumed_from"),
        ("repositories_v2", "merge_policy"),
    ] {
        if !column_exists(pool, table, col).await? {
            bail!("expected metis.{table}.{col} to exist after rollforward");
        }
    }

    // Columns dropped by this release's migrations.
    for (table, col) in [
        ("patches_v2", "created_by"),
        ("documents_v2", "created_by"),
        ("tasks_v2", "context"),
        ("tasks_v2", "prompt"),
        ("tasks_v2", "interactive"),
        ("tasks_v2", "conversation_resume_from"),
        ("tasks_v2", "model"),
        ("tasks_v2", "mcp_config"),
        ("issues_v2", "todo_list"),
        ("repositories_v2", "patch_workflow"),
    ] {
        if column_exists(pool, table, col).await? {
            bail!("expected metis.{table}.{col} to be dropped after rollforward");
        }
    }

    // Tables dropped by this release's migrations.
    for table in ["notifications", "conversation_session_state"] {
        if table_exists(pool, table).await? {
            bail!("expected metis.{table} to be dropped after rollforward");
        }
    }

    // Tables added by this release's migrations.
    for table in ["session_events_v2", "session_state_v2"] {
        if !table_exists(pool, table).await? {
            bail!("expected metis.{table} to exist after rollforward");
        }
    }

    // NOT NULL tightenings on tasks_v2.
    for col in ["mount_spec", "agent_config", "mode"] {
        if column_is_nullable(pool, "tasks_v2", col).await? {
            bail!("expected metis.tasks_v2.{col} to be NOT NULL after rollforward");
        }
    }

    Ok(())
}

async fn column_exists(pool: &PgPool, table: &str, column: &str) -> Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = $1 AND column_name = $2)",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<bool, _>(0))
}

async fn table_exists(pool: &PgPool, table: &str) -> Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'metis' AND table_name = $1)",
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<bool, _>(0))
}

async fn column_is_nullable(pool: &PgPool, table: &str, column: &str) -> Result<bool> {
    let row = sqlx::query(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = $1 AND column_name = $2",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .with_context(|| format!("look up nullability of metis.{table}.{column}"))?;
    let s: String = row.get(0);
    Ok(s.eq_ignore_ascii_case("YES"))
}

// ---------------------------------------------------------------------------
// §3.2 data-shape invariants (SQL-level)
// ---------------------------------------------------------------------------

async fn assert_data_shape_invariants(pool: &PgPool) -> Result<()> {
    // ---- issues_v2: assignee -> assignee_principal ----
    expect_assignee_principal(
        pool,
        "i-bare000001",
        Some(serde_json::json!({"kind": "user", "name": "jayantk"})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-userpfx0001",
        Some(serde_json::json!({"kind": "user", "name": "jayantk"})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-agentpfx001",
        Some(serde_json::json!({"kind": "agent", "name": "swe"})),
    )
    .await?;
    // external/<sys>/<x> is intentionally left NULL by the SQL backfill.
    expect_assignee_principal(pool, "i-extslash001", None).await?;
    expect_assignee_principal(pool, "i-nullasgn01", None).await?;

    // ---- patches_v2: reviews[*].author -> typed Principal ----
    expect_first_review_author(
        pool,
        "p-bareauth01",
        serde_json::json!({"kind": "user", "name": "jayantk"}),
    )
    .await?;
    expect_first_review_author(
        pool,
        "p-agentauth1",
        serde_json::json!({"kind": "agent", "name": "swe"}),
    )
    .await?;
    // Already-typed author must pass through the rewrite untouched.
    expect_first_review_author(
        pool,
        "p-typedauth1",
        serde_json::json!({"kind": "user", "name": "jayantk"}),
    )
    .await?;

    // ---- object_relationships: refers_to -> refers-to ----
    let row =
        sqlx::query("SELECT COUNT(*) FROM metis.object_relationships WHERE rel_type = 'refers_to'")
            .fetch_one(pool)
            .await?;
    let snake_count: i64 = row.get(0);
    if snake_count != 0 {
        bail!("expected 0 rows with rel_type='refers_to' after rename; got {snake_count}");
    }
    let row = sqlx::query(
        "SELECT COUNT(*) FROM metis.object_relationships \
         WHERE source_id = 'i-bare000001' AND target_id = 'i-userpfx0001' AND rel_type = 'refers-to'",
    )
    .fetch_one(pool)
    .await?;
    let kebab_count: i64 = row.get(0);
    if kebab_count != 1 {
        bail!(
            "expected the seeded refers_to row to be renamed to refers-to; matched {kebab_count}"
        );
    }

    // ---- auth_tokens: legacy row keeps NULL session_id, default is_revoked=FALSE ----
    let row = sqlx::query(
        "SELECT session_id, is_revoked FROM metis.auth_tokens \
         WHERE actor_name = 'agents/swe' AND token_hash = 'deadbeef'",
    )
    .fetch_one(pool)
    .await?;
    let session_id: Option<String> = row.get(0);
    let is_revoked: bool = row.get(1);
    if session_id.is_some() {
        bail!("expected legacy auth_token to have NULL session_id; got {session_id:?}");
    }
    if is_revoked {
        bail!("expected legacy auth_token to default is_revoked=FALSE");
    }

    // ---- tasks_v2: session-shape backfill produced the expected mode + agent_config ----
    let row = sqlx::query("SELECT mode::text FROM metis.tasks_v2 WHERE id = 's-headless01'")
        .fetch_one(pool)
        .await?;
    let mode_text: String = row.get(0);
    let mode: serde_json::Value = serde_json::from_str(&mode_text)?;
    let mode_type = mode.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if mode_type != "headless" {
        bail!("expected s-headless01.mode.type='headless'; got {mode}");
    }

    let row = sqlx::query("SELECT mode::text FROM metis.tasks_v2 WHERE id = 's-interact01'")
        .fetch_one(pool)
        .await?;
    let mode_text: String = row.get(0);
    let mode: serde_json::Value = serde_json::from_str(&mode_text)?;
    if mode.get("type").and_then(|v| v.as_str()) != Some("interactive") {
        bail!("expected s-interact01.mode.type='interactive'; got {mode}");
    }
    if mode.get("conversation_id").and_then(|v| v.as_str()) != Some("c-conv00001") {
        bail!("expected s-interact01.mode.conversation_id='c-conv00001'; got {mode}");
    }

    // resumed_from on s-interact02 should point at s-interact01 (the
    // is_latest-true predecessor in the same conversation).
    let row = sqlx::query("SELECT resumed_from FROM metis.tasks_v2 WHERE id = 's-interact02'")
        .fetch_one(pool)
        .await?;
    let resumed_from: Option<String> = row.get(0);
    if resumed_from.as_deref() != Some("s-interact01") {
        bail!("expected s-interact02.resumed_from='s-interact01'; got {resumed_from:?}");
    }

    Ok(())
}

async fn expect_assignee_principal(
    pool: &PgPool,
    issue_id: &str,
    expected: Option<serde_json::Value>,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT assignee_principal::text FROM metis.issues_v2 \
         WHERE id = $1 AND is_latest = TRUE",
    )
    .bind(issue_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read assignee_principal for {issue_id}"))?;
    let raw: Option<String> = row.get(0);
    let got = raw
        .map(|s| serde_json::from_str::<serde_json::Value>(&s))
        .transpose()
        .with_context(|| format!("decode assignee_principal JSON for {issue_id}"))?;
    if got != expected {
        bail!("issue {issue_id}: expected assignee_principal={expected:?}; got {got:?}");
    }
    Ok(())
}

async fn expect_first_review_author(
    pool: &PgPool,
    patch_id: &str,
    expected_author: serde_json::Value,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT (reviews->0->'author')::text FROM metis.patches_v2 \
         WHERE id = $1 AND is_latest = TRUE",
    )
    .bind(patch_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read reviews[0].author for {patch_id}"))?;
    let raw: Option<String> = row.get(0);
    let raw = raw.with_context(|| format!("patch {patch_id} has no reviews[0].author"))?;
    let got: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("decode reviews[0].author JSON for {patch_id}"))?;
    if got != expected_author {
        bail!("patch {patch_id}: expected reviews[0].author={expected_author}; got {got}");
    }
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
        let s = "-- baseline-version: 20260519000000\nINSERT INTO ...\n";
        assert_eq!(parse_baseline_pin(s).unwrap(), 20_260_519_000_000);
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
