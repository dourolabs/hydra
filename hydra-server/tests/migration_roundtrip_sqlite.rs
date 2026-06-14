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
use hydra_server::domain::users::Username;
use hydra_server::store::sqlite_store::{self, MIGRATOR, SqliteStore};
use hydra_server::store::{ReadOnlyStore, RelationshipType, Store, StoreError};
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

    drop_status_icon_migration_strips_default_seed(&pool).await?;
    drop_status_icon_migration_strips_custom_row(&pool).await?;
    drop_status_icon_migration_is_idempotent(&pool).await?;

    denormalize_creator_session_backfill(&pool).await?;
    denormalize_creator_user_backfill(&pool).await?;
    denormalize_creator_domain_roundtrip(&pool).await?;
    drop_actors_v2_migration_removes_table(&pool).await?;
    denormalize_creator_migration_is_idempotent(&pool).await?;

    add_projects_priority_backfill_sql_level(&pool).await?;
    add_projects_priority_backfill_domain_roundtrip(&pool).await?;

    drop_projects_default_status_key_migration_removes_column(&pool).await?;
    drop_projects_default_status_key_migration_preserves_typed_read(&pool).await?;
    drop_projects_default_status_key_migration_is_idempotent(&pool).await?;

    issues_v2_project_id_is_not_null(&pool).await?;
    issues_v2_project_id_rejects_null_insert(&pool).await?;
    issues_v2_project_id_not_null_migration_rejects_null_baseline().await?;

    // Schema- and data-shape assertions for the two new statuses
    // migrations land before the issues_v2_project_id_not_null
    // idempotency rerun further down, which does a full
    // CREATE / INSERT / DROP / RENAME rebuild of `issues_v2` against
    // an explicit (pre-status_sequence) column list — re-running that
    // body destroys the `status_sequence` column we just added.
    create_statuses_migration_schema_invariants(&pool).await?;
    create_statuses_migration_backfills_default_seed(&pool).await?;
    create_statuses_migration_backfills_custom_project(&pool).await?;
    add_issues_v2_status_sequence_schema_invariants(&pool).await?;
    add_issues_v2_status_sequence_backfills_issues(&pool).await?;
    create_statuses_migration_is_idempotent(&pool).await?;
    add_issues_v2_status_sequence_migration_is_idempotent(&pool).await?;
    add_issues_v2_status_sequence_migration_rejects_null_baseline().await?;

    issues_v2_project_id_not_null_migration_is_idempotent(&pool).await?;

    cutover_to_statuses_table_schema_invariants(&pool).await?;
    cutover_to_statuses_table_backfills_deploy_gap_project(&pool).await?;
    cutover_to_statuses_table_backfills_deploy_gap_issues(&pool).await?;
    cutover_to_statuses_table_fk_rejects_unknown_sequence(&pool).await?;
    cutover_to_statuses_table_fk_rejects_status_delete_with_active_issue(&pool).await?;

    reserve_hydra_id_shape_rewrites_project_keys(&pool).await?;
    reserve_hydra_id_shape_rewrites_status_keys(&pool).await?;
    reserve_hydra_id_shape_no_reserved_shape_remains(&pool).await?;
    reserve_hydra_id_shape_domain_roundtrip(&pool).await?;
    reserve_hydra_id_shape_migration_is_idempotent(&pool).await?;

    add_statuses_position_schema_invariants(&pool).await?;
    add_statuses_position_backfills_to_sequence(&pool).await?;
    add_statuses_position_domain_roundtrip(&pool).await?;
    add_statuses_position_migration_is_idempotent(&pool).await?;

    add_statuses_auto_archive_after_seconds_schema_invariants(&pool).await?;
    add_statuses_auto_archive_after_seconds_defaults_to_null(&pool).await?;
    add_statuses_auto_archive_after_seconds_domain_roundtrip(&pool).await?;
    add_statuses_auto_archive_after_seconds_migration_is_idempotent(&pool).await?;

    add_clear_assignee_to_default_terminal_statuses_post_migration_state(&pool).await?;
    add_clear_assignee_to_default_terminal_statuses_is_idempotent(&pool).await?;

    drop_is_assignment_agent_schema_invariants(&pool).await?;
    drop_is_assignment_agent_preserves_rows(&pool).await?;
    drop_is_assignment_agent_migration_is_idempotent(&pool).await?;

    split_agents_max_simultaneous_schema_invariants(&pool).await?;
    split_agents_max_simultaneous_backfills_baselines(&pool).await?;
    split_agents_max_simultaneous_migration_is_idempotent(&pool).await?;

    teardown_work_on_default_terminal_statuses_post_migration_state(&pool).await?;
    rename_kill_sessions_to_teardown_work_is_idempotent(&pool).await?;

    backfill_assignee_null_on_terminal_default_issues_nulls_targeted_rows(&pool).await?;
    backfill_assignee_null_on_terminal_default_issues_is_idempotent(&pool).await?;

    add_statuses_suppress_sessions_schema_invariants(&pool).await?;
    add_statuses_suppress_sessions_defaults_to_false(&pool).await?;
    add_statuses_suppress_sessions_migration_is_idempotent(&pool).await?;

    create_issue_comments_schema_invariants(&pool).await?;
    create_issue_comments_migration_is_idempotent(&pool).await?;

    add_statuses_max_simultaneous_sessions_schema_invariants(&pool).await?;
    add_statuses_max_simultaneous_sessions_defaults_to_null(&pool).await?;
    add_statuses_max_simultaneous_sessions_domain_roundtrip(&pool).await?;
    add_statuses_max_simultaneous_sessions_migration_is_idempotent(&pool).await?;

    add_statuses_session_settings_schema_invariants(&pool).await?;
    add_statuses_session_settings_defaults_to_null(&pool).await?;
    add_statuses_session_settings_migration_is_idempotent(&pool).await?;
    add_statuses_session_settings_store_roundtrip(&pool).await?;

    rename_projects_deleted_to_archived_schema_invariants(&pool).await?;
    rename_projects_deleted_to_archived_baseline_roundtrip(&pool).await?;
    rename_projects_deleted_to_archived_migration_is_idempotent(&pool).await?;

    add_statuses_archived_schema_invariants(&pool).await?;
    add_statuses_archived_defaults_to_false(&pool).await?;
    add_statuses_archived_migration_is_idempotent(&pool).await?;

    seed_progress_as_comments_drops_columns(&pool).await?;
    seed_progress_as_comments_seeds_one_comment_per_issue(&pool).await?;
    seed_progress_as_comments_drops_feedback_outright(&pool).await?;
    seed_progress_as_comments_migration_is_idempotent(&pool).await?;

    rename_deleted_to_archived_post_migration_state(&pool).await?;

    Ok(())
}

/// After the column-drop SQL migration at 20260721000000 lands, the
/// `progress` and `feedback` columns must be gone from `issues_v2`.
async fn seed_progress_as_comments_drops_columns(pool: &SqlitePool) -> Result<()> {
    if column_exists(pool, "issues_v2", "progress").await? {
        bail!("expected `issues_v2.progress` column to be dropped post-rollforward");
    }
    if column_exists(pool, "issues_v2", "feedback").await? {
        bail!("expected `issues_v2.feedback` column to be dropped post-rollforward");
    }
    Ok(())
}

/// For each baseline issue whose progress was populated, the
/// migration must have inserted exactly one `issue_comments` row whose
/// body matches the latest progress value, attributed to the actor
/// and timestamp of the version that last changed it.
async fn seed_progress_as_comments_seeds_one_comment_per_issue(pool: &SqlitePool) -> Result<()> {
    // i-progseed1: progress changed across two versions; the seeded
    // body must be the v2 value ("latest progress note"), attributed
    // to bob (v2's actor) at v2's created_at timestamp.
    let row = sqlx::query(
        "SELECT body, actor, created_at FROM issue_comments \
         WHERE issue_id = 'i-progseed1' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read seeded comments for i-progseed1")?;
    if row.len() != 1 {
        bail!(
            "expected exactly 1 seeded comment on i-progseed1; got {}",
            row.len()
        );
    }
    let body: String = row[0].try_get("body")?;
    let actor: String = row[0].try_get("actor")?;
    let created_at: String = row[0].try_get("created_at")?;
    if body != "latest progress note" {
        bail!("i-progseed1: expected seeded body to be the v2 progress; got {body:?}");
    }
    if !actor.contains("\"bob\"") {
        bail!("i-progseed1: expected seeded comment actor to be bob (v2 actor); got {actor:?}");
    }
    if !created_at.starts_with("2026-07-02T") {
        bail!(
            "i-progseed1: expected seeded comment created_at to match v2 timestamp; got {created_at:?}"
        );
    }

    // i-progseed2: single version. Seeded body matches v1 progress;
    // actor and created_at come from v1.
    let row = sqlx::query(
        "SELECT body, actor, created_at FROM issue_comments \
         WHERE issue_id = 'i-progseed2' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read seeded comments for i-progseed2")?;
    if row.len() != 1 {
        bail!(
            "expected exactly 1 seeded comment on i-progseed2; got {}",
            row.len()
        );
    }
    let body: String = row[0].try_get("body")?;
    let actor: String = row[0].try_get("actor")?;
    let created_at: String = row[0].try_get("created_at")?;
    if body != "only progress note" {
        bail!("i-progseed2: expected seeded body to be the v1 progress; got {body:?}");
    }
    if !actor.contains("\"carol\"") {
        bail!("i-progseed2: expected seeded comment actor to be carol; got {actor:?}");
    }
    if !created_at.starts_with("2026-07-03T") {
        bail!(
            "i-progseed2: expected seeded comment created_at to match v1 timestamp; got {created_at:?}"
        );
    }
    Ok(())
}

/// `i-progfb` had a populated `feedback` and empty `progress`. The
/// migration must not seed any comment for that issue — feedback is
/// dropped outright per the parent spec.
async fn seed_progress_as_comments_drops_feedback_outright(pool: &SqlitePool) -> Result<()> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM issue_comments WHERE issue_id = 'i-progfb'")
            .fetch_one(pool)
            .await
            .context("count seeded comments on i-progfb")?;
    if count != 0 {
        bail!("expected no seeded comments on i-progfb; got {count}");
    }
    Ok(())
}

/// Re-running the migration registry (which the server does on every
/// boot) must not duplicate the seeded comments.
async fn seed_progress_as_comments_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issue_comments")
        .fetch_one(pool)
        .await
        .context("count issue_comments before idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm seed-progress idempotency")?;
    let count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issue_comments")
        .fetch_one(pool)
        .await
        .context("count issue_comments after idempotency rerun")?;
    if count_before != count_after {
        bail!(
            "issue_comments row count changed across idempotency rerun: {count_before} -> {count_after}"
        );
    }
    Ok(())
}

/// 20260717000000_rename_deleted_to_archived. Renames the soft-delete
/// column on the ten entities not covered by the sibling project/status
/// rename. Post-state: `archived` exists on each table; `deleted` does
/// not. Idempotency is enforced by sqlx's per-migration checkpoint —
/// `ALTER TABLE ... RENAME COLUMN` is not naturally idempotent on
/// SQLite, but the sqlx migrator never re-runs an applied SQL body, so
/// a separate idempotency rerun would be checking the framework, not
/// the migration.
async fn rename_deleted_to_archived_post_migration_state(pool: &SqlitePool) -> Result<()> {
    for table in [
        "repositories_v2",
        "users_v2",
        "issues_v2",
        "patches_v2",
        "tasks_v2",
        "documents_v2",
        "agents",
        "labels",
        "conversations",
        "triggers",
    ] {
        if !column_exists(pool, table, "archived").await? {
            bail!("expected {table}.archived to exist post-rename");
        }
        if column_exists(pool, table, "deleted").await? {
            bail!("expected {table}.deleted to be renamed away post-rename");
        }
    }
    Ok(())
}

/// 20260614000000_cutover_to_statuses_table. The cutover drops
/// `projects.statuses` JSONB and `issues_v2.status` TEXT, tightens
/// `issues_v2.status_sequence` to NOT NULL, adds the FK to
/// `statuses(project_id, sequence)`, and adds the
/// `projects.next_status_sequence` high-water-mark column. These
/// assertions cover schema, deploy-gap catch-up backfills, and FK
/// enforcement.
async fn cutover_to_statuses_table_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if column_exists(pool, "projects", "statuses").await? {
        bail!("expected `projects.statuses` JSON column to be dropped post-cutover");
    }
    if column_exists(pool, "issues_v2", "status").await? {
        bail!("expected `issues_v2.status` TEXT column to be dropped post-cutover");
    }
    if !column_exists(pool, "projects", "next_status_sequence").await? {
        bail!("expected `projects.next_status_sequence` column to be present post-cutover");
    }
    if column_is_nullable(pool, "issues_v2", "status_sequence").await? {
        bail!("expected `issues_v2.status_sequence` to be NOT NULL post-cutover");
    }
    // Every project's next_status_sequence must be >= max(sequence)
    // for that project + 1, and >= 1 when the project has no
    // statuses.
    let rows = sqlx::query(
        "SELECT p.id, p.next_status_sequence, COALESCE((SELECT MAX(s.sequence) FROM statuses s WHERE s.project_id = p.id), 0) AS max_seq \
         FROM projects p WHERE p.is_latest = 1",
    )
    .fetch_all(pool)
    .await
    .context("read projects.next_status_sequence vs MAX(statuses.sequence)")?;
    for row in &rows {
        let id: String = row.try_get("id")?;
        let next: i64 = row.try_get("next_status_sequence")?;
        let max_seq: i64 = row.try_get("max_seq")?;
        if next < 1 {
            bail!("projects({id}).next_status_sequence={next}; must be >= 1");
        }
        if next <= max_seq {
            bail!(
                "projects({id}).next_status_sequence={next}; must be > MAX(statuses.sequence)={max_seq}"
            );
        }
    }
    Ok(())
}

async fn cutover_to_statuses_table_backfills_deploy_gap_project(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT sequence, key FROM statuses WHERE project_id = 'j-cutgapprj' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read deploy-gap project statuses rows")?;
    let pairs: Vec<(i64, String)> = rows
        .iter()
        .map(|r| {
            (
                r.try_get::<i64, _>("sequence").unwrap(),
                r.try_get::<String, _>("key").unwrap(),
            )
        })
        .collect();
    if pairs != vec![(1, "intake".to_string()), (2, "done".to_string())] {
        bail!("j-cutgapprj statuses: expected [(1,intake),(2,done)]; got {pairs:?}");
    }
    Ok(())
}

async fn cutover_to_statuses_table_backfills_deploy_gap_issues(pool: &SqlitePool) -> Result<()> {
    for (id, key) in &[("i-cutgapa", "intake"), ("i-cutgapdef", "open")] {
        let row = sqlx::query(
            "SELECT i.status_sequence, s.key AS resolved_key \
             FROM issues_v2 i \
             LEFT JOIN statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence \
             WHERE i.id = ?1 AND i.is_latest = 1",
        )
        .bind(*id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read deploy-gap issue {id}"))?;
        let seq: Option<i64> = row.try_get("status_sequence")?;
        let resolved: Option<String> = row.try_get("resolved_key")?;
        if seq.is_none() {
            bail!("{id}: expected non-NULL status_sequence post-cutover; got NULL");
        }
        if resolved.as_deref() != Some(*key) {
            bail!("{id}: expected resolved key '{key}'; got {resolved:?}");
        }
    }
    Ok(())
}

async fn cutover_to_statuses_table_fk_rejects_unknown_sequence(pool: &SqlitePool) -> Result<()> {
    // SQLite enforces the FK only when `PRAGMA foreign_keys=ON`. The
    // store layer's pool sets it on every connection; verify the pool
    // here too so the assertion exercises the enforced FK rather than
    // a no-op.
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(pool)
        .await
        .context("enable foreign_keys for FK enforcement check")?;
    let result = sqlx::query(
        "INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, project_id, status_sequence, is_latest) \
         VALUES ('i-fkbadseq', 1, 'task', 'fk test', 'system', 'j-defaul', 9999, 1)",
    )
    .execute(pool)
    .await;
    match result {
        Err(_) => Ok(()),
        Ok(_) => {
            bail!("expected FK to reject insert with unknown status_sequence; insert succeeded")
        }
    }
}

async fn cutover_to_statuses_table_fk_rejects_status_delete_with_active_issue(
    pool: &SqlitePool,
) -> Result<()> {
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(pool)
        .await
        .context("enable foreign_keys for FK enforcement check")?;
    // i-stsopena is on j-defaul with status `open` (sequence 1). The
    // FK must reject the DELETE while the issue still references the
    // row.
    let result = sqlx::query("DELETE FROM statuses WHERE project_id = 'j-defaul' AND sequence = 1")
        .execute(pool)
        .await;
    match result {
        Err(_) => Ok(()),
        Ok(_) => {
            bail!("expected FK to reject DELETE of statuses row while an issue still references it")
        }
    }
}

// ---------------------------------------------------------------------------
// 20260616000000_add_statuses_position. Adds `position REAL NOT NULL
// DEFAULT 0` to `statuses` and backfills `position = sequence` so the
// post-cutover display order matches today's `sequence ASC` order.
// ---------------------------------------------------------------------------

async fn add_statuses_position_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "statuses", "position").await? {
        bail!("expected `statuses.position` column to exist post-rollforward");
    }
    if column_is_nullable(pool, "statuses", "position").await? {
        bail!("expected `statuses.position` to be NOT NULL");
    }
    Ok(())
}

async fn add_statuses_position_backfills_to_sequence(pool: &SqlitePool) -> Result<()> {
    // `j-migsmoke` is inserted by `assert_recent_migration_store_smoke`
    // post-rollforward (i.e. after this migration runs), so its
    // position falls through to the column default 0 rather than the
    // backfill value. Exclude it from the backfill assertion.
    let rows = sqlx::query(
        "SELECT project_id, sequence, position FROM statuses WHERE project_id != 'j-migsmoke'",
    )
    .fetch_all(pool)
    .await
    .context("read statuses for position backfill check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let position: f64 = row.try_get("position")?;
        if (position - sequence as f64).abs() > f64::EPSILON {
            bail!(
                "statuses({project_id}, sequence={sequence}): expected position={sequence}.0; got {position}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_position_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    // Read j-defaul back through the production `get_project` path —
    // verifies that the new `position` column is included in the
    // `StatusRow` SELECT projection and round-trips into the
    // `StatusDefinition` value. The default-project seed is sequence-
    // ordered, so position should equal index+1 (sequence is 1-based).
    let store = SqliteStore::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("SqliteStore::get_project(j-defaul) post-position-migration")?;
    if fetched.item.statuses.is_empty() {
        bail!("expected j-defaul to have statuses post-rollforward");
    }
    for (idx, status) in fetched.item.statuses.iter().enumerate() {
        let expected = (idx + 1) as f64;
        if (status.position - expected).abs() > f64::EPSILON {
            bail!(
                "j-defaul.statuses[{idx}] ({key:?}): expected position={expected}; got {got}",
                key = status.key,
                got = status.position,
            );
        }
    }
    Ok(())
}

async fn add_statuses_position_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // Re-run the migration plan and confirm the backfill state is
    // preserved (the migration body is gated by sqlx's tracking table,
    // so the second pass is a no-op — this assertion catches any
    // regression that drops the gating and re-applies the
    // `UPDATE statuses SET position = sequence` body, which would
    // clobber positions set by callers after the initial backfill).
    let snapshot_before = sqlx::query(
        "SELECT project_id, sequence, position FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses before idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm position-migration idempotency")?;
    let snapshot_after = sqlx::query(
        "SELECT project_id, sequence, position FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses after idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across position-migration rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_pos: f64 = before.try_get("position")?;
        let after_pos: f64 = after.try_get("position")?;
        if (before_pos - after_pos).abs() > f64::EPSILON {
            bail!("position changed across rerun: {before_pos} -> {after_pos}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260617000000_add_statuses_auto_archive_after_seconds. Adds
// `auto_archive_after_seconds INTEGER NULL` to `statuses` — the
// per-status plumbing for the periodic auto-archive worker. `NULL`
// (the column default) leaves the feature off for the row.
// ---------------------------------------------------------------------------

async fn add_statuses_auto_archive_after_seconds_schema_invariants(
    pool: &SqlitePool,
) -> Result<()> {
    if !column_exists(pool, "statuses", "auto_archive_after_seconds").await? {
        bail!("expected `statuses.auto_archive_after_seconds` column to exist post-rollforward");
    }
    if !column_is_nullable(pool, "statuses", "auto_archive_after_seconds").await? {
        bail!("expected `statuses.auto_archive_after_seconds` to be NULLABLE");
    }
    Ok(())
}

async fn add_statuses_auto_archive_after_seconds_defaults_to_null(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query("SELECT project_id, sequence, auto_archive_after_seconds FROM statuses")
        .fetch_all(pool)
        .await
        .context("read statuses for auto_archive_after_seconds default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert default against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<i64> = row.try_get("auto_archive_after_seconds")?;
        if value.is_some() {
            bail!(
                "statuses({project_id}, sequence={sequence}): expected NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_auto_archive_after_seconds_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("SqliteStore::get_project(j-defaul) post-auto-archive migration")?;
    if fetched.item.statuses.is_empty() {
        bail!("expected j-defaul to have statuses post-rollforward");
    }
    for (idx, status) in fetched.item.statuses.iter().enumerate() {
        if status.auto_archive_after_seconds.is_some() {
            bail!(
                "j-defaul.statuses[{idx}] ({key:?}): expected auto_archive_after_seconds=None; got {got:?}",
                key = status.key,
                got = status.auto_archive_after_seconds,
            );
        }
    }
    Ok(())
}

async fn add_statuses_auto_archive_after_seconds_migration_is_idempotent(
    pool: &SqlitePool,
) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT project_id, sequence, auto_archive_after_seconds FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses before auto-archive idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm auto-archive idempotency")?;
    let snapshot_after = sqlx::query(
        "SELECT project_id, sequence, auto_archive_after_seconds FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses after auto-archive idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across auto-archive idempotency rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_val: Option<i64> = before.try_get("auto_archive_after_seconds")?;
        let after_val: Option<i64> = after.try_get("auto_archive_after_seconds")?;
        if before_val != after_val {
            bail!(
                "auto_archive_after_seconds changed across rerun: {before_val:?} -> {after_val:?}"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260618000000_add_clear_assignee_to_default_terminal_statuses. Seeds
// `on_enter.clear_assignee = true` on the three terminal default-project
// statuses (`closed`, `dropped`, `failed`) without disturbing
// `on_enter` on any other row. Idempotent under rerun.
// ---------------------------------------------------------------------------

async fn add_clear_assignee_to_default_terminal_statuses_post_migration_state(
    pool: &SqlitePool,
) -> Result<()> {
    for key in ["closed", "dropped", "failed"] {
        let row = sqlx::query(
            "SELECT on_enter FROM statuses \
             WHERE project_id = 'j-defaul' AND key = ?1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter")?;
        let on_enter_text = on_enter_text
            .ok_or_else(|| anyhow::anyhow!("j-defaul.{key}: expected on_enter NOT NULL"))?;
        let parsed: serde_json::Value = serde_json::from_str(&on_enter_text)
            .with_context(|| format!("decode on_enter JSON for j-defaul.{key}"))?;
        if parsed.get("clear_assignee") != Some(&serde_json::json!(true)) {
            bail!("j-defaul.{key}: expected on_enter.clear_assignee=true; got {parsed}");
        }
    }

    for key in ["open", "in-progress"] {
        let row = sqlx::query(
            "SELECT on_enter FROM statuses \
             WHERE project_id = 'j-defaul' AND key = ?1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter")?;
        if on_enter_text.is_some() {
            bail!("j-defaul.{key}: expected on_enter=NULL post-migration; got {on_enter_text:?}");
        }
    }
    Ok(())
}

async fn add_clear_assignee_to_default_terminal_statuses_is_idempotent(
    pool: &SqlitePool,
) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT key, on_enter FROM statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter before clear_assignee idempotency rerun")?;

    sqlx::raw_sql(
        "UPDATE statuses \
         SET on_enter = json_set( \
             COALESCE(on_enter, '{}'), \
             '$.clear_assignee', \
             json('true') \
         ) \
         WHERE project_id = 'j-defaul' \
           AND key IN ('closed', 'dropped', 'failed')",
    )
    .execute(pool)
    .await
    .context("re-apply clear_assignee seed migration body verbatim")?;

    let snapshot_after = sqlx::query(
        "SELECT key, on_enter FROM statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter after clear_assignee idempotency rerun")?;

    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across clear_assignee rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_key: String = before.try_get("key")?;
        let after_key: String = after.try_get("key")?;
        let before_on_enter: Option<String> = before.try_get("on_enter")?;
        let after_on_enter: Option<String> = after.try_get("on_enter")?;
        if before_key != after_key || before_on_enter != after_on_enter {
            bail!(
                "j-defaul.{before_key} changed across clear_assignee rerun: \
                 {before_on_enter:?} -> {after_on_enter:?}"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260619000000_drop_is_assignment_agent. The assignment-agent concept has
// been removed; per-status `on_enter.assign_to` is now the canonical routing
// mechanism. SQLite uses the rebuild-and-rename recipe (no native DROP COLUMN
// here), so these assertions verify the column is gone, the rows seeded by
// the 20260618000000 baseline survived the rebuild verbatim, and the boot
// path's idempotent rerun does not reapply the rebuild.
// ---------------------------------------------------------------------------

async fn drop_is_assignment_agent_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if column_exists(pool, "agents", "is_assignment_agent").await? {
        bail!("expected `agents.is_assignment_agent` column to be dropped post-rollforward");
    }
    // Sibling columns the migration is supposed to leave alone. By the
    // time this assertion runs, the later `rename_deleted_to_archived`
    // migration has flipped the soft-delete column over to `archived`,
    // and `split_agents_max_simultaneous` has renamed `max_simultaneous`
    // to `max_simultaneous_headless` (and added
    // `max_simultaneous_interactive`).
    for required in [
        "name",
        "prompt_path",
        "max_tries",
        "max_simultaneous_interactive",
        "max_simultaneous_headless",
        "archived",
        "created_at",
        "updated_at",
        "secrets",
        "mcp_config_path",
        "is_default_conversation_agent",
    ] {
        if !column_exists(pool, "agents", required).await? {
            bail!(
                "expected `agents.{required}` to remain present post-drop-is-assignment-agent; \
                 the rebuild lost it"
            );
        }
    }
    if column_exists(pool, "agents", "max_simultaneous").await? {
        bail!(
            "expected `agents.max_simultaneous` to be renamed to `max_simultaneous_headless` post-split"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260620000000_add_kill_sessions_to_default_terminal_statuses (seeds
// `on_enter.kill_sessions = true` on the three terminal default-project
// statuses) followed by
// 20260710000000_rename_kill_sessions_to_teardown_work (renames that
// JSON key to `teardown_work`). The post-state checks the end result of
// the pair: terminal rows carry `teardown_work=true` alongside the
// `clear_assignee=true` key seeded by the prior 20260618 migration, with
// no legacy `kill_sessions` key remaining. The rename migration is
// idempotent under rerun.
// ---------------------------------------------------------------------------

async fn teardown_work_on_default_terminal_statuses_post_migration_state(
    pool: &SqlitePool,
) -> Result<()> {
    for key in ["closed", "dropped", "failed"] {
        let row = sqlx::query(
            "SELECT on_enter FROM statuses \
             WHERE project_id = 'j-defaul' AND key = ?1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter")?;
        let on_enter_text = on_enter_text
            .ok_or_else(|| anyhow::anyhow!("j-defaul.{key}: expected on_enter NOT NULL"))?;
        let parsed: serde_json::Value = serde_json::from_str(&on_enter_text)
            .with_context(|| format!("decode on_enter JSON for j-defaul.{key}"))?;
        if parsed.get("teardown_work") != Some(&serde_json::json!(true)) {
            bail!("j-defaul.{key}: expected on_enter.teardown_work=true; got {parsed}");
        }
        if parsed.get("kill_sessions").is_some() {
            bail!(
                "j-defaul.{key}: expected legacy on_enter.kill_sessions key to be stripped; got {parsed}"
            );
        }
        // The prior 20260618 migration also sets clear_assignee=true on
        // these rows — the seed + rename pair must preserve it.
        if parsed.get("clear_assignee") != Some(&serde_json::json!(true)) {
            bail!(
                "j-defaul.{key}: expected on_enter.clear_assignee=true to be preserved; got {parsed}"
            );
        }
    }

    for key in ["open", "in-progress"] {
        let row = sqlx::query(
            "SELECT on_enter FROM statuses \
             WHERE project_id = 'j-defaul' AND key = ?1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter")?;
        if on_enter_text.is_some() {
            bail!("j-defaul.{key}: expected on_enter=NULL post-migration; got {on_enter_text:?}");
        }
    }
    Ok(())
}

async fn rename_kill_sessions_to_teardown_work_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT key, on_enter FROM statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter before rename idempotency rerun")?;

    sqlx::raw_sql(
        "UPDATE statuses \
         SET on_enter = json_remove( \
             json_set(on_enter, '$.teardown_work', json('true')), \
             '$.kill_sessions' \
         ) \
         WHERE on_enter IS NOT NULL \
           AND json_extract(on_enter, '$.kill_sessions') IS NOT NULL",
    )
    .execute(pool)
    .await
    .context("re-apply rename migration body verbatim")?;

    let snapshot_after = sqlx::query(
        "SELECT key, on_enter FROM statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter after rename idempotency rerun")?;

    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across rename rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_key: String = before.try_get("key")?;
        let after_key: String = after.try_get("key")?;
        let before_on_enter: Option<String> = before.try_get("on_enter")?;
        let after_on_enter: Option<String> = after.try_get("on_enter")?;
        if before_key != after_key || before_on_enter != after_on_enter {
            bail!(
                "j-defaul.{before_key} changed across rename rerun: \
                 {before_on_enter:?} -> {after_on_enter:?}"
            );
        }
    }
    Ok(())
}

async fn drop_is_assignment_agent_preserves_rows(pool: &SqlitePool) -> Result<()> {
    // `max_simultaneous` was renamed to `max_simultaneous_headless` by the
    // later `split_agents_max_simultaneous` migration; the baseline values
    // back-filled identically into `max_simultaneous_interactive`.
    let rows = sqlx::query(
        "SELECT name, prompt_path, max_tries, max_simultaneous_interactive, \
                max_simultaneous_headless, archived, \
                secrets, mcp_config_path, is_default_conversation_agent \
         FROM agents \
         WHERE name IN ('pm-baseline', 'chat-baseline', 'deleted-baseline') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("read agents rows seeded by the pre-drop-is-assignment-agent baseline")?;
    if rows.len() != 3 {
        bail!(
            "expected 3 baseline agents rows post-rebuild; got {}",
            rows.len()
        );
    }
    // Rows ordered by name: chat-baseline, deleted-baseline, pm-baseline.
    let chat = &rows[0];
    assert_eq!(chat.try_get::<String, _>("name")?, "chat-baseline");
    assert_eq!(chat.try_get::<i64, _>("max_tries")?, 5);
    assert_eq!(chat.try_get::<i64, _>("max_simultaneous_headless")?, 10);
    assert_eq!(chat.try_get::<i64, _>("max_simultaneous_interactive")?, 10);
    assert_eq!(chat.try_get::<i64, _>("archived")?, 0);
    assert_eq!(
        chat.try_get::<String, _>("secrets")?,
        "[\"OPENAI_API_KEY\"]"
    );
    assert_eq!(
        chat.try_get::<Option<String>, _>("mcp_config_path")?,
        Some("/agents/chat-baseline/mcp.json".to_string())
    );
    assert_eq!(chat.try_get::<i64, _>("is_default_conversation_agent")?, 1);

    let archived = &rows[1];
    assert_eq!(archived.try_get::<String, _>("name")?, "deleted-baseline");
    assert_eq!(archived.try_get::<i64, _>("archived")?, 1);
    assert_eq!(archived.try_get::<i64, _>("max_simultaneous_headless")?, 1);
    assert_eq!(
        archived.try_get::<i64, _>("max_simultaneous_interactive")?,
        1
    );

    let pm = &rows[2];
    assert_eq!(pm.try_get::<String, _>("name")?, "pm-baseline");
    assert_eq!(pm.try_get::<i64, _>("max_tries")?, 3);
    assert_eq!(
        pm.try_get::<i64, _>("max_simultaneous_headless")?,
        2147483647
    );
    assert_eq!(
        pm.try_get::<i64, _>("max_simultaneous_interactive")?,
        2147483647
    );
    assert_eq!(pm.try_get::<i64, _>("archived")?, 0);
    assert_eq!(pm.try_get::<i64, _>("is_default_conversation_agent")?, 0);

    Ok(())
}

async fn drop_is_assignment_agent_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT name, prompt_path, max_tries, \
                max_simultaneous_interactive, max_simultaneous_headless, archived, \
                secrets, mcp_config_path, is_default_conversation_agent \
         FROM agents ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("snapshot agents before drop-is-assignment-agent idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm drop-is-assignment-agent idempotency")?;
    let snapshot_after = sqlx::query(
        "SELECT name, prompt_path, max_tries, \
                max_simultaneous_interactive, max_simultaneous_headless, archived, \
                secrets, mcp_config_path, is_default_conversation_agent \
         FROM agents ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("snapshot agents after drop-is-assignment-agent idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across drop-is-assignment-agent idempotency rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_name: String = before.try_get("name")?;
        let after_name: String = after.try_get("name")?;
        if before_name != after_name {
            bail!("agents.name changed across rerun: {before_name} -> {after_name}");
        }
    }
    if column_exists(pool, "agents", "is_assignment_agent").await? {
        bail!(
            "`agents.is_assignment_agent` re-appeared after drop-is-assignment-agent \
             idempotency rerun; the rebuild reapplied"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260722000000_split_agents_max_simultaneous. Renames
// `agents.max_simultaneous` to `max_simultaneous_headless` and adds a new
// `max_simultaneous_interactive` column back-filled from the pre-migration
// value so existing per-agent caps are preserved on both axes.
// ---------------------------------------------------------------------------

async fn split_agents_max_simultaneous_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "agents", "max_simultaneous_interactive").await? {
        bail!("expected `agents.max_simultaneous_interactive` column to exist post-rollforward");
    }
    if !column_exists(pool, "agents", "max_simultaneous_headless").await? {
        bail!("expected `agents.max_simultaneous_headless` column to exist post-rollforward");
    }
    if column_exists(pool, "agents", "max_simultaneous").await? {
        bail!("expected `agents.max_simultaneous` column to be renamed away post-rollforward");
    }
    if column_is_nullable(pool, "agents", "max_simultaneous_interactive").await? {
        bail!("expected `agents.max_simultaneous_interactive` to be NOT NULL");
    }
    if column_is_nullable(pool, "agents", "max_simultaneous_headless").await? {
        bail!("expected `agents.max_simultaneous_headless` to be NOT NULL");
    }
    Ok(())
}

async fn split_agents_max_simultaneous_backfills_baselines(pool: &SqlitePool) -> Result<()> {
    // Every row carries `max_simultaneous_interactive ==
    // max_simultaneous_headless` after the back-fill (the rename copied
    // forward the prior value identically into both columns).
    let rows = sqlx::query(
        "SELECT name, max_simultaneous_interactive, max_simultaneous_headless \
         FROM agents ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("read agents rows for split_agents_max_simultaneous back-fill check")?;
    if rows.is_empty() {
        bail!("expected at least one agents row to assert back-fill against");
    }
    for row in &rows {
        let name: String = row.try_get("name")?;
        let interactive: i64 = row.try_get("max_simultaneous_interactive")?;
        let headless: i64 = row.try_get("max_simultaneous_headless")?;
        if interactive != headless {
            bail!(
                "agents.{name}: expected max_simultaneous_interactive == max_simultaneous_headless \
                 after back-fill; got interactive={interactive}, headless={headless}"
            );
        }
    }
    Ok(())
}

async fn split_agents_max_simultaneous_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT name, max_simultaneous_interactive, max_simultaneous_headless \
         FROM agents ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("snapshot agents before split_agents_max_simultaneous idempotency rerun")?;
    sqlite_store::run_migrations(pool, None).await.context(
        "re-apply sqlite migrations to confirm split_agents_max_simultaneous idempotency",
    )?;
    let snapshot_after = sqlx::query(
        "SELECT name, max_simultaneous_interactive, max_simultaneous_headless \
         FROM agents ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("snapshot agents after split_agents_max_simultaneous idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across split_agents_max_simultaneous idempotency rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_name: String = before.try_get("name")?;
        let after_name: String = after.try_get("name")?;
        let before_interactive: i64 = before.try_get("max_simultaneous_interactive")?;
        let after_interactive: i64 = after.try_get("max_simultaneous_interactive")?;
        let before_headless: i64 = before.try_get("max_simultaneous_headless")?;
        let after_headless: i64 = after.try_get("max_simultaneous_headless")?;
        if before_name != after_name
            || before_interactive != after_interactive
            || before_headless != after_headless
        {
            bail!(
                "agents row changed across split_agents_max_simultaneous rerun: \
                 {before_name}({before_interactive}, {before_headless}) -> \
                 {after_name}({after_interactive}, {after_headless})"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260621000000_backfill_assignee_null_on_terminal_default_issues.
// Sister to the postgres test of the same name in `migration_roundtrip.rs`.
// Seeds fresh rows post-rollforward with non-NULL assignees in each
// of the three terminal default-project statuses (plus a non-terminal
// control row and a terminal-status row in a non-default project that
// must stay untouched), replays the migration body verbatim, and reads
// back to confirm the targeted rows were nulled and the rest survived.
// ---------------------------------------------------------------------------

const BACKFILL_ASSIGNEE_BODY_SQLITE: &str = "\
    UPDATE issues_v2 \
    SET assignee = NULL, \
        assignee_principal = NULL \
    WHERE is_latest = 1 \
      AND project_id = 'j-defaul' \
      AND status_sequence IN ( \
          SELECT sequence \
          FROM statuses \
          WHERE project_id = 'j-defaul' \
            AND key IN ('closed', 'dropped', 'failed') \
      ) \
      AND (assignee IS NOT NULL OR assignee_principal IS NOT NULL)";

async fn backfill_assignee_null_on_terminal_default_issues_nulls_targeted_rows(
    pool: &SqlitePool,
) -> Result<()> {
    sqlx::raw_sql(
        "INSERT INTO issues_v2 \
         (id, version_number, issue_type, description, creator, project_id, status_sequence, assignee, assignee_principal, is_latest) \
         VALUES \
         ('i-bfacl', 1, 'task', 'closed with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM statuses WHERE project_id='j-defaul' AND key='closed'), \
            'agents/swe', '{\"Agent\":{\"name\":\"swe\"}}', 1), \
         ('i-bfadrp', 1, 'task', 'dropped with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM statuses WHERE project_id='j-defaul' AND key='dropped'), \
            'agents/pm', '{\"Agent\":{\"name\":\"pm\"}}', 1), \
         ('i-bfafld', 1, 'task', 'failed with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM statuses WHERE project_id='j-defaul' AND key='failed'), \
            'agents/reviewer', '{\"Agent\":{\"name\":\"reviewer\"}}', 1), \
         ('i-bfaopn', 1, 'task', 'open with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM statuses WHERE project_id='j-defaul' AND key='open'), \
            'agents/swe', '{\"Agent\":{\"name\":\"swe\"}}', 1), \
         ('i-bfacus', 1, 'task', 'shipped (terminal) in custom project', 'jayantk', 'j-cutsteady', \
            (SELECT sequence FROM statuses WHERE project_id='j-cutsteady' AND key='shipped'), \
            'users/jayantk', '{\"User\":{\"name\":\"jayantk\"}}', 1)",
    )
    .execute(pool)
    .await
    .context("seed backfill_assignee test rows")?;

    sqlx::raw_sql(BACKFILL_ASSIGNEE_BODY_SQLITE)
        .execute(pool)
        .await
        .context("re-apply backfill_assignee migration body")?;

    for id in ["i-bfacl", "i-bfadrp", "i-bfafld"] {
        let row = sqlx::query(
            "SELECT assignee, assignee_principal \
             FROM issues_v2 WHERE id = ?1 AND is_latest = 1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read backfill target {id}"))?;
        let assignee: Option<String> = row.try_get("assignee")?;
        let principal: Option<String> = row.try_get("assignee_principal")?;
        if assignee.is_some() || principal.is_some() {
            bail!(
                "{id}: expected assignee and assignee_principal NULL post-backfill; \
                 got assignee={assignee:?}, assignee_principal={principal:?}"
            );
        }
    }

    let row = sqlx::query("SELECT assignee FROM issues_v2 WHERE id = 'i-bfaopn' AND is_latest = 1")
        .fetch_one(pool)
        .await
        .context("read non-terminal control row")?;
    let assignee: Option<String> = row.try_get("assignee")?;
    if assignee.as_deref() != Some("agents/swe") {
        bail!("i-bfaopn (open default-project): expected assignee retained; got {assignee:?}");
    }

    let row = sqlx::query("SELECT assignee FROM issues_v2 WHERE id = 'i-bfacus' AND is_latest = 1")
        .fetch_one(pool)
        .await
        .context("read custom-project control row")?;
    let assignee: Option<String> = row.try_get("assignee")?;
    if assignee.as_deref() != Some("users/jayantk") {
        bail!("i-bfacus (terminal custom-project): expected assignee retained; got {assignee:?}");
    }

    Ok(())
}

async fn backfill_assignee_null_on_terminal_default_issues_is_idempotent(
    pool: &SqlitePool,
) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT id, assignee, assignee_principal \
         FROM issues_v2 WHERE is_latest = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .context("snapshot issues_v2 before backfill_assignee idempotency rerun")?;

    sqlx::raw_sql(BACKFILL_ASSIGNEE_BODY_SQLITE)
        .execute(pool)
        .await
        .context("re-apply backfill_assignee migration body for idempotency")?;

    let snapshot_after = sqlx::query(
        "SELECT id, assignee, assignee_principal \
         FROM issues_v2 WHERE is_latest = 1 ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .context("snapshot issues_v2 after backfill_assignee idempotency rerun")?;

    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across backfill_assignee rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_id: String = before.try_get("id")?;
        let after_id: String = after.try_get("id")?;
        let before_assignee: Option<String> = before.try_get("assignee")?;
        let after_assignee: Option<String> = after.try_get("assignee")?;
        let before_principal: Option<String> = before.try_get("assignee_principal")?;
        let after_principal: Option<String> = after.try_get("assignee_principal")?;
        if before_id != after_id
            || before_assignee != after_assignee
            || before_principal != after_principal
        {
            bail!(
                "issues_v2.{before_id} changed across backfill_assignee rerun: \
                 ({before_assignee:?}, {before_principal:?}) -> \
                 ({after_assignee:?}, {after_principal:?})"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260623000000_add_statuses_suppress_sessions. Adds
// `suppress_sessions BOOLEAN NOT NULL DEFAULT FALSE` to `statuses` — the
// schema-only prerequisite (PR-A) for the per-status session-suppression
// feature. No Rust code reads or writes the column yet (lands in PR-B);
// existing rows backfill to FALSE via the column default.
// ---------------------------------------------------------------------------

async fn add_statuses_suppress_sessions_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "statuses", "suppress_sessions").await? {
        bail!("expected `statuses.suppress_sessions` column to exist post-rollforward");
    }
    if column_is_nullable(pool, "statuses", "suppress_sessions").await? {
        bail!("expected `statuses.suppress_sessions` to be NOT NULL");
    }
    // pragma_table_info exposes the declared DEFAULT — verify the
    // FALSE default is what backs the no-backfill rollout.
    let row = sqlx::query(
        "SELECT dflt_value FROM pragma_table_info('statuses') WHERE name = 'suppress_sessions'",
    )
    .fetch_one(pool)
    .await
    .context("read pragma_table_info default for statuses.suppress_sessions")?;
    let default_text: Option<String> = row.try_get(0)?;
    let default_text = default_text
        .ok_or_else(|| anyhow::anyhow!("statuses.suppress_sessions has no declared default"))?;
    if !default_text.eq_ignore_ascii_case("FALSE") && default_text != "0" {
        bail!("expected statuses.suppress_sessions DEFAULT FALSE; got {default_text:?}");
    }
    Ok(())
}

async fn add_statuses_suppress_sessions_defaults_to_false(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query("SELECT project_id, sequence, suppress_sessions FROM statuses")
        .fetch_all(pool)
        .await
        .context("read statuses for suppress_sessions default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: bool = row.try_get("suppress_sessions")?;
        if value {
            bail!(
                "statuses({project_id}, sequence={sequence}): expected suppress_sessions=false (no backfill); got true"
            );
        }
    }
    Ok(())
}

async fn add_statuses_suppress_sessions_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT project_id, sequence, suppress_sessions FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses before suppress_sessions idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm suppress_sessions idempotency")?;
    let snapshot_after = sqlx::query(
        "SELECT project_id, sequence, suppress_sessions FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses after suppress_sessions idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across suppress_sessions idempotency rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_val: bool = before.try_get("suppress_sessions")?;
        let after_val: bool = after.try_get("suppress_sessions")?;
        if before_val != after_val {
            bail!("suppress_sessions changed across rerun: {before_val} -> {after_val}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260711000000_create_issue_comments. Creates the `issue_comments` table
// for the per-issue append-only comments feature. Append-only, no backfill,
// no FK to `issues_v2`. Sister Postgres migration is part of PR-3.
// ---------------------------------------------------------------------------

async fn create_issue_comments_schema_invariants(pool: &SqlitePool) -> Result<()> {
    let table = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'issue_comments'",
    )
    .fetch_optional(pool)
    .await
    .context("look up sqlite_master for issue_comments table")?;
    if table.is_none() {
        bail!("expected `issue_comments` table to exist post-rollforward");
    }

    for (column, nullable) in [
        ("issue_id", false),
        ("sequence", false),
        ("body", false),
        ("actor", false),
        ("created_at", false),
    ] {
        if !column_exists(pool, "issue_comments", column).await? {
            bail!("expected `issue_comments.{column}` column to exist post-rollforward");
        }
        if column_is_nullable(pool, "issue_comments", column).await? != nullable {
            bail!(
                "expected `issue_comments.{column}` nullable={nullable}; got {}",
                !nullable
            );
        }
    }

    // PK is (issue_id, sequence) — pragma_table_info exposes the `pk`
    // column with the 1-based PK ordinal of each member (0 for non-PK).
    let pk_rows = sqlx::query("SELECT name, pk FROM pragma_table_info('issue_comments')")
        .fetch_all(pool)
        .await
        .context("pragma_table_info(issue_comments) for PK shape")?;
    let mut pk: Vec<(i64, String)> = pk_rows
        .iter()
        .filter_map(|r| {
            let name: String = r.try_get("name").ok()?;
            let pk_pos: i64 = r.try_get("pk").ok()?;
            (pk_pos > 0).then_some((pk_pos, name))
        })
        .collect();
    pk.sort_by_key(|(pos, _)| *pos);
    let pk_names: Vec<String> = pk.into_iter().map(|(_, n)| n).collect();
    if pk_names != ["issue_id".to_string(), "sequence".to_string()] {
        bail!("expected issue_comments PK to be (issue_id, sequence); got {pk_names:?}");
    }

    // `created_at` declared default must be the SQLite strftime
    // expression; pragma_table_info wraps the body in parentheses.
    let row = sqlx::query(
        "SELECT dflt_value FROM pragma_table_info('issue_comments') WHERE name = 'created_at'",
    )
    .fetch_one(pool)
    .await
    .context("read pragma_table_info default for issue_comments.created_at")?;
    let default_text: Option<String> = row.try_get(0)?;
    let default_text = default_text
        .ok_or_else(|| anyhow::anyhow!("issue_comments.created_at has no declared default"))?;
    if !default_text.contains("strftime") {
        bail!("expected issue_comments.created_at default to use strftime; got {default_text:?}");
    }

    if !index_exists(pool, "issue_comments_issue_seq_desc_idx").await? {
        bail!("expected `issue_comments_issue_seq_desc_idx` index to exist post-rollforward");
    }

    Ok(())
}

async fn create_issue_comments_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let count_before: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM issue_comments")
        .fetch_one(pool)
        .await
        .context("count issue_comments before idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm create_issue_comments idempotency")?;
    let count_after: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM issue_comments")
        .fetch_one(pool)
        .await
        .context("count issue_comments after idempotency rerun")?;
    if count_before != count_after {
        bail!(
            "issue_comments row count changed across idempotency rerun: {count_before} -> {count_after}"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema-invariants: pagination indexes on the four list-* tables. All four
// paginate by `created_at DESC, id DESC`. `issues_v2` is covered by the
// partial `issues_v2_latest_pagination_idx (WHERE is_latest = true)` from
// 20260318000000; the other three are covered by full `(created_at DESC, id
// DESC)` indexes. Mirrors postgres migrations 20260315000000, 20260317000000,
// 20260318000000, 20260605000000, and 20260622000000; ported to SQLite by
// the matching sqlite-migrations entries.
// ---------------------------------------------------------------------------

async fn assert_pagination_indexes_exist(pool: &SqlitePool) -> Result<()> {
    for name in [
        "issues_v2_latest_pagination_idx",
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
    // The original `issues_v2_created_at_id_idx` (non-partial) was dropped by
    // 20260605000000; the planner uses the partial
    // `issues_v2_latest_pagination_idx` instead. Catch any future migration
    // that re-creates the non-partial form without thinking.
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
    // `issues_v2_updated_at_id_idx` was added by 20260605000000 when
    // `list_issues` paginated on `updated_at`, then dropped by 20260622000000
    // after the keyset was swung back to `created_at` (p-kzbakldw). Catch any
    // future migration that re-creates this known-dead index.
    let stale_updated_at = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'issues_v2_updated_at_id_idx'",
    )
    .fetch_optional(pool)
    .await
    .context("query sqlite_master for dropped index issues_v2_updated_at_id_idx")?;
    if stale_updated_at.is_some() {
        bail!(
            "issues_v2_updated_at_id_idx should have been dropped by 20260622000000; \
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
    // Recently-added tables only — the older set is covered implicitly
    // by the store-level smoke tests that read/write them. Listed
    // explicitly so a future rename without a baseline bump fails this
    // assertion loud. Tables here come from
    // 20260603020000_add_triggers_table.sql,
    // 20260604000001_create_projects.sql, and
    // 20260613000000_create_statuses.sql.
    for table in ["triggers", "projects", "statuses"] {
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

    // Column added by 20260609000000_add_creator_to_auth_tokens.sql.
    if !column_exists(pool, "auth_tokens", "creator").await? {
        bail!("expected `auth_tokens.creator` column to exist after rollforward");
    }
    if column_is_nullable(pool, "auth_tokens", "creator").await? {
        bail!("expected `auth_tokens.creator` to be NOT NULL after rollforward");
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
        "statuses_project_key_idx",
        "issues_v2_project_status_sequence_idx",
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
            last_fired_at, archived, actor, is_latest) \
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
    // Post-cutover (20260614000000), `projects.statuses` JSONB was
    // dropped and `metis.statuses` is the source of truth. Smoke
    // INSERT writes both the `projects` row (with the new
    // `next_status_sequence` high-water mark) and the matching
    // `statuses` row so the FK-bearing IssueRow inserts below resolve
    // their `(project_id, status_sequence)` references.
    sqlx::query(
        "INSERT INTO projects \
           (id, version_number, key, name, \
            creator, archived, actor, prompt_path, next_status_sequence, is_latest) \
         VALUES (?1, 1, ?2, ?3, ?4, 0, NULL, ?5, 2, 1)",
    )
    .bind(project_id)
    .bind("smoke")
    .bind("Smoke")
    .bind("alice")
    .bind("/projects/smoke/prompt.md")
    .execute(pool)
    .await
    .context("insert smoke project row")?;
    sqlx::query(
        "INSERT INTO statuses (project_id, sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive) \
         VALUES (?1, 1, 'todo', 'Todo', '#abcdef', 0, 0, 0, NULL, NULL, 0)",
    )
    .bind(project_id)
    .execute(pool)
    .await
    .context("insert smoke project statuses row")?;

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
    // Post-cutover (20260614000000), `projects.statuses` JSONB is gone;
    // the seed's status set lives in the `statuses` table. The
    // `projects` row still carries the rest of the seed payload.
    let row = sqlx::query(
        "SELECT id, version_number, key, name, \
                creator, archived, actor, is_latest, prompt_path \
         FROM projects WHERE id = 'j-defaul'",
    )
    .fetch_one(pool)
    .await
    .context("read seeded default project row 'j-defaul'")?;

    let id: String = row.try_get("id")?;
    let version_number: i64 = row.try_get("version_number")?;
    let key: String = row.try_get("key")?;
    let name: String = row.try_get("name")?;
    let creator: String = row.try_get("creator")?;
    let archived: i64 = row.try_get("archived")?;
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
    if creator != "system" {
        bail!("j-defaul: expected creator='system'; got {creator:?}");
    }
    if archived != 0 {
        bail!("j-defaul: expected archived=0; got {archived}");
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

    // The status set lives on `statuses` after the cutover. Read the
    // rows in sequence order and rebuild a `Vec<StatusDefinition>` for
    // comparison against `default_project_seed().statuses`.
    let status_rows = sqlx::query(
        "SELECT sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive \
         FROM statuses WHERE project_id = 'j-defaul' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read j-defaul rows from `statuses`")?;
    let mut statuses: Vec<StatusDefinition> = Vec::with_capacity(status_rows.len());
    for row in &status_rows {
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: i64 = row.try_get("unblocks_parents")?;
        let unblocks_dependents: i64 = row.try_get("unblocks_dependents")?;
        let cascades_to_children: i64 = row.try_get("cascades_to_children")?;
        let on_enter: Option<String> = row.try_get("on_enter")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: i64 = row.try_get("interactive")?;
        let on_enter_value = on_enter
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("decode statuses.on_enter")?;
        let mut def = StatusDefinition::new(
            hydra_common::api::v1::projects::StatusKey::try_new(key)
                .map_err(|e| anyhow::anyhow!("invalid status key: {e}"))?,
            label,
            color
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid status color: {e}"))?,
            unblocks_parents != 0,
            unblocks_dependents != 0,
            cascades_to_children != 0,
            on_enter_value,
        );
        def.prompt_path = prompt_path;
        def.interactive = interactive != 0;
        statuses.push(def);
    }
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

// ---------------------------------------------------------------------------
// 20260608000000_drop_status_icon — assert that the migration strips the
// `icon` key from every row's `projects.statuses` array (both the
// already-seeded `j-defaul` row and a custom fixture row), and that the
// migration body is idempotent on a second pass. The store-level read of
// `j-iconfix` doubles as the parent-issue's `SqliteStore` round-trip
// gate: the migrated JSON must deserialize into the new
// `Vec<StatusDefinition>` (no `icon` field) without serde error and with
// the expected `statuses.len()`. Covers [[i-jazguvll]] §E.
// ---------------------------------------------------------------------------

async fn drop_status_icon_migration_strips_default_seed(pool: &SqlitePool) -> Result<()> {
    use hydra_common::ProjectId;

    // Store-level smoke: read j-defaul through `SqliteStore::get_project`
    // so any drift between the post-strip JSON shape and the typed
    // `Vec<StatusDefinition>` (no `icon` field) deserializer fails loud.
    // `seed_default_project_migration_inserts_row` already compares the
    // Vec to `default_project_seed()`; this also exercises the
    // `SqliteStore`-driven SELECT projection per [[i-jazguvll]] §E.
    let store = SqliteStore::new(pool.clone());
    let pid = ProjectId::from_str("j-defaul").context("parse 'j-defaul'")?;
    let fetched = store
        .get_project(&pid, false)
        .await
        .context("SqliteStore::get_project(j-defaul) post-drop_status_icon")?;
    if fetched.item.statuses.len() != 5 {
        bail!(
            "j-defaul: expected 5 statuses post-drop_status_icon; got {}",
            fetched.item.statuses.len()
        );
    }

    // Belt-and-braces structural check post-cutover: the legacy
    // `projects.statuses` JSONB column was dropped by
    // 20260614000000_cutover_to_statuses_table; the per-status rows
    // now live in the `statuses` table, which has no `icon` column at
    // all. The store-level read above already validates the
    // post-strip column set against `default_project_seed()`; the
    // additional invariant we want to capture here is "no `icon`
    // anywhere in the new schema" — assert that by checking
    // `pragma_table_info` on the `statuses` table.
    let column_names: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM pragma_table_info('statuses')")
            .fetch_all(pool)
            .await
            .context("read statuses table info post-drop_status_icon")?;
    for (name,) in &column_names {
        if name == "icon" {
            bail!("statuses table still carries an `icon` column post-strip");
        }
    }
    Ok(())
}

async fn drop_status_icon_migration_strips_custom_row(pool: &SqlitePool) -> Result<()> {
    // The `j-iconfix` row was inserted by the
    // `20260607000000__pre_drop_status_icon` baseline with three statuses
    // that each carry `"icon": "<value>"`. Read back through
    // `SqliteStore::get_project` so any drift between the migration's
    // post-strip JSON shape and the Rust `StatusDefinition` serde impl
    // fails loud here (the typed deserializer must accept the migrated
    // rows). This is the §E "migration-roundtrip + serde" gate from
    // [[i-jazguvll]].
    let store = SqliteStore::new(pool.clone());
    let pid = ProjectId::from_str("j-iconfix").context("parse 'j-iconfix'")?;
    let fetched = store
        .get_project(&pid, false)
        .await
        .context("SqliteStore::get_project(j-iconfix) post-drop_status_icon")?;

    if fetched.item.key.as_str() != "iconfix" {
        bail!(
            "j-iconfix: expected key='iconfix'; got {:?}",
            fetched.item.key
        );
    }
    if fetched.item.name != "Icon Fixture" {
        bail!(
            "j-iconfix: expected name='Icon Fixture'; got {:?}",
            fetched.item.name
        );
    }
    if fetched.item.creator.as_str() != "jayantk" {
        bail!(
            "j-iconfix: expected creator='jayantk'; got {:?}",
            fetched.item.creator.as_str()
        );
    }
    if fetched.item.statuses.len() != 3 {
        bail!(
            "j-iconfix: expected 3 statuses; got {}",
            fetched.item.statuses.len()
        );
    }
    let expected_shapes: &[(&str, &str, &str, bool, bool, bool)] = &[
        ("todo", "Todo", "#abcdef", false, false, false),
        ("doing", "Doing", "#f1c40f", false, false, false),
        ("done", "Done", "#2ecc71", true, true, false),
    ];
    for (i, (k, label, color, up, ud, ctc)) in expected_shapes.iter().enumerate() {
        let s = &fetched.item.statuses[i];
        if s.key.as_str() != *k
            || s.label != *label
            || s.color.as_ref() != *color
            || s.unblocks_parents != *up
            || s.unblocks_dependents != *ud
            || s.cascades_to_children != *ctc
        {
            bail!(
                "j-iconfix.statuses[{i}]: expected ({k}, {label}, {color}, {up}, {ud}, {ctc}); got {s:?}"
            );
        }
    }

    // The legacy `projects.statuses` JSONB column was dropped by
    // 20260614000000_cutover_to_statuses_table. The store-level
    // read above already validates the post-strip column shape; the
    // schema-level "no icon column anywhere" check is performed once
    // in `drop_status_icon_migration_strips_default_seed` against
    // the `statuses` table (shared across every project), so there's
    // nothing per-row to re-assert here.

    Ok(())
}

async fn drop_status_icon_migration_is_idempotent(_pool: &SqlitePool) -> Result<()> {
    // Originally this re-executed `20260608000000_drop_status_icon.sql`
    // verbatim and asserted the resulting `projects.statuses` JSONB
    // arrays were unchanged. After
    // `20260614000000_cutover_to_statuses_table` drops that JSONB
    // column entirely, the body's `UPDATE projects SET statuses =
    // json_remove(statuses, '$.icon')` no longer references an
    // existing column and cannot run. The idempotency guarantee that
    // still matters here — "the strip was destructive of the `icon`
    // key, not of the migration-tracker state" — is captured by the
    // earlier `_strips_default_seed` invariant (no `icon` column on
    // the new `statuses` table) plus the migration tracker preventing
    // a second apply at the framework level.
    Ok(())
}

async fn seed_default_project_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // Original behavior of this assertion was to replay the seed
    // migration body verbatim. After
    // 20260611000000_drop_projects_default_status_key drops a column
    // the body references, a verbatim re-apply errors — so the
    // idempotency guarantee that matters here is now: after the full
    // migration plan rolls forward, exactly one j-defaul row exists.
    let row = sqlx::query("SELECT COUNT(*) FROM projects WHERE id = 'j-defaul'")
        .fetch_one(pool)
        .await
        .context("count projects rows for j-defaul after rollforward")?;
    let count: i64 = row.try_get(0)?;
    if count != 1 {
        bail!("expected exactly 1 projects row for j-defaul post-rollforward; got {count}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260609000000_add_creator_to_auth_tokens / 20260609010000_drop_actors_v2
// ---------------------------------------------------------------------------

/// SQL-level assertion: the session-bound token (`hash-session-alice`)
/// in the pre-denormalize baseline must end up with `creator = 'alice'`,
/// copied off `tasks_v2.s-alice001.creator`.
async fn denormalize_creator_session_backfill(pool: &SqlitePool) -> Result<()> {
    let row =
        sqlx::query("SELECT creator FROM auth_tokens WHERE token_hash = 'hash-session-alice'")
            .fetch_one(pool)
            .await
            .context("read back session-bound auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "alice" {
        bail!("session-bound auth_tokens.creator: expected 'alice'; got {creator:?}");
    }
    Ok(())
}

/// SQL-level assertion: the user CLI token (`hash-cli-bob`) in the
/// pre-denormalize baseline must end up with `creator = 'bob'`, parsed
/// off `users/bob`.
async fn denormalize_creator_user_backfill(pool: &SqlitePool) -> Result<()> {
    let row = sqlx::query("SELECT creator FROM auth_tokens WHERE token_hash = 'hash-cli-bob'")
        .fetch_one(pool)
        .await
        .context("read back user-CLI auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "bob" {
        bail!("user-CLI auth_tokens.creator: expected 'bob'; got {creator:?}");
    }
    Ok(())
}

/// Domain-level read-back via the running `SqliteStore::get_auth_token_by_hash`
/// — catches serde drift between the migration column shape and
/// `AuthTokenRow` deserialization.
async fn denormalize_creator_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let session_row = store
        .get_auth_token_by_hash("hash-session-alice")
        .await
        .context("SqliteStore::get_auth_token_by_hash(hash-session-alice)")?
        .context("session-bound row not found via domain API")?;
    if session_row.creator.as_str() != "alice" {
        bail!(
            "domain-level session-bound creator: expected 'alice'; got {:?}",
            session_row.creator
        );
    }
    if session_row.actor_name != "agents/swe" {
        bail!(
            "domain-level session-bound actor_name: expected 'agents/swe'; got {:?}",
            session_row.actor_name
        );
    }

    let user_row = store
        .get_auth_token_by_hash("hash-cli-bob")
        .await
        .context("SqliteStore::get_auth_token_by_hash(hash-cli-bob)")?
        .context("user-CLI row not found via domain API")?;
    if user_row.creator.as_str() != "bob" {
        bail!(
            "domain-level user-CLI creator: expected 'bob'; got {:?}",
            user_row.creator
        );
    }
    if user_row.session_id.is_some() {
        bail!(
            "domain-level user-CLI session_id: expected None; got {:?}",
            user_row.session_id
        );
    }
    Ok(())
}

/// The follow-on migration must drop the `actors_v2` table outright.
async fn drop_actors_v2_migration_removes_table(pool: &SqlitePool) -> Result<()> {
    if table_exists(pool, "actors_v2").await? {
        bail!("expected `actors_v2` table to be dropped after 20260609010000");
    }
    Ok(())
}

/// Both migrations must be idempotent under re-execution. We re-apply
/// the add-creator body (which lives inside a CREATE-NEW / RENAME dance
/// that would error on a re-run if not guarded) — except we can't
/// easily re-run that one on a real schema. Instead, assert that a
/// fresh INSERT into the post-migration `auth_tokens` shape works and
/// reads back through the domain API, which catches the most likely
/// regression: the migration leaving the table in a write-broken state.
async fn denormalize_creator_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let creator = Username::from("eve");
    let sid = SessionId::new();
    // Reuse the session row from the baseline by inserting a fresh
    // tasks_v2 row so the session id is parseable; the auth-token write
    // does not enforce FK so we can just write directly.
    store
        .add_auth_token("users/eve", "hash-eve", Some(&sid), &creator)
        .await
        .context("post-migration add_auth_token write")?;
    let fresh = store
        .get_auth_token_by_hash("hash-eve")
        .await
        .context("post-migration get_auth_token_by_hash")?
        .context("post-migration write should be readable")?;
    if fresh.creator != creator {
        bail!(
            "post-migration write read-back: expected creator='eve'; got {:?}",
            fresh.creator
        );
    }
    if fresh.session_id.as_ref() != Some(&sid) {
        bail!(
            "post-migration write read-back: expected session_id={sid:?}; got {:?}",
            fresh.session_id
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260610000000_add_projects_priority — assert that the new `priority`
// column is backfilled to `rank * 1000.0` over the latest-version rows
// (ranked by `created_at DESC, id DESC`), and that the
// `SqliteStore::list_projects` typed read path surfaces the backfilled
// value (this catches the `#[sqlx(default)]` / SELECT-projection
// foot-gun that the parent issue calls out: if `priority` is missing
// from the SELECT, the round-trip surfaces a `0.0` instead of the
// backfilled value).
// ---------------------------------------------------------------------------

async fn add_projects_priority_backfill_sql_level(pool: &SqlitePool) -> Result<()> {
    // The three baseline rows (j-prione, j-pritwo, j-pritri) have
    // explicit `created_at` values in 2027 — far ahead of any other
    // row's wall-clock timestamp — so they take ranks 1 / 2 / 3 and
    // come out with priorities 1000 / 2000 / 3000 respectively.
    let expected: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
    for (id, want) in expected {
        let row = sqlx::query(
            "SELECT priority FROM projects \
             WHERE id = ?1 AND is_latest = 1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read projects.priority for {id}"))?;
        let got: f64 = row.try_get("priority")?;
        if got != *want {
            bail!("projects({id}).priority: expected {want}; got {got}");
        }
    }
    Ok(())
}

async fn add_projects_priority_backfill_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    // Round-trip through `SqliteStore::list_projects` so any drift
    // between the migration's column shape and the typed `ProjectRow` /
    // `row_to_project` projection fails loud. The list is sorted by
    // `priority ASC, id ASC`; we filter to just the three baseline rows
    // to keep the assertion stable against unrelated smoke inserts in
    // `assert_recent_migration_store_smoke` that land at the default
    // `priority = 0.0`.
    let store = SqliteStore::new(pool.clone());
    let listed = store
        .list_projects(false)
        .await
        .context("SqliteStore::list_projects(include_archived = false)")?;

    let want: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
    // list_projects is already sorted by priority ASC; filter preserves
    // order. Filter to just the baseline rows and compare directly.
    let got: Vec<(String, f64)> = listed
        .iter()
        .filter_map(|(id, v)| {
            let id_str = id.as_ref().to_string();
            if want.iter().any(|(w, _)| *w == id_str.as_str()) {
                Some((id_str, v.item.priority))
            } else {
                None
            }
        })
        .collect();
    let want_owned: Vec<(String, f64)> = want.iter().map(|(s, p)| (s.to_string(), *p)).collect();
    if got != want_owned {
        bail!("list_projects filtered to baseline rows: expected {want_owned:?}; got {got:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260611000000_drop_projects_default_status_key — assert that the
// migration removes the `default_status_key` column from `projects`,
// that the seeded `j-defaul` row and the custom `j-dskdrop` baseline row
// still deserialize through `SqliteStore::get_project` into the new
// `Project` wire type (no `default_status_key` field), and that the
// table-rebuild body is idempotent on a second pass.
// ---------------------------------------------------------------------------

async fn drop_projects_default_status_key_migration_removes_column(
    pool: &SqlitePool,
) -> Result<()> {
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!(
            "expected projects.default_status_key to be dropped after \
             20260611000000_drop_projects_default_status_key"
        );
    }
    Ok(())
}

async fn drop_projects_default_status_key_migration_preserves_typed_read(
    pool: &SqlitePool,
) -> Result<()> {
    // Read the seeded `j-defaul` row plus the custom `j-dskdrop` baseline
    // row back through the typed store API. Both must deserialize into
    // `Project` without the `default_status_key` field — covers the
    // serde-projection foot-gun (post-migration SELECT must match the
    // ProjectRow struct, and the row must serde into the wire type).
    let store = SqliteStore::new(pool.clone());

    let defaul = ProjectId::from_str("j-defaul").context("parse 'j-defaul'")?;
    let fetched = store
        .get_project(&defaul, false)
        .await
        .context("SqliteStore::get_project(j-defaul) post-drop-default-status-key")?;
    if fetched.item.key.as_str() != "default" {
        bail!(
            "j-defaul: expected key='default'; got {:?}",
            fetched.item.key
        );
    }
    if fetched.item.statuses.len() != 5 {
        bail!(
            "j-defaul: expected 5 statuses; got {}",
            fetched.item.statuses.len()
        );
    }

    let dskdrop = ProjectId::from_str("j-dskdrop").context("parse 'j-dskdrop'")?;
    let fixture = store
        .get_project(&dskdrop, false)
        .await
        .context("SqliteStore::get_project(j-dskdrop) post-drop-default-status-key")?;
    if fixture.item.key.as_str() != "dskdrop" {
        bail!(
            "j-dskdrop: expected key='dskdrop'; got {:?}",
            fixture.item.key
        );
    }
    if fixture.item.statuses.len() != 3 {
        bail!(
            "j-dskdrop: expected 3 statuses; got {}",
            fixture.item.statuses.len()
        );
    }
    let keys: Vec<&str> = fixture
        .item
        .statuses
        .iter()
        .map(|s| s.key.as_str())
        .collect();
    if keys != ["todo", "doing", "done"] {
        bail!("j-dskdrop: expected statuses [todo,doing,done]; got {keys:?}");
    }
    Ok(())
}

async fn drop_projects_default_status_key_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // Originally this re-executed
    // `20260611000000_drop_projects_default_status_key.sql` verbatim
    // and compared the resulting `projects.statuses` JSONB snapshots.
    // After `20260614000000_cutover_to_statuses_table` drops the
    // `projects.statuses` column, the body's table-rebuild
    // (`INSERT INTO projects_new (..., statuses, ...) SELECT ..., statuses, ... FROM projects`)
    // no longer matches the current schema and cannot run verbatim.
    // The schema-shape invariant the test actually wants to lock down
    // — "`projects.default_status_key` is gone and stays gone" —
    // remains testable directly.
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!("drop_projects_default_status_key: column reappeared post-rollforward");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260612000000_issues_v2_project_id_not_null — assert the column is now
// NOT NULL, the post-migration table rejects fresh NULL inserts, the
// migration body is idempotent (table rebuild does not destabilize the
// `issues_v2` schema on second pass), and the pre-flight guard refuses
// to run when a stale NULL row remains in the table.
// ---------------------------------------------------------------------------

async fn issues_v2_project_id_is_not_null(pool: &SqlitePool) -> Result<()> {
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!(
            "expected `issues_v2.project_id` to be NOT NULL after \
             20260612000000_issues_v2_project_id_not_null"
        );
    }
    Ok(())
}

/// After the migration the table must reject fresh NULL `project_id`
/// inserts — the typed `Issue` shape no longer permits None, but
/// belt-and-braces verification at the SQL layer.
async fn issues_v2_project_id_rejects_null_insert(pool: &SqlitePool) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, project_id) \
         VALUES ('i-nullchk', 99, 'task', 'null project_id insert must fail', 'system', NULL)",
    )
    .execute(pool)
    .await;
    match result {
        Err(_) => Ok(()),
        Ok(_) => bail!(
            "expected NULL project_id INSERT to fail post-migration; \
             the NOT NULL constraint was not applied"
        ),
    }
}

/// Re-execute the migration body verbatim. The pre-flight guard passes
/// (no NULL rows survive) and the table rebuild rerun must produce the
/// same schema invariants.
async fn issues_v2_project_id_not_null_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // The body of `20260612000000_issues_v2_project_id_not_null.sql`
    // rebuilds `issues_v2` and explicitly enumerates `status` (the
    // legacy TEXT column). After
    // `20260614000000_cutover_to_statuses_table` drops that column,
    // the body can no longer be re-executed verbatim. Lock down the
    // surviving schema invariants directly: `project_id` is still
    // NOT NULL, and the indexes that survived the cutover rebuild
    // are still in place. (The cutover migration drops the old
    // `issues_v2_status_idx` along with the `status` column; the new
    // `issues_v2_project_status_sequence_idx` index replaces it.)
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!("expected `issues_v2.project_id` to stay NOT NULL post-rollforward");
    }
    for index in [
        "issues_v2_latest_idx",
        "issues_v2_latest_id_idx",
        "issues_v2_latest_pagination_idx",
        "issues_v2_project_id_idx",
        "issues_v2_project_status_sequence_idx",
    ] {
        if !index_exists(pool, index).await? {
            bail!("expected index `{index}` to survive the post-cutover schema");
        }
    }
    if index_exists(pool, "issues_v2_status_idx").await? {
        bail!(
            "expected legacy `issues_v2_status_idx` to be gone (dropped with the `status` column)"
        );
    }
    Ok(())
}

/// Pre-flight guard: against a fresh schema-at-baseline pool with a
/// stranded NULL `project_id` row, the migration body must fail loud
/// rather than silently coercing the row to the default project.
async fn issues_v2_project_id_not_null_migration_rejects_null_baseline() -> Result<()> {
    let pool = SqliteStore::init_pool("sqlite::memory:")
        .await
        .context("init in-memory sqlite pool for null-baseline rerun")?;

    // Roll forward to the prior migration so `issues_v2.project_id` is
    // still nullable.
    sqlite_store::run_migrations(&pool, Some(20260611000000))
        .await
        .context("roll forward to 20260611000000 baseline for null-guard test")?;

    // Seed a NULL `project_id` row.
    sqlx::query(
        "INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, project_id) \
         VALUES ('i-nullbase', 1, 'task', 'guard test row', 'system', NULL)",
    )
    .execute(&pool)
    .await
    .context("insert null project_id row")?;

    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("sqlite-migrations/20260612000000_issues_v2_project_id_not_null.sql"),
    )
    .context("read sqlite issues_v2_project_id_not_null migration body for null-baseline test")?;

    let result = sqlx::raw_sql(&body).execute(&pool).await;
    match result {
        Err(err) => {
            let msg = err.to_string();
            // SQLite's NOT NULL constraint violation on the
            // `issues_v2_new.project_id` column. The error message must
            // name the column so an operator knows where to look.
            if !msg.contains("project_id") {
                bail!("expected the migration error to mention 'project_id'; got: {msg}");
            }
            Ok(())
        }
        Ok(_) => bail!(
            "expected the migration body to fail loud on a NULL project_id \
             row; instead it completed successfully"
        ),
    }
}

// ---------------------------------------------------------------------------
// 20260613000000_create_statuses /
// 20260613010000_add_issues_v2_status_sequence — SQLite parity for the
// new `statuses` table and `issues_v2.status_sequence` backfill. Asserts
// the table exists with PK `(project_id, sequence)` and unique
// `(project_id, key)`, that the default-project JSONB seed backfills 1:1,
// that a custom baseline project's full-column-shape rows
// (`on_enter` JSON blob + `prompt_path` + `interactive: true`) round-trip,
// that every `issues_v2` row's `status_sequence` resolves back to its
// status key, that re-applying the `create_statuses` body adds no
// duplicates, and that the `add_issues_v2_status_sequence` pre-flight
// guard refuses to complete against a fresh schema-at-baseline pool
// with a stranded `(project_id, status)` row that has no matching
// `statuses` row. Covers [[i-jvmpqwwe]] acceptance criteria (a)–(d).
// ---------------------------------------------------------------------------

async fn create_statuses_migration_schema_invariants(pool: &SqlitePool) -> Result<()> {
    // SQLite primary key + unique index lookup goes through
    // `pragma_index_list` + `pragma_index_info`. The PK index name is
    // `sqlite_autoindex_statuses_1` for the first PRIMARY KEY constraint,
    // but the more robust check is to walk `pragma_index_list` and verify
    // we have a unique-primary-key entry whose columns are
    // `(project_id, sequence)`.
    let pk_cols = read_pk_columns(pool, "statuses").await?;
    if pk_cols != vec!["project_id".to_string(), "sequence".to_string()] {
        bail!("statuses PK: expected [project_id, sequence]; got {pk_cols:?}");
    }

    let cols = read_unique_index_columns(pool, "statuses_project_key_idx").await?;
    if cols != vec!["project_id".to_string(), "key".to_string()] {
        bail!("statuses_project_key_idx: expected unique [project_id, key]; got {cols:?}");
    }
    Ok(())
}

async fn read_pk_columns(pool: &SqlitePool, table: &str) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT name FROM pragma_table_info(?1) \
         WHERE pk > 0 ORDER BY pk",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .with_context(|| format!("read primary key columns for {table}"))?;
    let mut cols = Vec::with_capacity(rows.len());
    for row in rows {
        cols.push(row.try_get::<String, _>("name")?);
    }
    Ok(cols)
}

async fn read_unique_index_columns(pool: &SqlitePool, index: &str) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT il.\"unique\" AS is_unique, ii.name AS col \
         FROM pragma_index_list((SELECT tbl_name FROM sqlite_master WHERE name = ?1)) il \
         JOIN pragma_index_info(?1) ii \
         WHERE il.name = ?1 \
         ORDER BY ii.seqno",
    )
    .bind(index)
    .fetch_all(pool)
    .await
    .with_context(|| format!("read columns for index {index}"))?;
    if rows.is_empty() {
        bail!("index {index} not found");
    }
    let is_unique: i64 = rows[0].try_get("is_unique")?;
    if is_unique == 0 {
        bail!("index {index}: expected unique=true; got 0");
    }
    let mut cols = Vec::with_capacity(rows.len());
    for row in rows {
        cols.push(row.try_get::<String, _>("col")?);
    }
    Ok(cols)
}

async fn create_statuses_migration_backfills_default_seed(pool: &SqlitePool) -> Result<()> {
    let expected_seed = default_project_seed().statuses;
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter, prompt_path, interactive \
         FROM statuses WHERE project_id = 'j-defaul' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read statuses for j-defaul")?;
    if rows.len() != 5 {
        bail!("statuses(j-defaul): expected 5 rows; got {}", rows.len());
    }
    for (i, row) in rows.iter().enumerate() {
        let sequence: i64 = row.try_get("sequence")?;
        if sequence != (i + 1) as i64 {
            bail!(
                "statuses(j-defaul)[{i}]: expected sequence={}; got {sequence}",
                i + 1
            );
        }
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: i64 = row.try_get("unblocks_parents")?;
        let unblocks_dependents: i64 = row.try_get("unblocks_dependents")?;
        let cascades_to_children: i64 = row.try_get("cascades_to_children")?;
        let on_enter: Option<String> = row.try_get("on_enter")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: i64 = row.try_get("interactive")?;

        let expected = &expected_seed[i];
        let bool_to_int = |b: bool| if b { 1_i64 } else { 0 };
        if key != expected.key.as_str()
            || label != expected.label
            || color != expected.color.as_ref()
            || unblocks_parents != bool_to_int(expected.unblocks_parents)
            || unblocks_dependents != bool_to_int(expected.unblocks_dependents)
            || cascades_to_children != bool_to_int(expected.cascades_to_children)
            || prompt_path != expected.prompt_path
            || interactive != bool_to_int(expected.interactive)
        {
            bail!(
                "statuses(j-defaul)[{i}]: row did not match default_project_seed()[{i}]:\n  \
                 got: (key={key}, label={label}, color={color}, ub_par={unblocks_parents}, \
                 ub_dep={unblocks_dependents}, casc={cascades_to_children}, \
                 prompt_path={prompt_path:?}, interactive={interactive})\n  \
                 expected: {expected:?}"
            );
        }
        // The HEAD-state of `on_enter` reflects the
        // 20260618000000_add_clear_assignee_to_default_terminal_statuses,
        // 20260620000000_add_kill_sessions_to_default_terminal_statuses,
        // and 20260710000000_rename_kill_sessions_to_teardown_work
        // migrations: terminal rows carry
        // `{ clear_assignee: true, teardown_work: true }`; non-terminal
        // rows stay NULL. Compare against the expected
        // `default_project_seed()` shape rather than asserting NULL
        // outright.
        let expected_on_enter_json = expected
            .on_enter
            .as_ref()
            .map(|o| serde_json::to_value(o).expect("serialize StatusOnEnter"));
        let actual_on_enter_json = on_enter
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .with_context(|| format!("decode statuses(j-defaul)[{i}].on_enter JSON"))?;
        if expected_on_enter_json != actual_on_enter_json {
            bail!(
                "statuses(j-defaul)[{i}].on_enter: expected {expected_on_enter_json:?}; got {actual_on_enter_json:?}"
            );
        }
    }
    Ok(())
}

async fn create_statuses_migration_backfills_custom_project(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter, prompt_path, interactive \
         FROM statuses WHERE project_id = 'j-stsfixt' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read statuses for j-stsfixt")?;
    if rows.len() != 3 {
        bail!("statuses(j-stsfixt): expected 3 rows; got {}", rows.len());
    }

    struct Expected {
        sequence: i64,
        key: &'static str,
        label: &'static str,
        color: &'static str,
        unblocks_parents: i64,
        unblocks_dependents: i64,
        cascades_to_children: i64,
        on_enter: Option<serde_json::Value>,
        prompt_path: Option<&'static str>,
        interactive: i64,
    }

    let expectations: [Expected; 3] = [
        Expected {
            sequence: 1,
            key: "draft",
            label: "Draft",
            color: "#cccccc",
            unblocks_parents: 0,
            unblocks_dependents: 0,
            cascades_to_children: 0,
            on_enter: None,
            prompt_path: None,
            interactive: 0,
        },
        Expected {
            sequence: 2,
            key: "reviewing",
            label: "Reviewing",
            color: "#f1c40f",
            unblocks_parents: 0,
            unblocks_dependents: 0,
            cascades_to_children: 0,
            on_enter: Some(serde_json::json!({"assign_to": {"Agent": {"name": "reviewer"}}})),
            prompt_path: Some("/projects/stsfixt/reviewing.md"),
            interactive: 1,
        },
        Expected {
            sequence: 3,
            key: "merged",
            label: "Merged",
            color: "#2ecc71",
            unblocks_parents: 1,
            unblocks_dependents: 1,
            cascades_to_children: 0,
            on_enter: None,
            prompt_path: None,
            interactive: 0,
        },
    ];

    for (row, expected) in rows.iter().zip(expectations.iter()) {
        let sequence: i64 = row.try_get("sequence")?;
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: i64 = row.try_get("unblocks_parents")?;
        let unblocks_dependents: i64 = row.try_get("unblocks_dependents")?;
        let cascades_to_children: i64 = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter")?;
        let on_enter_value = on_enter_text
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .context("decode j-stsfixt.statuses.on_enter JSON")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: i64 = row.try_get("interactive")?;

        if sequence != expected.sequence
            || key != expected.key
            || label != expected.label
            || color != expected.color
            || unblocks_parents != expected.unblocks_parents
            || unblocks_dependents != expected.unblocks_dependents
            || cascades_to_children != expected.cascades_to_children
            || on_enter_value != expected.on_enter
            || prompt_path.as_deref() != expected.prompt_path
            || interactive != expected.interactive
        {
            bail!(
                "statuses(j-stsfixt) sequence={sequence}: did not match expected\n  \
                 got: (key={key}, label={label}, color={color}, ub_par={unblocks_parents}, \
                 ub_dep={unblocks_dependents}, casc={cascades_to_children}, on_enter={on_enter_value:?}, \
                 prompt_path={prompt_path:?}, interactive={interactive})\n  \
                 expected: sequence={}, key={}",
                expected.sequence,
                expected.key
            );
        }
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "issues_v2", "status_sequence").await? {
        bail!("expected `issues_v2.status_sequence` column to exist after rollforward");
    }
    // After 20260614000000_cutover_to_statuses_table, the column is
    // tightened to NOT NULL and carries the FK to `statuses`.
    if column_is_nullable(pool, "issues_v2", "status_sequence").await? {
        bail!("expected `issues_v2.status_sequence` to be NOT NULL post-cutover; got nullable");
    }
    if column_exists(pool, "issues_v2", "status").await? {
        bail!("expected `issues_v2.status` (TEXT) to be dropped post-cutover");
    }
    if column_exists(pool, "projects", "statuses").await? {
        bail!("expected `projects.statuses` (JSON) to be dropped post-cutover");
    }
    if !column_exists(pool, "projects", "next_status_sequence").await? {
        bail!("expected `projects.next_status_sequence` to be added post-cutover");
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_backfills_issues(pool: &SqlitePool) -> Result<()> {
    let cases: &[(&str, &str, &str, i64)] = &[
        ("i-stsopena", "j-defaul", "open", 1),
        ("i-stsiprog", "j-defaul", "in-progress", 2),
        ("i-stsclosd", "j-defaul", "closed", 3),
        ("i-stsdropd", "j-defaul", "dropped", 4),
        ("i-stsfaild", "j-defaul", "failed", 5),
        ("i-stsrevwg", "j-stsfixt", "reviewing", 2),
    ];
    for (issue_id, project_id, status_key, expected_sequence) in cases {
        let row = sqlx::query(
            "SELECT i.status_sequence, s.key AS resolved_key \
             FROM issues_v2 i \
             LEFT JOIN statuses s \
                ON s.project_id = i.project_id AND s.sequence = i.status_sequence \
             WHERE i.id = ?1 AND i.is_latest = 1",
        )
        .bind(issue_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read status_sequence for {issue_id}"))?;
        let status_sequence: Option<i64> = row.try_get("status_sequence")?;
        let resolved_key: Option<String> = row.try_get("resolved_key")?;
        if status_sequence != Some(*expected_sequence) {
            bail!(
                "{issue_id} (project={project_id}, status={status_key}): \
                 expected status_sequence={expected_sequence}; got {status_sequence:?}"
            );
        }
        if resolved_key.as_deref() != Some(*status_key) {
            bail!(
                "{issue_id} (project={project_id}, status={status_key}): \
                 join to statuses must recover key; got {resolved_key:?}"
            );
        }
    }

    let row = sqlx::query("SELECT COUNT(*) FROM issues_v2 WHERE status_sequence IS NULL")
        .fetch_one(pool)
        .await
        .context("count NULL status_sequence rows post-rollforward")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 issues_v2 rows with NULL status_sequence post-backfill; got {count}");
    }
    Ok(())
}

async fn create_statuses_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // PR 1's `20260613000000_create_statuses.sql` body sourced its
    // INSERT from `projects.statuses` JSONB. After
    // `20260614000000_cutover_to_statuses_table` drops that JSONB
    // column, re-executing the body verbatim is no longer possible
    // (it errors on `p.statuses`). The invariant that matters — "no
    // duplicate `(project_id, sequence)` or `(project_id, key)` rows
    // remain in the `statuses` table after a full migration plan
    // rolls forward, including a re-run through the tracker" — stays
    // testable directly, so check the duplicate-free shape on the
    // post-rollforward pool.
    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT project_id || ':' || sequence) AS dup FROM statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, sequence) rows in statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!(
            "statuses table carries {dup} duplicate (project_id, sequence) rows post-rollforward"
        );
    }
    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT project_id || ':' || key) AS dup FROM statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, key) rows in statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!("statuses table carries {dup} duplicate (project_id, key) rows post-rollforward");
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // The sqlite body cannot be re-executed verbatim — `ALTER TABLE ...
    // ADD COLUMN` has no `IF NOT EXISTS` form in sqlite, so a second
    // raw-SQL run would error on "duplicate column". The acceptance
    // criterion explicitly covers this via the orchestration tracker:
    // a second `run_migrations(&pool, None)` call must treat the
    // already-applied 20260613010000 migration as a no-op, leaving every
    // previously-backfilled `status_sequence` value untouched.
    let before_rows = sqlx::query(
        "SELECT id, version_number, status_sequence FROM issues_v2 \
         WHERE status_sequence IS NOT NULL ORDER BY id, version_number",
    )
    .fetch_all(pool)
    .await
    .context("snapshot non-NULL issues_v2.status_sequence before tracker rerun")?;
    let before: Vec<(String, i64, i64)> = before_rows
        .iter()
        .map(|r| {
            (
                r.try_get::<String, _>("id").unwrap(),
                r.try_get::<i64, _>("version_number").unwrap(),
                r.try_get::<i64, _>("status_sequence").unwrap(),
            )
        })
        .collect();

    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-run sqlite migrations through the tracker for idempotency check")?;

    let after_rows = sqlx::query(
        "SELECT id, version_number, status_sequence FROM issues_v2 \
         WHERE status_sequence IS NOT NULL ORDER BY id, version_number",
    )
    .fetch_all(pool)
    .await
    .context("re-snapshot non-NULL issues_v2.status_sequence after tracker rerun")?;
    let after: Vec<(String, i64, i64)> = after_rows
        .iter()
        .map(|r| {
            (
                r.try_get::<String, _>("id").unwrap(),
                r.try_get::<i64, _>("version_number").unwrap(),
                r.try_get::<i64, _>("status_sequence").unwrap(),
            )
        })
        .collect();
    if before != after {
        bail!(
            "add_issues_v2_status_sequence tracker rerun overwrote previously-backfilled rows: \
             before={before:?}; after={after:?}"
        );
    }

    let row = sqlx::query("SELECT COUNT(*) FROM issues_v2 WHERE status_sequence IS NULL")
        .fetch_one(pool)
        .await
        .context("count NULL status_sequence rows after tracker rerun")?;
    let null_count: i64 = row.try_get(0)?;
    if null_count != 0 {
        bail!(
            "add_issues_v2_status_sequence tracker rerun left {null_count} NULL status_sequence rows"
        );
    }
    Ok(())
}

/// Pre-flight guard for the issues_v2.status_sequence backfill. Against
/// a fresh schema-at-baseline pool with an issue row whose
/// `(project_id, status)` has no matching `statuses` row, the migration
/// body must fail loud rather than silently leaving the issue
/// orphan-pointing.
async fn add_issues_v2_status_sequence_migration_rejects_null_baseline() -> Result<()> {
    let pool = SqliteStore::init_pool("sqlite::memory:")
        .await
        .context("init in-memory sqlite pool for status_sequence null-baseline test")?;

    // Roll forward to the create_statuses migration so `statuses` exists
    // but `status_sequence` has not been added yet.
    sqlite_store::run_migrations(&pool, Some(20260613000000))
        .await
        .context("roll forward to 20260613000000 baseline for null-guard test")?;

    // Seed an issue whose `(project_id, status)` does not match any
    // existing `statuses` row. The default project's seeded statuses
    // never include `ghost`, so the join will yield NULL and the
    // pre-flight guard must trip.
    sqlx::query(
        "INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, project_id, status, is_latest) \
         VALUES ('i-nullseq', 1, 'task', 'guard test row', 'system', 'j-defaul', 'ghost', 1)",
    )
    .execute(&pool)
    .await
    .context("insert orphan-status row")?;

    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("sqlite-migrations/20260613010000_add_issues_v2_status_sequence.sql"),
    )
    .context("read sqlite add_issues_v2_status_sequence migration body for null-baseline test")?;

    let result = sqlx::raw_sql(&body).execute(&pool).await;
    match result {
        Err(err) => {
            // The scratch table's CHECK (null_count = 0) trips; the
            // error mentions either the constraint name or the column.
            let msg = err.to_string();
            if !msg.to_ascii_lowercase().contains("check")
                && !msg.contains("null_count")
                && !msg.contains("_status_sequence_null_guard")
            {
                bail!(
                    "expected the migration error to surface the CHECK / null_count guard; got: {msg}"
                );
            }
            Ok(())
        }
        Ok(_) => bail!(
            "expected the migration body to fail loud on an orphan (project_id, status) \
             row; instead it completed successfully"
        ),
    }
}

// ---------------------------------------------------------------------------
// 20260615000000_reserve_hydra_id_shape_in_keys — SQLite parity.
// Sister to `migration_roundtrip::reserve_hydra_id_shape_*`. Coverage
// matrix is identical: SQL-level rewrite assertions on projects and
// statuses, no-reserved-shape-anywhere invariant, typed
// `Store::get_project` / `Store::list_projects` round-trip, and
// migration-body idempotency.
// ---------------------------------------------------------------------------

async fn reserve_hydra_id_shape_rewrites_project_keys(pool: &SqlitePool) -> Result<()> {
    let expected: &[(&str, &str)] = &[
        ("j-rsvshapa", "renamed-j-foo"),
        ("j-rsvshapb", "engineering"),
        ("j-rsvshapc", "renamed-x-old"),
    ];
    for (id, want_key) in expected {
        let row = sqlx::query("SELECT key FROM projects WHERE id = ?1 AND is_latest = 1")
            .bind(*id)
            .fetch_one(pool)
            .await
            .with_context(|| format!("read projects.key for {id}"))?;
        let got: String = row.try_get("key")?;
        if got.as_str() != *want_key {
            bail!("projects({id}).key: expected {want_key:?}; got {got:?}");
        }
    }
    Ok(())
}

async fn reserve_hydra_id_shape_rewrites_status_keys(pool: &SqlitePool) -> Result<()> {
    let expected: &[(i64, &str)] = &[
        (1, "renamed-i-progress"),
        (2, "done"),
        (3, "renamed-s-todo-seq3"),
        (4, "renamed-s-todo"),
    ];
    for (sequence, want_key) in expected {
        let row = sqlx::query(
            "SELECT key FROM statuses WHERE project_id = 'j-rsvshapa' AND sequence = ?1",
        )
        .bind(*sequence)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read statuses.key for (j-rsvshapa, seq={sequence})"))?;
        let got: String = row.try_get("key")?;
        if got.as_str() != *want_key {
            bail!("statuses(j-rsvshapa, seq={sequence}).key: expected {want_key:?}; got {got:?}");
        }
    }
    Ok(())
}

async fn reserve_hydra_id_shape_no_reserved_shape_remains(pool: &SqlitePool) -> Result<()> {
    let row = sqlx::query("SELECT COUNT(*) FROM projects WHERE key GLOB '[a-z]-*'")
        .fetch_one(pool)
        .await
        .context("count projects.key rows still matching reserved shape")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 projects rows with key matching `[a-z]-*`; got {count}");
    }
    let row = sqlx::query("SELECT COUNT(*) FROM statuses WHERE key GLOB '[a-z]-*'")
        .fetch_one(pool)
        .await
        .context("count statuses.key rows still matching reserved shape")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 statuses rows with key matching `[a-z]-*`; got {count}");
    }
    Ok(())
}

async fn reserve_hydra_id_shape_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey, StatusKey};

    let store = SqliteStore::new(pool.clone());

    let cases: &[(&str, &str, &[&str])] = &[
        (
            "j-rsvshapa",
            "renamed-j-foo",
            &[
                "renamed-i-progress",
                "done",
                "renamed-s-todo-seq3",
                "renamed-s-todo",
            ],
        ),
        ("j-rsvshapb", "engineering", &[]),
        ("j-rsvshapc", "renamed-x-old", &[]),
    ];
    for (id, want_key, want_status_keys) in cases {
        let project_id =
            ProjectId::from_str(id).with_context(|| format!("parse project id '{id}'"))?;
        let fetched = store
            .get_project(&project_id, false)
            .await
            .with_context(|| {
                format!("SqliteStore::get_project({id}) post-reserve-hydra-id-shape")
            })?;
        let ApiProject { key, statuses, .. } = &fetched.item;
        let expected_key =
            ProjectKey::try_new(*want_key).expect("expected key passes new validator");
        if key != &expected_key {
            bail!("{id}: expected key={want_key:?}; got {key:?}");
        }
        if statuses.len() != want_status_keys.len() {
            bail!(
                "{id}: expected {} statuses; got {}",
                want_status_keys.len(),
                statuses.len()
            );
        }
        for (got, want_key_str) in statuses.iter().zip(want_status_keys.iter()) {
            let want_key = StatusKey::try_new(*want_key_str)
                .expect("expected status key passes new validator");
            if got.key != want_key {
                bail!(
                    "{id}: status key mismatch: expected {want_key_str:?}; got {:?}",
                    got.key
                );
            }
        }
    }

    let listed = store
        .list_projects(false)
        .await
        .context("SqliteStore::list_projects(false) post-reserve-hydra-id-shape")?;
    let want: &[(&str, &str)] = &[
        ("j-rsvshapa", "renamed-j-foo"),
        ("j-rsvshapb", "engineering"),
        ("j-rsvshapc", "renamed-x-old"),
    ];
    for (id, want_key) in want {
        let row = listed
            .iter()
            .find(|(pid, _)| pid.as_ref() == *id)
            .with_context(|| format!("list_projects: missing project {id}"))?;
        if row.1.item.key.as_str() != *want_key {
            bail!(
                "list_projects({id}).key: expected {want_key:?}; got {:?}",
                row.1.item.key
            );
        }
    }
    Ok(())
}

async fn reserve_hydra_id_shape_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    // Re-execute the migration body. The reserved-shape WHERE clauses
    // match nothing post-rewrite, so the body's plan tables stay empty,
    // the UPDATEs touch no rows, and the audit SELECTs print zero lines.
    // Re-assert the expected post-rewrite key set to confirm.
    //
    // The on-disk body still references `projects.deleted`, but a
    // subsequent migration (20260715000000) renamed that column to
    // `projects.archived`. Patch the loaded body to track the rename
    // so the literal replay still exercises the body's idempotency
    // against the current schema.
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("sqlite-migrations/20260615000000_reserve_hydra_id_shape_in_keys.sql"),
    )
    .context("read sqlite reserve_hydra_id_shape migration body for idempotency rerun")?;
    let body = body.replace("p2.deleted", "p2.archived");
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply sqlite reserve_hydra_id_shape migration body")?;
    reserve_hydra_id_shape_rewrites_project_keys(pool).await?;
    reserve_hydra_id_shape_rewrites_status_keys(pool).await?;
    reserve_hydra_id_shape_no_reserved_shape_remains(pool).await?;
    Ok(())
}

// 20260712000000_add_statuses_max_simultaneous_sessions. Adds
// `max_simultaneous_sessions INTEGER NULL` to `statuses` — the
// per-status cap on simultaneously-active sessions (interactive +
// headless, across all agents). `NULL` (the column default) leaves the
// cap off for the row.
// ---------------------------------------------------------------------------

async fn add_statuses_max_simultaneous_sessions_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "statuses", "max_simultaneous_sessions").await? {
        bail!("expected `statuses.max_simultaneous_sessions` column to exist post-rollforward");
    }
    if !column_is_nullable(pool, "statuses", "max_simultaneous_sessions").await? {
        bail!("expected `statuses.max_simultaneous_sessions` to be NULLABLE");
    }
    Ok(())
}

async fn add_statuses_max_simultaneous_sessions_defaults_to_null(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query("SELECT project_id, sequence, max_simultaneous_sessions FROM statuses")
        .fetch_all(pool)
        .await
        .context("read statuses for max_simultaneous_sessions default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert default against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<i64> = row.try_get("max_simultaneous_sessions")?;
        if value.is_some() {
            bail!(
                "statuses({project_id}, sequence={sequence}): expected NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_max_simultaneous_sessions_domain_roundtrip(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("SqliteStore::get_project(j-defaul) post-max-simultaneous-sessions migration")?;
    if fetched.item.statuses.is_empty() {
        bail!("expected j-defaul to have statuses post-rollforward");
    }
    for (idx, status) in fetched.item.statuses.iter().enumerate() {
        if status.max_simultaneous_sessions.is_some() {
            bail!(
                "j-defaul.statuses[{idx}] ({key:?}): expected max_simultaneous_sessions=None; got {got:?}",
                key = status.key,
                got = status.max_simultaneous_sessions,
            );
        }
    }
    Ok(())
}

async fn add_statuses_max_simultaneous_sessions_migration_is_idempotent(
    pool: &SqlitePool,
) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT project_id, sequence, max_simultaneous_sessions FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses before max_simultaneous_sessions idempotency rerun")?;
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm max_simultaneous_sessions idempotency")?;
    let snapshot_after = sqlx::query(
        "SELECT project_id, sequence, max_simultaneous_sessions FROM statuses ORDER BY project_id, sequence",
    )
    .fetch_all(pool)
    .await
    .context("snapshot statuses after max_simultaneous_sessions idempotency rerun")?;
    if snapshot_before.len() != snapshot_after.len() {
        bail!(
            "row count changed across max_simultaneous_sessions idempotency rerun: {} -> {}",
            snapshot_before.len(),
            snapshot_after.len()
        );
    }
    for (before, after) in snapshot_before.iter().zip(snapshot_after.iter()) {
        let before_val: Option<i64> = before.try_get("max_simultaneous_sessions")?;
        let after_val: Option<i64> = after.try_get("max_simultaneous_sessions")?;
        if before_val != after_val {
            bail!(
                "max_simultaneous_sessions changed across rerun: {before_val:?} -> {after_val:?}"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260713000000_add_statuses_session_settings. Adds the
// `session_settings_json TEXT NULL` column to `statuses` for the per-status
// `SessionSettings` override layer. Backfills to NULL — the read path then
// materializes `SessionSettings::default()`.
// ---------------------------------------------------------------------------

async fn add_statuses_session_settings_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "statuses", "session_settings_json").await? {
        bail!("expected `statuses.session_settings_json` column to exist post-rollforward");
    }
    if !column_is_nullable(pool, "statuses", "session_settings_json").await? {
        bail!("expected `statuses.session_settings_json` to be NULLABLE");
    }
    Ok(())
}

async fn add_statuses_session_settings_defaults_to_null(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query("SELECT project_id, sequence, session_settings_json FROM statuses")
        .fetch_all(pool)
        .await
        .context("read statuses for session_settings_json default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<String> = row.try_get("session_settings_json")?;
        if value.is_some() {
            bail!(
                "statuses({project_id}, sequence={sequence}): expected session_settings_json=NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_session_settings_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm session_settings idempotency")?;
    // After re-running, the column still exists and is still nullable.
    add_statuses_session_settings_schema_invariants(pool).await?;
    Ok(())
}

/// Per the issue spec: the test surface here MUST be a SqliteStore-level
/// roundtrip on `get_<status>` after a status with non-empty
/// `session_settings` is written. This catches the `#[sqlx(default)]`
/// foot-gun — every SELECT projection on `statuses` must include the new
/// `session_settings_json` column. A MemoryStore roundtrip is insufficient.
async fn add_statuses_session_settings_store_roundtrip(pool: &SqlitePool) -> Result<()> {
    use hydra_common::api::v1::issues::SessionSettings as ApiSessionSettings;
    use hydra_common::api::v1::projects::{ProjectKey, StatusKey};

    let store = SqliteStore::new(pool.clone());
    let actor = ActorRef::System {
        worker_name: "session_settings_roundtrip_test".to_string(),
        on_behalf_of: None,
    };
    let project = hydra_common::api::v1::projects::Project::new(
        ProjectKey::try_new("ssrnd").unwrap(),
        "Session Settings Roundtrip".to_string(),
        Vec::new(),
        ApiUsername::from("test"),
        false,
        0.0,
    );
    let (project_id, _) = store
        .add_project(project, &actor)
        .await
        .context("add_project for session_settings roundtrip")?;

    let mut status = StatusDefinition::new(
        StatusKey::try_new("frontend").unwrap(),
        "Frontend".to_string(),
        "#abcdef".parse().unwrap(),
        false,
        false,
        false,
        None,
    );
    let mut session_settings = ApiSessionSettings::default();
    session_settings.cpu_limit = Some("250m".to_string());
    session_settings.memory_limit = Some("256Mi".to_string());
    status.session_settings = session_settings.clone();
    store
        .add_status(&project_id, status, &actor)
        .await
        .context("add_status for session_settings roundtrip")?;

    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("get_project for session_settings roundtrip")?;
    let frontend = fetched
        .item
        .statuses
        .iter()
        .find(|s| s.key.as_str() == "frontend")
        .ok_or_else(|| anyhow::anyhow!("missing frontend status after add_status"))?;
    if frontend.session_settings != session_settings {
        bail!(
            "frontend.session_settings roundtrip mismatch: wrote {session_settings:?}, read {:?}",
            frontend.session_settings
        );
    }

    // Defaulted statuses must round-trip as `SessionSettings::default()`,
    // not NULL deserialization errors.
    let default_seed_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let default_project = store
        .get_project(&default_seed_id, false)
        .await
        .context("get_project(j-defaul) post-session_settings migration")?;
    for status in &default_project.item.statuses {
        if !ApiSessionSettings::is_default(&status.session_settings) {
            bail!(
                "j-defaul status {key:?} unexpectedly carried non-default session_settings: {got:?}",
                key = status.key,
                got = status.session_settings,
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260715000000_rename_projects_deleted_to_archived. Pure column
// rename: `projects.deleted` → `projects.archived`. No semantic change.
// The partial unique index `projects_key_unique_active_idx` has its
// `WHERE` clause auto-rewritten by SQLite's `ALTER TABLE RENAME COLUMN`,
// so no explicit index touch.
// ---------------------------------------------------------------------------

async fn rename_projects_deleted_to_archived_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if column_exists(pool, "projects", "deleted").await? {
        bail!("expected `projects.deleted` column to be renamed away post-rollforward");
    }
    if !column_exists(pool, "projects", "archived").await? {
        bail!("expected `projects.archived` column to exist post-rollforward");
    }
    if !index_exists(pool, "projects_key_unique_active_idx").await? {
        bail!(
            "expected partial unique index `projects_key_unique_active_idx` to survive the rename"
        );
    }
    Ok(())
}

/// Insert a row at the OLD `deleted` shape (via the baseline fixture
/// `20260713000000__pre_rename_projects_deleted_to_archived.sql`) and
/// confirm the rename migration preserves the value, round-tripping
/// through the current Store API as `Project.archived`.
async fn rename_projects_deleted_to_archived_baseline_roundtrip(pool: &SqlitePool) -> Result<()> {
    let store = SqliteStore::new(pool.clone());

    let archived_id = ProjectId::from_str("j-renarcha").context("parse j-renarcha")?;
    let archived = store
        .get_project(&archived_id, true)
        .await
        .context("SqliteStore::get_project(j-renarcha, include_archived=true)")?;
    if !archived.item.archived {
        bail!(
            "j-renarcha: expected archived=true after rename; got archived={}",
            archived.item.archived
        );
    }
    if archived.item.key.as_str() != "rename-archived" {
        bail!(
            "j-renarcha: expected key='rename-archived'; got {:?}",
            archived.item.key
        );
    }

    // include_archived=false hides the archived row.
    let archived_hidden = store.get_project(&archived_id, false).await;
    assert!(
        matches!(archived_hidden, Err(StoreError::ProjectNotFound(_))),
        "j-renarcha must not surface through include_archived=false; got {archived_hidden:?}"
    );

    let live_id = ProjectId::from_str("j-renarchb").context("parse j-renarchb")?;
    let live = store
        .get_project(&live_id, false)
        .await
        .context("SqliteStore::get_project(j-renarchb, include_archived=false)")?;
    if live.item.archived {
        bail!(
            "j-renarchb: expected archived=false after rename; got archived={}",
            live.item.archived
        );
    }

    // list_projects(false) surfaces only the live row.
    let listed = store
        .list_projects(false)
        .await
        .context("SqliteStore::list_projects(false) post-rename")?;
    let listed_ids: Vec<&str> = listed
        .iter()
        .map(|(pid, _)| pid.as_ref())
        .filter(|id| matches!(*id, "j-renarcha" | "j-renarchb"))
        .collect();
    if listed_ids != vec!["j-renarchb"] {
        bail!("list_projects(false) baseline filter: expected only j-renarchb; got {listed_ids:?}");
    }
    Ok(())
}

async fn rename_projects_deleted_to_archived_migration_is_idempotent(
    pool: &SqlitePool,
) -> Result<()> {
    sqlite_store::run_migrations(pool, None).await.context(
        "re-apply sqlite migrations to confirm rename_projects_deleted_to_archived idempotency",
    )?;
    rename_projects_deleted_to_archived_schema_invariants(pool).await?;
    rename_projects_deleted_to_archived_baseline_roundtrip(pool).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260716000000_add_statuses_archived. Schema-only ALTER TABLE that
// adds `archived BOOLEAN NOT NULL DEFAULT FALSE` to `statuses` — the
// foundation for Phase 3's `archive_status` (flip in place + cascade to
// issues). Existing rows backfill to FALSE via the column default.
// ---------------------------------------------------------------------------

async fn add_statuses_archived_schema_invariants(pool: &SqlitePool) -> Result<()> {
    if !column_exists(pool, "statuses", "archived").await? {
        bail!("expected `statuses.archived` column to exist post-rollforward");
    }
    if column_is_nullable(pool, "statuses", "archived").await? {
        bail!("expected `statuses.archived` to be NOT NULL");
    }
    let row =
        sqlx::query("SELECT dflt_value FROM pragma_table_info('statuses') WHERE name = 'archived'")
            .fetch_one(pool)
            .await
            .context("read pragma_table_info default for statuses.archived")?;
    let default_text: Option<String> = row.try_get(0)?;
    let default_text =
        default_text.ok_or_else(|| anyhow::anyhow!("statuses.archived has no declared default"))?;
    if !default_text.eq_ignore_ascii_case("FALSE") && default_text != "0" {
        bail!("expected statuses.archived DEFAULT FALSE; got {default_text:?}");
    }
    Ok(())
}

async fn add_statuses_archived_defaults_to_false(pool: &SqlitePool) -> Result<()> {
    let rows = sqlx::query("SELECT project_id, sequence, archived FROM statuses")
        .fetch_all(pool)
        .await
        .context("read statuses for archived default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: bool = row.try_get("archived")?;
        if value {
            bail!("statuses({project_id}, sequence={sequence}): expected archived=false; got true");
        }
    }
    Ok(())
}

async fn add_statuses_archived_migration_is_idempotent(pool: &SqlitePool) -> Result<()> {
    sqlite_store::run_migrations(pool, None)
        .await
        .context("re-apply sqlite migrations to confirm add_statuses_archived idempotency")?;
    add_statuses_archived_schema_invariants(pool).await?;
    add_statuses_archived_defaults_to_false(pool).await?;
    Ok(())
}
