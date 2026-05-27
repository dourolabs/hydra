//! Migration roundtrip integration test (PR-1 + PR-2 of the migration test
//! harness; see `/designs/pre-prod-deploy-test-plan.md`).
//!
//! Applies sqlx migrations to a baseline-shaped Postgres database, executes
//! the hand-curated `migration_baseline.sql` fixture, rolls the remaining
//! migrations forward, runs the external `migrate-events` pass through the
//! same library entry point the server's startup migration uses, and
//! asserts:
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
//! `hydra-server`'s postgres-specific code (`migration_tool/mod.rs:17`,
//! `ee/store/postgres_v2.rs`, etc.). PR-5 will give the test a dedicated CI
//! workflow.

#![cfg(feature = "postgres")]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::Principal;
use hydra_common::{HydraId, IssueId, PatchId, RepoName, SessionId};
use hydra_server::domain::actors::ActorRef;
use hydra_server::domain::issues::{Issue, IssueStatus, IssueType};
use hydra_server::domain::patches::{Patch, PatchStatus, Review};
use hydra_server::domain::sessions::{AgentConfig, Session, SessionEvent, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::Username;
use hydra_server::store::postgres_v2::{MIGRATOR, PostgresStoreV2};
use hydra_server::store::{ReadOnlyStore, RelationshipType, Store};
use sqlx::migrate::Migrate;
use sqlx::{PgPool, Row};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

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
// §3.3 store / domain-level smoke
//
// The §3.2 assertions above verify SQL-level shapes of the migrated rows. PR-2
// of `/designs/pre-prod-deploy-test-plan.md` adds this third layer: read the
// migrated rows back through the live `Store` trait and assert the typed
// domain objects (Principal, SessionMode, Review, SessionEvent, refers-to
// relationship) deserialize cleanly, then exercise a create→read-back cycle on
// the same APIs so any post-migration write path that diverged from the read
// path fails loud here instead of at first prod traffic.
//
// The store-level rows used here live in the fixture's "§3.3 store-level
// smoke" block — they mirror the earlier `i-bare000001` / `p-bareauth01`
// rows but use all-alphabetic id suffixes so the Rust `IssueId` / `PatchId` /
// `SessionId` newtypes (which reject digits) can parse them.
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
    // Headless task: mode backfill -> SessionMode::Headless.
    let session = store
        .get_session(&parse_session_id("s-headalpha")?, false)
        .await
        .context("Store::get_session(s-headalpha)")?;
    match &session.item.mode {
        SessionMode::Headless { prompt } if prompt == "do a thing" => {}
        other => bail!("s-headalpha: expected Headless('do a thing'); got {other:?}"),
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
        AgentConfig::default(),
        Default::default(),
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless {
            prompt: "smoke: do a thing".to_string(),
        },
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
    match &fetched.item.mode {
        SessionMode::Headless { prompt } if prompt == "smoke: do a thing" => Ok(()),
        other => bail!(
            "post-migration create_session did not round-trip SessionMode::Headless; got {other:?}"
        ),
    }
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
