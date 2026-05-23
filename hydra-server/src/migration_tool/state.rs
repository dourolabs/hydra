//! `migrate-state` pass: copy `conversation_session_state` rows into
//! `session_state`, keyed on the producing session id.
//!
//! Producing-session rule (per §3.5 step 4 of
//! `designs/sessions-orthogonality-redesign.md`):
//!   "the most recent session attached to the conversation that has a
//!    Suspending entry in conversation_events_v2. If a conversation has no
//!    Suspending event yet (state was persisted while still active), use
//!    the most recent linked session."
//!
//! The current `conversation_session_state` blob is uploaded inside
//! `emit_suspend` immediately after a worker emits a `Suspending` event
//! (see `hydra/src/worker/relay_adapter.rs::emit_suspend`). So the session
//! that produced the latest blob is exactly the session that emitted the
//! most recent `Suspending` event on the conversation — i.e. the actor of
//! that event in `conversation_events_v2`. If no Suspending event has been
//! emitted yet, fall back to the most recently linked session (the active
//! one mid-flight).

use super::{Backend, PlanAction, PlanEntry};
use anyhow::{Context, Result};
use hydra_common::{ActorId, ActorRef};

/// Run the migrate-state pass against `backend`. With `dry_run = true`, no
/// writes happen and every plan entry is a `would-*`. With `dry_run = false`,
/// rows are upserted into `session_state` (skipping any conflicts, so re-runs
/// are no-ops).
pub async fn run(backend: &Backend, dry_run: bool) -> Result<Vec<PlanEntry>> {
    match backend {
        Backend::Sqlite(pool) => sqlite::run(pool, dry_run).await,
        #[cfg(feature = "postgres")]
        Backend::Postgres(pool) => postgres::run(pool, dry_run).await,
    }
}

/// Emit a single plan entry to stdout as one JSON line. The dry-run output
/// matches the live-run plan exactly (only the action variants differ), so
/// `<dry-run output> | jq -r '.producing_session_id'` works for both.
pub fn emit_jsonl(entry: &PlanEntry) -> Result<()> {
    let line = serde_json::to_string(entry).context("serializing plan entry")?;
    println!("{line}");
    Ok(())
}

/// Extract the session id from a stored `ActorRef` JSON value, if the actor
/// is an authenticated session. Other actor shapes (service tokens, automation
/// triggers) return `None`, which makes the caller fall back to the
/// "latest linked session" rule.
fn session_id_from_actor_json(actor: &str) -> Option<String> {
    let parsed: ActorRef = serde_json::from_str(actor).ok()?;
    match parsed {
        ActorRef::Authenticated {
            actor_id: ActorId::Session(session_id),
        } => Some(session_id.as_ref().to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

mod sqlite {
    use super::{PlanAction, PlanEntry, Result, session_id_from_actor_json};
    use anyhow::Context;
    use sqlx::{Row, SqlitePool};

    pub async fn run(pool: &SqlitePool, dry_run: bool) -> Result<Vec<PlanEntry>> {
        // Source rows: live conversations with a session_state blob.
        let conv_rows = sqlx::query(
            "SELECT id, session_state \
             FROM conversations \
             WHERE is_latest = 1 \
               AND deleted = 0 \
               AND session_state IS NOT NULL \
             ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing conversations with session_state")?;

        let mut plan = Vec::with_capacity(conv_rows.len());

        for row in conv_rows {
            let conversation_id: String = row.try_get("id")?;
            let data: Vec<u8> = row.try_get("session_state")?;

            let Some(producing_session_id) =
                resolve_producing_session(pool, &conversation_id).await?
            else {
                eprintln!(
                    "warning: conversation {conversation_id} has session_state but no \
                     linked session — skipping",
                );
                continue;
            };

            let already_exists: bool = sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM session_state WHERE session_id = ?1)",
            )
            .bind(&producing_session_id)
            .fetch_one(pool)
            .await
            .context("checking session_state idempotency")?
                != 0;

            let action = if already_exists {
                if dry_run {
                    PlanAction::WouldSkip
                } else {
                    PlanAction::Skipped
                }
            } else if dry_run {
                PlanAction::WouldWrite
            } else {
                sqlx::query(
                    "INSERT INTO session_state (session_id, data, updated_at) \
                     VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                     ON CONFLICT(session_id) DO NOTHING",
                )
                .bind(&producing_session_id)
                .bind(&data)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("inserting session_state for session {producing_session_id}")
                })?;
                PlanAction::Wrote
            };

            plan.push(PlanEntry {
                conversation_id,
                producing_session_id,
                byte_len: data.len(),
                action,
            });
        }

        Ok(plan)
    }

    /// Implements the §3.5 step 4 producing-session rule against the SQLite
    /// schema: prefer the actor of the most recent `suspending` event on the
    /// conversation; fall back to the most recently linked session.
    async fn resolve_producing_session(
        pool: &SqlitePool,
        conversation_id: &str,
    ) -> Result<Option<String>> {
        let suspending_actor: Option<String> = sqlx::query_scalar(
            "SELECT actor FROM conversation_events \
             WHERE id = ?1 AND event_type = 'suspending' \
             ORDER BY version_number DESC \
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(pool)
        .await
        .with_context(|| {
            format!("looking up suspending event for conversation {conversation_id}")
        })?;

        if let Some(actor_json) = suspending_actor.as_deref()
            && let Some(session_id) = session_id_from_actor_json(actor_json)
        {
            return Ok(Some(session_id));
        }

        sqlx::query_scalar(
            "SELECT id FROM tasks_v2 \
             WHERE conversation_id = ?1 \
               AND is_latest = 1 \
               AND deleted = 0 \
             ORDER BY creation_time DESC, id DESC \
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("looking up latest linked session for {conversation_id}"))
    }
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::{PlanAction, PlanEntry, Result, session_id_from_actor_json};
    use anyhow::Context;
    use sqlx::PgPool;

    pub async fn run(pool: &PgPool, dry_run: bool) -> Result<Vec<PlanEntry>> {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT conversation_id, data \
             FROM metis.conversation_session_state \
             ORDER BY conversation_id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing metis.conversation_session_state")?;

        let mut plan = Vec::with_capacity(rows.len());

        for (conversation_id, data) in rows {
            let Some(producing_session_id) =
                resolve_producing_session(pool, &conversation_id).await?
            else {
                eprintln!(
                    "warning: conversation {conversation_id} has session_state but no \
                     linked session — skipping",
                );
                continue;
            };

            let already_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM metis.session_state_v2 WHERE session_id = $1)",
            )
            .bind(&producing_session_id)
            .fetch_one(pool)
            .await
            .context("checking session_state_v2 idempotency")?;

            let action = if already_exists {
                if dry_run {
                    PlanAction::WouldSkip
                } else {
                    PlanAction::Skipped
                }
            } else if dry_run {
                PlanAction::WouldWrite
            } else {
                sqlx::query(
                    "INSERT INTO metis.session_state_v2 (session_id, data) \
                     VALUES ($1, $2) \
                     ON CONFLICT (session_id) DO NOTHING",
                )
                .bind(&producing_session_id)
                .bind(&data)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("inserting metis.session_state_v2 for session {producing_session_id}")
                })?;
                PlanAction::Wrote
            };

            plan.push(PlanEntry {
                conversation_id,
                producing_session_id,
                byte_len: data.len(),
                action,
            });
        }

        Ok(plan)
    }

    /// Implements the §3.5 step 4 producing-session rule against the Postgres
    /// schema: prefer the actor of the most recent `suspending` event on the
    /// conversation; fall back to the most recently linked session.
    async fn resolve_producing_session(
        pool: &PgPool,
        conversation_id: &str,
    ) -> Result<Option<String>> {
        // Postgres `actor` is JSONB; `to_jsonb(NULL)` is NULL, so an absent
        // actor surfaces as `None` here.
        let suspending_actor: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT actor FROM metis.conversation_events_v2 \
             WHERE conversation_id = $1 AND event_type = 'suspending' \
             ORDER BY version_number DESC \
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("looking up suspending event for conversation {conversation_id}"))?
        .flatten();

        if let Some(actor_value) = suspending_actor
            && let Some(session_id) = session_id_from_actor_json(&actor_value.to_string())
        {
            return Ok(Some(session_id));
        }

        sqlx::query_scalar(
            "SELECT id FROM metis.tasks_v2 \
             WHERE conversation_id = $1 \
               AND is_latest = TRUE \
               AND deleted = FALSE \
             ORDER BY creation_time DESC, id DESC \
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("looking up latest linked session for {conversation_id}"))
    }
}

// ---------------------------------------------------------------------------
// SQLite integration tests
// ---------------------------------------------------------------------------
//
// Builds sqlite fixtures with single-session, multi-session-no-suspending,
// and multi-session-with-suspending-fork scenarios, runs the tool dry-run +
// live, and asserts the resulting `session_state` rows match §3.5 step 4.

#[cfg(test)]
mod tests {
    use super::{Backend, PlanAction, run};
    use crate::store::sqlite_store::SqliteStore;
    use chrono::{Duration, Utc};
    use hydra_common::{ActorId, ActorRef, ConversationId, SessionId};
    use sqlx::SqlitePool;

    /// Build an in-memory sqlite store with the production migrations
    /// applied. The migration tool runs against this same schema in prod.
    async fn fresh_pool() -> SqlitePool {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();
        pool
    }

    /// Insert a stub conversation row with `session_state = data`.
    async fn insert_conversation_with_state(
        pool: &SqlitePool,
        conv_id: &ConversationId,
        creator: &str,
        data: &[u8],
    ) {
        sqlx::query(
            "INSERT INTO conversations \
                (id, version_number, status, creator, deleted, session_state, is_latest) \
             VALUES (?1, 1, 'active', ?2, 0, ?3, 1)",
        )
        .bind(conv_id.as_ref())
        .bind(creator)
        .bind(data)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a minimally-valid `tasks_v2` row representing a session linked
    /// to `conv_id` with the given creation time.
    async fn insert_linked_session(
        pool: &SqlitePool,
        session_id: &SessionId,
        conv_id: &ConversationId,
        creator: &str,
        creation_time: chrono::DateTime<chrono::Utc>,
    ) {
        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, prompt, context, creator, image, env_vars, \
                 status, deleted, creation_time, interactive, conversation_id, is_latest) \
             VALUES (?1, 1, '', '{\"type\":\"none\"}', ?2, NULL, '{}', \
                     'complete', 0, ?3, 1, ?4, 1)",
        )
        .bind(session_id.as_ref())
        .bind(creator)
        .bind(creation_time.to_rfc3339())
        .bind(conv_id.as_ref())
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a `suspending` conversation event attributed to `actor_session`.
    async fn insert_suspending_event(
        pool: &SqlitePool,
        conv_id: &ConversationId,
        version: i64,
        actor_session: &SessionId,
    ) {
        let actor = ActorRef::Authenticated {
            actor_id: ActorId::Session(actor_session.clone()),
        };
        let actor_json = serde_json::to_string(&actor).unwrap();
        let event_data = serde_json::json!({
            "type": "suspending",
            "reason": "test",
            "timestamp": Utc::now().to_rfc3339(),
        })
        .to_string();
        sqlx::query(
            "INSERT INTO conversation_events \
                (id, version_number, event_type, event_data, actor) \
             VALUES (?1, ?2, 'suspending', ?3, ?4)",
        )
        .bind(conv_id.as_ref())
        .bind(version)
        .bind(event_data)
        .bind(actor_json)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Read back the session_state blob for `session_id`.
    async fn read_session_state(pool: &SqlitePool, session_id: &SessionId) -> Option<Vec<u8>> {
        sqlx::query_scalar::<_, Vec<u8>>("SELECT data FROM session_state WHERE session_id = ?1")
            .bind(session_id.as_ref())
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migrate_state_falls_back_to_latest_linked_when_no_suspending_event() {
        let pool = fresh_pool().await;

        // Conversation 1: single session, no suspending event yet.
        let conv_one = ConversationId::new();
        let sess_one = SessionId::new();
        let blob_one = b"state-for-conversation-one".to_vec();
        insert_conversation_with_state(&pool, &conv_one, "alice", &blob_one).await;
        insert_linked_session(&pool, &sess_one, &conv_one, "alice", Utc::now()).await;

        // Conversation 2: resumption chain (3 sessions), no suspending event
        // yet (state was persisted while still active). Latest linked = C.
        let conv_two = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let blob_two = b"state-for-conversation-two".to_vec();
        insert_conversation_with_state(&pool, &conv_two, "bob", &blob_two).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv_two, "bob", now - Duration::hours(2)).await;
        insert_linked_session(&pool, &sess_b, &conv_two, "bob", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_c, &conv_two, "bob", now).await;

        let backend = Backend::Sqlite(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert_eq!(plan.len(), 2);

        let entry_one = plan
            .iter()
            .find(|p| p.conversation_id == *conv_one.as_ref())
            .unwrap();
        let entry_two = plan
            .iter()
            .find(|p| p.conversation_id == *conv_two.as_ref())
            .unwrap();
        assert_eq!(entry_one.producing_session_id, *sess_one.as_ref());
        assert_eq!(entry_two.producing_session_id, *sess_c.as_ref());
        assert_eq!(
            read_session_state(&pool, &sess_c).await.as_deref(),
            Some(&blob_two[..])
        );
        // Predecessors should not have rows.
        assert!(read_session_state(&pool, &sess_a).await.is_none());
        assert!(read_session_state(&pool, &sess_b).await.is_none());
    }

    #[tokio::test]
    async fn migrate_state_keys_on_session_that_emitted_latest_suspending() {
        // Chain A → B → C with C mid-flight: B emitted Suspending (which is
        // what spawned C). The current state blob was uploaded by B inside
        // `emit_suspend`. C has not yet suspended. §3.5 step 4 requires the
        // blob to be keyed under B, not C.
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let blob = b"state-uploaded-by-b".to_vec();
        insert_conversation_with_state(&pool, &conv, "carol", &blob).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv, "carol", now - Duration::hours(2)).await;
        insert_linked_session(&pool, &sess_b, &conv, "carol", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_c, &conv, "carol", now).await;
        // A and B both suspended; C is mid-flight (no suspending event).
        insert_suspending_event(&pool, &conv, 1, &sess_a).await;
        insert_suspending_event(&pool, &conv, 2, &sess_b).await;

        let backend = Backend::Sqlite(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].producing_session_id, *sess_b.as_ref());
        assert_eq!(plan[0].action, PlanAction::Wrote);

        assert_eq!(
            read_session_state(&pool, &sess_b).await.as_deref(),
            Some(&blob[..])
        );
        // C is the latest linked session but did not produce the blob.
        assert!(read_session_state(&pool, &sess_c).await.is_none());
        assert!(read_session_state(&pool, &sess_a).await.is_none());
    }

    #[tokio::test]
    async fn migrate_state_is_idempotent_and_dry_run_matches_live_plan() {
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let blob = b"blob".to_vec();
        insert_conversation_with_state(&pool, &conv, "dave", &blob).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv, "dave", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_b, &conv, "dave", now).await;
        insert_suspending_event(&pool, &conv, 1, &sess_a).await;

        let backend = Backend::Sqlite(pool.clone());

        let dry = run(&backend, true).await.unwrap();
        assert_eq!(dry.len(), 1);
        assert_eq!(dry[0].producing_session_id, *sess_a.as_ref());
        assert_eq!(dry[0].action, PlanAction::WouldWrite);
        // Dry run must not write.
        assert!(read_session_state(&pool, &sess_a).await.is_none());

        let live = run(&backend, false).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].producing_session_id, *sess_a.as_ref());
        assert_eq!(live[0].action, PlanAction::Wrote);

        let rerun = run(&backend, false).await.unwrap();
        assert_eq!(rerun.len(), 1);
        assert_eq!(rerun[0].action, PlanAction::Skipped);
        assert_eq!(
            read_session_state(&pool, &sess_a).await.as_deref(),
            Some(&blob[..])
        );
    }

    #[tokio::test]
    async fn migrate_state_skips_conversations_with_no_linked_session() {
        let pool = fresh_pool().await;

        let orphan = ConversationId::new();
        insert_conversation_with_state(&pool, &orphan, "carol", b"orphan-blob").await;

        let backend = Backend::Sqlite(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert!(
            plan.is_empty(),
            "orphan conversation should be skipped rather than emitted",
        );
    }

    #[tokio::test]
    async fn migrate_state_ignores_deleted_conversations() {
        let pool = fresh_pool().await;

        let conv = ConversationId::new();
        let sess = SessionId::new();
        sqlx::query(
            "INSERT INTO conversations \
                (id, version_number, status, creator, deleted, session_state, is_latest) \
             VALUES (?1, 1, 'active', 'dave', 1, ?2, 1)",
        )
        .bind(conv.as_ref())
        .bind(b"dont-migrate" as &[u8])
        .execute(&pool)
        .await
        .unwrap();
        insert_linked_session(&pool, &sess, &conv, "dave", Utc::now()).await;

        let backend = Backend::Sqlite(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert!(plan.is_empty());
        assert!(read_session_state(&pool, &sess).await.is_none());
    }

    /// On-disk smoke test: the tool must accept a file-backed sqlite DSN, not
    /// just `sqlite::memory:`.
    #[tokio::test]
    async fn migrate_state_works_against_on_disk_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hydra.db");
        let dsn = format!("sqlite:{}?mode=rwc", path.display());

        // Initialize schema via the production helpers (same as the server).
        let pool = SqliteStore::init_pool(&dsn).await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();

        let conv = ConversationId::new();
        let sess = SessionId::new();
        let blob = b"disk-blob".to_vec();
        insert_conversation_with_state(&pool, &conv, "erin", &blob).await;
        insert_linked_session(&pool, &sess, &conv, "erin", Utc::now()).await;
        drop(pool);

        // Reconnect via the migration tool's `Backend::connect` to exercise
        // the DSN-scheme detection path.
        let backend = Backend::connect(&dsn).await.unwrap();
        let plan = run(&backend, false).await.unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].producing_session_id, *sess.as_ref());

        // Re-verify via a fresh pool that the row landed.
        let verify = SqliteStore::init_pool(&dsn).await.unwrap();
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM session_state WHERE session_id = ?1")
                .bind(sess.as_ref())
                .fetch_optional(&verify)
                .await
                .unwrap();
        assert_eq!(stored.as_deref(), Some(&blob[..]));
    }
}

// ---------------------------------------------------------------------------
// Postgres integration tests
// ---------------------------------------------------------------------------
//
// Mirrors the SQLite scenarios above against a real postgres backend. The
// postgres SQL path uses `$N` placeholders, the `metis.` schema, and parses
// JSONB actor values via `serde_json::Value::to_string()` — none of which the
// sqlite tests exercise. CI runs these via `cargo nextest --features
// enterprise --run-ignored all` against the workflow's postgres service; they
// are `#[ignore]`d so default `cargo test` runs do not require docker/postgres.

#[cfg(all(test, feature = "postgres"))]
mod tests_postgres {
    use super::{Backend, PlanAction, run};
    use chrono::{Duration, Utc};
    use hydra_common::{ActorId, ActorRef, ConversationId, SessionId};
    use sqlx::PgPool;

    /// Insert a `metis.conversation_session_state` row. The postgres pass
    /// iterates this table directly (it is independent of `conversations_v2`).
    async fn insert_session_state(pool: &PgPool, conv_id: &ConversationId, data: &[u8]) {
        sqlx::query(
            "INSERT INTO metis.conversation_session_state (conversation_id, data) \
             VALUES ($1, $2)",
        )
        .bind(conv_id.as_ref())
        .bind(data)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a minimally-valid `tasks_v2` row representing a session linked
    /// to `conv_id` with the given creation time. The BEFORE INSERT trigger
    /// sets `is_latest = true` automatically.
    async fn insert_linked_session(
        pool: &PgPool,
        session_id: &SessionId,
        conv_id: &ConversationId,
        creator: &str,
        creation_time: chrono::DateTime<chrono::Utc>,
    ) {
        sqlx::query(
            "INSERT INTO metis.tasks_v2 \
                (id, version_number, prompt, context, creator, env_vars, \
                 status, deleted, creation_time, interactive, conversation_id) \
             VALUES ($1, 1, '', '{\"type\":\"none\"}'::jsonb, $2, '{}'::jsonb, \
                     'complete', FALSE, $3, FALSE, $4)",
        )
        .bind(session_id.as_ref())
        .bind(creator)
        .bind(creation_time)
        .bind(conv_id.as_ref())
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a `suspending` conversation event attributed to `actor_session`.
    /// The `actor` column is JSONB and the resolver parses it via
    /// `Value::to_string()` → `serde_json::from_str::<ActorRef>`.
    async fn insert_suspending_event(
        pool: &PgPool,
        conv_id: &ConversationId,
        version: i64,
        actor_session: &SessionId,
    ) {
        let actor = ActorRef::Authenticated {
            actor_id: ActorId::Session(actor_session.clone()),
        };
        let actor_json = serde_json::to_value(&actor).unwrap();
        let event_data = serde_json::json!({
            "type": "suspending",
            "reason": "test",
            "timestamp": Utc::now().to_rfc3339(),
        });
        sqlx::query(
            "INSERT INTO metis.conversation_events_v2 \
                (conversation_id, version_number, event_type, event_data, actor) \
             VALUES ($1, $2, 'suspending', $3, $4)",
        )
        .bind(conv_id.as_ref())
        .bind(version)
        .bind(event_data)
        .bind(actor_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn read_session_state(pool: &PgPool, session_id: &SessionId) -> Option<Vec<u8>> {
        sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT data FROM metis.session_state_v2 WHERE session_id = $1",
        )
        .bind(session_id.as_ref())
        .fetch_optional(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migrate_state_keys_on_session_that_emitted_latest_suspending(pool: PgPool) {
        // Chain A → B → C with C mid-flight: B emitted the most recent
        // Suspending event. §3.5 step 4 requires the blob to be keyed under B,
        // not the most-recently-linked session C.
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let blob = b"state-uploaded-by-b".to_vec();
        insert_session_state(&pool, &conv, &blob).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv, "carol", now - Duration::hours(2)).await;
        insert_linked_session(&pool, &sess_b, &conv, "carol", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_c, &conv, "carol", now).await;
        insert_suspending_event(&pool, &conv, 1, &sess_a).await;
        insert_suspending_event(&pool, &conv, 2, &sess_b).await;

        let backend = Backend::Postgres(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].producing_session_id, *sess_b.as_ref());
        assert_eq!(plan[0].action, PlanAction::Wrote);

        assert_eq!(
            read_session_state(&pool, &sess_b).await.as_deref(),
            Some(&blob[..])
        );
        assert!(read_session_state(&pool, &sess_c).await.is_none());
        assert!(read_session_state(&pool, &sess_a).await.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migrate_state_falls_back_to_latest_linked_when_no_suspending_event(pool: PgPool) {
        // Conversation 1: single session, no suspending event yet.
        let conv_one = ConversationId::new();
        let sess_one = SessionId::new();
        let blob_one = b"state-for-conversation-one".to_vec();
        insert_session_state(&pool, &conv_one, &blob_one).await;
        insert_linked_session(&pool, &sess_one, &conv_one, "alice", Utc::now()).await;

        // Conversation 2: three linked sessions, no suspending event yet.
        // Latest linked = C.
        let conv_two = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let blob_two = b"state-for-conversation-two".to_vec();
        insert_session_state(&pool, &conv_two, &blob_two).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv_two, "bob", now - Duration::hours(2)).await;
        insert_linked_session(&pool, &sess_b, &conv_two, "bob", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_c, &conv_two, "bob", now).await;

        let backend = Backend::Postgres(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert_eq!(plan.len(), 2);

        let entry_one = plan
            .iter()
            .find(|p| p.conversation_id == *conv_one.as_ref())
            .unwrap();
        let entry_two = plan
            .iter()
            .find(|p| p.conversation_id == *conv_two.as_ref())
            .unwrap();
        assert_eq!(entry_one.producing_session_id, *sess_one.as_ref());
        assert_eq!(entry_two.producing_session_id, *sess_c.as_ref());
        assert_eq!(
            read_session_state(&pool, &sess_c).await.as_deref(),
            Some(&blob_two[..])
        );
        assert!(read_session_state(&pool, &sess_a).await.is_none());
        assert!(read_session_state(&pool, &sess_b).await.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migrate_state_is_idempotent_and_dry_run_matches_live_plan(pool: PgPool) {
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let blob = b"blob".to_vec();
        insert_session_state(&pool, &conv, &blob).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv, "dave", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_b, &conv, "dave", now).await;
        insert_suspending_event(&pool, &conv, 1, &sess_a).await;

        let backend = Backend::Postgres(pool.clone());

        let dry = run(&backend, true).await.unwrap();
        assert_eq!(dry.len(), 1);
        assert_eq!(dry[0].producing_session_id, *sess_a.as_ref());
        assert_eq!(dry[0].action, PlanAction::WouldWrite);
        // Dry run must not write.
        assert!(read_session_state(&pool, &sess_a).await.is_none());

        let live = run(&backend, false).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].producing_session_id, *sess_a.as_ref());
        assert_eq!(live[0].action, PlanAction::Wrote);

        let rerun = run(&backend, false).await.unwrap();
        assert_eq!(rerun.len(), 1);
        assert_eq!(rerun[0].action, PlanAction::Skipped);
        assert_eq!(
            read_session_state(&pool, &sess_a).await.as_deref(),
            Some(&blob[..])
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migrate_state_skips_conversations_with_no_linked_session(pool: PgPool) {
        let orphan = ConversationId::new();
        insert_session_state(&pool, &orphan, b"orphan-blob").await;

        let backend = Backend::Postgres(pool.clone());
        let plan = run(&backend, false).await.unwrap();
        assert!(
            plan.is_empty(),
            "orphan conversation should be skipped rather than emitted",
        );
    }
}
