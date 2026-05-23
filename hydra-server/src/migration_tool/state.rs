//! `migrate-state` pass: copy `conversation_session_state` rows into
//! `session_state`, keyed on the producing session id.
//!
//! Producing-session rule (per §3.5 step 4):
//!   "the most recent session attached to the conversation that has a
//!    Suspending entry in conversation_events_v2. If a conversation has no
//!    Suspending event yet (state was persisted while still active), use
//!    the most recent linked session."
//!
//! In both branches the producing session is the latest linked session by
//! creation time — the resumption protocol is strictly sequential, so the
//! current `conversation_session_state` blob is always the one written by
//! the most recently attached session.

use super::{Backend, PlanAction, PlanEntry};
use anyhow::{Context, Result};

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

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

mod sqlite {
    use super::{PlanAction, PlanEntry, Result};
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

            // Latest session linked to this conversation. The resumption
            // protocol is strictly sequential, so the latest linked session
            // is the producing session for the current state blob (with or
            // without a Suspending event).
            let producing_session: Option<String> = sqlx::query_scalar(
                "SELECT id FROM tasks_v2 \
                 WHERE conversation_id = ?1 \
                   AND is_latest = 1 \
                   AND deleted = 0 \
                 ORDER BY creation_time DESC, id DESC \
                 LIMIT 1",
            )
            .bind(&conversation_id)
            .fetch_optional(pool)
            .await
            .with_context(|| {
                format!("looking up producing session for conversation {conversation_id}")
            })?;

            let Some(producing_session_id) = producing_session else {
                // No linked session: cannot migrate without a key. Surface as
                // a warning rather than failing the whole pass so an operator
                // can investigate without blocking the rest of the migration.
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
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::{PlanAction, PlanEntry, Result};
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
            let producing_session: Option<String> = sqlx::query_scalar(
                "SELECT id FROM metis.tasks_v2 \
                 WHERE conversation_id = $1 \
                   AND is_latest = TRUE \
                   AND deleted = FALSE \
                 ORDER BY creation_time DESC, id DESC \
                 LIMIT 1",
            )
            .bind(&conversation_id)
            .fetch_optional(pool)
            .await
            .with_context(|| {
                format!("looking up producing session for conversation {conversation_id}")
            })?;

            let Some(producing_session_id) = producing_session else {
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
}

// ---------------------------------------------------------------------------
// SQLite integration test
// ---------------------------------------------------------------------------
//
// Builds a sqlite fixture with two conversations (single-session and a
// multi-session resumption chain), populates `conversations.session_state`,
// runs the tool dry-run + live, and asserts the resulting `session_state`
// rows match the design's keying rule. Re-running live mode is a no-op.

#[cfg(test)]
mod tests {
    use super::{Backend, PlanAction, run};
    use crate::store::sqlite_store::SqliteStore;
    use chrono::{Duration, Utc};
    use hydra_common::{ConversationId, SessionId};
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

    /// Read back the session_state blob for `session_id`.
    async fn read_session_state(pool: &SqlitePool, session_id: &SessionId) -> Option<Vec<u8>> {
        sqlx::query_scalar::<_, Vec<u8>>("SELECT data FROM session_state WHERE session_id = ?1")
            .bind(session_id.as_ref())
            .fetch_optional(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn migrate_state_keys_on_latest_linked_session_and_is_idempotent() {
        let pool = fresh_pool().await;

        // Conversation 1: single session.
        let conv_one = ConversationId::new();
        let sess_one = SessionId::new();
        let blob_one = b"state-for-conversation-one".to_vec();
        insert_conversation_with_state(&pool, &conv_one, "alice", &blob_one).await;
        insert_linked_session(&pool, &sess_one, &conv_one, "alice", Utc::now()).await;

        // Conversation 2: resumption chain (3 sessions); current state was
        // produced by the most recent (third) session.
        let conv_two = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let blob_two = b"state-for-conversation-two-from-c".to_vec();
        insert_conversation_with_state(&pool, &conv_two, "bob", &blob_two).await;
        let now = Utc::now();
        insert_linked_session(&pool, &sess_a, &conv_two, "bob", now - Duration::hours(2)).await;
        insert_linked_session(&pool, &sess_b, &conv_two, "bob", now - Duration::hours(1)).await;
        insert_linked_session(&pool, &sess_c, &conv_two, "bob", now).await;

        let backend = Backend::Sqlite(pool.clone());

        // Dry-run: nothing should be written, but the plan should match.
        let dry_plan = run(&backend, true).await.unwrap();
        assert_eq!(dry_plan.len(), 2);
        for entry in &dry_plan {
            assert_eq!(entry.action, PlanAction::WouldWrite);
        }
        let dry_for_conv_one = dry_plan
            .iter()
            .find(|p| p.conversation_id == *conv_one.as_ref())
            .unwrap();
        let dry_for_conv_two = dry_plan
            .iter()
            .find(|p| p.conversation_id == *conv_two.as_ref())
            .unwrap();
        assert_eq!(dry_for_conv_one.producing_session_id, *sess_one.as_ref());
        assert_eq!(dry_for_conv_one.byte_len, blob_one.len());
        assert_eq!(dry_for_conv_two.producing_session_id, *sess_c.as_ref());
        assert_eq!(dry_for_conv_two.byte_len, blob_two.len());

        // Dry-run must not have written anything.
        assert!(read_session_state(&pool, &sess_one).await.is_none());
        assert!(read_session_state(&pool, &sess_c).await.is_none());

        // Live run: writes happen, plan matches dry-run exactly (only the
        // action variant differs).
        let live_plan = run(&backend, false).await.unwrap();
        assert_eq!(live_plan.len(), dry_plan.len());
        for entry in &live_plan {
            assert_eq!(entry.action, PlanAction::Wrote);
        }
        assert_eq!(
            read_session_state(&pool, &sess_one).await.as_deref(),
            Some(&blob_one[..])
        );
        assert_eq!(
            read_session_state(&pool, &sess_c).await.as_deref(),
            Some(&blob_two[..])
        );
        // Predecessors in the chain should NOT have rows.
        assert!(read_session_state(&pool, &sess_a).await.is_none());
        assert!(read_session_state(&pool, &sess_b).await.is_none());

        // Re-run is a no-op: every entry becomes Skipped, no data changes.
        let rerun_plan = run(&backend, false).await.unwrap();
        assert_eq!(rerun_plan.len(), live_plan.len());
        for entry in &rerun_plan {
            assert_eq!(entry.action, PlanAction::Skipped);
        }
        assert_eq!(
            read_session_state(&pool, &sess_one).await.as_deref(),
            Some(&blob_one[..])
        );
        assert_eq!(
            read_session_state(&pool, &sess_c).await.as_deref(),
            Some(&blob_two[..])
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
