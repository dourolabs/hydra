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
//! Scope (per [[i-toeamhmw]]): the `actor_variant_cleanup` SQLite arm's
//! `session_events` and `conversation_events` rewrites — the exact code
//! paths surfaced by the `(session_id, version_number) AS __pk`
//! parse-reject bug that shipped past CI ([[i-ccchbxha]], fixed by
//! [[i-nmcnqeyn]] / [[p-fcxmstwd]]). Future SQLite-only migration bugs
//! get caught by extending this fixture tree + this file.
//!
//! Runs under the default `cargo test --workspace` — no `#[ignore]`, no
//! feature gate. The postgres test is CI-only because it needs a live
//! postgres; SQLite has no such constraint and uses `sqlite::memory:`.

use anyhow::{Context, Result, bail};
use hydra_common::{ConversationId, IssueId, SessionId};
use hydra_server::domain::actors::{ActorId, ActorRef};
use hydra_server::store::ReadOnlyStore;
use hydra_server::store::sqlite_store::{self, MIGRATOR, SqliteStore};
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
