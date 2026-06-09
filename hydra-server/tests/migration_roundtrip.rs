//! Migration roundtrip integration test.
//!
//! Enumerates the versioned baseline fixtures under
//! `tests/fixtures/migration_baselines/`, interleaving each baseline's
//! INSERTs with `postgres_v2::run_migrations(&pool, Some(version))` so every
//! migration with a higher version sees the file's rows. After the loop, a
//! final `run_migrations(&pool, None)` rolls to HEAD (and runs any Rust
//! migrations whose version sits beyond the last baseline). See
//! `/designs/migration-testing-redesign.md` §3, §4, §7 for the algorithm.
//!
//! Asserts:
//!
//! 1. (§3.1) schema invariants — columns / tables added / dropped / tightened
//!    by this release's migrations.
//! 2. (§3.2) data-shape invariants — SQL-level read-back of the backfilled
//!    rows.
//! 3. (§3.3) store / domain-level smoke — high-level `Store` API reads of the
//!    migrated rows confirm the typed `Principal` / `SessionMode` / refers-to
//!    domain values deserialize as expected, plus a fresh CREATE → read-back
//!    cycle exercises the post-migration write paths.
//!
//! Gated behind the `postgres` Cargo feature to match the rest of
//! `hydra-server`'s postgres-specific code.

#![cfg(feature = "postgres")]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::projects::StatusDefinition;
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::{ExternalSystem, Principal};
use hydra_common::{
    ConversationId, DocumentId, HydraId, IssueId, PatchId, ProjectId, RepoName, SessionId,
    TriggerId,
};
use hydra_server::domain::actors::ActorRef;
use hydra_server::domain::issues::{Issue, IssueStatus, IssueType};
use hydra_server::domain::patches::{Patch, PatchStatus, Review};
use hydra_server::domain::projects::default_project_seed;
use hydra_server::domain::sessions::{AgentConfig, Session, SessionEvent, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::Username;
use hydra_server::store::postgres_v2::{self, MIGRATOR, PostgresStoreV2};
use hydra_server::store::{ReadOnlyStore, RelationshipType, Store};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

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

    let baselines = load_baselines(baselines_dir())?;
    let mut prev: Option<u64> = None;
    for b in &baselines {
        if let Some(p) = prev {
            assert!(
                b.version > p,
                "baselines out of order: {} after {p}",
                b.version
            );
        }
        assert!(
            MIGRATOR.iter().any(|m| m.version as u64 == b.version),
            "baseline {} has no matching sqlx migration on this checkout",
            b.version
        );
        postgres_v2::run_migrations(&pool, Some(b.version))
            .await
            .with_context(|| format!("apply migrations up to baseline {}", b.version))?;
        sqlx::raw_sql(&b.body)
            .execute(&pool)
            .await
            .with_context(|| format!("execute baseline {}", b.version))?;
        prev = Some(b.version);
    }

    postgres_v2::run_migrations(&pool, None)
        .await
        .context("apply remaining migrations (SQL + Rust) past the last baseline")?;

    assert_schema_invariants(&pool).await?;
    assert_data_shape_invariants(&pool).await?;
    assert_events_migration_edge_cases(&pool)
        .await
        .context("events migration edge-case partitioning assertions")?;
    assert_actor_variant_cleanup(&pool)
        .await
        .context("actor_variant_cleanup rewrite assertions")?;
    assert_store_level_smoke(&pool)
        .await
        .context("§3.3 store / domain-level smoke")?;
    assert_recent_migration_data_shape(&pool)
        .await
        .context("triggers / projects post-rollforward data-shape round-trip")?;

    seed_default_project_migration_inserts_row(&pool)
        .await
        .context("seed_default_project: inserted row matches expected shape")?;
    seed_default_project_migration_backfills_null_project_ids(&pool)
        .await
        .context("seed_default_project: backfill of NULL project_id values")?;
    seed_default_project_migration_is_idempotent(&pool)
        .await
        .context("seed_default_project: idempotency on re-applied migration body")?;

    drop_status_icon_migration_strips_default_seed(&pool)
        .await
        .context("drop_status_icon: j-defaul statuses no longer carry an `icon` key")?;
    drop_status_icon_migration_strips_custom_row(&pool)
        .await
        .context("drop_status_icon: j-iconfix statuses round-trip without `icon`")?;
    drop_status_icon_migration_is_idempotent(&pool)
        .await
        .context("drop_status_icon: idempotency on re-applied migration body")?;

    denormalize_creator_session_backfill(&pool)
        .await
        .context("denormalize_creator: session-bound auth_tokens row backfilled")?;
    denormalize_creator_user_backfill(&pool)
        .await
        .context("denormalize_creator: user-CLI auth_tokens row backfilled")?;
    denormalize_creator_domain_roundtrip(&pool)
        .await
        .context("denormalize_creator: domain-level AuthTokenRow round-trip")?;
    drop_actors_v2_migration_removes_table(&pool)
        .await
        .context("drop_actors_v2: metis.actors_v2 table is gone after rollforward")?;
    denormalize_creator_migration_is_idempotent(&pool)
        .await
        .context("denormalize_creator: post-migration write/read works")?;

    add_projects_priority_backfill_sql_level(&pool)
        .await
        .context("add_projects_priority: SQL-level rank backfill")?;
    add_projects_priority_backfill_domain_roundtrip(&pool)
        .await
        .context("add_projects_priority: domain-level list_projects round-trip")?;

    drop_projects_default_status_key_migration_removes_column(&pool)
        .await
        .context(
            "drop_projects_default_status_key: metis.projects.default_status_key column is gone",
        )?;
    drop_projects_default_status_key_migration_preserves_typed_read(&pool)
        .await
        .context("drop_projects_default_status_key: seeded + baseline rows deserialize through Store::get_project")?;
    drop_projects_default_status_key_migration_is_idempotent(&pool)
        .await
        .context("drop_projects_default_status_key: idempotency on re-applied migration body")?;

    issues_v2_project_id_is_not_null(&pool)
        .await
        .context("issues_v2_project_id_not_null: column is NOT NULL after rollforward")?;
    issues_v2_project_id_rejects_null_insert(&pool)
        .await
        .context("issues_v2_project_id_not_null: NOT NULL rejects fresh NULL inserts")?;
    issues_v2_project_id_not_null_migration_is_idempotent(&pool)
        .await
        .context("issues_v2_project_id_not_null: idempotency on re-applied body")?;
    // The pre-flight NULL-guard test uses a fresh sqlite in-memory pool
    // in the sister `migration_roundtrip_sqlite` runner; we don't repeat
    // it here because the postgres path would have to reset and reroll
    // the schema, and downstream idempotency assertions in this run
    // depend on the seeded baseline data.

    create_statuses_migration_schema_invariants(&pool)
        .await
        .context("create_statuses: schema invariants on metis.statuses")?;
    create_statuses_migration_backfills_default_seed(&pool)
        .await
        .context("create_statuses: j-defaul backfill matches default_project_seed()")?;
    create_statuses_migration_backfills_custom_project(&pool)
        .await
        .context("create_statuses: j-stsfixt backfill covers full column shape")?;
    add_issues_v2_status_sequence_schema_invariants(&pool)
        .await
        .context(
            "add_issues_v2_status_sequence: status_sequence column exists as nullable BIGINT",
        )?;
    add_issues_v2_status_sequence_backfills_issues(&pool)
        .await
        .context(
            "add_issues_v2_status_sequence: every issue's status_sequence resolves back to its key",
        )?;
    create_statuses_migration_is_idempotent(&pool)
        .await
        .context("create_statuses: re-applying body adds no duplicate rows")?;
    add_issues_v2_status_sequence_migration_is_idempotent(&pool)
        .await
        .context("add_issues_v2_status_sequence: re-applying body does not overwrite sequences")?;

    // Re-run the migration plan to confirm the cleanup is idempotent —
    // every classify rule treats post-cleanup shapes as no-ops, so a
    // second pass must produce no extra writes.
    postgres_v2::run_migrations(&pool, None)
        .await
        .context("re-apply migrations to confirm idempotency")?;
    assert_actor_variant_cleanup(&pool)
        .await
        .context("actor_variant_cleanup idempotent second-pass assertions")?;

    Ok(())
}

/// Drop and recreate the `metis` schema, and drop the sqlx migration tracking
/// table so the next `run_migrations` call replays from scratch.
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

// ---------------------------------------------------------------------------
// Baseline directory enumeration
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Baseline {
    version: u64,
    body: String,
}

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_baselines")
}

fn load_baselines(dir: impl AsRef<Path>) -> Result<Vec<Baseline>> {
    let dir = dir.as_ref();
    let entries = std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))?;
    let mut baselines = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read entry under {}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .with_context(|| format!("baseline filename is not UTF-8: {}", path.display()))?;
        let version = parse_baseline_filename(name)
            .with_context(|| format!("parse baseline filename '{name}'"))?;
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("read baseline {}", path.display()))?;
        baselines.push(Baseline { version, body });
    }
    baselines.sort_by_key(|b| b.version);
    Ok(baselines)
}

/// Parse `<version>__<description>.sql` into the leading `u64` version. The
/// description after `__` is human-readable and not validated against
/// anything. Filenames not matching the pattern are rejected so typos and
/// leftover files surface loudly. (§3, §4 of `/designs/migration-testing-redesign.md`.)
fn parse_baseline_filename(name: &str) -> Result<u64> {
    let stem = name
        .strip_suffix(".sql")
        .with_context(|| format!("baseline '{name}' must end in `.sql`"))?;
    let (version, desc) = stem
        .split_once("__")
        .with_context(|| format!("baseline '{name}' must match `<version>__<description>.sql`"))?;
    if desc.is_empty() {
        bail!(
            "baseline '{name}' has an empty description (expected `<version>__<description>.sql`)"
        );
    }
    version
        .parse::<u64>()
        .with_context(|| format!("baseline '{name}' version prefix '{version}' is not a u64"))
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
        ("auth_tokens", "creator"),
        ("tasks_v2", "mount_spec"),
        ("tasks_v2", "agent_config"),
        ("tasks_v2", "mode"),
        ("tasks_v2", "resumed_from"),
        ("repositories_v2", "merge_policy"),
        ("issues_v2", "project_id"),
        ("tasks_v2", "proxy_targets"),
    ] {
        if !column_exists(pool, table, col).await? {
            bail!("expected metis.{table}.{col} to exist after rollforward");
        }
    }

    // The proxy_targets column must be nullable so existing rows inflate to
    // an empty `Vec<ProxyTarget>` on read.
    if !column_is_nullable(pool, "tasks_v2", "proxy_targets").await? {
        bail!("expected metis.tasks_v2.proxy_targets to be nullable after rollforward");
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
    for table in [
        "notifications",
        "conversation_session_state",
        "conversation_events_v2",
        "actors_v2",
    ] {
        if table_exists(pool, table).await? {
            bail!("expected metis.{table} to be dropped after rollforward");
        }
    }

    // Tables added by this release's migrations.
    for table in [
        "session_events_v2",
        "session_state_v2",
        "triggers",
        "projects",
        "statuses",
    ] {
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

    // NOT NULL tightening on auth_tokens.creator landed by
    // 20260609000000_add_creator_to_auth_tokens.sql.
    if column_is_nullable(pool, "auth_tokens", "creator").await? {
        bail!("expected metis.auth_tokens.creator to be NOT NULL after rollforward");
    }

    // Column added by 20260606010000_add_projects_prompt_path.sql.
    if !column_exists(pool, "projects", "prompt_path").await? {
        bail!("expected metis.projects.prompt_path column to exist after rollforward");
    }
    if !column_is_nullable(pool, "projects", "prompt_path").await? {
        bail!("expected metis.projects.prompt_path to be nullable after rollforward");
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
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-userpfx0001",
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-agentpfx001",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // external/<sys>/<x> is intentionally left NULL by the SQL backfill.
    expect_assignee_principal(pool, "i-extslash001", None).await?;
    expect_assignee_principal(pool, "i-nullasgn01", None).await?;

    // ---- patches_v2: reviews[*].author -> typed Principal ----
    expect_first_review_author(
        pool,
        "p-bareauth01",
        serde_json::json!({"User": {"name": "jayantk"}}),
    )
    .await?;
    expect_first_review_author(
        pool,
        "p-agentauth1",
        serde_json::json!({"Agent": {"name": "swe"}}),
    )
    .await?;
    // Already-typed author must pass through the rewrite untouched.
    expect_first_review_author(
        pool,
        "p-typedauth1",
        serde_json::json!({"User": {"name": "jayantk"}}),
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
         WHERE actor_name = 'users/legacy' AND token_hash = 'deadbeef'",
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

    let row = sqlx::query("SELECT mode::text FROM metis.tasks_v2 WHERE id = 's-interbase'")
        .fetch_one(pool)
        .await?;
    let mode_text: String = row.get(0);
    let mode: serde_json::Value = serde_json::from_str(&mode_text)?;
    if mode.get("type").and_then(|v| v.as_str()) != Some("interactive") {
        bail!("expected s-interbase.mode.type='interactive'; got {mode}");
    }
    if mode.get("conversation_id").and_then(|v| v.as_str()) != Some("c-convbase") {
        bail!("expected s-interbase.mode.conversation_id='c-convbase'; got {mode}");
    }

    // resumed_from on s-interresume should point at s-interbase (the
    // is_latest-true predecessor in the same conversation).
    let row = sqlx::query("SELECT resumed_from FROM metis.tasks_v2 WHERE id = 's-interresume'")
        .fetch_one(pool)
        .await?;
    let resumed_from: Option<String> = row.get(0);
    if resumed_from.as_deref() != Some("s-interbase") {
        bail!("expected s-interresume.resumed_from='s-interbase'; got {resumed_from:?}");
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
// Events migration partitioning edge cases.
//
// The baseline fixture seeds four conversations that exercise the tolerant
// assignment rules added to `src/store/migrations/events.rs`:
//
//   * `c-convnoses`  — conversation has message events but zero linked
//                      sessions: migration must NOT bail; events are dropped.
//   * `c-convbefore` — single session whose creation_time is AFTER the only
//                      message event; event lands on the first session.
//   * `c-convgap`    — two sessions with a gap; event in the gap lands on
//                      the subsequent session.
//   * `c-convafter`  — two sessions where the last suspends; event past the
//                      suspend timestamp lands on the last session.
// ---------------------------------------------------------------------------

async fn assert_events_migration_edge_cases(pool: &PgPool) -> Result<()> {
    // 1. c-convnoses: no linked sessions, so no session_events for it.
    //    The real check is that the migration ran to completion above.
    let row = sqlx::query(
        "SELECT COUNT(*) FROM metis.session_events_v2 se \
         JOIN metis.tasks_v2 t ON t.id = se.session_id \
         WHERE t.conversation_id = 'c-convnoses'",
    )
    .fetch_one(pool)
    .await
    .context("count session_events for c-convnoses")?;
    let count: i64 = row.get(0);
    if count != 0 {
        bail!("expected 0 session_events for c-convnoses (no sessions exist); got {count}");
    }

    // 2. c-convbefore: the single message event predating s-beforeone's
    //    creation_time must still land on s-beforeone.
    expect_session_event_count(pool, "s-beforeone", 1).await?;

    // 3. c-convgap: the gap event must land on s-gaptwo (subsequent
    //    session), NOT s-gapone.
    expect_session_event_count(pool, "s-gapone", 0).await?;
    expect_session_event_count(pool, "s-gaptwo", 1).await?;

    // 4. c-convafter: the post-suspend event on the last session lands on
    //    s-aftertwo. s-afterone owns nothing (no events in its window).
    expect_session_event_count(pool, "s-afterone", 0).await?;
    expect_session_event_count(pool, "s-aftertwo", 1).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// actor_variant_cleanup rewrite assertions
// ---------------------------------------------------------------------------
//
// The baseline at `20260603000000__pre_actor_variant_cleanup.sql` seeds
// one row per pre-cleanup actor shape (Username, Session, Issue ±match,
// Service ±valid, Legacy bare-string ±parseable, multi-key map,
// already-typed). After the cleanup migration runs, every row's
// `actor` / `actor_id` column must hold either the rewritten
// post-cleanup wire shape or NULL.
//
// Round-2 store/domain-level smoke: deserializing the rewritten rows
// through `PostgresStoreV2` exercises `ActorRef::deserialize` against
// the migration's raw `serde_json::Value` output and catches serde
// drift between the two.

async fn assert_actor_variant_cleanup(pool: &PgPool) -> Result<()> {
    // 1. Username -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actuname",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 2. Session -> Adhoc
    expect_issue_actor_actor_id(
        pool,
        "i-actsess",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-sessone"}})),
    )
    .await?;
    // 3. Issue with matching tasks_v2 row -> resolved User
    expect_issue_actor_actor_id(
        pool,
        "i-actiss",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 4. Issue without matching tasks_v2 row -> External-legacy(<iid>)
    expect_issue_actor_actor_id(pool, "i-actissno", Some(external_legacy("i-actisstwo"))).await?;
    // 5. Service with valid AgentName -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actsvcok",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 6. Service with invalid AgentName -> External-legacy(<name>)
    expect_issue_actor_actor_id(pool, "i-actsvcno", Some(external_legacy("has space"))).await?;
    // 7. Legacy users/<x> -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actlegu",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 8. Legacy agents/<x> -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actlega",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 9. Legacy unparseable -> External-legacy(<bare-string>)
    expect_issue_actor_actor_id(
        pool,
        "i-actlegx",
        Some(external_legacy("definitely not an actor")),
    )
    .await?;
    // 10. Already-typed User -> no-op (still User)
    expect_issue_actor_actor_id(
        pool,
        "i-actuser",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 11. Multi-key actor_id -> External-legacy(JSON form of the map)
    expect_issue_actor_actor_id(
        pool,
        "i-actmulti",
        Some(external_legacy(
            serde_json::json!({"kind": "user", "name": "alice"}).to_string(),
        )),
    )
    .await?;
    // 12. Legacy adhoc/<sid> -> Adhoc
    expect_issue_actor_actor_id(
        pool,
        "i-actadhoc",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-adhocone"}})),
    )
    .await?;
    // 13. Legacy external/<sys>/<user> -> External
    expect_issue_actor_actor_id(
        pool,
        "i-actextn",
        Some(serde_json::json!({"External": {"system": "github", "username": "jayantk"}})),
    )
    .await?;
    // 14. Legacy u-<x> shorthand -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actushrt",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 15. Legacy s-<sid> shorthand -> Adhoc (session_id preserves the s- prefix)
    expect_issue_actor_actor_id(
        pool,
        "i-actsshrt",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-abcdef"}})),
    )
    .await?;
    // 16. Legacy svc-<n> shorthand -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actsvshr",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 17. Legacy users/<x> with invalid Username -> External-legacy("users/<x>")
    expect_issue_actor_actor_id(pool, "i-actubad", Some(external_legacy("users/has space")))
        .await?;
    // 18. Legacy agents/<x> with invalid AgentName -> External-legacy("agents/<x>")
    expect_issue_actor_actor_id(
        pool,
        "i-actabad",
        Some(external_legacy("agents/with space")),
    )
    .await?;
    // 19. Legacy external/<sys>/<x> with invalid system -> External-legacy(<bare-string>)
    expect_issue_actor_actor_id(
        pool,
        "i-actexbad",
        Some(external_legacy("external/has space/foo")),
    )
    .await?;
    // 20. Legacy a-<issue_id> shorthand -> External-legacy("a-<issue_id>")
    expect_issue_actor_actor_id(pool, "i-actashrt", Some(external_legacy("a-i-actissone"))).await?;

    // Issue-arm tie-break edges: multi-match / deleted-only / not-latest /
    // chained Issue all fall back to External-legacy with the original
    // parent issue id preserved as the username, because
    // `load_issue_to_actor_id` only inserts when exactly one
    // post-cleanup-resolvable task is associated with the issue.
    for (issue_id, expected_iid) in [
        ("i-actrefmny", "i-actissmany"),
        ("i-actrefdel", "i-actissdel"),
        ("i-actrefold", "i-actissold"),
        ("i-actrefchn", "i-actisschn"),
    ] {
        expect_issue_actor_actor_id(pool, issue_id, Some(external_legacy(expected_iid))).await?;
    }

    // Nested ActorRef rewrites — System.on_behalf_of resolved/unresolved
    // and Automation.triggered_by resolved/unresolved. Unresolved sub-actors
    // collapse to null WITHIN the ActorRef rather than NULLing the row.
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actsysu",
        Some(serde_json::json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": {"User": {"name": "alice"}}
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actsysn",
        Some(serde_json::json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": null
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actauto",
        Some(serde_json::json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": {
                    "Authenticated": {"actor_id": {"User": {"name": "alice"}}}
                }
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actauton",
        Some(serde_json::json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": null
            }
        })),
    )
    .await?;

    // Multi-table coverage — every table in `ACTOR_REF_TABLES_COMMON`
    // (plus session_events_v2 below) carries one Username-legacy row
    // that the cleanup must rewrite to a User-tagged ActorRef.
    let expected_user_authenticated = serde_json::json!({
        "Authenticated": {"actor_id": {"User": {"name": "alice"}}}
    });
    for (table, id) in [
        ("repositories_v2", "r-actreplc"),
        ("users_v2", "u-actusrlc"),
        ("patches_v2", "p-actpchlc"),
        ("tasks_v2", "s-actrowx"),
        ("documents_v2", "d-actdoclc"),
    ] {
        expect_table_actor(pool, table, id, Some(expected_user_authenticated.clone())).await?;
    }

    // session_events_v2 has a (session_id, version_number) primary key
    // and no is_latest column, so it gets a one-off inline assertion.
    let row = sqlx::query(
        "SELECT actor::text AS pl FROM metis.session_events_v2 \
         WHERE session_id = 's-actrowx' AND version_number = 1",
    )
    .fetch_one(pool)
    .await
    .context("read session_events_v2.actor for (s-actrowx, 1)")?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode session_events_v2.actor")?;
    if got != Some(expected_user_authenticated.clone()) {
        bail!(
            "session_events_v2(s-actrowx, 1).actor: expected {expected_user_authenticated}; got {got:?}",
        );
    }

    // conversations_v2 — the new walker added in [[i-jyhvstcj]]
    // rewrites the `actor` column on this table too. The fixture seeds
    // the exact prod symptom (`Session`-tagged inner actor_id) so the
    // §3.3 store-level smoke below proves `get_conversation` stops
    // failing to deserialize.
    expect_table_actor(
        pool,
        "conversations_v2",
        "c-actconvx",
        Some(serde_json::json!({
            "Authenticated": {"actor_id": {"Adhoc": {"session_id": "s-csessacx"}}}
        })),
    )
    .await?;

    // issues_v2.form_response — the new walker added in [[i-jyhvstcj]]
    // rewrites the embedded `.actor` while preserving sibling fields
    // (`action_id`, `values`, `submitted_at`).
    let row = sqlx::query(
        "SELECT form_response::text AS pl FROM metis.issues_v2 \
         WHERE id = 'i-actform' AND is_latest = TRUE",
    )
    .fetch_one(pool)
    .await
    .context("read issues_v2.form_response for i-actform")?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode issues_v2.form_response JSON for i-actform")?;
    let expected_form_response = serde_json::json!({
        "action_id": "approve",
        "actor": {"User": {"name": "alice"}},
        "values": {"score": 4},
        "submitted_at": "2026-05-10T11:00:00Z"
    });
    if got.as_ref() != Some(&expected_form_response) {
        bail!("issues_v2(i-actform).form_response: expected {expected_form_response}; got {got:?}");
    }

    // §3.3 smoke: read every row through the store and confirm
    // `ActorRef::deserialize` accepts the rewritten payload — including
    // the new External-legacy fallback shape that catches serde drift
    // between the migration's raw JSON and
    // `hydra_common::ActorId::deserialize`.
    use hydra_server::domain::actors::{ActorId, ActorRef as DomainActorRef};
    let store = PostgresStoreV2::new(pool.clone());
    let legacy = ExternalSystem::try_new("legacy").unwrap();
    let external_legacy_actor = |username: &str| ActorId::External {
        system: legacy.clone(),
        username: username.to_string(),
    };
    for (issue_id, expected_actor_id) in [
        (
            "i-actuname",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        ("i-actsess", ActorId::Adhoc("s-sessone".parse().unwrap())),
        (
            "i-actiss",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        // Previously NULL — now External-legacy.
        ("i-actissno", external_legacy_actor("i-actisstwo")),
        (
            "i-actsvcok",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        ("i-actsvcno", external_legacy_actor("has space")),
        (
            "i-actlegu",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        (
            "i-actlega",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        (
            "i-actlegx",
            external_legacy_actor("definitely not an actor"),
        ),
        (
            "i-actuser",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        (
            "i-actmulti",
            external_legacy_actor(
                &serde_json::json!({"kind": "user", "name": "alice"}).to_string(),
            ),
        ),
        ("i-actadhoc", ActorId::Adhoc("s-adhocone".parse().unwrap())),
        (
            "i-actextn",
            ActorId::External {
                system: ExternalSystem::try_new("github").unwrap(),
                username: "jayantk".to_string(),
            },
        ),
        (
            "i-actushrt",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        ("i-actsshrt", ActorId::Adhoc("s-abcdef".parse().unwrap())),
        (
            "i-actsvshr",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        ("i-actubad", external_legacy_actor("users/has space")),
        ("i-actabad", external_legacy_actor("agents/with space")),
        (
            "i-actexbad",
            external_legacy_actor("external/has space/foo"),
        ),
        ("i-actashrt", external_legacy_actor("a-i-actissone")),
        // Issue-arm tie-break edges fall back with the parent issue id.
        ("i-actrefmny", external_legacy_actor("i-actissmany")),
        ("i-actrefdel", external_legacy_actor("i-actissdel")),
        ("i-actrefold", external_legacy_actor("i-actissold")),
        ("i-actrefchn", external_legacy_actor("i-actisschn")),
    ] {
        let issue = store
            .get_issue(&parse_issue_id(issue_id)?, false)
            .await
            .with_context(|| format!("store-level read of {issue_id}"))?;
        let actor = issue.actor.with_context(|| {
            format!("expected non-None actor on {issue_id} after cleanup; got None")
        })?;
        match &actor {
            DomainActorRef::Authenticated { actor_id, .. } if actor_id == &expected_actor_id => {}
            other => bail!(
                "{issue_id}: expected Authenticated(actor_id={expected_actor_id:?}); got {other:?}"
            ),
        }
    }

    // Nested ActorRef shapes — `System` and `Automation` rows must
    // round-trip through `ActorRef::deserialize` with the inner
    // `on_behalf_of` / `triggered_by` rewritten or collapsed to None.
    let sysu = store
        .get_issue(&parse_issue_id("i-actsysu")?, false)
        .await
        .context("store-level read of i-actsysu")?;
    match sysu.actor.as_ref() {
        Some(DomainActorRef::System {
            worker_name,
            on_behalf_of: Some(ActorId::User(name)),
        }) if worker_name == "task-spawner" && name.as_str() == "alice" => {}
        other => bail!(
            "i-actsysu: expected System(worker_name=task-spawner, on_behalf_of=User(alice)); got {other:?}"
        ),
    }
    let sysn = store
        .get_issue(&parse_issue_id("i-actsysn")?, false)
        .await
        .context("store-level read of i-actsysn")?;
    match sysn.actor.as_ref() {
        Some(DomainActorRef::System {
            worker_name,
            on_behalf_of: None,
        }) if worker_name == "task-spawner" => {}
        other => bail!(
            "i-actsysn: expected System(worker_name=task-spawner, on_behalf_of=None); got {other:?}"
        ),
    }
    let auto = store
        .get_issue(&parse_issue_id("i-actauto")?, false)
        .await
        .context("store-level read of i-actauto")?;
    match auto.actor.as_ref() {
        Some(DomainActorRef::Automation {
            automation_name,
            triggered_by: Some(boxed),
        }) if automation_name == "github_pr_sync" => match boxed.as_ref() {
            DomainActorRef::Authenticated {
                actor_id: ActorId::User(name),
                ..
            } if name.as_str() == "alice" => {}
            other => {
                bail!("i-actauto.triggered_by: expected Authenticated(User(alice)); got {other:?}")
            }
        },
        other => bail!(
            "i-actauto: expected Automation(github_pr_sync, triggered_by=Authenticated(User(alice))); got {other:?}"
        ),
    }
    let auton = store
        .get_issue(&parse_issue_id("i-actauton")?, false)
        .await
        .context("store-level read of i-actauton")?;
    match auton.actor.as_ref() {
        Some(DomainActorRef::Automation {
            automation_name,
            triggered_by: None,
        }) if automation_name == "github_pr_sync" => {}
        other => bail!(
            "i-actauton: expected Automation(github_pr_sync, triggered_by=None); got {other:?}"
        ),
    }

    // Multi-table coverage — read the post-cleanup actor through the
    // store for tables that expose an `id`-keyed getter so any serde
    // drift between the per-table walker's raw JSON output and the
    // domain `ActorRef` shows up here. `repositories_v2`/`users_v2`
    // lookups go through name-typed keys (RepoName / Username) that
    // the bare-id fixtures don't satisfy, so those stay at SQL-level
    // only above. `session_events_v2` is data-store-only.
    let multi_user = ActorId::User(ApiUsername::try_new("alice").unwrap());
    let patch = store
        .get_patch(&parse_patch_id("p-actpchlc")?, false)
        .await
        .context("store-level read of p-actpchlc")?;
    match patch.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("p-actpchlc: expected Authenticated(User(alice)); got {other:?}"),
    }
    let session = store
        .get_session(&parse_session_id("s-actrowx")?, false)
        .await
        .context("store-level read of s-actrowx")?;
    match session.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("s-actrowx: expected Authenticated(User(alice)); got {other:?}"),
    }
    let document = store
        .get_document(
            &DocumentId::from_str("d-actdoclc").context("parse d-actdoclc as DocumentId")?,
            false,
        )
        .await
        .context("store-level read of d-actdoclc")?;
    match document.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("d-actdoclc: expected Authenticated(User(alice)); got {other:?}"),
    }

    // conversations_v2 store-level smoke — the load-bearing acceptance
    // criterion for [[i-jyhvstcj]]: prod's `GET /v1/conversations` was
    // failing on this exact shape pre-fix.
    let conversation = store
        .get_conversation(
            &ConversationId::from_str("c-actconvx")
                .context("parse c-actconvx as ConversationId")?,
            false,
        )
        .await
        .context("store-level read of c-actconvx")?;
    let conv_session_id: SessionId = "s-csessacx".parse().unwrap();
    match conversation.actor.as_ref() {
        Some(DomainActorRef::Authenticated {
            actor_id: ActorId::Adhoc(sid),
            ..
        }) if sid == &conv_session_id => {}
        other => bail!("c-actconvx: expected Authenticated(Adhoc(s-csessacx)); got {other:?}"),
    }

    // issues_v2 form_response store-level smoke — the other load-bearing
    // acceptance criterion for [[i-jyhvstcj]]: prod's `GET /v1/issues`
    // was failing on the embedded `Username` actor pre-fix.
    let form_issue = store
        .get_issue(&parse_issue_id("i-actform")?, false)
        .await
        .context("store-level read of i-actform")?;
    let form_response = form_issue
        .item
        .form_response
        .as_ref()
        .context("i-actform: expected form_response to be Some after cleanup")?;
    match &form_response.actor {
        hydra_common::ActorId::User(name) if name.as_str() == "alice" => {}
        other => bail!("i-actform.form_response.actor: expected User(alice); got {other:?}"),
    }
    if form_response.action_id != "approve" {
        bail!(
            "i-actform.form_response.action_id: expected 'approve'; got {:?}",
            form_response.action_id
        );
    }

    Ok(())
}

/// Read the `actor` JSONB column from any `id`+`is_latest`-keyed table
/// and compare it to `expected`. Used by the §3.2 multi-table coverage
/// assertions and the nested `ActorRef` (System/Automation) checks.
async fn expect_table_actor(
    pool: &PgPool,
    table: &str,
    id: &str,
    expected: Option<serde_json::Value>,
) -> Result<()> {
    let sql = format!(
        "SELECT actor::text AS pl FROM metis.{table} \
         WHERE id = $1 AND is_latest = TRUE",
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read {table}.actor for {id}"))?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .with_context(|| format!("decode {table}.actor JSON for {id}"))?;
    if got != expected {
        bail!("{table}({id}).actor: expected {expected:?}; got {got:?}");
    }
    Ok(())
}

async fn expect_issue_actor_actor_id(
    pool: &PgPool,
    issue_id: &str,
    expected_actor_id: Option<serde_json::Value>,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT actor::text AS pl FROM metis.issues_v2 \
         WHERE id = $1 AND is_latest = TRUE",
    )
    .bind(issue_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read issues_v2.actor for {issue_id}"))?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .with_context(|| format!("decode issues_v2.actor JSON for {issue_id}"))?;
    let actor_id = got.as_ref().and_then(|v| {
        v.get("Authenticated")
            .and_then(|a| a.get("actor_id"))
            .cloned()
    });
    if actor_id != expected_actor_id {
        bail!(
            "issue {issue_id}: expected Authenticated.actor_id={expected_actor_id:?}; got full actor={got:?}",
        );
    }
    Ok(())
}

/// Canonical External-legacy fallback JSON wire shape.
fn external_legacy(username: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "External": {"system": "legacy", "username": username.into()}
    })
}

async fn expect_session_event_count(pool: &PgPool, session_id: &str, expected: i64) -> Result<()> {
    let row = sqlx::query("SELECT COUNT(*) FROM metis.session_events_v2 WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("count session_events_v2 for {session_id}"))?;
    let got: i64 = row.get(0);
    if got != expected {
        bail!("session {session_id}: expected {expected} session_events; got {got}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// §3.3 store / domain-level smoke
//
// The §3.2 assertions above verify SQL-level shapes of the migrated rows. This
// third layer reads them back through the live `Store` trait and asserts the
// typed domain objects (Principal, SessionMode, Review, SessionEvent,
// refers-to relationship) deserialize cleanly, then exercises a
// create→read-back cycle on the same APIs so any post-migration write path
// that diverged from the read path fails loud here instead of at first prod
// traffic.
//
// Preserved verbatim — this is the round-2 acceptance criterion from the
// prior design. See memory rule `migration-test-read-migrated` and design
// §3.3 / §7.
// ---------------------------------------------------------------------------

async fn assert_store_level_smoke(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());

    smoke_read_issues(&store).await?;
    smoke_read_patches(&store).await?;
    smoke_read_sessions(&store).await?;
    smoke_read_refers_to(&store).await?;
    smoke_read_session_events(&store).await?;

    smoke_create_issue(&store).await?;
    smoke_create_patch(&store).await?;
    smoke_create_session(&store).await?;
    smoke_create_relationship(&store).await?;

    Ok(())
}

async fn smoke_read_issues(store: &PostgresStoreV2) -> Result<()> {
    // Bare-string assignee → Principal::User { name }.
    let issue = store
        .get_issue(&parse_issue_id("i-bareasgn")?, false)
        .await
        .context("Store::get_issue(i-bareasgn)")?;
    match &issue.item.assignee {
        Some(Principal::User { name }) if name.as_str() == "jayantk" => {}
        other => bail!("i-bareasgn: expected Principal::User(jayantk); got {other:?}"),
    }

    // `users/jayantk` → Principal::User { name }.
    let issue = store
        .get_issue(&parse_issue_id("i-userpath")?, false)
        .await
        .context("Store::get_issue(i-userpath)")?;
    match &issue.item.assignee {
        Some(Principal::User { name }) if name.as_str() == "jayantk" => {}
        other => bail!("i-userpath: expected Principal::User(jayantk); got {other:?}"),
    }

    // `agents/swe` → Principal::Agent { name }.
    let issue = store
        .get_issue(&parse_issue_id("i-agentpath")?, false)
        .await
        .context("Store::get_issue(i-agentpath)")?;
    match &issue.item.assignee {
        Some(Principal::Agent { name }) if name.as_str() == "swe" => {}
        other => bail!("i-agentpath: expected Principal::Agent(swe); got {other:?}"),
    }

    // `external/github/foo` is intentionally left NULL by the SQL backfill.
    let issue = store
        .get_issue(&parse_issue_id("i-extpath")?, false)
        .await
        .context("Store::get_issue(i-extpath)")?;
    if issue.item.assignee.is_some() {
        bail!(
            "i-extpath: expected assignee=None (external left NULL by backfill); got {:?}",
            issue.item.assignee
        );
    }

    // Bare NULL assignee → None.
    let issue = store
        .get_issue(&parse_issue_id("i-nullasgn")?, false)
        .await
        .context("Store::get_issue(i-nullasgn)")?;
    if issue.item.assignee.is_some() {
        bail!(
            "i-nullasgn: expected assignee=None; got {:?}",
            issue.item.assignee
        );
    }

    Ok(())
}

async fn smoke_read_patches(store: &PostgresStoreV2) -> Result<()> {
    let cases = [
        (
            "p-barerev",
            Principal::User {
                name: ApiUsername::try_new("jayantk").expect("jayantk validates"),
            },
        ),
        (
            "p-agentrev",
            Principal::Agent {
                name: AgentName::try_new("swe").expect("swe validates"),
            },
        ),
        (
            "p-typedrev",
            Principal::User {
                name: ApiUsername::try_new("jayantk").expect("jayantk validates"),
            },
        ),
    ];
    for (id, expected) in cases {
        let patch = store
            .get_patch(&parse_patch_id(id)?, false)
            .await
            .with_context(|| format!("Store::get_patch({id})"))?;
        let author = patch
            .item
            .reviews
            .first()
            .with_context(|| format!("{id}: expected at least one review"))?
            .author
            .clone();
        if author != expected {
            bail!("{id}: expected reviews[0].author={expected:?}; got {author:?}");
        }
    }
    Ok(())
}

async fn smoke_read_sessions(store: &PostgresStoreV2) -> Result<()> {
    // Headless task: mode backfill -> unit-like SessionMode::Headless, with
    // the legacy `prompt` column backfilled onto `agent_config.system_prompt`.
    let session = store
        .get_session(&parse_session_id("s-headalpha")?, false)
        .await
        .context("Store::get_session(s-headalpha)")?;
    if !matches!(&session.item.mode, SessionMode::Headless) {
        bail!(
            "s-headalpha: expected unit-like SessionMode::Headless; got {:?}",
            session.item.mode
        );
    }
    if session.item.agent_config.system_prompt.as_deref() != Some("do a thing") {
        bail!(
            "s-headalpha: expected agent_config.system_prompt='do a thing'; got {:?}",
            session.item.agent_config.system_prompt
        );
    }

    // Interactive task: mode backfill -> SessionMode::Interactive { conversation_id, .. }.
    let session = store
        .get_session(&parse_session_id("s-interone")?, false)
        .await
        .context("Store::get_session(s-interone)")?;
    match &session.item.mode {
        SessionMode::Interactive {
            conversation_id, ..
        } if conversation_id.as_ref() == "c-convalpha" => {}
        other => bail!("s-interone: expected Interactive(c-convalpha); got {other:?}"),
    }

    // Resumed interactive task: resumed_from backfill points at the predecessor.
    let session = store
        .get_session(&parse_session_id("s-intertwo")?, false)
        .await
        .context("Store::get_session(s-intertwo)")?;
    match session.item.resumed_from.as_ref().map(|s| s.as_ref()) {
        Some("s-interone") => {}
        other => bail!("s-intertwo: expected resumed_from=s-interone; got {other:?}"),
    }
    Ok(())
}

async fn smoke_read_refers_to(store: &PostgresStoreV2) -> Result<()> {
    // The fixture's snake_case `refers_to` row between i-bareasgn and
    // i-userpath should have been renamed to `refers-to` by the
    // 20260529000000_rename_refers_to_to_kebab_case migration, and
    // `Store::get_relationships` should surface it with the typed
    // `RelationshipType::RefersTo` discriminant.
    let source: HydraId = parse_issue_id("i-bareasgn")?.into();
    let rels = store
        .get_relationships(Some(&source), None, Some(RelationshipType::RefersTo))
        .await
        .context("Store::get_relationships(refers-to from i-bareasgn)")?;
    let target_expected: HydraId = parse_issue_id("i-userpath")?.into();
    if !rels
        .iter()
        .any(|r| r.target_id == target_expected && r.rel_type == RelationshipType::RefersTo)
    {
        bail!("expected a refers-to relationship from i-bareasgn to i-userpath; got {rels:?}");
    }
    Ok(())
}

async fn smoke_read_session_events(store: &PostgresStoreV2) -> Result<()> {
    // `migrate-events` partitioned the two c-convalpha events into s-interone's
    // window (`[14:00, 15:00)`). The §3.3 smoke confirms `Store::get_session_events`
    // round-trips them into typed `SessionEvent::UserMessage` /
    // `AssistantMessage` variants so any serde drift between
    // `conversation_events_v2.event_data` and `session_events_v2.event_data`
    // fails loud here.
    let events = store
        .get_session_events(&parse_session_id("s-interone")?)
        .await
        .context("Store::get_session_events(s-interone)")?;
    if events.len() != 2 {
        bail!(
            "expected 2 migrated session_events for s-interone; got {} ({events:?})",
            events.len(),
        );
    }
    match &events[0].item {
        SessionEvent::UserMessage { content, .. } if content == "smoke hello" => {}
        other => bail!("s-interone[0]: expected UserMessage('smoke hello'); got {other:?}"),
    }
    match &events[1].item {
        SessionEvent::AssistantMessage { content, .. } if content == "smoke hi" => {}
        other => bail!("s-interone[1]: expected AssistantMessage('smoke hi'); got {other:?}"),
    }
    Ok(())
}

async fn smoke_create_issue(store: &PostgresStoreV2) -> Result<()> {
    let agent = AgentName::try_new("swe").expect("swe validates as an agent name");
    let issue = Issue::new(
        IssueType::Task,
        "smoke: create issue with agent assignee".to_string(),
        "post-migration write-path round-trip for Principal::Agent assignees".to_string(),
        Username::from("jayantk"),
        String::new(),
        IssueStatus::Open.into(),
        hydra_server::domain::projects::default_project_id(),
        Some(Principal::Agent {
            name: agent.clone(),
        }),
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
        None,
    );
    let (id, _) = store
        .add_issue(issue, &ActorRef::test())
        .await
        .context("Store::add_issue post-migration")?;
    let fetched = store
        .get_issue(&id, false)
        .await
        .context("Store::get_issue post-migration")?;
    match &fetched.item.assignee {
        Some(Principal::Agent { name }) if name.as_str() == "swe" => Ok(()),
        other => bail!(
            "post-migration create_issue did not round-trip Principal::Agent(swe); got {other:?}"
        ),
    }
}

async fn smoke_create_patch(store: &PostgresStoreV2) -> Result<()> {
    let author = Principal::User {
        name: ApiUsername::try_new("jayantk").expect("jayantk validates"),
    };
    let review = Review::new(
        "smoke approval".to_string(),
        true,
        author.clone(),
        Some(Utc::now()),
    );
    let patch = Patch::new(
        "smoke: create patch with typed review author".to_string(),
        "post-migration write-path round-trip for typed Review.author".to_string(),
        String::new(),
        PatchStatus::Open,
        false,
        Username::from("jayantk"),
        vec![review],
        RepoName::from_str("dourolabs/hydra").expect("repo name validates"),
        None,
        None,
        None,
        None,
    );
    let (id, _) = store
        .add_patch(patch, &ActorRef::test())
        .await
        .context("Store::add_patch post-migration")?;
    let fetched = store
        .get_patch(&id, false)
        .await
        .context("Store::get_patch post-migration")?;
    let fetched_author = fetched
        .item
        .reviews
        .first()
        .context("post-migration patch: expected one review")?
        .author
        .clone();
    if fetched_author != author {
        bail!(
            "post-migration create_patch did not round-trip the typed Review.author: \
             expected {author:?}; got {fetched_author:?}"
        );
    }
    Ok(())
}

async fn smoke_create_session(store: &PostgresStoreV2) -> Result<()> {
    let session = Session::new(
        Username::from("jayantk"),
        None,
        None,
        AgentConfig::new(None, None, Some("smoke: do a thing".to_string()), None),
        Default::default(),
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        Status::Complete,
        None,
        None,
    );
    let (id, _) = store
        .add_session(session, Utc::now(), &ActorRef::test())
        .await
        .context("Store::add_session post-migration")?;
    let fetched = store
        .get_session(&id, false)
        .await
        .context("Store::get_session post-migration")?;
    if !matches!(&fetched.item.mode, SessionMode::Headless) {
        bail!(
            "post-migration create_session did not round-trip SessionMode::Headless; got {:?}",
            fetched.item.mode
        );
    }
    if fetched.item.agent_config.system_prompt.as_deref() != Some("smoke: do a thing") {
        bail!(
            "post-migration create_session did not round-trip agent_config.system_prompt; \
             got {:?}",
            fetched.item.agent_config.system_prompt
        );
    }
    Ok(())
}

async fn smoke_create_relationship(store: &PostgresStoreV2) -> Result<()> {
    // The fixture already seeded a refers-to between i-bareasgn → i-userpath
    // (verified above). Add a fresh refers-to between two different fixture
    // issues to confirm the post-rename write path accepts the kebab-case
    // value.
    let source: HydraId = parse_issue_id("i-nullasgn")?.into();
    let target: HydraId = parse_issue_id("i-agentpath")?.into();
    let inserted = store
        .add_relationship(&source, &target, RelationshipType::RefersTo)
        .await
        .context("Store::add_relationship(refers-to) post-migration")?;
    if !inserted {
        bail!(
            "post-migration add_relationship reported no insert — \
             the fixture already had a refers-to from i-nullasgn to i-agentpath?"
        );
    }
    let rels = store
        .get_relationships(Some(&source), None, Some(RelationshipType::RefersTo))
        .await
        .context("Store::get_relationships(refers-to from i-nullasgn)")?;
    if !rels
        .iter()
        .any(|r| r.target_id == target && r.rel_type == RelationshipType::RefersTo)
    {
        bail!("post-migration: expected to read back the just-inserted refers-to; got {rels:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recent-migration data-shape round-trip
//
// The three most recent feature migrations (`20260603020000_add_triggers_table`,
// `20260604000000_drop_conversation_events_v2`, `20260604000001_create_projects`)
// land schema after the last baseline pin, so no fixture rows exist for them.
// Seed one row in each of `metis.triggers` and `metis.projects` via raw SQL
// post-rollforward, then read back through `PostgresStoreV2::get_trigger` /
// `get_project` to confirm:
//
//   * the `BEFORE INSERT` `maintain_latest_version` triggers flipped
//     `is_latest = true` (`get_*` relies on the `id`-keyed lookups, which
//     would surface a NULL row if the trigger never fired);
//   * the JSONB columns (`schedule` / `actions` for triggers,
//     `statuses` for projects) deserialize cleanly into the typed domain
//     objects;
//   * the primary key (`id`, `version_number`) round-trips through the store.
// ---------------------------------------------------------------------------

async fn assert_recent_migration_data_shape(pool: &PgPool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey};
    use hydra_common::triggers::{Schedule, Trigger as ApiTrigger};

    // ---- triggers -----------------------------------------------------
    let trigger_id = parse_trigger_id("t-rtrip")?;
    let schedule_json = serde_json::json!({
        "Cron": {"expression": "0 9 * * MON", "timezone": "UTC"}
    });
    let actions_json = serde_json::json!([]);
    sqlx::query(
        "INSERT INTO metis.triggers \
         (id, version_number, enabled, creator, schedule, actions) \
         VALUES ($1, 1, TRUE, 'jayantk', $2, $3)",
    )
    .bind(trigger_id.as_ref())
    .bind(&schedule_json)
    .bind(&actions_json)
    .execute(pool)
    .await
    .context("seed metis.triggers row for round-trip assertion")?;

    let store = PostgresStoreV2::new(pool.clone());
    let fetched = store
        .get_trigger(&trigger_id, false)
        .await
        .context("Store::get_trigger post-migration")?;
    if fetched.version != 1 {
        bail!(
            "trigger {trigger_id}: expected version 1; got {}",
            fetched.version
        );
    }
    let ApiTrigger {
        enabled,
        schedule,
        actions,
        creator,
        ..
    } = &fetched.item;
    if !enabled {
        bail!("trigger {trigger_id}: expected enabled=true");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "trigger {trigger_id}: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    match schedule {
        Schedule::Cron {
            expression,
            timezone,
        } if expression == "0 9 * * MON" && timezone.as_deref() == Some("UTC") => {}
        other => bail!("trigger {trigger_id}: expected Cron(0 9 * * MON / UTC); got {other:?}"),
    }
    if !actions.is_empty() {
        bail!(
            "trigger {trigger_id}: expected empty actions; got {} entries",
            actions.len()
        );
    }

    // ---- projects -----------------------------------------------------
    let project_id = parse_project_id("j-rtrip")?;
    let statuses_json = serde_json::json!([
        {
            "key": "open",
            "label": "Open",
            "color": "#3498db",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false
        }
    ]);
    // Include `prompt_path` so the new column added by
    // 20260606010000_add_projects_prompt_path.sql is exercised by the
    // seed INSERT and the Store::get_project read back below.
    sqlx::query(
        "INSERT INTO metis.projects \
         (id, version_number, key, name, statuses, creator, prompt_path) \
         VALUES ($1, 1, 'roundtrip', 'Roundtrip', $2, 'jayantk', $3)",
    )
    .bind(project_id.as_ref())
    .bind(&statuses_json)
    .bind("/projects/roundtrip/prompt.md")
    .execute(pool)
    .await
    .context("seed metis.projects row for round-trip assertion")?;

    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("Store::get_project post-migration")?;
    if fetched.version != 1 {
        bail!(
            "project {project_id}: expected version 1; got {}",
            fetched.version
        );
    }
    let ApiProject {
        key,
        name,
        statuses,
        creator,
        prompt_path,
        ..
    } = &fetched.item;
    if key != &ProjectKey::try_new("roundtrip").unwrap() {
        bail!("project {project_id}: expected key='roundtrip'; got {key:?}");
    }
    if name != "Roundtrip" {
        bail!("project {project_id}: expected name='Roundtrip'; got {name:?}");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "project {project_id}: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    if prompt_path.as_deref() != Some("/projects/roundtrip/prompt.md") {
        bail!(
            "project {project_id}: expected prompt_path='/projects/roundtrip/prompt.md'; got {prompt_path:?}"
        );
    }
    let Some(status) = statuses.first() else {
        bail!("project {project_id}: expected one status; got none");
    };
    if status.key.as_str() != "open"
        || status.label != "Open"
        || status.color.as_ref() != "#3498db"
        || status.unblocks_parents
        || status.unblocks_dependents
        || status.cascades_to_children
    {
        bail!(
            "project {project_id}: status[0] did not round-trip the seeded JSONB shape; got {status:?}"
        );
    }

    Ok(())
}

fn parse_issue_id(s: &str) -> Result<IssueId> {
    IssueId::from_str(s).with_context(|| format!("parse issue id '{s}'"))
}

fn parse_trigger_id(s: &str) -> Result<TriggerId> {
    TriggerId::from_str(s).with_context(|| format!("parse trigger id '{s}'"))
}

fn parse_project_id(s: &str) -> Result<ProjectId> {
    ProjectId::from_str(s).with_context(|| format!("parse project id '{s}'"))
}

fn parse_patch_id(s: &str) -> Result<PatchId> {
    PatchId::from_str(s).with_context(|| format!("parse patch id '{s}'"))
}

fn parse_session_id(s: &str) -> Result<SessionId> {
    SessionId::from_str(s).with_context(|| format!("parse session id '{s}'"))
}

// ---------------------------------------------------------------------------
// 20260607000000_seed_default_project — assert that the seed INSERT, the
// `metis.issues_v2.project_id` backfill UPDATE, and the migration's
// idempotency guard (`ON CONFLICT (id, version_number) DO NOTHING`) all
// behave as designed. Coverage gap closed by [[i-bivbnsgb]] (follow-up to
// [[p-xtixlxfy]]) — the merged seed migration shipped with in-store
// round-trip tests but no migration-framework coverage.
// ---------------------------------------------------------------------------

async fn seed_default_project_migration_inserts_row(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT id, version_number, key, name, \
                statuses::text AS statuses, creator, deleted, \
                actor::text AS actor, is_latest, prompt_path \
         FROM metis.projects WHERE id = 'j-defaul'",
    )
    .fetch_one(pool)
    .await
    .context("read seeded default project row 'j-defaul'")?;

    let id: String = row.try_get("id")?;
    let version_number: i64 = row.try_get("version_number")?;
    let key: String = row.try_get("key")?;
    let name: String = row.try_get("name")?;
    let statuses_text: String = row.try_get("statuses")?;
    let creator: String = row.try_get("creator")?;
    let deleted: bool = row.try_get("deleted")?;
    let is_latest: bool = row.try_get("is_latest")?;
    let actor: Option<String> = row.try_get("actor")?;
    let prompt_path: Option<String> = row.try_get("prompt_path")?;

    if id != "j-defaul" {
        bail!("j-defaul: expected id='j-defaul'; got {id:?}");
    }
    if version_number != 1 {
        bail!("j-defaul: expected version_number=1; got {version_number}");
    }
    if key != "default" {
        bail!("j-defaul: expected key='default'; got {key:?}");
    }
    if name != "Default" {
        bail!("j-defaul: expected name='Default'; got {name:?}");
    }
    if creator != "system" {
        bail!("j-defaul: expected creator='system'; got {creator:?}");
    }
    if deleted {
        bail!("j-defaul: expected deleted=FALSE; got TRUE");
    }
    if !is_latest {
        bail!("j-defaul: expected is_latest=TRUE; got FALSE");
    }
    if actor.is_some() {
        bail!("j-defaul: expected actor=NULL; got {actor:?}");
    }
    if prompt_path.as_deref() != Some("/projects/default/prompt.md") {
        bail!("j-defaul: expected prompt_path='/projects/default/prompt.md'; got {prompt_path:?}");
    }

    // `statuses` JSONB must deserialize into a Vec<StatusDefinition> that
    // matches `default_project_seed()` byte-for-byte. Comparing against
    // the Rust seed locks the SQL literal to the Rust constant: any drift
    // in either direction fails loud here.
    let statuses: Vec<StatusDefinition> = serde_json::from_str(&statuses_text)
        .context("deserialize projects.statuses into Vec<StatusDefinition>")?;
    let expected = default_project_seed().statuses;
    if statuses != expected {
        bail!(
            "j-defaul: statuses do not match default_project_seed(): \
             expected {expected:?}; got {statuses:?}"
        );
    }
    Ok(())
}

async fn seed_default_project_migration_backfills_null_project_ids(pool: &PgPool) -> Result<()> {
    // Every fixture row that had NULL `project_id` at baseline-insert
    // time (single-version and multi-version) must now point at
    // `'j-defaul'`. The multi-version rows verify that the UPDATE
    // touches every NULL row regardless of `is_latest`.
    for (id, version) in [("i-seedone", 1_i64), ("i-seedmv", 1), ("i-seedmv", 2)] {
        let row = sqlx::query(
            "SELECT project_id FROM metis.issues_v2 \
             WHERE id = $1 AND version_number = $2",
        )
        .bind(id)
        .bind(version)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read project_id for metis.issues_v2({id}, {version})"))?;
        let project_id: Option<String> = row.try_get("project_id")?;
        if project_id.as_deref() != Some("j-defaul") {
            bail!(
                "metis.issues_v2({id}, {version}).project_id: \
                 expected 'j-defaul'; got {project_id:?}"
            );
        }
    }

    // Catch-all: no `issues_v2` row should be left with NULL project_id
    // post-backfill. The migration's UPDATE is unconditional on
    // `is_latest`, so older / soft-deleted versions get backfilled too.
    let row = sqlx::query("SELECT COUNT(*) FROM metis.issues_v2 WHERE project_id IS NULL")
        .fetch_one(pool)
        .await
        .context("count remaining NULL project_id rows after backfill")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 metis.issues_v2 rows with NULL project_id post-backfill; got {count}");
    }
    Ok(())
}

async fn seed_default_project_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Original behavior of this assertion was to replay the seed
    // migration body verbatim. After
    // 20260611000000_drop_projects_default_status_key drops a column
    // the body references, a verbatim re-apply errors — so the
    // idempotency guarantee that matters here is now: after the full
    // migration plan rolls forward, exactly one j-defaul row exists.
    let row = sqlx::query("SELECT COUNT(*) FROM metis.projects WHERE id = 'j-defaul'")
        .fetch_one(pool)
        .await
        .context("count projects rows for j-defaul after rollforward")?;
    let count: i64 = row.try_get(0)?;
    if count != 1 {
        bail!("expected exactly 1 metis.projects row for j-defaul post-rollforward; got {count}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260608000000_drop_status_icon — assert that the migration strips the
// `icon` key from every row's `metis.projects.statuses` array (both the
// already-seeded `j-defaul` row and a custom fixture row), the migration
// body is idempotent, and the post-migration JSON deserializes cleanly
// through `Vec<StatusDefinition>` (which no longer carries an `icon`
// field). Covers [[i-jazguvll]].
// ---------------------------------------------------------------------------

async fn drop_status_icon_migration_strips_default_seed(pool: &PgPool) -> Result<()> {
    // The `j-defaul` row was inserted by 20260607000000_seed_default_project
    // with `"icon": "..."` on every status; 20260608000000_drop_status_icon
    // must have stripped each. `seed_default_project_migration_inserts_row`
    // above already compares the deserialized Vec<StatusDefinition>
    // against `default_project_seed()`; here we additionally assert at
    // the JSONB level so a regression that re-adds the key shows up
    // independently of the Rust type's serde shape.
    let row =
        sqlx::query("SELECT statuses::text AS statuses FROM metis.projects WHERE id = 'j-defaul'")
            .fetch_one(pool)
            .await
            .context("read j-defaul statuses post-drop_status_icon")?;
    let statuses_text: String = row.try_get("statuses")?;
    let statuses_json: serde_json::Value = serde_json::from_str(&statuses_text)
        .context("decode j-defaul.statuses JSON post-drop_status_icon")?;
    let arr = statuses_json
        .as_array()
        .context("expected j-defaul.statuses to be a JSON array")?;
    if arr.len() != 5 {
        bail!(
            "j-defaul: expected 5 statuses post-drop_status_icon; got {}",
            arr.len()
        );
    }
    for (i, elem) in arr.iter().enumerate() {
        let obj = elem
            .as_object()
            .with_context(|| format!("j-defaul.statuses[{i}] is not a JSON object"))?;
        if obj.contains_key("icon") {
            bail!("j-defaul.statuses[{i}]: expected no `icon` key; got {elem}");
        }
    }
    Ok(())
}

async fn drop_status_icon_migration_strips_custom_row(pool: &PgPool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey};

    // The `j-iconfix` row was inserted by the
    // `20260607000000__pre_drop_status_icon` baseline with three statuses
    // that each carry `"icon": "<value>"`. Read back through
    // `Store::get_project` so any drift between the migration's
    // post-strip JSON shape and the Rust `StatusDefinition` serde impl
    // fails loud here (the typed deserializer must accept the migrated
    // rows).
    let project_id = parse_project_id("j-iconfix")?;
    let store = PostgresStoreV2::new(pool.clone());
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("Store::get_project(j-iconfix) post-drop_status_icon")?;

    let ApiProject {
        key,
        name,
        statuses,
        creator,
        ..
    } = &fetched.item;
    if key != &ProjectKey::try_new("iconfix").unwrap() {
        bail!("j-iconfix: expected key='iconfix'; got {key:?}");
    }
    if name != "Icon Fixture" {
        bail!("j-iconfix: expected name='Icon Fixture'; got {name:?}");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "j-iconfix: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    if statuses.len() != 3 {
        bail!("j-iconfix: expected 3 statuses; got {}", statuses.len());
    }

    let expected_shapes: &[(&str, &str, &str, bool, bool, bool)] = &[
        ("todo", "Todo", "#abcdef", false, false, false),
        ("doing", "Doing", "#f1c40f", false, false, false),
        ("done", "Done", "#2ecc71", true, true, false),
    ];
    for (i, (k, label, color, up, ud, ctc)) in expected_shapes.iter().enumerate() {
        let s = &statuses[i];
        if s.key.as_str() != *k
            || s.label != *label
            || s.color.as_ref() != *color
            || s.unblocks_parents != *up
            || s.unblocks_dependents != *ud
            || s.cascades_to_children != *ctc
        {
            bail!(
                "j-iconfix.statuses[{i}]: expected ({k}, {label}, {color}, {up}, {ud}, {ctc}); got {s:?}"
            );
        }
    }

    // Belt-and-braces JSONB-level check: confirm the raw column shape
    // has no surviving `icon` keys, independent of the typed-serde path
    // (e.g. a future `StatusDefinition` that silently ignores unknown
    // fields wouldn't catch a regression).
    let row =
        sqlx::query("SELECT statuses::text AS statuses FROM metis.projects WHERE id = 'j-iconfix'")
            .fetch_one(pool)
            .await
            .context("read j-iconfix raw statuses for icon-presence check")?;
    let statuses_text: String = row.try_get("statuses")?;
    let statuses_json: serde_json::Value = serde_json::from_str(&statuses_text)
        .context("decode j-iconfix.statuses JSON post-drop_status_icon")?;
    let arr = statuses_json
        .as_array()
        .context("expected j-iconfix.statuses to be a JSON array")?;
    for (i, elem) in arr.iter().enumerate() {
        let obj = elem
            .as_object()
            .with_context(|| format!("j-iconfix.statuses[{i}] is not a JSON object"))?;
        if obj.contains_key("icon") {
            bail!("j-iconfix.statuses[{i}]: expected no `icon` key post-strip; got {elem}");
        }
    }

    Ok(())
}

async fn drop_status_icon_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Re-execute the migration body verbatim. `elem - 'icon'` is a no-op
    // on rows whose statuses no longer carry the key, so a second pass
    // must produce no change. Reading the file rather than hard-coding
    // the SQL keeps this test honest if the migration's body ever
    // changes shape.
    let before = snapshot_status_arrays(pool).await?;
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260608000000_drop_status_icon.sql"),
    )
    .context("read postgres drop_status_icon migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres drop_status_icon migration body")?;
    let after = snapshot_status_arrays(pool).await?;
    if before != after {
        bail!(
            "drop_status_icon: expected no change on re-apply; before={before:?}, after={after:?}"
        );
    }
    Ok(())
}

/// Read every `(id, statuses)` pair from `metis.projects` and return it
/// keyed by `id`. Used by the idempotency check above to compare the
/// statuses JSON byte-for-byte across re-applications.
async fn snapshot_status_arrays(pool: &PgPool) -> Result<HashMap<String, String>> {
    let rows = sqlx::query("SELECT id, statuses::text AS statuses FROM metis.projects")
        .fetch_all(pool)
        .await
        .context("read all projects rows for statuses snapshot")?;
    let mut snap = HashMap::new();
    for row in rows {
        let id: String = row.try_get("id")?;
        let statuses: String = row.try_get("statuses")?;
        snap.insert(id, statuses);
    }
    Ok(snap)
}

// ---------------------------------------------------------------------------
// 20260609000000_add_creator_to_auth_tokens / 20260609010000_drop_actors_v2
// ---------------------------------------------------------------------------

async fn denormalize_creator_session_backfill(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT creator FROM metis.auth_tokens WHERE token_hash = 'hash-session-alice'",
    )
    .fetch_one(pool)
    .await
    .context("read back session-bound auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "alice" {
        bail!("session-bound auth_tokens.creator: expected 'alice'; got {creator:?}");
    }
    Ok(())
}

async fn denormalize_creator_user_backfill(pool: &PgPool) -> Result<()> {
    let row =
        sqlx::query("SELECT creator FROM metis.auth_tokens WHERE token_hash = 'hash-cli-bob'")
            .fetch_one(pool)
            .await
            .context("read back user-CLI auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "bob" {
        bail!("user-CLI auth_tokens.creator: expected 'bob'; got {creator:?}");
    }
    Ok(())
}

async fn denormalize_creator_domain_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let session_row = store
        .get_auth_token_by_hash("hash-session-alice")
        .await
        .context("PostgresStoreV2::get_auth_token_by_hash(hash-session-alice)")?
        .context("session-bound row not found via domain API")?;
    if session_row.creator.as_str() != "alice" {
        bail!(
            "domain-level session-bound creator: expected 'alice'; got {:?}",
            session_row.creator
        );
    }
    if session_row.actor_name != "agents/swe" {
        bail!(
            "domain-level session-bound actor_name: expected 'agents/swe'; got {:?}",
            session_row.actor_name
        );
    }

    let user_row = store
        .get_auth_token_by_hash("hash-cli-bob")
        .await
        .context("PostgresStoreV2::get_auth_token_by_hash(hash-cli-bob)")?
        .context("user-CLI row not found via domain API")?;
    if user_row.creator.as_str() != "bob" {
        bail!(
            "domain-level user-CLI creator: expected 'bob'; got {:?}",
            user_row.creator
        );
    }
    if user_row.session_id.is_some() {
        bail!(
            "domain-level user-CLI session_id: expected None; got {:?}",
            user_row.session_id
        );
    }
    Ok(())
}

async fn drop_actors_v2_migration_removes_table(pool: &PgPool) -> Result<()> {
    if table_exists(pool, "actors_v2").await? {
        bail!("expected `metis.actors_v2` table to be dropped after 20260609010000");
    }
    Ok(())
}

async fn denormalize_creator_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let creator = Username::from("eve");
    let sid = SessionId::new();
    store
        .add_auth_token("users/eve", "hash-eve", Some(&sid), &creator)
        .await
        .context("post-migration add_auth_token write")?;
    let fresh = store
        .get_auth_token_by_hash("hash-eve")
        .await
        .context("post-migration get_auth_token_by_hash")?
        .context("post-migration write should be readable")?;
    if fresh.creator != creator {
        bail!(
            "post-migration write read-back: expected creator='eve'; got {:?}",
            fresh.creator
        );
    }
    if fresh.session_id.as_ref() != Some(&sid) {
        bail!(
            "post-migration write read-back: expected session_id={sid:?}; got {:?}",
            fresh.session_id
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260610000000_add_projects_priority — assert that the new `priority`
// column is backfilled to `rank * 1000.0` over the latest-version rows
// (ranked by `created_at DESC, id DESC`), and that the
// `PostgresStoreV2::list_projects` typed read path surfaces the
// backfilled value. Sister to the SQLite assertion in
// `migration_roundtrip_sqlite.rs`; both catch the
// `#[sqlx(default)]` / SELECT-projection foot-gun the parent issue
// calls out.
// ---------------------------------------------------------------------------

async fn add_projects_priority_backfill_sql_level(pool: &PgPool) -> Result<()> {
    let expected: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
    for (id, want) in expected {
        let row = sqlx::query(
            "SELECT priority FROM metis.projects \
             WHERE id = $1 AND is_latest = true",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read metis.projects.priority for {id}"))?;
        let got: f64 = row.try_get("priority")?;
        if got != *want {
            bail!("metis.projects({id}).priority: expected {want}; got {got}");
        }
    }
    Ok(())
}

async fn add_projects_priority_backfill_domain_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let listed = store
        .list_projects(false)
        .await
        .context("PostgresStoreV2::list_projects(include_deleted = false)")?;

    let want: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
    let got: Vec<(String, f64)> = listed
        .iter()
        .filter_map(|(id, v)| {
            let id_str = id.as_ref().to_string();
            if want.iter().any(|(w, _)| *w == id_str.as_str()) {
                Some((id_str, v.item.priority))
            } else {
                None
            }
        })
        .collect();
    let want_owned: Vec<(String, f64)> = want.iter().map(|(s, p)| (s.to_string(), *p)).collect();
    if got != want_owned {
        bail!("list_projects filtered to baseline rows: expected {want_owned:?}; got {got:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260611000000_drop_projects_default_status_key — assert that the
// migration removes the `default_status_key` column from
// `metis.projects`, that the seeded `j-defaul` row and the custom
// `j-dskdrop` baseline row still deserialize through
// `PostgresStoreV2::get_project` into the new `Project` wire type (no
// `default_status_key` field), and that the migration body is
// idempotent on a second pass.
// ---------------------------------------------------------------------------

async fn drop_projects_default_status_key_migration_removes_column(pool: &PgPool) -> Result<()> {
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!(
            "expected metis.projects.default_status_key to be dropped after \
             20260611000000_drop_projects_default_status_key"
        );
    }
    Ok(())
}

async fn drop_projects_default_status_key_migration_preserves_typed_read(
    pool: &PgPool,
) -> Result<()> {
    // Read the seeded `j-defaul` row plus the custom `j-dskdrop` baseline
    // row back through the typed store API. Both must deserialize into
    // `Project` without the `default_status_key` field — covers the
    // serde-projection foot-gun (post-migration SELECT must match the
    // ProjectRow struct, and the row must serde into the wire type).
    let store = PostgresStoreV2::new(pool.clone());

    let defaul = parse_project_id("j-defaul")?;
    let fetched = store
        .get_project(&defaul, false)
        .await
        .context("PostgresStoreV2::get_project(j-defaul) post-drop-default-status-key")?;
    if fetched.item.key.as_str() != "default" {
        bail!(
            "j-defaul: expected key='default'; got {:?}",
            fetched.item.key
        );
    }
    if fetched.item.statuses.len() != 5 {
        bail!(
            "j-defaul: expected 5 statuses; got {}",
            fetched.item.statuses.len()
        );
    }

    let dskdrop = parse_project_id("j-dskdrop")?;
    let fixture = store
        .get_project(&dskdrop, false)
        .await
        .context("PostgresStoreV2::get_project(j-dskdrop) post-drop-default-status-key")?;
    if fixture.item.key.as_str() != "dskdrop" {
        bail!(
            "j-dskdrop: expected key='dskdrop'; got {:?}",
            fixture.item.key
        );
    }
    if fixture.item.statuses.len() != 3 {
        bail!(
            "j-dskdrop: expected 3 statuses; got {}",
            fixture.item.statuses.len()
        );
    }
    let keys: Vec<&str> = fixture
        .item
        .statuses
        .iter()
        .map(|s| s.key.as_str())
        .collect();
    if keys != ["todo", "doing", "done"] {
        bail!("j-dskdrop: expected statuses [todo,doing,done]; got {keys:?}");
    }
    Ok(())
}

async fn drop_projects_default_status_key_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Re-execute the migration body verbatim. `ALTER TABLE ... DROP
    // COLUMN IF EXISTS` is intrinsically idempotent: a second pass on
    // the already-stripped table is a no-op. Reading the file rather
    // than hard-coding the SQL keeps this test honest if the
    // migration's body ever changes shape.
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260611000000_drop_projects_default_status_key.sql"),
    )
    .context(
        "read postgres drop_projects_default_status_key migration body for idempotency rerun",
    )?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres drop_projects_default_status_key migration body")?;
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!("drop_projects_default_status_key: column reappeared after idempotency rerun");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260612000000_issues_v2_project_id_not_null — Postgres parity for the
// schema-tightening migration. Sister to
// `migration_roundtrip_sqlite::issues_v2_project_id_*`. Asserts the
// column is NOT NULL, fresh NULL inserts are rejected, and the body is
// idempotent under re-execution. The pre-flight NULL-guard exercise
// lives in the sister sqlite roundtrip — repeating it here would require
// resetting the shared postgres database mid-test, which would invalidate
// the downstream idempotency assertion at the end of the run.
// ---------------------------------------------------------------------------

async fn issues_v2_project_id_is_not_null(pool: &PgPool) -> Result<()> {
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!(
            "expected `metis.issues_v2.project_id` to be NOT NULL after \
             20260612000000_issues_v2_project_id_not_null"
        );
    }
    Ok(())
}

async fn issues_v2_project_id_rejects_null_insert(pool: &PgPool) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, project_id) \
         VALUES ('i-nullchk', 99, 'task', 'null project_id insert must fail', 'system', NULL)",
    )
    .execute(pool)
    .await;
    match result {
        Err(_) => Ok(()),
        Ok(_) => bail!(
            "expected NULL project_id INSERT to fail post-migration; \
             the NOT NULL constraint was not applied"
        ),
    }
}

async fn issues_v2_project_id_not_null_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260612000000_issues_v2_project_id_not_null.sql"),
    )
    .context("read postgres issues_v2_project_id_not_null migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres issues_v2_project_id_not_null migration body")?;
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!("expected `metis.issues_v2.project_id` to stay NOT NULL after idempotency rerun");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260613000000_create_statuses /
// 20260613010000_add_issues_v2_status_sequence — assert the new
// `metis.statuses` table exists with the expected PK and unique index,
// that the seeded `j-defaul` project's status JSONB array backfills
// into matching `metis.statuses` rows (sequence 1..=5 in JSONB array
// order), that a custom-project baseline row covers the full column
// shape (`on_enter`, `prompt_path`, `interactive`), that every
// `metis.issues_v2` row has `status_sequence` populated and joins back
// to the original status key, and that re-applying either migration
// body is a no-op. Covers [[i-jvmpqwwe]] acceptance criteria (a)–(d).
// ---------------------------------------------------------------------------

async fn create_statuses_migration_schema_invariants(pool: &PgPool) -> Result<()> {
    // `pg_index.indkey` is `int2vector`, which is not an `anyarray` in
    // Postgres 16 — passing it directly to `array_position` errors at
    // function lookup. Cast through `text` (its output form is a
    // space-separated list) and split to get an ordinary `int[]` so
    // `unnest ... WITH ORDINALITY` can yield a 1-indexed column order.
    let row = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY k.ord) AS cols \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN unnest(string_to_array(i.indkey::text, ' ')::int[]) WITH ORDINALITY AS k(attnum, ord) ON TRUE \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE n.nspname = 'metis' AND c.relname = 'statuses' AND i.indisprimary",
    )
    .fetch_one(pool)
    .await
    .context("look up metis.statuses primary key columns")?;
    let pk_cols: Vec<String> = row.try_get("cols")?;
    if pk_cols != vec!["project_id".to_string(), "sequence".to_string()] {
        bail!("metis.statuses PK: expected [project_id, sequence]; got {pk_cols:?}");
    }

    let row = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY k.ord) AS cols \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_class ic ON ic.oid = i.indexrelid \
         JOIN unnest(string_to_array(i.indkey::text, ' ')::int[]) WITH ORDINALITY AS k(attnum, ord) ON TRUE \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE n.nspname = 'metis' AND c.relname = 'statuses' \
           AND ic.relname = 'statuses_project_key_idx'",
    )
    .fetch_one(pool)
    .await
    .context("look up metis.statuses_project_key_idx columns")?;
    let idx_cols: Vec<String> = row.try_get("cols")?;
    if idx_cols != vec!["project_id".to_string(), "key".to_string()] {
        bail!(
            "metis.statuses_project_key_idx columns: expected [project_id, key]; got {idx_cols:?}"
        );
    }
    let row = sqlx::query(
        "SELECT i.indisunique \
         FROM pg_index i \
         JOIN pg_class ic ON ic.oid = i.indexrelid \
         WHERE ic.relname = 'statuses_project_key_idx'",
    )
    .fetch_one(pool)
    .await
    .context("read uniqueness flag for statuses_project_key_idx")?;
    let is_unique: bool = row.try_get(0)?;
    if !is_unique {
        bail!("metis.statuses_project_key_idx: expected unique index; got non-unique");
    }
    Ok(())
}

async fn create_statuses_migration_backfills_default_seed(pool: &PgPool) -> Result<()> {
    // The seeded j-defaul row has 5 statuses in this exact order. The
    // backfill must produce sequence 1..=5 with the same column values,
    // matching `default_project_seed()` byte-for-byte after the icon
    // column was stripped by 20260608000000.
    let expected_seed = default_project_seed().statuses;
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter::text AS on_enter_text, prompt_path, interactive \
         FROM metis.statuses WHERE project_id = 'j-defaul' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read metis.statuses for j-defaul")?;
    if rows.len() != 5 {
        bail!(
            "metis.statuses(j-defaul): expected 5 rows; got {}",
            rows.len()
        );
    }
    for (i, row) in rows.iter().enumerate() {
        let sequence: i64 = row.try_get("sequence")?;
        if sequence != (i + 1) as i64 {
            bail!(
                "metis.statuses(j-defaul)[{i}]: expected sequence={}; got {sequence}",
                i + 1
            );
        }
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: bool = row.try_get("unblocks_parents")?;
        let unblocks_dependents: bool = row.try_get("unblocks_dependents")?;
        let cascades_to_children: bool = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: bool = row.try_get("interactive")?;

        let expected = &expected_seed[i];
        if key != expected.key.as_str() {
            bail!(
                "metis.statuses(j-defaul)[{i}].key: expected {}; got {key}",
                expected.key.as_str()
            );
        }
        if label != expected.label {
            bail!(
                "metis.statuses(j-defaul)[{i}].label: expected {}; got {label}",
                expected.label
            );
        }
        if color != expected.color.as_ref() {
            bail!(
                "metis.statuses(j-defaul)[{i}].color: expected {}; got {color}",
                expected.color
            );
        }
        if unblocks_parents != expected.unblocks_parents
            || unblocks_dependents != expected.unblocks_dependents
            || cascades_to_children != expected.cascades_to_children
        {
            bail!(
                "metis.statuses(j-defaul)[{i}] flags mismatch: \
                 expected ({}, {}, {}); got ({unblocks_parents}, {unblocks_dependents}, {cascades_to_children})",
                expected.unblocks_parents,
                expected.unblocks_dependents,
                expected.cascades_to_children
            );
        }
        if prompt_path != expected.prompt_path {
            bail!(
                "metis.statuses(j-defaul)[{i}].prompt_path: expected {:?}; got {prompt_path:?}",
                expected.prompt_path
            );
        }
        if interactive != expected.interactive {
            bail!(
                "metis.statuses(j-defaul)[{i}].interactive: expected {}; got {interactive}",
                expected.interactive
            );
        }
        // The default seed has no on_enter on any status.
        if on_enter_text.is_some() {
            bail!(
                "metis.statuses(j-defaul)[{i}].on_enter: expected NULL (default seed has none); got {on_enter_text:?}"
            );
        }
    }
    Ok(())
}

async fn create_statuses_migration_backfills_custom_project(pool: &PgPool) -> Result<()> {
    // The j-stsfixt baseline row's 3 statuses exercise the full column
    // shape: draft (minimal), reviewing (on_enter + prompt_path +
    // interactive), merged (unblocks_* flags set).
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter::text AS on_enter_text, prompt_path, interactive \
         FROM metis.statuses WHERE project_id = 'j-stsfixt' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read metis.statuses for j-stsfixt")?;
    if rows.len() != 3 {
        bail!(
            "metis.statuses(j-stsfixt): expected 3 rows; got {}",
            rows.len()
        );
    }

    struct Expected {
        sequence: i64,
        key: &'static str,
        label: &'static str,
        color: &'static str,
        unblocks_parents: bool,
        unblocks_dependents: bool,
        cascades_to_children: bool,
        on_enter: Option<serde_json::Value>,
        prompt_path: Option<&'static str>,
        interactive: bool,
    }

    let expectations: [Expected; 3] = [
        Expected {
            sequence: 1,
            key: "draft",
            label: "Draft",
            color: "#cccccc",
            unblocks_parents: false,
            unblocks_dependents: false,
            cascades_to_children: false,
            on_enter: None,
            prompt_path: None,
            interactive: false,
        },
        Expected {
            sequence: 2,
            key: "reviewing",
            label: "Reviewing",
            color: "#f1c40f",
            unblocks_parents: false,
            unblocks_dependents: false,
            cascades_to_children: false,
            on_enter: Some(serde_json::json!({"assign_to": {"Agent": {"name": "reviewer"}}})),
            prompt_path: Some("/projects/stsfixt/reviewing.md"),
            interactive: true,
        },
        Expected {
            sequence: 3,
            key: "merged",
            label: "Merged",
            color: "#2ecc71",
            unblocks_parents: true,
            unblocks_dependents: true,
            cascades_to_children: false,
            on_enter: None,
            prompt_path: None,
            interactive: false,
        },
    ];

    for (row, expected) in rows.iter().zip(expectations.iter()) {
        let sequence: i64 = row.try_get("sequence")?;
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: bool = row.try_get("unblocks_parents")?;
        let unblocks_dependents: bool = row.try_get("unblocks_dependents")?;
        let cascades_to_children: bool = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let on_enter_value = on_enter_text
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .context("decode j-stsfixt.statuses.on_enter JSON")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: bool = row.try_get("interactive")?;

        if sequence != expected.sequence
            || key != expected.key
            || label != expected.label
            || color != expected.color
            || unblocks_parents != expected.unblocks_parents
            || unblocks_dependents != expected.unblocks_dependents
            || cascades_to_children != expected.cascades_to_children
            || on_enter_value != expected.on_enter
            || prompt_path.as_deref() != expected.prompt_path
            || interactive != expected.interactive
        {
            bail!(
                "metis.statuses(j-stsfixt) sequence={sequence}: did not match expected\n  \
                 got: (key={key}, label={label}, color={color}, \
                 unblocks_parents={unblocks_parents}, unblocks_dependents={unblocks_dependents}, \
                 cascades_to_children={cascades_to_children}, on_enter={on_enter_value:?}, \
                 prompt_path={prompt_path:?}, interactive={interactive})\n  \
                 expected: sequence={}, key={}",
                expected.sequence,
                expected.key
            );
        }
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "issues_v2", "status_sequence").await? {
        bail!("expected metis.issues_v2.status_sequence column to exist after rollforward");
    }
    if !column_is_nullable(pool, "issues_v2", "status_sequence").await? {
        bail!(
            "expected metis.issues_v2.status_sequence to be nullable (FK + NOT NULL deferred to PR 3); \
             got NOT NULL"
        );
    }
    // Confirm the supporting index landed for the join in PR 2/3.
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes \
         WHERE schemaname = 'metis' AND indexname = 'issues_v2_project_status_sequence_idx')",
    )
    .fetch_one(pool)
    .await?;
    let exists: bool = row.get(0);
    if !exists {
        bail!("expected metis.issues_v2_project_status_sequence_idx to exist after rollforward");
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_backfills_issues(pool: &PgPool) -> Result<()> {
    // Per-status default-project coverage: each baseline issue must
    // round-trip `(project_id, status_sequence) → metis.statuses.key`.
    let cases: &[(&str, &str, &str, i64)] = &[
        ("i-stsopena", "j-defaul", "open", 1),
        ("i-stsiprog", "j-defaul", "in-progress", 2),
        ("i-stsclosd", "j-defaul", "closed", 3),
        ("i-stsdropd", "j-defaul", "dropped", 4),
        ("i-stsfaild", "j-defaul", "failed", 5),
        ("i-stsrevwg", "j-stsfixt", "reviewing", 2),
    ];
    for (issue_id, project_id, status_key, expected_sequence) in cases {
        let row = sqlx::query(
            "SELECT i.status_sequence, s.key AS resolved_key \
             FROM metis.issues_v2 i \
             LEFT JOIN metis.statuses s \
                ON s.project_id = i.project_id AND s.sequence = i.status_sequence \
             WHERE i.id = $1 AND i.is_latest = TRUE",
        )
        .bind(issue_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read status_sequence for {issue_id}"))?;
        let status_sequence: Option<i64> = row.try_get("status_sequence")?;
        let resolved_key: Option<String> = row.try_get("resolved_key")?;
        if status_sequence != Some(*expected_sequence) {
            bail!(
                "{issue_id} (project={project_id}, status={status_key}): \
                 expected status_sequence={expected_sequence}; got {status_sequence:?}"
            );
        }
        if resolved_key.as_deref() != Some(*status_key) {
            bail!(
                "{issue_id} (project={project_id}, status={status_key}): \
                 join to metis.statuses must recover key; got {resolved_key:?}"
            );
        }
    }
    // Note: no catch-all NULL check post-rollforward. The migration body's
    // `DO $$ ... RAISE EXCEPTION` already guarantees no pre-migration row
    // remained NULL (otherwise the migration would have aborted before this
    // test runs). `assert_store_level_smoke` inserts fresh rows through the
    // Store layer *after* migrations roll forward, and the Store does not
    // write `status_sequence` yet (PR 2 wires it up), so those rows
    // legitimately sit at NULL until PR 3 tightens the column.
    Ok(())
}

async fn create_statuses_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Re-applying the body must not produce duplicate `(project_id,
    // sequence)` rows or duplicate `(project_id, key)` rows. (A new
    // project inserted between runs — e.g. `j-rtrip` from
    // `assert_recent_migration_data_shape` — *will* be picked up on
    // re-apply, which is correct: that project's statuses weren't
    // backfilled at initial migration time. Check for duplicates, not
    // for a frozen row count.)
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/20260613000000_create_statuses.sql"),
    )
    .context("read postgres create_statuses migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres create_statuses migration body")?;

    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT (project_id, sequence)) AS dup FROM metis.statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, sequence) rows in metis.statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!("create_statuses re-apply produced {dup} duplicate (project_id, sequence) rows");
    }
    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT (project_id, key)) AS dup FROM metis.statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, key) rows in metis.statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!("create_statuses re-apply produced {dup} duplicate (project_id, key) rows");
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Snapshot every (id, version_number, status_sequence) row whose
    // status_sequence was already backfilled (non-NULL) so we can
    // confirm the re-application does not overwrite an existing
    // sequence. (Post-rollforward INSERTs through the smoke creates
    // can land issues with NULL status_sequence — the re-apply
    // legitimately backfills those; the snapshot subset isolates the
    // overwrite-detection.)
    let before_rows = sqlx::query(
        "SELECT id, version_number, status_sequence FROM metis.issues_v2 \
         WHERE status_sequence IS NOT NULL ORDER BY id, version_number",
    )
    .fetch_all(pool)
    .await
    .context("snapshot non-NULL metis.issues_v2.status_sequence before idempotency rerun")?;
    let before: Vec<(String, i64, i64)> = before_rows
        .iter()
        .map(|r| {
            (
                r.try_get::<String, _>("id").unwrap(),
                r.try_get::<i64, _>("version_number").unwrap(),
                r.try_get::<i64, _>("status_sequence").unwrap(),
            )
        })
        .collect();

    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260613010000_add_issues_v2_status_sequence.sql"),
    )
    .context("read postgres add_issues_v2_status_sequence migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres add_issues_v2_status_sequence migration body")?;

    // For the subset of rows that already had a non-NULL
    // status_sequence before, the re-apply must leave the value
    // untouched (the body's `WHERE status_sequence IS NULL` clause
    // skips them).
    let before_ids: Vec<String> = before.iter().map(|(id, _, _)| id.clone()).collect();
    let before_versions: Vec<i64> = before.iter().map(|(_, v, _)| *v).collect();
    let after_rows = sqlx::query(
        "SELECT id, version_number, status_sequence FROM metis.issues_v2 \
         WHERE (id, version_number) IN ( \
             SELECT * FROM UNNEST($1::text[], $2::bigint[]) AS t(id, version_number) \
         ) ORDER BY id, version_number",
    )
    .bind(&before_ids)
    .bind(&before_versions)
    .fetch_all(pool)
    .await
    .context("re-snapshot the same subset after idempotency rerun")?;
    let after: Vec<(String, i64, Option<i64>)> = after_rows
        .iter()
        .map(|r| {
            (
                r.try_get::<String, _>("id").unwrap(),
                r.try_get::<i64, _>("version_number").unwrap(),
                r.try_get::<Option<i64>, _>("status_sequence").unwrap(),
            )
        })
        .collect();
    for ((bid, bv, bs), (aid, av, ass)) in before.iter().zip(after.iter()) {
        if bid != aid || bv != av || Some(*bs) != *ass {
            bail!(
                "add_issues_v2_status_sequence re-apply overwrote previously-backfilled rows: \
                 ({bid}, {bv}, {bs}) → ({aid}, {av}, {ass:?})"
            );
        }
    }

    // Pre-flight guard: no metis.issues_v2 row may remain with NULL
    // status_sequence after the re-apply (the body's RAISE EXCEPTION
    // would have aborted otherwise, but verify the post-state directly).
    let row = sqlx::query("SELECT COUNT(*) FROM metis.issues_v2 WHERE status_sequence IS NULL")
        .fetch_one(pool)
        .await
        .context("count NULL status_sequence rows after idempotency rerun")?;
    let null_count: i64 = row.try_get(0)?;
    if null_count != 0 {
        bail!("add_issues_v2_status_sequence re-apply left {null_count} NULL status_sequence rows");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// parse_baseline_filename unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::parse_baseline_filename;

    #[test]
    fn parses_well_formed_filename() {
        assert_eq!(
            parse_baseline_filename("20260519000000__pre_actor_overhaul.sql").unwrap(),
            20_260_519_000_000
        );
    }

    #[test]
    fn parses_minimal_description() {
        assert_eq!(parse_baseline_filename("42__x.sql").unwrap(), 42);
    }

    #[test]
    fn rejects_missing_sql_suffix() {
        let err = parse_baseline_filename("20260519000000__pre_actor_overhaul")
            .unwrap_err()
            .to_string();
        assert!(err.contains(".sql"), "expected `.sql` mention; got: {err}");
    }

    #[test]
    fn rejects_missing_double_underscore() {
        let err = parse_baseline_filename("20260519000000_pre_actor_overhaul.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("<version>__<description>"),
            "expected the filename-shape mention; got: {err}"
        );
    }

    #[test]
    fn rejects_empty_description() {
        let err = parse_baseline_filename("20260519000000__.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("empty description"),
            "expected empty-description mention; got: {err}"
        );
    }

    #[test]
    fn rejects_non_numeric_version() {
        let err = parse_baseline_filename("not_a_number__desc.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a u64"),
            "expected u64 parse failure; got: {err}"
        );
    }
}
