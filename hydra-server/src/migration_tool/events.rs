//! `migrate-events` pass: partition historical `conversation_events_v2`
//! user/assistant message rows by "active session at write time" and write
//! them to `session_events` (sqlite) / `metis.session_events_v2` (postgres).
//!
//! ## Partitioning rule (per §3.5 step 3 of
//! `designs/sessions-orthogonality-redesign.md`)
//!
//! For each conversation, walk the linked sessions in `creation_time` order.
//! Each session owns the half-open window
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
//! Each `user_message` / `assistant_message` row is assigned to the session
//! whose window contains the row's `created_at`. We skip `suspending`,
//! `resumed`, and `closed` rows (design §3.5 step 5 leaves them on
//! `conversation_events*`).
//!
//! ## Edge case — last session in chain is suspended but never resumed
//!
//! The literal "earlier of" rule above would cut the last session's window
//! at its `Suspending` timestamp, leaving any post-suspend message rows
//! unassigned. Such rows shouldn't exist in practice (a worker doesn't
//! emit messages after suspending), but the issue spec requires the tool
//! to "not panic". So we override: when the LAST session in the chain has
//! a `Suspending` / `Closed` event AND no successor session, we extend its
//! window to `+∞` so any stray rows still land somewhere instead of failing.
//!
//! Other gap scenarios (e.g., middle-session `Suspending` at `T1`,
//! next-session creation at `T2 > T1`, with a stray row in `[T1, T2)`)
//! fail loud — per the SWE note, "prefer failing-loud if a row can't be
//! assigned (e.g., no session active in the window) over guessing."
//!
//! ## Idempotency
//!
//! Re-running must be a no-op. The target table's primary key is
//! `(session_id, version_number)` but `version_number` is per-session
//! monotonic and assigned at insert time, so we can't match source rows to
//! target rows by version alone. Instead we check, per target session:
//! if `session_events*` already has any rows for that session, we skip the
//! whole session (every plan entry for it becomes `skipped` / `would-skip`).
//! This is conservative — it relies on the operator running this pass
//! BEFORE enabling dual-writes (PR-1, `i-aankjvnz`), so the only existing
//! rows on a re-run came from a previous run of this same tool.
//!
//! ## `--up-to` cut-over
//!
//! With `--up-to <T>` only rows whose `created_at < T` are migrated;
//! anything `>= T` is left for the dual-write path. Without the flag,
//! every message row is processed.

use super::Backend;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hydra_common::{ActorId, ActorRef};
use std::collections::HashMap;

/// Status of a single `conversation_events_v2` row in the migration plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventPlanAction {
    /// Dry-run: a write that *would* happen if we re-ran without `--dry-run`.
    WouldWrite,
    /// Dry-run: the target session already has `session_events*` rows; would
    /// be skipped per the idempotency rule.
    WouldSkip,
    /// Live run: row was inserted into `session_events*`.
    Wrote,
    /// Live run: skipped because the target session already had rows.
    Skipped,
}

/// One row of the migrate-events plan. Serialized as JSON Lines on stdout.
///
/// Field order matches the issue spec: greppable
/// `(source_conversation_id, source_event_id, target_session_id, source_created_at)`.
/// `source_version_number` is the per-conversation event id (the
/// `(conversation_id, version_number)` unique key on `conversation_events*`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventPlanEntry {
    pub source_conversation_id: String,
    pub source_version_number: i64,
    pub target_session_id: String,
    pub source_created_at: DateTime<Utc>,
    pub action: EventPlanAction,
}

/// Run the migrate-events pass against `backend`. With `dry_run = true`
/// no writes happen and every plan entry is a `would-*`. With `dry_run =
/// false`, rows are appended to `session_events*` (skipping any whose
/// target session already has rows, so re-runs are no-ops). With
/// `up_to = Some(t)`, only source rows whose `created_at < t` are
/// processed.
pub async fn run(
    backend: &Backend,
    dry_run: bool,
    up_to: Option<DateTime<Utc>>,
) -> Result<Vec<EventPlanEntry>> {
    match backend {
        Backend::Sqlite(pool) => sqlite::run(pool, dry_run, up_to).await,
        #[cfg(feature = "postgres")]
        Backend::Postgres(pool) => postgres::run(pool, dry_run, up_to).await,
    }
}

/// Emit a single plan entry to stdout as one JSON line.
pub fn emit_jsonl(entry: &EventPlanEntry) -> Result<()> {
    let line = serde_json::to_string(entry).context("serializing plan entry")?;
    println!("{line}");
    Ok(())
}

/// Extract the session id from a stored `ActorRef` JSON value, if the
/// actor is an authenticated session. Other actor shapes (service tokens,
/// automation triggers) return `None`.
fn session_id_from_actor_json(actor: &str) -> Option<String> {
    let parsed: ActorRef = serde_json::from_str(actor).ok()?;
    match parsed {
        ActorRef::Authenticated {
            actor_id: ActorId::Session(session_id),
        } => Some(session_id.as_ref().to_string()),
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
/// overlapping sessions (issue spec edge case 3).
fn build_windows(sessions: &[SessionInChain]) -> Vec<Window> {
    let mut windows = Vec::with_capacity(sessions.len());
    for (i, s) in sessions.iter().enumerate() {
        let next_creation = sessions.get(i + 1).map(|n| n.creation_time);
        let is_last = i + 1 == sessions.len();
        let end = match (s.suspend_or_close_ts, next_creation) {
            // Edge case 4: last session is suspended-and-never-resumed.
            // Extend the window to +∞ so stray post-suspend rows still
            // land on the suspended session instead of failing loud.
            (Some(_), None) if is_last => None,
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

/// Assign `event_ts` to the first window that contains it, or `None` if
/// no window matches.
fn assign_to_window(windows: &[Window], event_ts: DateTime<Utc>) -> Option<&str> {
    windows
        .iter()
        .find(|w| w.start <= event_ts && w.end.is_none_or(|e| event_ts < e))
        .map(|w| w.session_id.as_str())
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

    pub async fn run(
        pool: &SqlitePool,
        dry_run: bool,
        up_to: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventPlanEntry>> {
        // Conversations to process: any with at least one message event.
        let conv_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT id FROM conversation_events \
             WHERE event_type IN ('user_message', 'assistant_message') \
             ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing conversations with message events")?;

        let mut plan = Vec::new();
        for conv_id in conv_ids {
            let entries = process_conversation(pool, &conv_id, dry_run, up_to).await?;
            plan.extend(entries);
        }
        Ok(plan)
    }

    async fn process_conversation(
        pool: &SqlitePool,
        conv_id: &str,
        dry_run: bool,
        up_to: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventPlanEntry>> {
        let sessions = load_sessions_in_chain(pool, conv_id).await?;
        if sessions.is_empty() {
            // No linked sessions but messages exist — fail loud, don't
            // silently drop history.
            anyhow::bail!(
                "conversation {conv_id} has message events in conversation_events but no \
                 linked session in tasks_v2 — cannot partition; investigate manually"
            );
        }
        let windows = build_windows(&sessions);

        let rows = load_message_rows(pool, conv_id).await?;

        let mut plan = Vec::with_capacity(rows.len());
        // Map session_id -> Vec of (plan-index, row), preserving the
        // source-order traversal so we insert in created_at order per session.
        let mut per_session: HashMap<String, Vec<usize>> = HashMap::new();

        for row in &rows {
            if let Some(cutoff) = up_to
                && row.created_at >= cutoff
            {
                // Honor --up-to cut-over: leave the row to the dual-write path.
                continue;
            }

            let target = assign_to_window(&windows, row.created_at).ok_or_else(|| {
                anyhow!(
                    "conversation {conv_id} version {ver} (created_at {ts}) falls outside every \
                     linked session's window — refuse to guess; fix the data or extend the \
                     partitioning rule",
                    ver = row.source_version_number,
                    ts = row.created_at,
                )
            })?;

            let idx = plan.len();
            plan.push(EventPlanEntry {
                source_conversation_id: conv_id.to_string(),
                source_version_number: row.source_version_number,
                target_session_id: target.to_string(),
                source_created_at: row.created_at,
                action: EventPlanAction::WouldWrite, // placeholder; set below
            });
            per_session.entry(target.to_string()).or_default().push(idx);
        }

        // Per-session idempotency check + insert.
        for (session_id, indices) in &per_session {
            let existing: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM session_events WHERE session_id = ?1")
                    .bind(session_id)
                    .fetch_one(pool)
                    .await
                    .with_context(|| {
                        format!("checking existing session_events for {session_id}")
                    })?;

            let session_has_data = existing > 0;
            for &idx in indices {
                let entry = &mut plan[idx];
                let row = rows
                    .iter()
                    .find(|r| r.source_version_number == entry.source_version_number)
                    .expect("plan entry must trace back to a loaded row");

                entry.action = if session_has_data {
                    if dry_run {
                        EventPlanAction::WouldSkip
                    } else {
                        EventPlanAction::Skipped
                    }
                } else if dry_run {
                    EventPlanAction::WouldWrite
                } else {
                    insert_row(pool, session_id, row).await?;
                    EventPlanAction::Wrote
                };
            }
        }

        Ok(plan)
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
    use anyhow::{Context, anyhow};
    use sqlx::{PgPool, Row};

    pub async fn run(
        pool: &PgPool,
        dry_run: bool,
        up_to: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventPlanEntry>> {
        let conv_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT conversation_id FROM metis.conversation_events_v2 \
             WHERE event_type IN ('user_message', 'assistant_message') \
             ORDER BY conversation_id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing conversations with message events")?;

        let mut plan = Vec::new();
        for conv_id in conv_ids {
            let entries = process_conversation(pool, &conv_id, dry_run, up_to).await?;
            plan.extend(entries);
        }
        Ok(plan)
    }

    async fn process_conversation(
        pool: &PgPool,
        conv_id: &str,
        dry_run: bool,
        up_to: Option<DateTime<Utc>>,
    ) -> Result<Vec<EventPlanEntry>> {
        let sessions = load_sessions_in_chain(pool, conv_id).await?;
        if sessions.is_empty() {
            anyhow::bail!(
                "conversation {conv_id} has message events in metis.conversation_events_v2 \
                 but no linked session in metis.tasks_v2 — cannot partition; investigate \
                 manually"
            );
        }
        let windows = build_windows(&sessions);

        let rows = load_message_rows(pool, conv_id).await?;

        let mut plan = Vec::with_capacity(rows.len());
        let mut per_session: HashMap<String, Vec<usize>> = HashMap::new();

        for row in &rows {
            if let Some(cutoff) = up_to
                && row.created_at >= cutoff
            {
                continue;
            }

            let target = assign_to_window(&windows, row.created_at).ok_or_else(|| {
                anyhow!(
                    "conversation {conv_id} version {ver} (created_at {ts}) falls outside every \
                     linked session's window — refuse to guess; fix the data or extend the \
                     partitioning rule",
                    ver = row.source_version_number,
                    ts = row.created_at,
                )
            })?;

            let idx = plan.len();
            plan.push(EventPlanEntry {
                source_conversation_id: conv_id.to_string(),
                source_version_number: row.source_version_number,
                target_session_id: target.to_string(),
                source_created_at: row.created_at,
                action: EventPlanAction::WouldWrite,
            });
            per_session.entry(target.to_string()).or_default().push(idx);
        }

        for (session_id, indices) in &per_session {
            let existing: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM metis.session_events_v2 WHERE session_id = $1",
            )
            .bind(session_id)
            .fetch_one(pool)
            .await
            .with_context(|| format!("checking existing session_events_v2 for {session_id}"))?;

            let session_has_data = existing > 0;
            for &idx in indices {
                let entry = &mut plan[idx];
                let row = rows
                    .iter()
                    .find(|r| r.source_version_number == entry.source_version_number)
                    .expect("plan entry must trace back to a loaded row");

                entry.action = if session_has_data {
                    if dry_run {
                        EventPlanAction::WouldSkip
                    } else {
                        EventPlanAction::Skipped
                    }
                } else if dry_run {
                    EventPlanAction::WouldWrite
                } else {
                    insert_row(pool, session_id, row).await?;
                    EventPlanAction::Wrote
                };
            }
        }

        Ok(plan)
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
    use hydra_common::{ActorId, ActorRef, ConversationId, SessionId};
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
        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, prompt, context, creator, image, env_vars, \
                 status, deleted, creation_time, interactive, conversation_id, is_latest) \
             VALUES (?1, 1, '', '{\"type\":\"none\"}', ?2, NULL, '{}', \
                     'complete', 0, ?3, 1, ?4, 1)",
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
        let actor = ActorRef::Authenticated {
            actor_id: ActorId::Session(actor_session.clone()),
        };
        let actor_json = serde_json::to_string(&actor).unwrap();
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
        let plan = run(&backend, false, None).await.unwrap();
        assert_eq!(plan.len(), 3);
        for entry in &plan {
            assert_eq!(entry.target_session_id, *sess.as_ref());
            assert_eq!(entry.action, EventPlanAction::Wrote);
        }
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
        let plan = run(&backend, false, None).await.unwrap();
        assert_eq!(plan.len(), 6, "6 message rows, 2 suspending rows skipped");

        let by_session = |id: &SessionId| {
            plan.iter()
                .filter(|p| p.target_session_id == *id.as_ref())
                .map(|p| p.source_version_number)
                .collect::<Vec<_>>()
        };
        assert_eq!(by_session(&sess_a), vec![1, 2]);
        assert_eq!(by_session(&sess_b), vec![4, 5]);
        assert_eq!(by_session(&sess_c), vec![7, 8]);

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
        let plan = run(&backend, false, None).await.unwrap();
        let targets: Vec<_> = plan.iter().map(|p| p.target_session_id.clone()).collect();
        assert_eq!(
            targets,
            vec![
                sess_a.as_ref().to_string(),
                sess_b.as_ref().to_string(),
                sess_b.as_ref().to_string()
            ]
        );
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
        let plan = run(&backend, false, None).await.unwrap();
        // 3 message rows: 1 to A, 2 to B (including ghost).
        let a_rows: Vec<_> = plan
            .iter()
            .filter(|p| p.target_session_id == *sess_a.as_ref())
            .map(|p| p.source_version_number)
            .collect();
        let b_rows: Vec<_> = plan
            .iter()
            .filter(|p| p.target_session_id == *sess_b.as_ref())
            .map(|p| p.source_version_number)
            .collect();
        assert_eq!(a_rows, vec![1]);
        assert_eq!(b_rows, vec![3, 5]);
    }

    #[tokio::test]
    async fn dry_run_matches_live_plan_and_is_idempotent() {
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

        let dry = run(&backend, true, None).await.unwrap();
        assert_eq!(dry.len(), 2);
        for entry in &dry {
            assert_eq!(entry.action, EventPlanAction::WouldWrite);
            assert_eq!(entry.target_session_id, *sess.as_ref());
        }
        assert!(read_session_events(&pool, &sess).await.is_empty());

        let live = run(&backend, false, None).await.unwrap();
        assert_eq!(live.len(), 2);
        for entry in &live {
            assert_eq!(entry.action, EventPlanAction::Wrote);
            assert_eq!(entry.target_session_id, *sess.as_ref());
        }
        // (source_version_number, target_session_id, source_created_at)
        // tuples agree between dry-run and live.
        let key = |e: &EventPlanEntry| {
            (
                e.source_version_number,
                e.target_session_id.clone(),
                e.source_created_at,
            )
        };
        assert_eq!(
            dry.iter().map(key).collect::<Vec<_>>(),
            live.iter().map(key).collect::<Vec<_>>(),
        );

        let rerun = run(&backend, false, None).await.unwrap();
        assert_eq!(rerun.len(), 2);
        for entry in &rerun {
            assert_eq!(entry.action, EventPlanAction::Skipped);
        }
        assert_eq!(read_session_events(&pool, &sess).await.len(), 2);
    }

    #[tokio::test]
    async fn up_to_cutoff_excludes_newer_rows() {
        let pool = fresh_pool().await;
        let conv = ConversationId::new();
        let sess = SessionId::new();
        // Round to millisecond precision up-front so the cutoff round-trips
        // through sqlite's RFC3339 storage without sub-ms drift skewing the
        // boundary-case assertion below.
        let t0 = DateTime::parse_from_rfc3339(
            &(Utc::now() - Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
        .unwrap()
        .with_timezone(&Utc);
        let cutoff = t0 + Duration::minutes(5);

        insert_conversation(&pool, &conv, "frank").await;
        insert_session(&pool, &sess, &conv, "frank", t0).await;
        insert_message_event(
            &pool,
            &conv,
            1,
            "user_message",
            "old",
            t0 + Duration::minutes(1),
            t0 + Duration::minutes(1),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            2,
            "assistant_message",
            "old2",
            t0 + Duration::minutes(4),
            t0 + Duration::minutes(4),
        )
        .await;
        insert_message_event(
            &pool,
            &conv,
            3,
            "user_message",
            "new",
            t0 + Duration::minutes(6),
            t0 + Duration::minutes(6),
        )
        .await;
        // Boundary case: created_at == cutoff is EXCLUDED (cutoff is
        // exclusive — dual-write owns anything at or after the cut-over).
        insert_message_event(&pool, &conv, 4, "assistant_message", "edge", cutoff, cutoff).await;

        let backend = Backend::Sqlite(pool.clone());
        let plan = run(&backend, false, Some(cutoff)).await.unwrap();
        assert_eq!(plan.len(), 2, "only versions 1 and 2 are < cutoff");
        let versions: Vec<i64> = plan.iter().map(|p| p.source_version_number).collect();
        assert_eq!(versions, vec![1, 2]);
        assert_eq!(read_session_events(&pool, &sess).await.len(), 2);
    }

    #[tokio::test]
    async fn fail_loud_when_row_falls_in_gap_between_sessions() {
        // A suspends at t0+5m. B doesn't start until t0+10m.
        // A message row at t0+7m falls in [5m, 10m) — no session active.
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
        let err = run(&backend, true, None).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("falls outside every linked session's window"),
            "expected fail-loud error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn missing_linked_session_for_message_events_fails_loud() {
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
        let err = run(&backend, true, None).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no linked session in tasks_v2"),
            "expected linked-session fail-loud error, got: {msg}"
        );
    }
}
