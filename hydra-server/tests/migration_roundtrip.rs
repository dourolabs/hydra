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
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::{ExternalSystem, Principal};
use hydra_common::{DocumentId, HydraId, IssueId, PatchId, RepoName, SessionId};
use hydra_server::domain::actors::ActorRef;
use hydra_server::domain::issues::{Issue, IssueStatus, IssueType};
use hydra_server::domain::patches::{Patch, PatchStatus, Review};
use hydra_server::domain::sessions::{Session, SessionEvent, SessionMode};
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

    // actors_v2 — bare `actor_id` column rewrites. `actor_id` is NOT
    // NULL in this table since `20260205000000_add_v2_tables.sql`, so
    // every shape must end up with a non-null value after the cleanup.
    // The last two rows specifically exercise previously-NULLable
    // paths that would have violated the constraint pre-fix.
    for (id, expected) in [
        ("actu-aname", serde_json::json!({"User": {"name": "alice"}})),
        ("actu-asvc", serde_json::json!({"Agent": {"name": "swe"}})),
        ("actu-aiss", external_legacy("i-actisstwo")),
        ("actu-asvcbad", external_legacy("has space")),
    ] {
        let row = sqlx::query(
            "SELECT actor_id::text AS pl FROM metis.actors_v2 \
             WHERE id = $1 AND is_latest = TRUE",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read actors_v2.actor_id for {id}"))?;
        let raw: Option<String> = row.get("pl");
        let got: Option<serde_json::Value> = raw
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .with_context(|| format!("decode actor_id for {id}"))?;
        if got.as_ref() != Some(&expected) {
            bail!("actors_v2({id}).actor_id: expected {expected}; got {got:?}");
        }
    }

    // conversation_events_v2 — Session inside Authenticated is rewritten to Adhoc.
    let row = sqlx::query(
        "SELECT actor::text AS pl FROM metis.conversation_events_v2 \
         WHERE conversation_id = 'c-actclean' AND version_number = 1",
    )
    .fetch_one(pool)
    .await
    .context("read conversation_events_v2.actor for c-actclean")?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode conversation_events_v2.actor")?;
    let expected = serde_json::json!({
        "Authenticated": {
            "actor_id": {"Adhoc": {"session_id": "s-cesessx"}}
        }
    });
    if got != Some(expected.clone()) {
        bail!("conversation_events_v2.actor: expected {expected}; got {got:?}");
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
    if session.item.system_prompt.as_deref() != Some("do a thing") {
        bail!(
            "s-headalpha: expected system_prompt='do a thing'; got {:?}",
            session.item.system_prompt
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
        IssueStatus::Open,
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
        None,
        None,
        Some("smoke: do a thing".to_string()),
        None,
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
    if fetched.item.system_prompt.as_deref() != Some("smoke: do a thing") {
        bail!(
            "post-migration create_session did not round-trip system_prompt; \
             got {:?}",
            fetched.item.system_prompt
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

fn parse_issue_id(s: &str) -> Result<IssueId> {
    IssueId::from_str(s).with_context(|| format!("parse issue id '{s}'"))
}

fn parse_patch_id(s: &str) -> Result<PatchId> {
    PatchId::from_str(s).with_context(|| format!("parse patch id '{s}'"))
}

fn parse_session_id(s: &str) -> Result<SessionId> {
    SessionId::from_str(s).with_context(|| format!("parse session id '{s}'"))
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
