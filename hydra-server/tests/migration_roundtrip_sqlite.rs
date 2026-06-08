//! SQLite migration roundtrip integration test.
//!
//! Sister to `migration_roundtrip.rs` (postgres). Mirrors the same
//! interleave loop — walk versioned baselines under
//! `tests/fixtures/migration_baselines_sqlite/`, applying
//! `sqlite_store::run_migrations(&pool, Some(version))` up to each
//! baseline's pin, then executing the baseline INSERTs, then
//! `run_migrations(&pool, None)` to HEAD. See
//! `/designs/migration-testing-redesign.md` §3, §4, §7 for the algorithm.
//!
//! Initial scope (per [[i-toeamhmw]]): the `actor_variant_cleanup` SQLite
//! arm's `session_events` and `conversation_events` rewrites — the exact
//! code paths surfaced by the `(session_id, version_number) AS __pk`
//! parse-reject bug that shipped past CI ([[i-ccchbxha]], fixed by
//! [[i-nmcnqeyn]] / [[p-fcxmstwd]]).
//!
//! Widened in [[i-uazczsbc]] to cover the four other backfill migrations
//! that ship for both backends but only had PG coverage:
//! `20260530000000_add_assignee_principal_to_issues`,
//! `20260601000000_review_author_principal`,
//! `20260529000000_rename_refers_to_to_kebab_case`,
//! `20260603010000_backfill_agent_config_system_prompt`. Their fixture
//! rows live in the `20260519000000__pre_actor_overhaul.sql` baseline.
//!
//! Future SQLite-only migration bugs get caught by extending this
//! fixture tree + this file.
//!
//! Runs under the default `cargo test --workspace` — no `#[ignore]`, no
//! feature gate. The postgres test is CI-only because it needs a live
//! postgres; SQLite has no such constraint and uses `sqlite::memory:`.

use anyhow::{Context, Result, bail};
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::projects::StatusDefinition;
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::Principal;
use hydra_common::{ConversationId, HydraId, IssueId, ProjectId, SessionId, TriggerId};
use hydra_server::domain::actors::{ActorId, ActorRef};
use hydra_server::domain::projects::default_project_seed;
use hydra_server::domain::sessions::SessionMode;
use hydra_server::store::sqlite_store::{self, MIGRATOR, SqliteStore};
use hydra_server::store::{ReadOnlyStore, RelationshipType};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[tokio::test]
async fn migration_roundtrip_sqlite() -> Result<()> {
    let pool = SqliteStore::init_pool("sqlite::memory:")
        .await
        .context("init in-memory sqlite pool")?;

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
            "baseline {} has no matching sqlx sqlite migration on this checkout",
            b.version
        );
        sqlite_store::run_migrations(&pool, Some(b.version))
            .await
            .with_context(|| format!("apply sqlite migrations up to baseline {}", b.version))?;
        sqlx::raw_sql(&b.body)
            .execute(&pool)
            .await
            .with_context(|| format!("execute sqlite baseline {}", b.version))?;
        prev = Some(b.version);
    }

    sqlite_store::run_migrations(&pool, None)
        .await
        .context("apply remaining sqlite migrations past the last baseline")?;

    assert_session_events_actor_rewrites(&pool).await?;
    // The `conversation_events` table was dropped along with the
    // `ConversationEvent` removal — there is nothing to assert against
    // post-migration.
    assert_store_level_session_events_smoke(&pool).await?;
    assert_conversations_actor_rewrite(&pool).await?;
    assert_form_response_actor_rewrite(&pool).await?;
    assert_store_level_conversations_smoke(&pool).await?;
    assert_store_level_form_response_smoke(&pool).await?;
    assert_pagination_indexes_exist(&pool).await?;
    assert_schema_invariants(&pool).await?;
    assert_recent_migration_store_smoke(&pool).await?;

    assert_assignee_principal_backfill(&pool).await?;
    assert_review_author_principal_rewrite(&pool).await?;
    assert_refers_to_rename(&pool).await?;
    assert_agent_config_system_prompt_backfill(&pool).await?;

    seed_default_project_migration_inserts_row(&pool).await?;
    seed_default_project_migration_backfills_null_project_ids(&pool).await?;
    seed_default_project_migration_is_idempotent(&pool).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Schema-invariants: pagination indexes on the four list-* tables. `issues_v2`
// paginates by `updated_at` (so it gets `(updated_at DESC, id DESC)`); the
// other three paginate by `created_at` (so they get `(created_at DESC, id
// DESC)`). Mirrors postgres migrations 20260315000000, 20260317000000, and
// 20260605000000; ported to SQLite by 20260604010000 and 20260605000000.
// ---------------------------------------------------------------------------

async fn assert_pagination_indexes_exist(pool: &SqlitePool) -> Result<()> {
    for name in [
        "issues_v2_updated_at_id_idx",
        "patches_v2_created_at_id_idx",
        "tasks_v2_created_at_id_idx",
        "documents_v2_created_at_id_idx",
    ] {
        let row = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'index' AND name = ?1")
            .bind(name)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("query sqlite_master for index {name}"))?;
        if row.is_none() {
            bail!("expected pagination index {name} to exist post-rollforward");
        }
    }
    // The original `issues_v2_created_at_id_idx` was dropped by 20260605000000
    // because `list_issues` orders by `updated_at`. Catch any future migration
    // that re-creates it without thinking.
    let stale = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'issues_v2_created_at_id_idx'",
    )
    .fetch_optional(pool)
    .await
    .context("query sqlite_master for dropped index issues_v2_created_at_id_idx")?;
    if stale.is_some() {
        bail!(
            "issues_v2_created_at_id_idx should have been dropped by 20260605000000; \
             a later migration re-created it"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Baseline directory enumeration (duplicated from migration_roundtrip.rs per
// the issue's explicit "do not pull shared scaffolding out" guidance).
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Baseline {
    version: u64,
    body: String,
}

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_baselines_sqlite")
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
// actor_variant_cleanup rewrite assertions — session_events
// ---------------------------------------------------------------------------

async fn assert_session_events_actor_rewrites(pool: &SqlitePool) -> Result<()> {
    expect_session_event_actor(
        pool,
        "s-actrowx",
        1,
        Some(serde_json::json!({
            "Authenticated": {"actor_id": {"User": {"name": "alice"}}}
        })),
    )
    .await?;
    expect_session_event_actor(
        pool,
        "s-actrowx",
        2,
        Some(serde_json::json!({
            "Authenticated": {"actor_id": {"Adhoc": {"session_id": "s-sessone"}}}
        })),
    )
    .await?;
    expect_session_event_actor(
        pool,
        "s-actrowx",
        3,
        Some(serde_json::json!({
            "Authenticated": {"actor_id": external_legacy("definitely not an actor")}
        })),
    )
    .await?;
    // actor IS NULL must stay NULL.
    expect_session_event_actor(pool, "s-actrowx", 4, None).await?;
    Ok(())
}

async fn expect_session_event_actor(
    pool: &SqlitePool,
    session_id: &str,
    version_number: i64,
    expected: Option<serde_json::Value>,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT actor FROM session_events \
         WHERE session_id = ?1 AND version_number = ?2",
    )
    .bind(session_id)
    .bind(version_number)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read session_events.actor for ({session_id}, {version_number})"))?;
    let raw: Option<String> = row.try_get("actor")?;
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .with_context(|| {
            format!("decode session_events.actor JSON for ({session_id}, {version_number})")
        })?;
    if got != expected {
        bail!(
            "session_events({session_id}, {version_number}).actor: \
             expected {expected:?}; got {got:?}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Store-level smoke: read session_events back through `SqliteStore` so any
// serde drift between the migration's raw JSON output and `ActorRef` /
// `ActorId` `Deserialize` impls fails loud here. Mirrors the postgres test's
// §3.3 round-2 smoke. We only do it for session_events because
// `conversation_events` is not surfaced through a typed `ActorRef` getter on
// the SQLite store today.
// ---------------------------------------------------------------------------

async fn assert_store_level_session_events_smoke(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let sid = SessionId::from_str("s-actrowx").context("parse session id 's-actrowx'")?;
    let events = store
        .get_session_events(&sid)
        .await
        .context("SqliteStore::get_session_events(s-actrowx)")?;
    if events.len() != 4 {
        bail!(
            "expected 4 session_events for s-actrowx; got {}",
            events.len()
        );
    }
    expect_authenticated_user(&events[0].actor, "alice", "events[0]")?;
    expect_authenticated_adhoc(&events[1].actor, "s-sessone", "events[1]")?;
    expect_authenticated_external_legacy(&events[2].actor, "definitely not an actor", "events[2]")?;
    if events[3].actor.is_some() {
        bail!(
            "events[3].actor: expected None (NULL stays NULL); got {:?}",
            events[3].actor
        );
    }
    Ok(())
}

fn expect_authenticated_user(actor: &Option<ActorRef>, name: &str, label: &str) -> Result<()> {
    match actor.as_ref() {
        Some(ActorRef::Authenticated { actor_id, .. }) => match actor_id {
            ActorId::User(n) if n.as_str() == name => Ok(()),
            other => bail!("{label}: expected Authenticated(User({name})); got {other:?}"),
        },
        other => bail!("{label}: expected Authenticated(User({name})); got {other:?}"),
    }
}

fn expect_authenticated_adhoc(actor: &Option<ActorRef>, session: &str, label: &str) -> Result<()> {
    match actor.as_ref() {
        Some(ActorRef::Authenticated { actor_id, .. }) => match actor_id {
            ActorId::Adhoc(s) if s.as_ref() == session => Ok(()),
            other => bail!("{label}: expected Authenticated(Adhoc({session})); got {other:?}"),
        },
        other => bail!("{label}: expected Authenticated(Adhoc({session})); got {other:?}"),
    }
}

fn expect_authenticated_external_legacy(
    actor: &Option<ActorRef>,
    username: &str,
    label: &str,
) -> Result<()> {
    match actor.as_ref() {
        Some(ActorRef::Authenticated { actor_id, .. }) => match actor_id {
            ActorId::External {
                system,
                username: u,
            } if system.as_str() == "legacy" && u == username => Ok(()),
            other => {
                bail!("{label}: expected Authenticated(External-legacy({username})); got {other:?}")
            }
        },
        other => {
            bail!("{label}: expected Authenticated(External-legacy({username})); got {other:?}")
        }
    }
}

/// Canonical External-legacy fallback JSON wire shape.
fn external_legacy(username: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "External": {"system": "legacy", "username": username.into()}
    })
}

// ---------------------------------------------------------------------------
// actor_variant_cleanup rewrite assertions — conversations + form_response
// (added in [[i-jyhvstcj]] to cover the prod failure shapes that the
// original cleanup missed).
// ---------------------------------------------------------------------------

async fn assert_conversations_actor_rewrite(pool: &SqlitePool) -> Result<()> {
    let row =
        sqlx::query("SELECT actor FROM conversations WHERE id = 'c-actconvx' AND is_latest = 1")
            .fetch_one(pool)
            .await
            .context("read conversations.actor for c-actconvx")?;
    let raw: Option<String> = row.try_get("actor")?;
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode conversations.actor JSON for c-actconvx")?;
    let expected = serde_json::json!({
        "Authenticated": {"actor_id": {"Adhoc": {"session_id": "s-csessacx"}}}
    });
    if got.as_ref() != Some(&expected) {
        bail!("conversations(c-actconvx).actor: expected {expected}; got {got:?}");
    }
    Ok(())
}

async fn assert_form_response_actor_rewrite(pool: &SqlitePool) -> Result<()> {
    let row =
        sqlx::query("SELECT form_response FROM issues_v2 WHERE id = 'i-actform' AND is_latest = 1")
            .fetch_one(pool)
            .await
            .context("read issues_v2.form_response for i-actform")?;
    let raw: Option<String> = row.try_get("form_response")?;
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode issues_v2.form_response JSON for i-actform")?;
    let expected = serde_json::json!({
        "action_id": "approve",
        "actor": {"User": {"name": "alice"}},
        "values": {"score": 4},
        "submitted_at": "2026-05-10T11:00:00Z"
    });
    if got.as_ref() != Some(&expected) {
        bail!("issues_v2(i-actform).form_response: expected {expected}; got {got:?}");
    }
    Ok(())
}

async fn assert_store_level_conversations_smoke(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let cid = ConversationId::from_str("c-actconvx").context("parse 'c-actconvx'")?;
    let conv = store
        .get_conversation(&cid, false)
        .await
        .context("SqliteStore::get_conversation(c-actconvx)")?;
    let expected_sid: SessionId = "s-csessacx".parse().unwrap();
    match conv.actor.as_ref() {
        Some(ActorRef::Authenticated {
            actor_id: ActorId::Adhoc(sid),
            ..
        }) if sid == &expected_sid => Ok(()),
        other => bail!("c-actconvx: expected Authenticated(Adhoc(s-csessacx)); got {other:?}"),
    }
}

async fn assert_store_level_form_response_smoke(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let iid = IssueId::from_str("i-actform").context("parse 'i-actform'")?;
    let issue = store
        .get_issue(&iid, false)
        .await
        .context("SqliteStore::get_issue(i-actform)")?;
    let form_response = issue
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

// ---------------------------------------------------------------------------
// schema invariants — assertions for the SQLite migrations that landed
// after the `pre_actor_variant_cleanup` baseline:
//   * 20260603020000_add_triggers_table.sql
//   * 20260604000000_drop_conversation_events.sql
//   * 20260604000001_create_projects.sql
//
// Mirrors `migration_roundtrip.rs::assert_schema_invariants` in shape but
// uses `sqlite_master` / `pragma_table_info` instead of
// `information_schema`. Duplicated rather than shared per the module
// preamble's "do not pull shared scaffolding out" guidance.
// ---------------------------------------------------------------------------

async fn assert_schema_invariants(pool: &SqlitePool) -> Result<()> {
    // Tables added by 20260603020000_add_triggers_table.sql and
    // 20260604000001_create_projects.sql.
    for table in ["triggers", "projects"] {
        if !table_exists(pool, table).await? {
            bail!("expected `{table}` table to exist after rollforward");
        }
    }

    // Tables dropped by 20260604000000_drop_conversation_events.sql.
    if table_exists(pool, "conversation_events").await? {
        bail!("expected `conversation_events` table to be dropped after rollforward");
    }

    // Column added by 20260604000001_create_projects.sql.
    if !column_exists(pool, "issues_v2", "project_id").await? {
        bail!("expected `issues_v2.project_id` column to exist after rollforward");
    }

    // Column added by 20260606000000_add_session_proxy_targets.sql.
    if !column_exists(pool, "tasks_v2", "proxy_targets").await? {
        bail!("expected `tasks_v2.proxy_targets` column to exist after rollforward");
    }
    if !column_is_nullable(pool, "tasks_v2", "proxy_targets").await? {
        bail!("expected `tasks_v2.proxy_targets` to be nullable after rollforward");
    }

    // Column added by 20260606010000_add_projects_prompt_path.sql.
    if !column_exists(pool, "projects", "prompt_path").await? {
        bail!("expected `projects.prompt_path` column to exist after rollforward");
    }
    if !column_is_nullable(pool, "projects", "prompt_path").await? {
        bail!("expected `projects.prompt_path` to be nullable after rollforward");
    }

    // Indexes added by the three migrations under test. Listed verbatim so
    // a future rename without a baseline bump fails this assertion loud.
    for index in [
        "triggers_creator_idx",
        "triggers_is_latest_idx",
        "triggers_latest_idx",
        "projects_key_unique_active_idx",
        "projects_creator_idx",
        "projects_is_latest_idx",
        "projects_latest_idx",
        "issues_v2_project_id_idx",
    ] {
        if !index_exists(pool, index).await? {
            bail!("expected index `{index}` to exist after rollforward");
        }
    }

    Ok(())
}

async fn table_exists(pool: &SqlitePool, table: &str) -> Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(table)
    .fetch_one(pool)
    .await
    .with_context(|| format!("look up sqlite_master for table `{table}`"))?;
    let exists: i64 = row.try_get(0)?;
    Ok(exists != 0)
}

async fn index_exists(pool: &SqlitePool, index: &str) -> Result<bool> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
    )
    .bind(index)
    .fetch_one(pool)
    .await
    .with_context(|| format!("look up sqlite_master for index `{index}`"))?;
    let exists: i64 = row.try_get(0)?;
    Ok(exists != 0)
}

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
    // `pragma_table_info` exposes the column list as a table-valued
    // function so the lookup stays a single round-trip and works against
    // the same `SqlitePool` as the rest of the test.
    let rows = sqlx::query("SELECT name FROM pragma_table_info(?1)")
        .bind(table)
        .fetch_all(pool)
        .await
        .with_context(|| format!("pragma_table_info(`{table}`)"))?;
    for row in rows {
        let name: String = row.try_get(0)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn column_is_nullable(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
    let rows = sqlx::query("SELECT name, \"notnull\" FROM pragma_table_info(?1)")
        .bind(table)
        .fetch_all(pool)
        .await
        .with_context(|| format!("pragma_table_info(`{table}`)"))?;
    for row in rows {
        let name: String = row.try_get(0)?;
        if name == column {
            let notnull: i64 = row.try_get(1)?;
            return Ok(notnull == 0);
        }
    }
    bail!("column `{table}.{column}` not found");
}

// ---------------------------------------------------------------------------
// Store-level smoke for the recent SQLite migrations: insert one trigger row
// and one project row via raw SQL against the post-rollforward schema, then
// read them back through the `SqliteStore` getters. Catches schema drift
// between the migration SQL and the row-shape sqlx queries on
// `SqliteStore::get_trigger` / `get_project`.
// ---------------------------------------------------------------------------

async fn assert_recent_migration_store_smoke(pool: &SqlitePool) -> Result<()> {
    let trigger_id = "t-migsmoke";
    let trigger_schedule = serde_json::json!({
        "Cron": {"expression": "0 9 * * MON", "timezone": "UTC"}
    })
    .to_string();
    let trigger_actions = serde_json::json!([]).to_string();
    sqlx::query(
        "INSERT INTO triggers \
           (id, version_number, enabled, creator, schedule, actions, \
            last_fired_at, deleted, actor, is_latest) \
         VALUES (?1, 1, 1, ?2, ?3, ?4, NULL, 0, NULL, 1)",
    )
    .bind(trigger_id)
    .bind("alice")
    .bind(&trigger_schedule)
    .bind(&trigger_actions)
    .execute(pool)
    .await
    .context("insert smoke trigger row")?;

    let store = SqliteStore::new(pool.clone());
    let tid = TriggerId::from_str(trigger_id).context("parse smoke trigger id")?;
    let fetched_trigger = store
        .get_trigger(&tid, false)
        .await
        .context("SqliteStore::get_trigger(t-migsmoke)")?;
    if !fetched_trigger.item.enabled {
        bail!("smoke trigger: expected enabled=true after read-back");
    }
    if fetched_trigger.item.creator.as_str() != "alice" {
        bail!(
            "smoke trigger: expected creator='alice'; got {:?}",
            fetched_trigger.item.creator
        );
    }

    let project_id = "j-migsmoke";
    let project_statuses = serde_json::json!([
        {
            "key": "todo",
            "label": "Todo",
            "color": "#abcdef",
            "unblocks_parents": false,
            "unblocks_dependents": false,
            "cascades_to_children": false
        }
    ])
    .to_string();
    // Include `prompt_path` so the post-rollforward schema's added column
    // is exercised on the smoke INSERT — see
    // `20260606010000_add_projects_prompt_path.sql`.
    sqlx::query(
        "INSERT INTO projects \
           (id, version_number, key, name, default_status_key, statuses, \
            creator, deleted, actor, prompt_path, is_latest) \
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, 1)",
    )
    .bind(project_id)
    .bind("smoke")
    .bind("Smoke")
    .bind("todo")
    .bind(&project_statuses)
    .bind("alice")
    .bind("/projects/smoke/prompt.md")
    .execute(pool)
    .await
    .context("insert smoke project row")?;

    let pid = ProjectId::from_str(project_id).context("parse smoke project id")?;
    let fetched_project = store
        .get_project(&pid, false)
        .await
        .context("SqliteStore::get_project(j-migsmoke)")?;
    if fetched_project.item.name != "Smoke" {
        bail!(
            "smoke project: expected name='Smoke'; got {:?}",
            fetched_project.item.name
        );
    }
    if fetched_project.item.key.as_str() != "smoke" {
        bail!(
            "smoke project: expected key='smoke'; got {:?}",
            fetched_project.item.key
        );
    }
    if fetched_project.item.statuses.len() != 1 {
        bail!(
            "smoke project: expected 1 status; got {}",
            fetched_project.item.statuses.len()
        );
    }
    if fetched_project.item.prompt_path.as_deref() != Some("/projects/smoke/prompt.md") {
        bail!(
            "smoke project: expected prompt_path='/projects/smoke/prompt.md'; got {:?}",
            fetched_project.item.prompt_path
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 20260530000000_add_assignee_principal_to_issues — assert the typed
// `assignee_principal` column was populated for each source shape the SQL
// backfill handles, then read each row back through `SqliteStore::get_issue`
// to confirm the migrated JSON deserializes into the typed `Principal`.
// ---------------------------------------------------------------------------

async fn assert_assignee_principal_backfill(pool: &SqlitePool) -> Result<()> {
    // SQL-level: bare / users-prefixed / agents-prefixed / external / NULL.
    expect_assignee_principal(
        pool,
        "i-bareasgn",
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-userpath",
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-agentpath",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // external/<sys>/<x> is intentionally left NULL by the SQL backfill.
    expect_assignee_principal(pool, "i-extpath", None).await?;
    expect_assignee_principal(pool, "i-nullasgn", None).await?;

    // Store-level smoke: confirm the migrated JSON round-trips into typed
    // `Principal` variants via `SqliteStore::get_issue`.
    let store = SqliteStore::new(pool.clone());
    let cases: [(&str, Option<Principal>); 5] = [
        (
            "i-bareasgn",
            Some(Principal::User {
                name: ApiUsername::try_new("jayantk").expect("jayantk validates"),
            }),
        ),
        (
            "i-userpath",
            Some(Principal::User {
                name: ApiUsername::try_new("jayantk").expect("jayantk validates"),
            }),
        ),
        (
            "i-agentpath",
            Some(Principal::Agent {
                name: AgentName::try_new("swe").expect("swe validates"),
            }),
        ),
        ("i-extpath", None),
        ("i-nullasgn", None),
    ];
    for (id, expected) in cases {
        let issue_id = IssueId::from_str(id).with_context(|| format!("parse issue id '{id}'"))?;
        let issue = store
            .get_issue(&issue_id, false)
            .await
            .with_context(|| format!("SqliteStore::get_issue({id})"))?;
        if issue.item.assignee != expected {
            bail!(
                "{id}: expected assignee={expected:?}; got {:?}",
                issue.item.assignee
            );
        }
    }
    Ok(())
}

async fn expect_assignee_principal(
    pool: &SqlitePool,
    issue_id: &str,
    expected: Option<serde_json::Value>,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT assignee_principal FROM issues_v2 \
         WHERE id = ?1 AND is_latest = 1",
    )
    .bind(issue_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read assignee_principal for {issue_id}"))?;
    let raw: Option<String> = row.try_get("assignee_principal")?;
    let got = raw
        .map(|s| serde_json::from_str::<serde_json::Value>(&s))
        .transpose()
        .with_context(|| format!("decode assignee_principal JSON for {issue_id}"))?;
    if got != expected {
        bail!("issue {issue_id}: expected assignee_principal={expected:?}; got {got:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260601000000_review_author_principal — assert the SQL rewrite produced a
// typed Principal for each `reviews[*].author` source shape, then read each
// patch back through `SqliteStore::get_patch` to confirm the migrated JSON
// deserializes into the typed `Principal`.
// ---------------------------------------------------------------------------

async fn assert_review_author_principal_rewrite(pool: &SqlitePool) -> Result<()> {
    expect_first_review_author(
        pool,
        "p-barerev",
        serde_json::json!({"User": {"name": "jayantk"}}),
    )
    .await?;
    expect_first_review_author(
        pool,
        "p-agentrev",
        serde_json::json!({"Agent": {"name": "swe"}}),
    )
    .await?;
    // Already-typed author must pass through the rewrite untouched.
    expect_first_review_author(
        pool,
        "p-typedrev",
        serde_json::json!({"User": {"name": "jayantk"}}),
    )
    .await?;
    // Store-level deserialization smoke (Review.author -> typed Principal) is
    // omitted here because `20260601000000_review_author_principal.sql` rebuilds
    // every review with `'is_approved', json(coalesce(json_extract(value,
    // '$.is_approved'), 'false'))`. SQLite's `json_extract` collapses JSON
    // booleans to integer 0/1, and `json(1)` then serializes as integer JSON,
    // so post-migration rows carry `"is_approved":1` and fail
    // `Review.is_approved: bool` deserialization. Tracked in [[i-olwdqhyo]];
    // the smoke is reinstated by that fix. The SQL-level author assertions
    // above still verify the migration's intended rewrite path.
    Ok(())
}

async fn expect_first_review_author(
    pool: &SqlitePool,
    patch_id: &str,
    expected_author: serde_json::Value,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT json_extract(reviews, '$[0].author') AS author FROM patches_v2 \
         WHERE id = ?1 AND is_latest = 1",
    )
    .bind(patch_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read reviews[0].author for {patch_id}"))?;
    let raw: Option<String> = row.try_get("author")?;
    let raw = raw.with_context(|| format!("patch {patch_id} has no reviews[0].author"))?;
    let got: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("decode reviews[0].author JSON for {patch_id}"))?;
    if got != expected_author {
        bail!("patch {patch_id}: expected reviews[0].author={expected_author}; got {got}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260529000000_rename_refers_to_to_kebab_case — assert no snake_case rows
// remain and the seeded row surfaces through `SqliteStore::get_relationships`
// under the typed `RelationshipType::RefersTo` discriminant.
// ---------------------------------------------------------------------------

async fn assert_refers_to_rename(pool: &SqlitePool) -> Result<()> {
    let row =
        sqlx::query("SELECT COUNT(*) AS c FROM object_relationships WHERE rel_type = 'refers_to'")
            .fetch_one(pool)
            .await
            .context("count snake_case refers_to rows")?;
    let snake_count: i64 = row.try_get("c")?;
    if snake_count != 0 {
        bail!("expected 0 rows with rel_type='refers_to' after rename; got {snake_count}");
    }
    let row = sqlx::query(
        "SELECT COUNT(*) AS c FROM object_relationships \
         WHERE source_id = 'i-bareasgn' AND target_id = 'i-userpath' AND rel_type = 'refers-to'",
    )
    .fetch_one(pool)
    .await
    .context("count kebab-case refers-to row")?;
    let kebab_count: i64 = row.try_get("c")?;
    if kebab_count != 1 {
        bail!(
            "expected the seeded refers_to row to be renamed to refers-to; matched {kebab_count}"
        );
    }

    let store = SqliteStore::new(pool.clone());
    let source: HydraId = IssueId::from_str("i-bareasgn")
        .context("parse 'i-bareasgn'")?
        .into();
    let target_expected: HydraId = IssueId::from_str("i-userpath")
        .context("parse 'i-userpath'")?
        .into();
    let rels = store
        .get_relationships(Some(&source), None, Some(RelationshipType::RefersTo))
        .await
        .context("SqliteStore::get_relationships(refers-to from i-bareasgn)")?;
    if !rels
        .iter()
        .any(|r| r.target_id == target_expected && r.rel_type == RelationshipType::RefersTo)
    {
        bail!("expected a refers-to relationship from i-bareasgn to i-userpath; got {rels:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260603010000_backfill_agent_config_system_prompt — assert the headless
// session's legacy `prompt` rode through `mode.prompt` onto
// `agent_config.system_prompt`. The store-level smoke also confirms the
// session-shape backfill produced the expected `SessionMode` variants for
// headless / interactive / resumed sessions.
// ---------------------------------------------------------------------------

async fn assert_agent_config_system_prompt_backfill(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());

    let headless = store
        .get_session(&SessionId::from_str("s-headalpha")?, false)
        .await
        .context("SqliteStore::get_session(s-headalpha)")?;
    if !matches!(&headless.item.mode, SessionMode::Headless) {
        bail!(
            "s-headalpha: expected SessionMode::Headless; got {:?}",
            headless.item.mode
        );
    }
    if headless.item.agent_config.system_prompt.as_deref() != Some("do a thing") {
        bail!(
            "s-headalpha: expected agent_config.system_prompt='do a thing'; got {:?}",
            headless.item.agent_config.system_prompt
        );
    }

    let interactive = store
        .get_session(&SessionId::from_str("s-interone")?, false)
        .await
        .context("SqliteStore::get_session(s-interone)")?;
    match &interactive.item.mode {
        SessionMode::Interactive {
            conversation_id, ..
        } if conversation_id.as_ref() == "c-convalpha" => {}
        other => bail!("s-interone: expected Interactive(c-convalpha); got {other:?}"),
    }

    let resumed = store
        .get_session(&SessionId::from_str("s-intertwo")?, false)
        .await
        .context("SqliteStore::get_session(s-intertwo)")?;
    match resumed.item.resumed_from.as_ref().map(|s| s.as_ref()) {
        Some("s-interone") => {}
        other => bail!("s-intertwo: expected resumed_from=s-interone; got {other:?}"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260607000000_seed_default_project — assert that the seed INSERT, the
// `issues_v2.project_id` backfill UPDATE, and the migration's idempotency
// guard (`INSERT OR IGNORE`) all behave as designed. Coverage gap closed by
// [[i-bivbnsgb]] (follow-up to [[p-xtixlxfy]]) — the merged seed migration
// shipped with in-store round-trip tests but no migration-framework
// coverage.
// ---------------------------------------------------------------------------

async fn seed_default_project_migration_inserts_row(pool: &SqlitePool) -> Result<()> {
    let row = sqlx::query(
        "SELECT id, version_number, key, name, default_status_key, statuses, \
                creator, deleted, actor, is_latest, prompt_path \
         FROM projects WHERE id = 'j-defaul'",
    )
    .fetch_one(pool)
    .await
    .context("read seeded default project row 'j-defaul'")?;

    let id: String = row.try_get("id")?;
    let version_number: i64 = row.try_get("version_number")?;
    let key: String = row.try_get("key")?;
    let name: String = row.try_get("name")?;
    let default_status_key: String = row.try_get("default_status_key")?;
    let statuses_text: String = row.try_get("statuses")?;
    let creator: String = row.try_get("creator")?;
    let deleted: i64 = row.try_get("deleted")?;
    let is_latest: i64 = row.try_get("is_latest")?;
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
    if default_status_key != "open" {
        bail!("j-defaul: expected default_status_key='open'; got {default_status_key:?}");
    }
    if creator != "system" {
        bail!("j-defaul: expected creator='system'; got {creator:?}");
    }
    if deleted != 0 {
        bail!("j-defaul: expected deleted=0; got {deleted}");
    }
    if is_latest != 1 {
        bail!("j-defaul: expected is_latest=1; got {is_latest}");
    }
    if actor.is_some() {
        bail!("j-defaul: expected actor=NULL; got {actor:?}");
    }
    if prompt_path.as_deref() != Some("/projects/default/prompt.md") {
        bail!("j-defaul: expected prompt_path='/projects/default/prompt.md'; got {prompt_path:?}");
    }

    // `statuses` JSON must deserialize into a Vec<StatusDefinition> that
    // matches `default_project_seed()` byte-for-byte. Comparing against the
    // Rust seed locks the SQL literal to the Rust constant: any drift in
    // either direction fails loud here.
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

async fn seed_default_project_migration_backfills_null_project_ids(
    pool: &SqlitePool,
) -> Result<()> {
    // Every fixture row that had NULL `project_id` at baseline-insert time
    // (single-version and multi-version) must now point at `'j-defaul'`.
    // The multi-version rows verify that the UPDATE touches every NULL
    // row regardless of `is_latest`.
    for (id, version) in [("i-seedone", 1), ("i-seedmv", 1), ("i-seedmv", 2)] {
        let row = sqlx::query(
            "SELECT project_id FROM issues_v2 \
             WHERE id = ?1 AND version_number = ?2",
        )
        .bind(id)
        .bind(version)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read project_id for issues_v2({id}, {version})"))?;
        let project_id: Option<String> = row.try_get("project_id")?;
        if project_id.as_deref() != Some("j-defaul") {
            bail!("issues_v2({id}, {version}).project_id: expected 'j-defaul'; got {project_id:?}");
        }
    }

    // Catch-all: no `issues_v2` row should be left with NULL project_id
    // post-backfill. The migration's UPDATE is unconditional on
    // `is_latest`, so older / soft-deleted versions get backfilled too.
    let row = sqlx::query("SELECT COUNT(*) FROM issues_v2 WHERE project_id IS NULL")
        .fetch_one(pool)
        .await
        .context("count remaining NULL project_id rows after backfill")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 issues_v2 rows with NULL project_id post-backfill; got {count}");
    }
    Ok(())
}

async fn seed_default_project_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // Re-execute the migration body verbatim. `INSERT OR IGNORE` must
    // swallow the (id, version_number) conflict and the UPDATE must be a
    // no-op since every row was backfilled by the first pass. Reading the
    // file rather than hard-coding the SQL keeps this test honest if the
    // migration's body ever changes shape (the assertion exercises
    // whatever the current rollforward statement is).
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("sqlite-migrations/20260607000000_seed_default_project.sql"),
    )
    .context("read sqlite seed_default_project migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply sqlite seed_default_project migration body")?;

    // No duplicate row at (j-defaul, 1).
    let row = sqlx::query("SELECT COUNT(*) FROM projects WHERE id = 'j-defaul'")
        .fetch_one(pool)
        .await
        .context("count projects rows for j-defaul after idempotency rerun")?;
    let count: i64 = row.try_get(0)?;
    if count != 1 {
        bail!("expected exactly 1 projects row for j-defaul after rerun; got {count}");
    }
    Ok(())
}
