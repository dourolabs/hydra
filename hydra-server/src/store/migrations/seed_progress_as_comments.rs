//! Final cutover: seed each issue's most-recently-written `progress`
//! value as a comment on the issue, attributed to the actor and
//! timestamp of the version that last wrote that progress value.
//!
//! ## Why
//!
//! The `Issue.progress` field (free-text working notes maintained by
//! the assignee) is being removed in favour of the per-issue
//! `issue_comments` stream introduced earlier. By the time this
//! migration runs the wire types, CLI, frontend, and feedback route
//! are already gone — only the DB column and any historical values
//! survive. This pass migrates those historical values into the new
//! comments stream so no working notes are lost; the column-drop SQL
//! migration at version 20260721000000 immediately follows.
//!
//! Feedback is intentionally NOT seeded — the field represents an
//! unactioned redirect, the agent it was redirecting is gone with the
//! fields, and the user explicitly directed dropping it.
//!
//! ## Per-issue strategy
//!
//! 1. List every issue (latest version) whose current `progress` is
//!    non-empty.
//! 2. Walk that issue's version history (oldest → newest) and find the
//!    *most recent* version whose `progress` value differs from the
//!    immediately preceding version. The transition into that value is
//!    what we attribute the comment to.
//! 3. Use that version's `actor` (or NULL) and `created_at` for the
//!    seeded comment; the body is the current `progress` value.
//! 4. Append a row to `issue_comments` at sequence `MAX(sequence) + 1`
//!    (defaulting to 1 if the issue has no comments yet).
//!
//! ## Idempotency
//!
//! Re-running must be a no-op. The skip rule is: if the issue already
//! has any comment with the exact body equal to the issue's current
//! `progress` value, skip. That keeps repeat boots from duplicating
//! the seeded row even though the comment table itself has no unique
//! constraint on `body`. The same skip also covers the case where a
//! user manually posted the migrated body as a comment before the
//! migration ran — preferring de-dup over a strict "did we seed?"
//! flag — because adding a tracking column for one transient
//! migration is heavy and the duplication risk in practice is
//! negligible.
//!
//! ## Schema dependency
//!
//! Reads `issues_v2.progress` (TEXT) and the per-version `actor` JSON
//! column, INSERTs into `issue_comments`. Both backends share the
//! same column shape on those tables (see
//! `20260711000000_create_issue_comments.sql` for sqlite and
//! `20260714000000_create_issue_comments.sql` for postgres).

use super::{Backend, RustMigration};
use anyhow::{Context, Result};

/// The sqlx migration version this Rust step must run *after*. The
/// no-op SQL anchor at the matching version
/// (`20260720000000_seed_progress_as_comments_anchor.sql`) gates the
/// Rust step in the interleaved plan; the column-drop SQL migration
/// at `20260721000000` runs immediately after.
pub const SEED_PROGRESS_AS_COMMENTS_VERSION: u64 = 20_260_720_000_000;

pub struct SeedProgressAsCommentsMigration;

#[async_trait::async_trait]
impl RustMigration for SeedProgressAsCommentsMigration {
    fn version(&self) -> u64 {
        SEED_PROGRESS_AS_COMMENTS_VERSION
    }

    fn name(&self) -> &'static str {
        "seed-progress-as-comments"
    }

    async fn run(&self, backend: &Backend) -> Result<()> {
        match backend {
            Backend::Sqlite(pool) => sqlite::run(pool).await,
            #[cfg(feature = "postgres")]
            Backend::Postgres(pool) => postgres::run(pool).await,
        }
    }
}

mod sqlite {
    use super::*;
    use sqlx::{Row, SqlitePool};

    pub async fn run(pool: &SqlitePool) -> Result<()> {
        // Short-circuit if the `progress` column is already gone — that
        // means a subsequent boot is replaying the migration registry
        // and the column-drop SQL migration has already landed. The
        // migration is then a no-op by construction.
        if !column_exists(pool, "issues_v2", "progress").await? {
            return Ok(());
        }
        // Also short-circuit if `issue_comments` doesn't exist yet —
        // the sqlx interleave plan should have run the create-table
        // migration before this, but we want a tolerant no-op if a
        // partial schema replay reorders things.
        if !table_exists(pool, "issue_comments").await? {
            return Ok(());
        }

        let issue_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM issues_v2 \
             WHERE is_latest = 1 AND COALESCE(progress, '') <> '' \
             ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing issues with populated progress")?;

        for issue_id in issue_ids {
            process_issue(pool, &issue_id)
                .await
                .with_context(|| format!("seeding progress-as-comment for issue {issue_id}"))?;
        }
        Ok(())
    }

    async fn process_issue(pool: &SqlitePool, issue_id: &str) -> Result<()> {
        // Pull every version (ASC) so we can find the LAST transition
        // where `progress` changed. The newest version's progress is
        // the body we want to seed.
        let rows = sqlx::query(
            "SELECT version_number, progress, actor, created_at \
             FROM issues_v2 WHERE id = ?1 \
             ORDER BY version_number ASC",
        )
        .bind(issue_id)
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let mut prev_progress: Option<String> = None;
        let mut last_change_actor: Option<String> = None;
        let mut last_change_created_at: Option<String> = None;
        let mut last_progress = String::new();
        for row in &rows {
            let progress: String = row.try_get("progress").unwrap_or_default();
            let actor: Option<String> = row.try_get("actor").ok();
            let created_at: String = row.try_get("created_at")?;
            let changed = match &prev_progress {
                Some(prev) => prev != &progress,
                None => !progress.is_empty(),
            };
            if changed {
                last_change_actor = actor;
                last_change_created_at = Some(created_at);
            }
            prev_progress = Some(progress.clone());
            last_progress = progress;
        }

        if last_progress.is_empty() {
            return Ok(());
        }

        // Idempotency check: if a comment with this exact body already
        // exists for this issue, skip — we already migrated, or the
        // user posted it manually before we ran.
        let already_present: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM issue_comments \
             WHERE issue_id = ?1 AND body = ?2",
        )
        .bind(issue_id)
        .bind(&last_progress)
        .fetch_one(pool)
        .await
        .context("checking existing seeded comment")?;
        if already_present > 0 {
            return Ok(());
        }

        // Actor stored as TEXT (JSON) in sqlite — fall back to a
        // system actor if the source row had a NULL actor.
        let actor_json = last_change_actor.unwrap_or_else(|| {
            "{\"System\":{\"worker_name\":\"seed-progress-as-comments\"}}".to_string()
        });
        let created_at = last_change_created_at.unwrap_or_else(|| {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });

        let mut tx = pool.begin().await.context("begin tx")?;
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 \
             FROM issue_comments WHERE issue_id = ?1",
        )
        .bind(issue_id)
        .fetch_one(&mut *tx)
        .await
        .context("allocating sequence for seeded comment")?;

        sqlx::query(
            "INSERT INTO issue_comments \
                 (issue_id, sequence, body, actor, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(issue_id)
        .bind(next_seq)
        .bind(&last_progress)
        .bind(&actor_json)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .context("inserting seeded comment row")?;
        tx.commit().await.context("commit tx")?;
        Ok(())
    }

    async fn table_exists(pool: &SqlitePool, name: &str) -> Result<bool> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1")
                .bind(name)
                .fetch_optional(pool)
                .await?;
        Ok(row.is_some())
    }

    async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> Result<bool> {
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await?;
        for row in rows {
            let name: String = row.try_get("name")?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use serde_json::Value;
    use sqlx::{PgPool, Row};

    pub async fn run(pool: &PgPool) -> Result<()> {
        if !column_exists(pool, "metis", "issues_v2", "progress").await? {
            return Ok(());
        }
        if !table_exists(pool, "metis", "issue_comments").await? {
            return Ok(());
        }

        let issue_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM metis.issues_v2 \
             WHERE is_latest = TRUE AND COALESCE(progress, '') <> '' \
             ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .context("listing issues with populated progress")?;

        for issue_id in issue_ids {
            process_issue(pool, &issue_id)
                .await
                .with_context(|| format!("seeding progress-as-comment for issue {issue_id}"))?;
        }
        Ok(())
    }

    async fn process_issue(pool: &PgPool, issue_id: &str) -> Result<()> {
        let rows = sqlx::query(
            "SELECT version_number, progress, actor, created_at \
             FROM metis.issues_v2 WHERE id = $1 \
             ORDER BY version_number ASC",
        )
        .bind(issue_id)
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            return Ok(());
        }

        let mut prev_progress: Option<String> = None;
        let mut last_change_actor: Option<Value> = None;
        let mut last_change_created_at: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_progress = String::new();
        for row in &rows {
            let progress: String = row.try_get("progress").unwrap_or_default();
            let actor: Option<Value> = row.try_get("actor").ok();
            let created_at: chrono::DateTime<chrono::Utc> = row.try_get("created_at")?;
            let changed = match &prev_progress {
                Some(prev) => prev != &progress,
                None => !progress.is_empty(),
            };
            if changed {
                last_change_actor = actor;
                last_change_created_at = Some(created_at);
            }
            prev_progress = Some(progress.clone());
            last_progress = progress;
        }

        if last_progress.is_empty() {
            return Ok(());
        }

        let already_present: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM metis.issue_comments \
             WHERE issue_id = $1 AND body = $2",
        )
        .bind(issue_id)
        .bind(&last_progress)
        .fetch_one(pool)
        .await
        .context("checking existing seeded comment")?;
        if already_present > 0 {
            return Ok(());
        }

        let actor_json: Value = last_change_actor.unwrap_or_else(
            || serde_json::json!({"System": {"worker_name": "seed-progress-as-comments"}}),
        );
        let created_at = last_change_created_at.unwrap_or_else(chrono::Utc::now);

        let mut tx = pool.begin().await.context("begin tx")?;
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 \
             FROM metis.issue_comments WHERE issue_id = $1",
        )
        .bind(issue_id)
        .fetch_one(&mut *tx)
        .await
        .context("allocating sequence for seeded comment")?;

        sqlx::query(
            "INSERT INTO metis.issue_comments \
                 (issue_id, sequence, body, actor, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(issue_id)
        .bind(next_seq)
        .bind(&last_progress)
        .bind(&actor_json)
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .context("inserting seeded comment row")?;
        tx.commit().await.context("commit tx")?;
        Ok(())
    }

    async fn table_exists(pool: &PgPool, schema: &str, name: &str) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = $1 AND table_name = $2 \
             )",
        )
        .bind(schema)
        .bind(name)
        .fetch_one(pool)
        .await?;
        Ok(exists)
    }

    async fn column_exists(pool: &PgPool, schema: &str, table: &str, column: &str) -> Result<bool> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 AND column_name = $3 \
             )",
        )
        .bind(schema)
        .bind(table)
        .bind(column)
        .fetch_one(pool)
        .await?;
        Ok(exists)
    }
}
