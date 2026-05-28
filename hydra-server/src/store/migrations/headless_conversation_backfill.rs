//! Per-row backfill that materializes a `conversations_v2` row + a first
//! `session_events_v2 { event_type: 'user_message' }` row for every
//! pre-existing `tasks_v2` row whose `mode` JSON is still in the
//! pre-PR-2 `Headless { prompt }` shape, and rewrites the `mode` JSON
//! to add `conversation_id` while preserving `prompt` for the duration
//! of the PR-2/PR-3 transition (the field is dropped from the Rust
//! type in PR-3).
//!
//! ## Why a Rust migration rather than pure SQL
//!
//! Each headless row needs three coordinated writes plus a fresh
//! identifier (`conversation_id`). SQLite's `json_set` can rewrite the
//! `mode` column in place, but generating a unique `ConversationId`
//! that round-trips through `hydra_common::ConversationId::FromStr` is
//! awkward to express in pure SQL without leaning on a UUID extension
//! that isn't shipped in our sqlx-tracked migrations. Rust gives us
//! the right primitive (`ConversationId::generate(...)`) and lets us
//! reuse the same dual-store backend abstraction the
//! `actor_variant_cleanup` migration already uses.
//!
//! ## Idempotency
//!
//! Per-row strategy: read the `mode` JSON, classify as
//! [`Classify::NeedsBackfill`] only when `mode.type == "headless"` and
//! `mode.conversation_id` is absent. Already-migrated rows whose mode
//! already carries `conversation_id` are no-ops. After a successful
//! run every headless row's `mode.conversation_id` is populated AND
//! `tasks_v2.conversation_id` (the denormalized column) points at the
//! same id, so a second run is a no-op by construction.

use super::{Backend, RustMigration};
use anyhow::{Context, Result};
use chrono::Utc;
use hydra_common::ConversationId;
use serde_json::{Value, json};

/// The sqlx migration version this Rust step must run *after*. Pin
/// to the no-op SQL anchor at
/// `migrations/20260604000002_headless_conversation_backfill_anchor.sql`
/// (and its sqlite mirror) so the interleaved plan in
/// [`crate::store::migrations::plan_migrations`] runs this step
/// immediately after the anchor.
pub const HEADLESS_CONVERSATION_BACKFILL_VERSION: u64 = 20_260_604_000_002;

pub struct HeadlessConversationBackfillMigration;

#[async_trait::async_trait]
impl RustMigration for HeadlessConversationBackfillMigration {
    fn version(&self) -> u64 {
        HEADLESS_CONVERSATION_BACKFILL_VERSION
    }

    fn name(&self) -> &'static str {
        "headless-conversation-backfill"
    }

    async fn run(&self, backend: &Backend) -> Result<()> {
        match backend {
            Backend::Sqlite(pool) => sqlite::run(pool).await,
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => postgres::run(pool).await,
        }
    }
}

/// Outcome of inspecting a `tasks_v2.mode` JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Classify {
    /// Not a headless row, or already backfilled — skip.
    Skip,
    /// Headless row that still needs a `conversation_id`. Carries the
    /// `prompt` string (possibly empty) verbatim so the migration can
    /// stage it as the first `session_events_v2` row.
    NeedsBackfill { prompt: String },
}

/// Inspect a `mode` JSON blob. Public-in-crate for unit tests.
fn classify(mode: &Value) -> Classify {
    let Some(map) = mode.as_object() else {
        return Classify::Skip;
    };
    if map.get("type").and_then(Value::as_str) != Some("headless") {
        return Classify::Skip;
    }
    if map.get("conversation_id").is_some_and(|v| !v.is_null()) {
        return Classify::Skip;
    }
    let prompt = map
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Classify::NeedsBackfill { prompt }
}

/// Build the rewritten `mode` JSON for a backfilled headless row.
/// `prompt` is preserved verbatim (vestigial — the PR-3 wire-format
/// cutover drops the field from the Rust type); `conversation_id`
/// becomes the canonical reference the worker will use post-PR-3.
fn rewrite_mode(prompt: &str, conversation_id: &ConversationId) -> Value {
    json!({
        "type": "headless",
        "prompt": prompt,
        "conversation_id": conversation_id.as_ref(),
    })
}

/// Serialize a `SessionEvent::UserMessage` payload in the on-the-wire
/// shape stored in `session_events_v2.event_data` / `session_events.event_data`.
/// Matches the `#[serde(tag = "type", rename_all = "snake_case")]`
/// representation of `hydra_common::api::v1::sessions::SessionEvent`.
fn user_message_event_data(content: &str, timestamp_rfc3339: &str) -> String {
    json!({
        "type": "user_message",
        "content": content,
        "timestamp": timestamp_rfc3339,
    })
    .to_string()
}

#[cfg(test)]
mod classify_tests {
    use super::*;

    #[test]
    fn skips_interactive_mode() {
        let mode = json!({"type": "interactive", "conversation_id": "c-abc"});
        assert_eq!(classify(&mode), Classify::Skip);
    }

    #[test]
    fn skips_already_backfilled_headless() {
        let mode = json!({
            "type": "headless",
            "prompt": "hi",
            "conversation_id": "c-xyz",
        });
        assert_eq!(classify(&mode), Classify::Skip);
    }

    #[test]
    fn flags_legacy_headless_with_prompt() {
        let mode = json!({"type": "headless", "prompt": "do thing"});
        assert_eq!(
            classify(&mode),
            Classify::NeedsBackfill {
                prompt: "do thing".to_string()
            }
        );
    }

    #[test]
    fn flags_legacy_headless_without_prompt() {
        let mode = json!({"type": "headless"});
        assert_eq!(
            classify(&mode),
            Classify::NeedsBackfill {
                prompt: String::new(),
            }
        );
    }

    #[test]
    fn skips_non_object_mode() {
        let mode = json!("nope");
        assert_eq!(classify(&mode), Classify::Skip);
    }

    #[test]
    fn rewrite_mode_preserves_prompt_and_adds_conversation_id() {
        let cid: ConversationId = "c-abcdef".parse().unwrap();
        let rewritten = rewrite_mode("hello world", &cid);
        assert_eq!(rewritten["type"], "headless");
        assert_eq!(rewritten["prompt"], "hello world");
        assert_eq!(rewritten["conversation_id"], "c-abcdef");
    }

    #[test]
    fn user_message_event_data_round_trips_via_session_event() {
        use hydra_common::api::v1::sessions::SessionEvent;
        let ts = Utc::now();
        let data = user_message_event_data("the prompt", &ts.to_rfc3339());
        let parsed: SessionEvent = serde_json::from_str(&data).unwrap();
        match parsed {
            SessionEvent::UserMessage { content, .. } => assert_eq!(content, "the prompt"),
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod sqlite_integration_tests {
    use super::*;
    use crate::store::sqlite_store::SqliteStore;
    use hydra_common::api::v1::sessions::SessionEvent;
    use sqlx::{Row, SqlitePool};

    async fn fresh_pool() -> SqlitePool {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        // Apply the full SQL+Rust migration chain so the headless
        // backfill ALSO runs as part of the registry — i.e. the test
        // exercises the same call path the production startup hook
        // uses, including the interleave plan.
        SqliteStore::run_migrations(&pool).await.unwrap();
        pool
    }

    /// Insert a legacy-shape headless tasks_v2 row directly via SQL,
    /// then re-run the registry to confirm the per-row backfill
    /// rewrites it. Re-running is the production startup pattern, so
    /// the second invocation also exercises the idempotency rule.
    #[tokio::test]
    async fn backfill_rewrites_legacy_headless_row() {
        let pool = fresh_pool().await;

        // Seed a legacy headless row. `conversation_id` column is NULL,
        // `mode` is `{"type":"headless","prompt":"hello"}`.
        let session_id = "s-legacyhdls";
        let original_prompt = "hello from the past";
        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, creator, image, env_vars, \
                 status, deleted, creation_time, conversation_id, \
                 mount_spec, agent_config, mode, is_latest, greet_user) \
             VALUES (?1, 1, 'alice', NULL, '{}', \
                     'complete', 0, '2026-05-01T00:00:00.000Z', NULL, \
                     '{\"working_dir\":\"repo\",\"mounts\":[]}', \
                     '{}', \
                     json_object('type', 'headless', 'prompt', ?2), \
                     1, 0)",
        )
        .bind(session_id)
        .bind(original_prompt)
        .execute(&pool)
        .await
        .unwrap();

        // Trigger the backfill (the chain ran once at fresh_pool, but the
        // row was inserted AFTER that — re-run to pick it up). This also
        // exercises the rule that migration runs are safe to repeat.
        super::sqlite::run(&pool).await.unwrap();

        // Assert the row was rewritten.
        let row = sqlx::query("SELECT mode, conversation_id FROM tasks_v2 WHERE id = ?1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let mode_text: String = row.try_get("mode").unwrap();
        let conv_col: Option<String> = row.try_get("conversation_id").unwrap();
        let mode: Value = serde_json::from_str(&mode_text).unwrap();
        let conv_in_mode = mode
            .get("conversation_id")
            .and_then(Value::as_str)
            .expect("backfill must stamp conversation_id into mode JSON");
        assert_eq!(
            conv_col.as_deref(),
            Some(conv_in_mode),
            "tasks_v2.conversation_id column must mirror mode.conversation_id"
        );
        assert_eq!(
            mode.get("prompt").and_then(Value::as_str),
            Some(original_prompt),
            "prompt is preserved verbatim during the PR-2 transition"
        );

        // Assert the conversation row exists.
        let conv_row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE id = ?1 AND is_latest = 1")
                .bind(conv_in_mode)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            conv_row.0, 1,
            "exactly one conversation row should exist for the backfilled session"
        );

        // Assert the first session_event is a UserMessage with the original prompt.
        let event_row = sqlx::query(
            "SELECT event_type, event_data FROM session_events \
             WHERE session_id = ?1 ORDER BY version_number ASC LIMIT 1",
        )
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let event_type: String = event_row.try_get("event_type").unwrap();
        let event_data: String = event_row.try_get("event_data").unwrap();
        assert_eq!(event_type, "user_message");
        let parsed: SessionEvent = serde_json::from_str(&event_data).unwrap();
        match parsed {
            SessionEvent::UserMessage { content, .. } => assert_eq!(content, original_prompt),
            other => panic!("expected UserMessage, got {other:?}"),
        }

        // Re-run the migration: no extra rows should be created, no
        // extra updates. The classify() rule is the load-bearing piece
        // — after the first pass the row's mode.conversation_id is set,
        // so the second pass treats it as `Skip`.
        super::sqlite::run(&pool).await.unwrap();
        let conv_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            conv_count.0, 1,
            "idempotent re-run must NOT create a second conversation"
        );
        let event_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM session_events WHERE session_id = ?1")
                .bind(session_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            event_count.0, 1,
            "idempotent re-run must NOT append a duplicate UserMessage"
        );
    }

    /// Interactive rows must be left alone.
    #[tokio::test]
    async fn backfill_skips_interactive_rows() {
        let pool = fresh_pool().await;
        let session_id = "s-leaveme";
        let conv_id = "c-existing";

        // Seed a conversation row that the interactive task points at.
        sqlx::query(
            "INSERT INTO conversations \
                (id, version_number, status, creator, deleted, is_latest) \
             VALUES (?1, 1, 'active', 'alice', 0, 1)",
        )
        .bind(conv_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, creator, image, env_vars, \
                 status, deleted, creation_time, conversation_id, \
                 mount_spec, agent_config, mode, is_latest, greet_user) \
             VALUES (?1, 1, 'alice', NULL, '{}', \
                     'complete', 0, '2026-05-01T00:00:00.000Z', ?2, \
                     '{\"working_dir\":\"repo\",\"mounts\":[]}', \
                     '{}', \
                     json_object('type', 'interactive', 'conversation_id', ?2, \
                                 'idle_timeout_secs', 300), \
                     1, 0)",
        )
        .bind(session_id)
        .bind(conv_id)
        .execute(&pool)
        .await
        .unwrap();

        super::sqlite::run(&pool).await.unwrap();

        // mode must be unchanged; no event must have been appended.
        let mode_text: String = sqlx::query_scalar("SELECT mode FROM tasks_v2 WHERE id = ?1")
            .bind(session_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let mode: Value = serde_json::from_str(&mode_text).unwrap();
        assert_eq!(
            mode.get("type").and_then(Value::as_str),
            Some("interactive")
        );
        let event_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM session_events WHERE session_id = ?1")
                .bind(session_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            event_count.0, 0,
            "interactive rows must not receive a synthesized first UserMessage"
        );
    }

    /// Headless row whose `mode` JSON already carries `conversation_id`
    /// (e.g. a row written by a previous boot's backfill) must be skipped.
    #[tokio::test]
    async fn backfill_skips_already_migrated_headless_row() {
        let pool = fresh_pool().await;
        let session_id = "s-alreadymig";
        let conv_id = "c-prevmig";

        sqlx::query(
            "INSERT INTO conversations \
                (id, version_number, status, creator, deleted, is_latest) \
             VALUES (?1, 1, 'active', 'alice', 0, 1)",
        )
        .bind(conv_id)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO tasks_v2 \
                (id, version_number, creator, image, env_vars, \
                 status, deleted, creation_time, conversation_id, \
                 mount_spec, agent_config, mode, is_latest, greet_user) \
             VALUES (?1, 1, 'alice', NULL, '{}', \
                     'complete', 0, '2026-05-01T00:00:00.000Z', ?2, \
                     '{\"working_dir\":\"repo\",\"mounts\":[]}', \
                     '{}', \
                     json_object('type', 'headless', 'prompt', 'hi', \
                                 'conversation_id', ?2), \
                     1, 0)",
        )
        .bind(session_id)
        .bind(conv_id)
        .execute(&pool)
        .await
        .unwrap();

        super::sqlite::run(&pool).await.unwrap();

        // No new conversation row created — count should still be 1
        // (the one we seeded above for `c-prevmig`).
        let conv_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(conv_count.0, 1);
        // No synthesized UserMessage either.
        let event_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM session_events WHERE session_id = ?1")
                .bind(session_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(event_count.0, 0);
    }
}

// ---------------------------------------------------------------------------
// SQLite driver
// ---------------------------------------------------------------------------

mod sqlite {
    use super::*;
    use sqlx::{Row, SqlitePool};

    pub async fn run(pool: &SqlitePool) -> Result<()> {
        // Load every latest, non-deleted `tasks_v2` row whose mode is
        // some form of headless. `is_latest = 1` keeps the per-id
        // multi-version history alone — we only stamp the head row.
        let rows = sqlx::query(
            "SELECT id, version_number, mode FROM tasks_v2 \
             WHERE is_latest = 1 \
               AND deleted = 0 \
               AND mode IS NOT NULL \
               AND json_extract(mode, '$.type') = 'headless'",
        )
        .fetch_all(pool)
        .await
        .context("scan tasks_v2 for headless rows")?;

        let mut rewrote = 0usize;
        for row in rows {
            let session_id: String = row.try_get("id")?;
            let version_number: i64 = row.try_get("version_number")?;
            let mode_text: String = row.try_get("mode")?;
            let mode: Value = serde_json::from_str(&mode_text)
                .with_context(|| format!("decode tasks_v2.mode JSON for {session_id}"))?;
            let Classify::NeedsBackfill { prompt } = classify(&mode) else {
                continue;
            };

            // Generate a unique conversation id. The actor_variant_cleanup
            // migration sets the precedent of self-contained id generation
            // inside Rust migrations (it avoids depending on per-store
            // `next_*_id` helpers, which themselves depend on row-count
            // caches that aren't initialized at migration time).
            let conversation_id = ConversationId::generate(8)
                .context("generate ConversationId for headless backfill (length within bounds)")?;
            let now = Utc::now();
            let now_rfc3339 = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

            // Run the three coordinated writes in a transaction so a
            // crash mid-row leaves the source row untouched and the
            // next migration run picks it up cleanly.
            let mut tx = pool.begin().await.context("begin sqlite tx")?;

            // 1. New `conversations` row. Title/agent_name/session_settings
            //    use the same lightweight defaults the in-app
            //    `next_conversation_id` -> `add_conversation` path would
            //    pick for a fresh row with no extra metadata.
            sqlx::query(
                "INSERT INTO conversations \
                    (id, version_number, title, agent_name, session_settings, \
                     status, creator, deleted, actor, is_latest, created_at, updated_at) \
                 VALUES (?1, 1, NULL, NULL, '{}', 'active', ?2, 0, NULL, 1, ?3, ?3)",
            )
            .bind(conversation_id.as_ref())
            .bind(legacy_creator_for_row(&mut *tx, &session_id).await?)
            .bind(&now_rfc3339)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("insert conversations row for headless backfill of {session_id}")
            })?;

            // 2. First session_event: UserMessage with the headless prompt.
            //    Keyed by session_id so it lands on the same session
            //    `get_session_events` queries off (per the post-Phase-E
            //    architecture — see `migrations/events.rs` module docs).
            let event_data = user_message_event_data(&prompt, &now_rfc3339);
            sqlx::query(
                "INSERT INTO session_events \
                    (session_id, version_number, event_type, event_data, actor, created_at) \
                 VALUES (?1, 1, 'user_message', ?2, NULL, ?3)",
            )
            .bind(&session_id)
            .bind(&event_data)
            .bind(&now_rfc3339)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("insert session_events UserMessage for headless backfill of {session_id}")
            })?;

            // 3. Rewrite the tasks_v2 row's mode JSON to add
            //    conversation_id, AND populate the denormalized
            //    conversation_id column so list/lookup queries
            //    that join on tasks_v2.conversation_id surface the
            //    backfilled headless row's conversation.
            let new_mode = rewrite_mode(&prompt, &conversation_id).to_string();
            sqlx::query(
                "UPDATE tasks_v2 \
                 SET mode = ?1, conversation_id = ?2 \
                 WHERE id = ?3 AND version_number = ?4",
            )
            .bind(&new_mode)
            .bind(conversation_id.as_ref())
            .bind(&session_id)
            .bind(version_number)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("update tasks_v2.mode for headless backfill of {session_id}")
            })?;

            tx.commit()
                .await
                .with_context(|| format!("commit headless backfill tx for {session_id}"))?;
            rewrote += 1;
        }

        if rewrote > 0 {
            tracing::info!(
                target: "headless_conversation_backfill",
                rewrote,
                "headless-conversation-backfill: materialized {rewrote} conversation(s)",
            );
        }
        Ok(())
    }

    /// Look up the `creator` of a session for the new conversation row.
    /// Conversation rows carry a `creator NOT NULL`; mirroring the
    /// owning session is the only attribution available at migration
    /// time. Falls back to a sentinel only if the source row is gone
    /// (race with a delete in another process — should not happen at
    /// app startup, but we leave the conversation creatable rather
    /// than failing the whole migration).
    async fn legacy_creator_for_row<'e, E>(executor: E, session_id: &str) -> Result<String>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let creator: Option<String> =
            sqlx::query_scalar("SELECT creator FROM tasks_v2 WHERE id = ?1 AND is_latest = 1")
                .bind(session_id)
                .fetch_optional(executor)
                .await
                .with_context(|| format!("lookup creator for tasks_v2 {session_id}"))?;
        Ok(creator.unwrap_or_else(|| "system".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Postgres driver
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use sqlx::{PgPool, Row};

    pub async fn run(pool: &PgPool) -> Result<()> {
        let rows = sqlx::query(
            "SELECT id, version_number, mode FROM metis.tasks_v2 \
             WHERE is_latest = TRUE \
               AND deleted = FALSE \
               AND mode IS NOT NULL \
               AND mode->>'type' = 'headless'",
        )
        .fetch_all(pool)
        .await
        .context("scan metis.tasks_v2 for headless rows")?;

        let mut rewrote = 0usize;
        for row in rows {
            let session_id: String = row.try_get("id")?;
            let version_number: i64 = row.try_get("version_number")?;
            let mode: Value = row.try_get("mode")?;
            let Classify::NeedsBackfill { prompt } = classify(&mode) else {
                continue;
            };

            let conversation_id = ConversationId::generate(8)
                .context("generate ConversationId for headless backfill (length within bounds)")?;
            let now = Utc::now();
            let now_rfc3339 = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

            let mut tx = pool.begin().await.context("begin postgres tx")?;

            let creator = legacy_creator_for_row(&mut *tx, &session_id).await?;
            sqlx::query(
                "INSERT INTO metis.conversations_v2 \
                    (id, version_number, title, agent_name, session_settings, \
                     status, creator, deleted, actor, created_at, updated_at) \
                 VALUES ($1, 1, NULL, NULL, '{}'::jsonb, 'active', $2, FALSE, NULL, NOW(), NOW())",
            )
            .bind(conversation_id.as_ref())
            .bind(&creator)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("insert metis.conversations_v2 row for headless backfill of {session_id}")
            })?;

            let event_data: Value =
                serde_json::from_str(&user_message_event_data(&prompt, &now_rfc3339))
                    .expect("user_message_event_data builds valid JSON");
            sqlx::query(
                "INSERT INTO metis.session_events_v2 \
                    (session_id, version_number, event_type, event_data, actor, created_at) \
                 VALUES ($1, 1, 'user_message', $2, NULL, NOW())",
            )
            .bind(&session_id)
            .bind(&event_data)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "insert metis.session_events_v2 UserMessage for headless backfill of {session_id}"
                )
            })?;

            let new_mode = rewrite_mode(&prompt, &conversation_id);
            sqlx::query(
                "UPDATE metis.tasks_v2 \
                 SET mode = $1, conversation_id = $2 \
                 WHERE id = $3 AND version_number = $4",
            )
            .bind(&new_mode)
            .bind(conversation_id.as_ref())
            .bind(&session_id)
            .bind(version_number)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!("update metis.tasks_v2.mode for headless backfill of {session_id}")
            })?;

            tx.commit()
                .await
                .with_context(|| format!("commit headless backfill tx for {session_id}"))?;
            rewrote += 1;
        }

        if rewrote > 0 {
            tracing::info!(
                target: "headless_conversation_backfill",
                rewrote,
                "headless-conversation-backfill: materialized {rewrote} conversation(s)",
            );
        }
        Ok(())
    }

    async fn legacy_creator_for_row<'e, E>(executor: E, session_id: &str) -> Result<String>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let creator: Option<String> = sqlx::query_scalar(
            "SELECT creator FROM metis.tasks_v2 WHERE id = $1 AND is_latest = TRUE",
        )
        .bind(session_id)
        .fetch_optional(executor)
        .await
        .with_context(|| format!("lookup creator for metis.tasks_v2 {session_id}"))?;
        Ok(creator.unwrap_or_else(|| "system".to_string()))
    }
}
