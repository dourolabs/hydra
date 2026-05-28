//! `migrate-events` pass: partition historical `conversation_events_v2`
//! user/assistant message rows by "active session at write time" and write
//! them to `session_events` (sqlite) / `metis.session_events_v2` (postgres).
//!
//! ## Partitioning rule
//!
//! For each conversation, walk the linked sessions in `(creation_time ASC,
//! id ASC)` order. Each session owns the half-open window
//!
//! ```text
//! [creation_time,  min(suspend_or_closed_ts, next_session.creation_time))
//! ```
//!
//! where:
//!   * `suspend_or_closed_ts` is the row-level `created_at` of the first
//!     `suspending` or `closed` event on `conversation_events*` whose
//!     `actor` is this session (or `+∞` if none).
//!   * `next_session.creation_time` comes from the next session in the
//!     creation-time-ordered chain (or `+∞` if this is the last session).
//!
//! We skip `suspending`, `resumed`, and `closed` rows (design §3.5 step 5
//! leaves them on `conversation_events*`). For each remaining `user_message`
//! / `assistant_message` row, the assignment is tolerant of the two real-DB
//! inconsistencies we have seen in practice:
//!
//!   * **before all sessions** → first session.
//!   * **inside a session's window** → that session.
//!   * **in a gap between session N and session N+1** → session N+1
//!     (the *subsequent* session — NOT N).
//!   * **after the last session ends** → the last session.
//!
//! A conversation that has message events but no linked sessions at all is
//! NOT a fatal condition: the rows are silently dropped and a `warn!` log
//! records the conversation id and dropped count so operators can spot the
//! orphan in retrospect. Re-running the migration on a fixed source is still
//! a no-op because the per-session skip rule (below) is unchanged.
//!
//! ## Idempotency
//!
//! Re-running must be a no-op. The target table's primary key is
//! `(session_id, version_number)` but `version_number` is per-session
//! monotonic and assigned at insert time, so we can't match source rows to
//! target rows by version alone. Instead we check, per target session:
//! if `session_events*` already has any rows for that session, we skip the
//! whole session. This is the property the server's startup hook relies on
//! so that repeated boots against the same database don't re-process
//! already-migrated rows.

use super::{Backend, RustMigration};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// The sqlx migration version this Rust step must run *after*. Both target
/// tables (`session_events` / `metis.session_events_v2`) landed well before
/// PR-B was authored; pinning to the highest existing SQL migration version
/// at PR-B time keeps the events backfill as the final post-SQL step, which
/// is where it ran historically under the old `tokio::spawn` startup hook.
pub const EVENTS_MIGRATION_VERSION: u64 = 20_260_601_000_000;

/// Backfill historical `conversation_events*` user/assistant message rows
/// into the per-session `session_events*` tables. Idempotent: the
/// per-session skip rule in [`run`] keeps repeat runs as no-ops, which is
/// what the server's startup hook (and the migration roundtrip test) rely
/// on.
pub struct EventsMigration;

#[async_trait::async_trait]
impl RustMigration for EventsMigration {
    fn version(&self) -> u64 {
        EVENTS_MIGRATION_VERSION
    }

    fn name(&self) -> &'static str {
        "migrate-events"
    }

    async fn run(&self, backend: &Backend) -> Result<()> {
        run(backend).await
    }
}

/// Free-function entry point retained for callers that already hold a
/// [`Backend`]. Equivalent to `EventsMigration.run(backend)`. Kept public
/// so the integration test and ad-hoc reruns can invoke the pass without
/// going through the trait object.
pub async fn run(backend: &Backend) -> Result<()> {
    match backend {
        Backend::Sqlite(pool) => sqlite::run(pool).await,
        #[cfg(feature = "postgres")]
        Backend::Postgres(pool) => postgres::run(pool).await,
    }
}

/// Extract the session id from a stored `ActorRef` JSON value, if the
/// actor is an authenticated session. Other actor shapes (service tokens,
/// automation triggers, user / agent / external actors) return `None`.
///
/// This reader is intentionally tolerant of BOTH the pre-cleanup
/// (`{"Session":"s-..."}`) and post-cleanup
/// (`{"Adhoc":{"session_id":"s-..."}}`) wire shapes for the inner
/// `actor_id`. The events migration runs at version `20260601000000`,
/// before the `actor_variant_cleanup` migration at `20260603000000`. On
/// a fresh DB (CI / migration_roundtrip test) the events migration
/// reads pre-cleanup rows; on a soaked deploy the previous boot
/// already ran the events migration and cleanup hasn't rewritten its
/// own rows — both interleavings must work.
///
/// We don't go through `hydra_common::ActorRef::deserialize` because
/// post-cleanup `ActorId` no longer accepts the `{"Session":"..."}`
/// shape; raw `serde_json::Value` inspection sidesteps the type
/// dependency.
fn session_id_from_actor_json(actor: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(actor).ok()?;
    let inner = value.get("Authenticated")?.get("actor_id")?;
    extract_session_id(inner)
}

fn extract_session_id(actor_id: &serde_json::Value) -> Option<String> {
    let map = actor_id.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let (tag, payload) = map.iter().next()?;
    match (tag.as_str(), payload) {
        // Pre-cleanup wire shape.
        ("Session", serde_json::Value::String(sid)) => Some(sid.clone()),
        // Post-cleanup wire shape.
        ("Adhoc", serde_json::Value::Object(obj)) => obj
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        _ => None,
    }
}

/// One linked session for a conversation, with its creation time and the
/// timestamp of its first `Suspending` / `Closed` event on the conversation
/// (if any).
#[derive(Debug, Clone)]
struct SessionInChain {
    id: String,
    creation_time: DateTime<Utc>,
    suspend_or_close_ts: Option<DateTime<Utc>>,
}

/// A half-open window `[start, end)` owned by a session. `end == None`
/// means `+∞`.
#[derive(Debug, Clone)]
struct Window {
    session_id: String,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
}

/// Compute per-session windows per the partitioning rule documented at the
/// top of this module. `sessions` MUST be sorted by `(creation_time ASC,
/// id ASC)` — we rely on the order to break sub-millisecond ties between
/// overlapping sessions, and the fallback rule in [`assign_event`] depends
/// on `windows[i].start` being non-decreasing in `i`.
fn build_windows(sessions: &[SessionInChain]) -> Vec<Window> {
    let mut windows = Vec::with_capacity(sessions.len());
    for (i, s) in sessions.iter().enumerate() {
        let next_creation = sessions.get(i + 1).map(|n| n.creation_time);
        let end = match (s.suspend_or_close_ts, next_creation) {
            (Some(suspend), Some(next)) => Some(suspend.min(next)),
            (Some(suspend), None) => Some(suspend),
            (None, Some(next)) => Some(next),
            (None, None) => None,
        };
        windows.push(Window {
            session_id: s.id.clone(),
            start: s.creation_time,
            end,
        });
    }
    windows
}

/// Pick the session that should own `event_ts` per the partitioning rule:
/// inside a window → that session; before all sessions → first session; in
/// a gap between sessions N and N+1 → session N+1; past the last session's
/// end → last session. Panics if `windows` is empty — callers must check
/// `sessions.is_empty()` ahead of this and short-circuit.
fn assign_event(windows: &[Window], event_ts: DateTime<Utc>) -> &str {
    debug_assert!(!windows.is_empty(), "assign_event called with no windows");
    if let Some(w) = windows
        .iter()
        .find(|w| w.start <= event_ts && w.end.is_none_or(|e| event_ts < e))
    {
        return w.session_id.as_str();
    }
    // Not inside any window: either before the first, in a gap, or past the
    // last window's finite end. `windows` is sorted by start, so the first
    // window whose start exceeds `event_ts` is either windows[0] (before
    // everything) or the subsequent session of a gap.
    if let Some(w) = windows.iter().find(|w| w.start > event_ts) {
        return w.session_id.as_str();
    }
    // All starts are <= event_ts and no window contains it: every window has
    // a finite `end` and event_ts is past all of them. Land on the last.
    windows
        .last()
        .expect("debug_assert above guarantees non-empty")
        .session_id
        .as_str()
}

/// One message row to migrate. Owned-string form to keep the partitioning
/// step backend-agnostic.
#[derive(Debug, Clone)]
struct MessageRowSqlite {
    source_version_number: i64,
    event_type: String,
    event_data: String,
    actor: Option<String>,
    created_at: DateTime<Utc>,
}

#[cfg(feature = "postgres")]
#[derive(Debug, Clone)]
struct MessageRowPostgres {
    source_version_number: i64,
    event_type: String,
    event_data: serde_json::Value,
    actor: Option<serde_json::Value>,
    created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

mod sqlite {
    use super::*;
    use anyhow::{Context, anyhow};
    use sqlx::{Row, SqlitePool};

    pub async fn run(pool: &SqlitePool) -> Result<()> {
        // Conversations to process: any with at least one message event.
        let conv_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT id FROM conversation_events \
             WHERE event_type IN ('user_message', 'assistant_message') \
             ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing conversations with message events")?;

        for conv_id in conv_ids {
            process_conversation(pool, &conv_id).await?;
        }
        Ok(())
    }

    async fn process_conversation(pool: &SqlitePool, conv_id: &str) -> Result<()> {
        let sessions = load_sessions_in_chain(pool, conv_id).await?;
        let rows = load_message_rows(pool, conv_id).await?;

        if sessions.is_empty() {
            // No linked sessions: drop the message events rather than fail
            // the migration. Real production data has orphan conversations
            // (no row in tasks_v2 at all) and blocking startup on them is
            // worse than losing the history.
            if !rows.is_empty() {
                tracing::warn!(
                    conversation_id = %conv_id,
                    dropped = rows.len(),
                    "events migration: conversation has no linked sessions; dropping message events",
                );
            }
            return Ok(());
        }
        let windows = build_windows(&sessions);

        let mut per_session: HashMap<String, Vec<&MessageRowSqlite>> = HashMap::new();
        for row in &rows {
            let target = assign_event(&windows, row.created_at);
            per_session.entry(target.to_string()).or_default().push(row);
        }

        for (session_id, rows) in &per_session {
            let existing: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM session_events WHERE session_id = ?1")
                    .bind(session_id)
                    .fetch_one(pool)
                    .await
                    .with_context(|| {
                        format!("checking existing session_events for {session_id}")
                    })?;

            if existing > 0 {
                continue;
            }

            for row in rows {
                insert_row(pool, session_id, row).await?;
            }
        }

        Ok(())
    }

    async fn load_sessions_in_chain(
        pool: &SqlitePool,
        conv_id: &str,
    ) -> Result<Vec<SessionInChain>> {
        // tasks_v2.creation_time is TEXT (RFC3339). We pull it as a string
        // and parse manually to share one timestamp-parsing path with the
        // conversation_events.created_at column.
        let session_rows = sqlx::query(
            "SELECT id, creation_time FROM tasks_v2 \
             WHERE conversation_id = ?1 AND is_latest = 1 AND deleted = 0 \
             ORDER BY creation_time ASC, id ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading linked sessions for {conv_id}"))?;

        let mut sessions: Vec<SessionInChain> = Vec::with_capacity(session_rows.len());
        for row in session_rows {
            let id: String = row.try_get("id")?;
            let creation_time_str: String = row.try_get("creation_time")?;
            let creation_time = parse_timestamp(&creation_time_str)
                .with_context(|| format!("parsing tasks_v2.creation_time for session {id}"))?;
            sessions.push(SessionInChain {
                id,
                creation_time,
                suspend_or_close_ts: None,
            });
        }

        // Look up Suspending / Closed events for this conversation, keyed
        // by the session id that emitted them.
        let boundary_rows = sqlx::query(
            "SELECT actor, created_at FROM conversation_events \
             WHERE id = ?1 AND event_type IN ('suspending', 'closed') \
             ORDER BY version_number ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading boundary events for {conv_id}"))?;

        let mut earliest: HashMap<String, DateTime<Utc>> = HashMap::new();
        for row in boundary_rows {
            let actor: Option<String> = row.try_get("actor")?;
            let created_at_str: String = row.try_get("created_at")?;
            let created_at = parse_timestamp(&created_at_str)
                .with_context(|| format!("parsing conversation_events.created_at for {conv_id}"))?;
            let Some(actor_json) = actor.as_deref() else {
                continue;
            };
            let Some(session_id) = session_id_from_actor_json(actor_json) else {
                continue;
            };
            earliest.entry(session_id).or_insert(created_at);
        }

        for s in &mut sessions {
            s.suspend_or_close_ts = earliest.get(&s.id).copied();
        }
        Ok(sessions)
    }

    async fn load_message_rows(pool: &SqlitePool, conv_id: &str) -> Result<Vec<MessageRowSqlite>> {
        // Order by created_at then version_number so events that share a
        // sub-millisecond timestamp still have a stable order (matches the
        // production append order, since version_number is monotonic per
        // conversation).
        let rows = sqlx::query(
            "SELECT version_number, event_type, event_data, actor, created_at \
             FROM conversation_events \
             WHERE id = ?1 AND event_type IN ('user_message', 'assistant_message') \
             ORDER BY created_at ASC, version_number ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading message events for {conv_id}"))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let version_number: i64 = row.try_get("version_number")?;
            let event_type: String = row.try_get("event_type")?;
            let event_data: String = row.try_get("event_data")?;
            let actor: Option<String> = row.try_get("actor")?;
            let created_at_str: String = row.try_get("created_at")?;
            let created_at = parse_timestamp(&created_at_str).with_context(|| {
                format!("parsing conversation_events.created_at for {conv_id} v{version_number}")
            })?;
            out.push(MessageRowSqlite {
                source_version_number: version_number,
                event_type,
                event_data,
                actor,
                created_at,
            });
        }
        Ok(out)
    }

    async fn insert_row(pool: &SqlitePool, session_id: &str, row: &MessageRowSqlite) -> Result<()> {
        // Atomic: compute next version inside the same transaction as the
        // INSERT to avoid races with concurrent appenders (the production
        // dual-write path opens its own append on a different session, but
        // we still want a clean serialization for repeated migrations).
        let mut tx = pool.begin().await.context("begin tx")?;
        let next_version: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version_number), 0) + 1 \
             FROM session_events WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("computing next version_number for {session_id}"))?;

        // Preserve the source row's `created_at` on the target row so
        // forensic replay / spot-checks can match historical timestamps.
        let created_at_iso = row
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        sqlx::query(
            "INSERT INTO session_events \
                 (session_id, version_number, event_type, event_data, actor, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(session_id, version_number) DO NOTHING",
        )
        .bind(session_id)
        .bind(next_version)
        .bind(&row.event_type)
        .bind(&row.event_data)
        .bind(&row.actor)
        .bind(&created_at_iso)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "inserting session_events row for {session_id} v{next_version} \
                 (source conv v{src})",
                src = row.source_version_number
            )
        })?;
        tx.commit().await.context("commit tx")?;
        Ok(())
    }

    fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
        // Mirror the format set the production sqlite store accepts (see
        // `parse_sqlite_timestamp` in `store/sqlite_store.rs`).
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|_| {
                DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%:z")
                    .map(|dt| dt.with_timezone(&Utc))
            })
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                    .map(|ndt| ndt.and_utc())
            })
            .map_err(|e| anyhow!("failed to parse timestamp '{s}': {e}"))
    }
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use anyhow::Context;
    use sqlx::{PgPool, Row};

    pub async fn run(pool: &PgPool) -> Result<()> {
        let conv_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT conversation_id FROM metis.conversation_events_v2 \
             WHERE event_type IN ('user_message', 'assistant_message') \
             ORDER BY conversation_id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing conversations with message events")?;

        for conv_id in conv_ids {
            process_conversation(pool, &conv_id).await?;
        }
        Ok(())
    }

    async fn process_conversation(pool: &PgPool, conv_id: &str) -> Result<()> {
        let sessions = load_sessions_in_chain(pool, conv_id).await?;
        let rows = load_message_rows(pool, conv_id).await?;

        if sessions.is_empty() {
            if !rows.is_empty() {
                tracing::warn!(
                    conversation_id = %conv_id,
                    dropped = rows.len(),
                    "events migration: conversation has no linked sessions; dropping message events",
                );
            }
            return Ok(());
        }
        let windows = build_windows(&sessions);

        let mut per_session: HashMap<String, Vec<&MessageRowPostgres>> = HashMap::new();
        for row in &rows {
            let target = assign_event(&windows, row.created_at);
            per_session.entry(target.to_string()).or_default().push(row);
        }

        for (session_id, rows) in &per_session {
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM metis.session_events_v2 WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_one(pool)
            .await
            .with_context(|| format!("checking existing session_events_v2 for {session_id}"))?;

            if existing > 0 {
                continue;
            }

            for row in rows {
                insert_row(pool, session_id, row).await?;
            }
        }

        Ok(())
    }

    async fn load_sessions_in_chain(pool: &PgPool, conv_id: &str) -> Result<Vec<SessionInChain>> {
        let session_rows: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, creation_time FROM metis.tasks_v2 \
             WHERE conversation_id = $1 AND is_latest = TRUE AND deleted = FALSE \
             ORDER BY creation_time ASC, id ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading linked sessions for {conv_id}"))?;

        let mut sessions: Vec<SessionInChain> = session_rows
            .into_iter()
            .map(|(id, creation_time)| SessionInChain {
                id,
                creation_time,
                suspend_or_close_ts: None,
            })
            .collect();

        let boundary_rows = sqlx::query(
            "SELECT actor, created_at FROM metis.conversation_events_v2 \
             WHERE conversation_id = $1 AND event_type IN ('suspending', 'closed') \
             ORDER BY version_number ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading boundary events for {conv_id}"))?;

        let mut earliest: HashMap<String, DateTime<Utc>> = HashMap::new();
        for row in boundary_rows {
            let actor: Option<serde_json::Value> = row.try_get("actor")?;
            let created_at: DateTime<Utc> = row.try_get("created_at")?;
            let Some(actor_value) = actor else { continue };
            let Some(session_id) = session_id_from_actor_json(&actor_value.to_string()) else {
                continue;
            };
            earliest.entry(session_id).or_insert(created_at);
        }

        for s in &mut sessions {
            s.suspend_or_close_ts = earliest.get(&s.id).copied();
        }
        Ok(sessions)
    }

    async fn load_message_rows(pool: &PgPool, conv_id: &str) -> Result<Vec<MessageRowPostgres>> {
        let rows = sqlx::query(
            "SELECT version_number, event_type, event_data, actor, created_at \
             FROM metis.conversation_events_v2 \
             WHERE conversation_id = $1 \
               AND event_type IN ('user_message', 'assistant_message') \
             ORDER BY created_at ASC, version_number ASC",
        )
        .bind(conv_id)
        .fetch_all(pool)
        .await
        .with_context(|| format!("loading message events for {conv_id}"))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let version_number: i64 = row.try_get("version_number")?;
            let event_type: String = row.try_get("event_type")?;
            let event_data: serde_json::Value = row.try_get("event_data")?;
            let actor: Option<serde_json::Value> = row.try_get("actor")?;
            let created_at: DateTime<Utc> = row.try_get("created_at")?;
            out.push(MessageRowPostgres {
                source_version_number: version_number,
                event_type,
                event_data,
                actor,
                created_at,
            });
        }
        Ok(out)
    }

    async fn insert_row(pool: &PgPool, session_id: &str, row: &MessageRowPostgres) -> Result<()> {
        let mut tx = pool.begin().await.context("begin tx")?;
        // Lock existing rows for this session to serialize concurrent
        // appenders (mirrors the FOR UPDATE pattern in
        // `append_session_event` in `ee/store/postgres_v2.rs`).
        let _lock: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM metis.session_events_v2 WHERE session_id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await
        .with_context(|| format!("locking session_events_v2 for {session_id}"))?;

        let next_version: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version_number), 0) + 1 \
             FROM metis.session_events_v2 WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .with_context(|| format!("computing next version_number for {session_id}"))?;

        sqlx::query(
            "INSERT INTO metis.session_events_v2 \
                 (session_id, version_number, event_type, event_data, actor, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (session_id, version_number) DO NOTHING",
        )
        .bind(session_id)
        .bind(next_version)
        .bind(&row.event_type)
        .bind(&row.event_data)
        .bind(&row.actor)
        .bind(row.created_at)
        .execute(&mut *tx)
        .await
        .with_context(|| {
            format!(
                "inserting metis.session_events_v2 row for {session_id} v{next_version} \
                 (source conv v{src})",
                src = row.source_version_number
            )
        })?;
        tx.commit().await.context("commit tx")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SQLite integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sqlite_store::SqliteStore;
    use chrono::Duration;
    use hydra_common::{ConversationId, SessionId};
    use sqlx::SqlitePool;

    async fn fresh_pool() -> SqlitePool {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();
        pool
    }

    /// Minimal conversation row required for the migration tool to find the
    /// conversation (the migrate-events pass only reads events + sessions,
    /// not the conversation row itself, but the production schema keeps the
    /// FK alive, so we mirror that here for fidelity).
    async fn insert_conversation(pool: &SqlitePool, conv_id: &ConversationId, creator: &str) {
        sqlx::query(
            "INSERT INTO conversations \
                (id, version_number, status, creator, deleted, is_latest) \
             VALUES (?1, 1, 'active', ?2, 0, 1)",
        )
        .bind(conv_id.as_ref())
        .bind(creator)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_session(
        pool: &SqlitePool,
        session_id: &SessionId,
        conv_id: &ConversationId,
        creator: &str,
        creation_time: DateTime<Utc>,
    ) {
        // Phase E step 16 dropped `prompt` / `context` / `interactive`;
        // hand-crafted rows now only populate the surviving columns
        // (mount_spec / agent_config / mode / conversation_id).
        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, creator, image, env_vars, \
                 status, deleted, creation_time, conversation_id, \
                 mount_spec, agent_config, mode, is_latest) \
             VALUES (?1, 1, ?2, NULL, '{}', \
                     'complete', 0, ?3, ?4, \
                     '{\"working_dir\":\"repo\",\"mounts\":[]}', \
                     '{}', \
                     json_object('type', 'interactive', 'conversation_id', ?4), \
                     1)",
        )
        .bind(session_id.as_ref())
        .bind(creator)
        .bind(creation_time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .bind(conv_id.as_ref())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_message_event(
        pool: &SqlitePool,
        conv_id: &ConversationId,
        version: i64,
        kind: &str, // "user_message" | "assistant_message"
        content: &str,
        timestamp: DateTime<Utc>,
        created_at: DateTime<Utc>,
    ) {
        let event_data = serde_json::json!({
            "type": kind,
            "content": content,
            "timestamp": timestamp.to_rfc3339(),
        })
        .to_string();
        sqlx::query(
            "INSERT INTO conversation_events \
                (id, version_number, event_type, event_data, actor, created_at) \
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        )
        .bind(conv_id.as_ref())
        .bind(version)
        .bind(kind)
        .bind(event_data)
        .bind(created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_suspending_event(
        pool: &SqlitePool,
        conv_id: &ConversationId,
        version: i64,
        actor_session: &SessionId,
        created_at: DateTime<Utc>,
    ) {
        // Write the pre-cleanup `{"Session":"s-..."}` shape directly so
        // `session_id_from_actor_json`'s dual-shape reader exercises
        // the same path the migration_roundtrip baseline produces. The
        // post-cleanup shape is exercised by the unit tests below.
        let actor_json = serde_json::json!({
            "Authenticated": { "actor_id": { "Session": actor_session.as_ref() } }
        })
        .to_string();
        let event_data = serde_json::json!({
            "type": "suspending",
            "reason": "test",
            "timestamp": created_at.to_rfc3339(),
        })
        .to_string();
        sqlx::query(
            "INSERT INTO conversation_events \
                (id, version_number, event_type, event_data, actor, created_at) \
             VALUES (?1, ?2, 'suspending', ?3, ?4, ?5)",
        )
        .bind(conv_id.as_ref())
        .bind(version)
        .bind(event_data)
        .bind(actor_json)
        .bind(created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .execute(pool)
        .await
        .unwrap();
    }

    /// Returns the per-session sequence of `(version_number, event_type,
    /// event_data, created_at)` written to `session_events`.
    async fn read_session_events(
        pool: &SqlitePool,
        session_id: &SessionId,
    ) -> Vec<(i64, String, String, String)> {
        sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT version_number, event_type, event_data, created_at \
             FROM session_events WHERE session_id = ?1 ORDER BY version_number ASC",
        )
        .bind(session_id.as_ref())
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn single_session_chain_assigns_all_events_to_one_session() {
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess = SessionId::new();
        let t0 = Utc::now() - Duration::hours(2);
        insert_conversation(&pool, &conv, "alice").await;
        insert_session(&pool, &sess, &conv, "alice", t0).await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "hi",
            t0 + Duration::minutes(1),
            t0 + Duration::minutes(1),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            2,
            "assistant_message",
            "hello",
            t0 + Duration::minutes(2),
            t0 + Duration::minutes(2),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            3,
            "user_message",
            "bye",
            t0 + Duration::minutes(3),
            t0 + Duration::minutes(3),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();
        let written = read_session_events(&pool, &sess).await;
        assert_eq!(written.len(), 3);
        assert_eq!(written[0].0, 1);
        assert_eq!(written[1].0, 2);
        assert_eq!(written[2].0, 3);
        // Source created_at is preserved on the target row.
        let t1 = (t0 + Duration::minutes(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        assert_eq!(written[0].3, t1);
        assert_eq!(written[0].1, "user_message");
        assert_eq!(written[1].1, "assistant_message");
    }

    #[tokio::test]
    async fn multi_session_chain_partitions_on_suspend_resume_boundaries() {
        // A: created at t0, suspends at t0+10m
        // B: created at t0+10m (resumes from A), suspends at t0+20m
        // C: created at t0+20m (resumes from B), still active
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let sess_c = SessionId::new();
        let t0 = Utc::now() - Duration::hours(2);
        let t_a_start = t0;
        let t_a_suspend = t0 + Duration::minutes(10);
        let t_b_start = t0 + Duration::minutes(10);
        let t_b_suspend = t0 + Duration::minutes(20);
        let t_c_start = t0 + Duration::minutes(20);

        insert_conversation(&pool, &conv, "bob").await;
        insert_session(&pool, &sess_a, &conv, "bob", t_a_start).await;
        insert_session(&pool, &sess_b, &conv, "bob", t_b_start).await;
        insert_session(&pool, &sess_c, &conv, "bob", t_c_start).await;

        // A's events.
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "a1",
            t0 + Duration::minutes(1),
            t0 + Duration::minutes(1),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            2,
            "assistant_message",
            "a2",
            t0 + Duration::minutes(2),
            t0 + Duration::minutes(2),
        )
        .await;
        // A suspends.
        insert_suspending_event(&pool, &conv, 3, &sess_a, t_a_suspend).await;
        // B's events.
        insert_message_event(
            &pool,
            &conv,
            4,
            "user_message",
            "b1",
            t0 + Duration::minutes(11),
            t0 + Duration::minutes(11),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            5,
            "assistant_message",
            "b2",
            t0 + Duration::minutes(12),
            t0 + Duration::minutes(12),
        )
        .await;
        // B suspends.
        insert_suspending_event(&pool, &conv, 6, &sess_b, t_b_suspend).await;
        // C's events.
        insert_message_event(
            &pool,
            &conv,
            7,
            "user_message",
            "c1",
            t0 + Duration::minutes(21),
            t0 + Duration::minutes(21),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            8,
            "assistant_message",
            "c2",
            t0 + Duration::minutes(22),
            t0 + Duration::minutes(22),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();

        // session_events for each session: 2 rows each, monotonic versions 1..=2.
        for sess in [&sess_a, &sess_b, &sess_c] {
            let evs = read_session_events(&pool, sess).await;
            assert_eq!(evs.len(), 2);
            assert_eq!(evs[0].0, 1);
            assert_eq!(evs[1].0, 2);
        }
    }

    #[tokio::test]
    async fn overlapping_sessions_break_ties_on_creation_time_order() {
        // A's window: [t0, t0+1ms). B's window: [t0+1ms, +∞).
        // Event at t0+500us → A. Event at t0+1ms → B. Event at t0+2ms → B.
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let t0 = Utc::now() - Duration::hours(2);
        let t_b = t0 + Duration::milliseconds(1);

        insert_conversation(&pool, &conv, "carol").await;
        insert_session(&pool, &sess_a, &conv, "carol", t0).await;
        insert_session(&pool, &sess_b, &conv, "carol", t_b).await;

        // Note: sqlite RFC3339 round-trip is millisecond-resolution, so
        // sub-ms event timestamps would all collide at the same DB value.
        // We use whole milliseconds and rely on creation-time ordering to
        // break ties — `B.creation_time == event.created_at` lands in B's
        // window (start-inclusive), per the half-open `[start, end)` rule.
        insert_message_event(&pool, &conv, 1, "user_message", "early", t0, t0).await;
        insert_message_event(&pool, &conv, 2, "assistant_message", "on-tie", t_b, t_b).await;
        insert_message_event(
            &pool,
            &conv,
            3,
            "user_message",
            "after",
            t_b + Duration::milliseconds(1),
            t_b + Duration::milliseconds(1),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();
        let a_events = read_session_events(&pool, &sess_a).await;
        let b_events = read_session_events(&pool, &sess_b).await;
        assert_eq!(a_events.len(), 1, "A owns the early row");
        assert_eq!(b_events.len(), 2, "B owns the on-tie and after rows");
    }

    #[tokio::test]
    async fn last_session_suspended_with_no_resume_owns_post_suspend_rows() {
        // A → suspend → B (still suspended, no successor).
        // A post-suspend message on B (anomalous but must not panic).
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let t0 = Utc::now() - Duration::hours(2);
        let t_a_suspend = t0 + Duration::minutes(5);
        let t_b_start = t0 + Duration::minutes(5);
        let t_b_suspend = t0 + Duration::minutes(15);

        insert_conversation(&pool, &conv, "dave").await;
        insert_session(&pool, &sess_a, &conv, "dave", t0).await;
        insert_session(&pool, &sess_b, &conv, "dave", t_b_start).await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "a1",
            t0 + Duration::minutes(1),
            t0 + Duration::minutes(1),
        )
        .await;
        insert_suspending_event(&pool, &conv, 2, &sess_a, t_a_suspend).await;
        insert_message_event(
            &pool,
            &conv,
            3,
            "user_message",
            "b1",
            t0 + Duration::minutes(7),
            t0 + Duration::minutes(7),
        )
        .await;
        insert_suspending_event(&pool, &conv, 4, &sess_b, t_b_suspend).await;
        // Anomalous post-suspend row on the LAST session — edge case 4.
        insert_message_event(
            &pool,
            &conv,
            5,
            "assistant_message",
            "ghost",
            t_b_suspend + Duration::minutes(1),
            t_b_suspend + Duration::minutes(1),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();

        let a_rows = read_session_events(&pool, &sess_a).await;
        let b_rows = read_session_events(&pool, &sess_b).await;
        assert_eq!(a_rows.len(), 1, "A owns the single pre-suspend row");
        assert_eq!(b_rows.len(), 2, "B owns its message + the ghost row");
    }

    #[tokio::test]
    async fn repeat_run_is_idempotent() {
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess = SessionId::new();
        let t0 = Utc::now() - Duration::hours(1);
        insert_conversation(&pool, &conv, "erin").await;
        insert_session(&pool, &sess, &conv, "erin", t0).await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "one",
            t0 + Duration::minutes(1),
            t0 + Duration::minutes(1),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            2,
            "assistant_message",
            "two",
            t0 + Duration::minutes(2),
            t0 + Duration::minutes(2),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());

        run(&backend).await.unwrap();
        let first = read_session_events(&pool, &sess).await;
        assert_eq!(first.len(), 2);

        // Second run is a no-op — the per-session skip rule sees the
        // existing rows and bails out before any further INSERT.
        run(&backend).await.unwrap();
        let second = read_session_events(&pool, &sess).await;
        assert_eq!(second, first, "repeat run must not duplicate rows");
    }

    #[tokio::test]
    async fn gap_event_assigned_to_subsequent_session() {
        // A suspends at t0+5m. B doesn't start until t0+10m.
        // A message row at t0+7m falls in the [5m, 10m) gap — under the
        // tolerant rule it lands on B (the subsequent session), not A.
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let t0 = Utc::now() - Duration::hours(1);
        insert_conversation(&pool, &conv, "grace").await;
        insert_session(&pool, &sess_a, &conv, "grace", t0).await;
        insert_session(&pool, &sess_b, &conv, "grace", t0 + Duration::minutes(10)).await;
        insert_suspending_event(&pool, &conv, 1, &sess_a, t0 + Duration::minutes(5)).await;
        insert_message_event(
            &pool,
            &conv,
            2,
            "user_message",
            "orphan",
            t0 + Duration::minutes(7),
            t0 + Duration::minutes(7),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();
        let a_rows = read_session_events(&pool, &sess_a).await;
        let b_rows = read_session_events(&pool, &sess_b).await;
        assert!(
            a_rows.is_empty(),
            "A must not own the gap event: {a_rows:?}"
        );
        assert_eq!(b_rows.len(), 1, "B owns the gap event: {b_rows:?}");
        assert_eq!(b_rows[0].1, "user_message");
    }

    #[tokio::test]
    async fn missing_linked_session_drops_events_silently() {
        // A conversation with message events but no tasks_v2 rows. The
        // migration must succeed (not bail), leaving session_events empty.
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        insert_conversation(&pool, &conv, "henry").await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "lonely",
            Utc::now(),
            Utc::now(),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "no sessions exist for this conversation; no rows should be written",
        );
    }

    #[tokio::test]
    async fn event_before_all_sessions_assigned_to_first_session() {
        // Sessions A (created t0+10m) and B (created t0+20m). A message at
        // t0+5m predates both. Under the tolerant rule it lands on A (the
        // first session), not on a "no window" failure.
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess_a = SessionId::new();
        let sess_b = SessionId::new();
        let t0 = Utc::now() - Duration::hours(1);
        insert_conversation(&pool, &conv, "ivy").await;
        insert_session(&pool, &sess_a, &conv, "ivy", t0 + Duration::minutes(10)).await;
        insert_session(&pool, &sess_b, &conv, "ivy", t0 + Duration::minutes(20)).await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "early-bird",
            t0 + Duration::minutes(5),
            t0 + Duration::minutes(5),
        )
        .await;

        let backend = Backend::Sqlite(pool.clone());
        run(&backend).await.unwrap();
        let a_rows = read_session_events(&pool, &sess_a).await;
        let b_rows = read_session_events(&pool, &sess_b).await;
        assert_eq!(a_rows.len(), 1, "A owns the pre-A event: {a_rows:?}");
        assert!(b_rows.is_empty(), "B must not own anything: {b_rows:?}");
        assert_eq!(a_rows[0].1, "user_message");
    }

    // --- session_id_from_actor_json: both wire shapes must work ---

    #[test]
    fn session_id_reader_accepts_precleanup_session_shape() {
        // `{"Authenticated":{"actor_id":{"Session":"s-abcdef"}}}` —
        // what events migration sees on a fresh DB before the
        // `actor_variant_cleanup` migration rewrites the row.
        let raw = serde_json::json!({
            "Authenticated": { "actor_id": { "Session": "s-abcdef" } }
        })
        .to_string();
        assert_eq!(
            super::session_id_from_actor_json(&raw),
            Some("s-abcdef".to_string())
        );
    }

    #[test]
    fn session_id_reader_accepts_postcleanup_adhoc_shape() {
        // `{"Authenticated":{"actor_id":{"Adhoc":{"session_id":"s-abcdef"}}}}`
        // — the wire shape `actor_variant_cleanup` rewrites to. On a
        // soaked deploy the events migration may run against rows
        // that were rewritten by a prior cleanup pass.
        let raw = serde_json::json!({
            "Authenticated": {
                "actor_id": { "Adhoc": { "session_id": "s-abcdef" } }
            }
        })
        .to_string();
        assert_eq!(
            super::session_id_from_actor_json(&raw),
            Some("s-abcdef".to_string())
        );
    }

    #[test]
    fn session_id_reader_returns_none_for_non_session_actors() {
        // User / Agent / External / System / Automation actors don't
        // belong to a single session — the reader must skip them.
        for raw in [
            r#"{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}"#,
            r#"{"Authenticated":{"actor_id":{"Agent":{"name":"swe"}}}}"#,
            r#"{"System":{"worker_name":"bg","on_behalf_of":null}}"#,
        ] {
            assert_eq!(super::session_id_from_actor_json(raw), None);
        }
    }

    #[test]
    fn session_id_reader_returns_none_for_malformed_input() {
        // Anything that doesn't parse as JSON, or doesn't match the
        // `Authenticated.actor_id.<tag>` shape, returns None.
        assert_eq!(super::session_id_from_actor_json("not-json"), None);
        assert_eq!(super::session_id_from_actor_json("{}"), None);
        assert_eq!(
            super::session_id_from_actor_json(r#"{"Authenticated":{"actor_id":{}}}"#),
            None
        );
        assert_eq!(
            super::session_id_from_actor_json(r#"{"Authenticated":{"actor_id":{"Session":42}}}"#),
            None
        );
    }
}
