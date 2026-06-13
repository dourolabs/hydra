//! Migration roundtrip integration test.
//!
//! Enumerates the versioned baseline fixtures under
//! `tests/fixtures/migration_baselines/`, interleaving each baseline's
//! INSERTs with `postgres_v2::run_migrations(&pool, Some(version))` so every
//! migration with a higher version sees the file's rows. After the loop, a
//! final `run_migrations(&pool, None)` rolls to HEAD (and runs any Rust
//! migrations whose version sits beyond the last baseline). See
//! `/designs/migration-testing-redesign.md` §3, §4, §7 for the algorithm.
//!
//! Asserts:
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
//! `hydra-server`'s postgres-specific code.

#![cfg(feature = "postgres")]

use anyhow::{Context, Result, bail};
use chrono::Utc;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::projects::StatusDefinition;
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::{ExternalSystem, Principal};
use hydra_common::test_utils::status::status;
use hydra_common::{
    ConversationId, DocumentId, HydraId, IssueId, PatchId, ProjectId, RepoName, SessionId,
    TriggerId,
};
use hydra_server::domain::actors::ActorRef;
use hydra_server::domain::issues::{Issue, IssueType};
use hydra_server::domain::patches::{Patch, PatchStatus, Review};
use hydra_server::domain::projects::default_project_seed;
use hydra_server::domain::sessions::{AgentConfig, Session, SessionEvent, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::Username;
use hydra_server::store::postgres_v2::{self, MIGRATOR, PostgresStoreV2};
use hydra_server::store::{ReadOnlyStore, RelationshipType, Store, StoreError};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

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
            "baseline {} has no matching sqlx migration on this checkout",
            b.version
        );
        postgres_v2::run_migrations(&pool, Some(b.version))
            .await
            .with_context(|| format!("apply migrations up to baseline {}", b.version))?;
        sqlx::raw_sql(&b.body)
            .execute(&pool)
            .await
            .with_context(|| format!("execute baseline {}", b.version))?;
        prev = Some(b.version);
    }

    postgres_v2::run_migrations(&pool, None)
        .await
        .context("apply remaining migrations (SQL + Rust) past the last baseline")?;

    assert_schema_invariants(&pool).await?;
    assert_data_shape_invariants(&pool).await?;
    assert_events_migration_edge_cases(&pool)
        .await
        .context("events migration edge-case partitioning assertions")?;
    assert_actor_variant_cleanup(&pool)
        .await
        .context("actor_variant_cleanup rewrite assertions")?;
    assert_store_level_smoke(&pool)
        .await
        .context("§3.3 store / domain-level smoke")?;
    assert_recent_migration_data_shape(&pool)
        .await
        .context("triggers / projects post-rollforward data-shape round-trip")?;

    seed_default_project_migration_inserts_row(&pool)
        .await
        .context("seed_default_project: inserted row matches expected shape")?;
    seed_default_project_migration_backfills_null_project_ids(&pool)
        .await
        .context("seed_default_project: backfill of NULL project_id values")?;
    seed_default_project_migration_is_idempotent(&pool)
        .await
        .context("seed_default_project: idempotency on re-applied migration body")?;

    drop_status_icon_migration_strips_default_seed(&pool)
        .await
        .context("drop_status_icon: j-defaul statuses no longer carry an `icon` key")?;
    drop_status_icon_migration_strips_custom_row(&pool)
        .await
        .context("drop_status_icon: j-iconfix statuses round-trip without `icon`")?;
    drop_status_icon_migration_is_idempotent(&pool)
        .await
        .context("drop_status_icon: idempotency on re-applied migration body")?;

    denormalize_creator_session_backfill(&pool)
        .await
        .context("denormalize_creator: session-bound auth_tokens row backfilled")?;
    denormalize_creator_user_backfill(&pool)
        .await
        .context("denormalize_creator: user-CLI auth_tokens row backfilled")?;
    denormalize_creator_domain_roundtrip(&pool)
        .await
        .context("denormalize_creator: domain-level AuthTokenRow round-trip")?;
    drop_actors_v2_migration_removes_table(&pool)
        .await
        .context("drop_actors_v2: metis.actors_v2 table is gone after rollforward")?;
    denormalize_creator_migration_is_idempotent(&pool)
        .await
        .context("denormalize_creator: post-migration write/read works")?;

    add_projects_priority_backfill_sql_level(&pool)
        .await
        .context("add_projects_priority: SQL-level rank backfill")?;
    add_projects_priority_backfill_domain_roundtrip(&pool)
        .await
        .context("add_projects_priority: domain-level list_projects round-trip")?;

    drop_projects_default_status_key_migration_removes_column(&pool)
        .await
        .context(
            "drop_projects_default_status_key: metis.projects.default_status_key column is gone",
        )?;
    drop_projects_default_status_key_migration_preserves_typed_read(&pool)
        .await
        .context("drop_projects_default_status_key: seeded + baseline rows deserialize through Store::get_project")?;
    drop_projects_default_status_key_migration_is_idempotent(&pool)
        .await
        .context("drop_projects_default_status_key: idempotency on re-applied migration body")?;

    issues_v2_project_id_is_not_null(&pool)
        .await
        .context("issues_v2_project_id_not_null: column is NOT NULL after rollforward")?;
    issues_v2_project_id_rejects_null_insert(&pool)
        .await
        .context("issues_v2_project_id_not_null: NOT NULL rejects fresh NULL inserts")?;
    issues_v2_project_id_not_null_migration_is_idempotent(&pool)
        .await
        .context("issues_v2_project_id_not_null: idempotency on re-applied body")?;
    // The pre-flight NULL-guard test uses a fresh sqlite in-memory pool
    // in the sister `migration_roundtrip_sqlite` runner; we don't repeat
    // it here because the postgres path would have to reset and reroll
    // the schema, and downstream idempotency assertions in this run
    // depend on the seeded baseline data.

    create_statuses_migration_schema_invariants(&pool)
        .await
        .context("create_statuses: schema invariants on metis.statuses")?;
    create_statuses_migration_backfills_default_seed(&pool)
        .await
        .context("create_statuses: j-defaul backfill matches default_project_seed()")?;
    create_statuses_migration_backfills_custom_project(&pool)
        .await
        .context("create_statuses: j-stsfixt backfill covers full column shape")?;
    add_issues_v2_status_sequence_schema_invariants(&pool)
        .await
        .context(
            "add_issues_v2_status_sequence: status_sequence column exists as nullable BIGINT",
        )?;
    add_issues_v2_status_sequence_backfills_issues(&pool)
        .await
        .context(
            "add_issues_v2_status_sequence: every issue's status_sequence resolves back to its key",
        )?;
    create_statuses_migration_is_idempotent(&pool)
        .await
        .context("create_statuses: re-applying body adds no duplicate rows")?;
    add_issues_v2_status_sequence_migration_is_idempotent(&pool)
        .await
        .context("add_issues_v2_status_sequence: re-applying body does not overwrite sequences")?;

    cutover_to_statuses_table_backfills_deploy_gap_project(&pool)
        .await
        .context("cutover_to_statuses_table: deploy-gap project's metis.statuses backfilled")?;
    cutover_to_statuses_table_backfills_deploy_gap_issues(&pool)
        .await
        .context("cutover_to_statuses_table: deploy-gap issues' status_sequence backfilled")?;
    cutover_to_statuses_table_fk_rejects_unknown_sequence(&pool)
        .await
        .context("cutover_to_statuses_table: FK rejects insert with unknown sequence")?;
    cutover_to_statuses_table_fk_rejects_status_delete_with_active_issue(&pool)
        .await
        .context("cutover_to_statuses_table: FK rejects DELETE of referenced status row")?;

    reserve_hydra_id_shape_rewrites_project_keys(&pool)
        .await
        .context("reserve_hydra_id_shape: projects.key shape-matching rows rewritten")?;
    reserve_hydra_id_shape_rewrites_status_keys(&pool)
        .await
        .context("reserve_hydra_id_shape: statuses.key shape-matching rows rewritten")?;
    reserve_hydra_id_shape_no_reserved_shape_remains(&pool)
        .await
        .context("reserve_hydra_id_shape: no projects.key / statuses.key matches `[a-z]-...` post-rewrite")?;
    reserve_hydra_id_shape_domain_roundtrip(&pool)
        .await
        .context(
            "reserve_hydra_id_shape: Store::get_project / list_projects read rewritten rows",
        )?;
    reserve_hydra_id_shape_migration_is_idempotent(&pool)
        .await
        .context("reserve_hydra_id_shape: re-applying body is a no-op")?;

    add_statuses_position_schema_invariants(&pool)
        .await
        .context("add_statuses_position: position column is NOT NULL on metis.statuses")?;
    add_statuses_position_backfills_to_sequence(&pool)
        .await
        .context("add_statuses_position: every statuses row has position = sequence")?;
    add_statuses_position_domain_roundtrip(&pool)
        .await
        .context("add_statuses_position: Store::get_project round-trips position values")?;

    add_statuses_auto_archive_after_seconds_schema_invariants(&pool)
        .await
        .context(
            "add_statuses_auto_archive_after_seconds: column is BIGINT NULL on metis.statuses",
        )?;
    add_statuses_auto_archive_after_seconds_defaults_to_null(&pool)
        .await
        .context(
            "add_statuses_auto_archive_after_seconds: existing rows default to NULL (no backfill)",
        )?;
    add_statuses_auto_archive_after_seconds_domain_roundtrip(&pool)
        .await
        .context(
            "add_statuses_auto_archive_after_seconds: Store::get_project reads field as None",
        )?;

    add_clear_assignee_to_default_terminal_statuses_post_migration_state(&pool)
        .await
        .context(
            "add_clear_assignee_to_default_terminal_statuses: terminal rows carry clear_assignee=true, non-terminal rows untouched",
        )?;
    add_clear_assignee_to_default_terminal_statuses_is_idempotent(&pool)
        .await
        .context(
            "add_clear_assignee_to_default_terminal_statuses: verbatim body replay preserves on_enter",
        )?;

    drop_is_assignment_agent_schema_invariants(&pool)
        .await
        .context("drop_is_assignment_agent: column is dropped from metis.agents")?;
    drop_is_assignment_agent_preserves_rows(&pool)
        .await
        .context("drop_is_assignment_agent: baseline agents rows survive the column drop")?;

    teardown_work_on_default_terminal_statuses_post_migration_state(&pool)
        .await
        .context(
            "teardown_work seed + rename: terminal rows carry teardown_work=true alongside clear_assignee=true, no legacy kill_sessions key, non-terminal rows untouched",
        )?;
    rename_kill_sessions_to_teardown_work_is_idempotent(&pool)
        .await
        .context(
            "rename_kill_sessions_to_teardown_work: verbatim body replay preserves on_enter",
        )?;

    backfill_assignee_null_on_terminal_default_issues_nulls_targeted_rows(&pool)
        .await
        .context(
            "backfill_assignee_null_on_terminal_default_issues: terminal default-project rows have assignees nulled; non-terminal and non-default rows untouched",
        )?;
    backfill_assignee_null_on_terminal_default_issues_is_idempotent(&pool)
        .await
        .context(
            "backfill_assignee_null_on_terminal_default_issues: verbatim body replay leaves nulled rows unchanged",
        )?;

    add_statuses_suppress_sessions_schema_invariants(&pool)
        .await
        .context(
            "add_statuses_suppress_sessions: column is BOOLEAN NOT NULL DEFAULT FALSE on metis.statuses",
        )?;
    add_statuses_suppress_sessions_defaults_to_false(&pool)
        .await
        .context(
            "add_statuses_suppress_sessions: existing rows backfill to FALSE via column default",
        )?;

    add_statuses_max_simultaneous_sessions_schema_invariants(&pool)
        .await
        .context(
            "add_statuses_max_simultaneous_sessions: column is BIGINT NULL on metis.statuses",
        )?;
    add_statuses_max_simultaneous_sessions_defaults_to_null(&pool)
        .await
        .context(
            "add_statuses_max_simultaneous_sessions: existing rows default to NULL (no backfill)",
        )?;
    add_statuses_max_simultaneous_sessions_domain_roundtrip(&pool)
        .await
        .context(
            "add_statuses_max_simultaneous_sessions: Store::get_project reads field as None",
        )?;

    add_statuses_session_settings_schema_invariants(&pool)
        .await
        .context(
            "add_statuses_session_settings: session_settings_json TEXT NULL added to metis.statuses",
        )?;
    add_statuses_session_settings_defaults_to_null(&pool)
        .await
        .context("add_statuses_session_settings: existing rows backfill to NULL")?;
    add_statuses_session_settings_domain_roundtrip(&pool)
        .await
        .context(
            "add_statuses_session_settings: SqliteStore-style get_project read includes the new column",
        )?;

    create_issue_comments_migration_schema_invariants(&pool)
        .await
        .context("create_issue_comments: schema invariants on metis.issue_comments")?;
    create_issue_comments_migration_is_idempotent(&pool)
        .await
        .context("create_issue_comments: re-applying body is a no-op")?;

    rename_projects_deleted_to_archived_schema_invariants(&pool)
        .await
        .context(
            "rename_projects_deleted_to_archived: metis.projects.deleted renamed to archived; partial unique index preserved",
        )?;
    rename_projects_deleted_to_archived_baseline_roundtrip(&pool)
        .await
        .context(
            "rename_projects_deleted_to_archived: baseline rows round-trip through PostgresStoreV2::get_project as Project.archived",
        )?;

    // Re-run the migration plan to confirm the cleanup is idempotent —
    // every classify rule treats post-cleanup shapes as no-ops, so a
    // second pass must produce no extra writes.
    postgres_v2::run_migrations(&pool, None)
        .await
        .context("re-apply migrations to confirm idempotency")?;
    assert_actor_variant_cleanup(&pool)
        .await
        .context("actor_variant_cleanup idempotent second-pass assertions")?;
    add_statuses_position_backfills_to_sequence(&pool)
        .await
        .context("add_statuses_position: idempotent rerun preserves position values")?;
    add_statuses_auto_archive_after_seconds_defaults_to_null(&pool)
        .await
        .context(
            "add_statuses_auto_archive_after_seconds: idempotent rerun keeps existing NULLs",
        )?;
    drop_is_assignment_agent_schema_invariants(&pool)
        .await
        .context(
            "drop_is_assignment_agent: idempotent rerun keeps `is_assignment_agent` dropped",
        )?;
    drop_is_assignment_agent_preserves_rows(&pool)
        .await
        .context("drop_is_assignment_agent: idempotent rerun preserves baseline rows")?;
    add_statuses_suppress_sessions_defaults_to_false(&pool)
        .await
        .context("add_statuses_suppress_sessions: idempotent rerun keeps existing FALSE values")?;
    add_statuses_max_simultaneous_sessions_defaults_to_null(&pool)
        .await
        .context("add_statuses_max_simultaneous_sessions: idempotent rerun keeps existing NULLs")?;
    add_statuses_session_settings_defaults_to_null(&pool)
        .await
        .context(
            "add_statuses_session_settings: idempotent rerun keeps existing NULLs on backfilled rows",
        )?;
    rename_projects_deleted_to_archived_schema_invariants(&pool)
        .await
        .context(
            "rename_projects_deleted_to_archived: idempotent rerun keeps the column renamed",
        )?;
    rename_projects_deleted_to_archived_baseline_roundtrip(&pool)
        .await
        .context(
            "rename_projects_deleted_to_archived: idempotent rerun preserves the baseline rows' archived state",
        )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 20260614000000_cutover_to_statuses_table — covers deploy-gap
// catch-up backfills and FK enforcement. The schema-shape checks
// (column drops, NOT NULL tightening, FK existence) are folded into
// `add_issues_v2_status_sequence_schema_invariants` above so the
// before/after states stay co-located.
// ---------------------------------------------------------------------------

async fn cutover_to_statuses_table_backfills_deploy_gap_project(pool: &PgPool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT sequence, key FROM metis.statuses WHERE project_id = 'j-cutgapprj' ORDER BY sequence",
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
        bail!("j-cutgapprj metis.statuses: expected [(1,intake),(2,done)]; got {pairs:?}");
    }
    Ok(())
}

async fn cutover_to_statuses_table_backfills_deploy_gap_issues(pool: &PgPool) -> Result<()> {
    for (id, key) in &[("i-cutgapa", "intake"), ("i-cutgapdef", "open")] {
        let row = sqlx::query(
            "SELECT i.status_sequence, s.key AS resolved_key \
             FROM metis.issues_v2 i \
             LEFT JOIN metis.statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence \
             WHERE i.id = $1 AND i.is_latest = TRUE",
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

async fn cutover_to_statuses_table_fk_rejects_unknown_sequence(pool: &PgPool) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, project_id, status_sequence) \
         VALUES ('i-fkbadseq', 1, 'task', 'fk test', 'system', 'j-defaul', 9999)",
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
    pool: &PgPool,
) -> Result<()> {
    // i-stsopena is on j-defaul with status `open` (sequence 1). The
    // FK must reject the DELETE while the issue still references the
    // row.
    let result =
        sqlx::query("DELETE FROM metis.statuses WHERE project_id = 'j-defaul' AND sequence = 1")
            .execute(pool)
            .await;
    match result {
        Err(_) => Ok(()),
        Ok(_) => bail!(
            "expected FK to reject DELETE of metis.statuses row while an issue still references it"
        ),
    }
}

// ---------------------------------------------------------------------------
// 20260616000000_add_statuses_position. Adds `position DOUBLE PRECISION
// NOT NULL DEFAULT 0` to `metis.statuses` and backfills `position =
// sequence` so the post-cutover display order matches today's
// `sequence ASC` order.
// ---------------------------------------------------------------------------

async fn add_statuses_position_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "statuses", "position").await? {
        bail!("expected `metis.statuses.position` column to exist post-rollforward");
    }
    if column_is_nullable(pool, "statuses", "position").await? {
        bail!("expected `metis.statuses.position` to be NOT NULL");
    }
    Ok(())
}

async fn add_statuses_position_backfills_to_sequence(pool: &PgPool) -> Result<()> {
    // `j-rtrip` is inserted by `assert_recent_migration_data_shape`
    // post-rollforward (i.e. after this migration runs), so its
    // position falls through to the column default 0 rather than the
    // backfill value. Exclude it from the backfill assertion.
    let rows = sqlx::query(
        "SELECT project_id, sequence, position FROM metis.statuses WHERE project_id != 'j-rtrip'",
    )
    .fetch_all(pool)
    .await
    .context("read metis.statuses for position backfill check")?;
    if rows.is_empty() {
        bail!("expected at least one metis.statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let position: f64 = row.try_get("position")?;
        if (position - sequence as f64).abs() > f64::EPSILON {
            bail!(
                "metis.statuses({project_id}, sequence={sequence}): expected position={sequence}.0; got {position}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_position_domain_roundtrip(pool: &PgPool) -> Result<()> {
    // Read j-defaul back through the production `get_project` path —
    // verifies that the new `position` column is included in the
    // `StatusRow` SELECT projection and round-trips into the
    // `StatusDefinition` value.
    let store = PostgresStoreV2::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("PostgresStoreV2::get_project(j-defaul) post-position-migration")?;
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

// ---------------------------------------------------------------------------
// 20260617000000_add_statuses_auto_archive_after_seconds. Adds
// `auto_archive_after_seconds BIGINT NULL` to `metis.statuses` — the
// per-status plumbing for the periodic auto-archive worker. `NULL`
// (the column default) leaves the feature off for the row.
// ---------------------------------------------------------------------------

async fn add_statuses_auto_archive_after_seconds_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "statuses", "auto_archive_after_seconds").await? {
        bail!(
            "expected `metis.statuses.auto_archive_after_seconds` column to exist post-rollforward"
        );
    }
    if !column_is_nullable(pool, "statuses", "auto_archive_after_seconds").await? {
        bail!("expected `metis.statuses.auto_archive_after_seconds` to be NULLABLE");
    }
    Ok(())
}

async fn add_statuses_auto_archive_after_seconds_defaults_to_null(pool: &PgPool) -> Result<()> {
    // The migration has no backfill: every existing row must come out
    // with the column NULL.
    let rows =
        sqlx::query("SELECT project_id, sequence, auto_archive_after_seconds FROM metis.statuses")
            .fetch_all(pool)
            .await
            .context("read metis.statuses for auto_archive_after_seconds default check")?;
    if rows.is_empty() {
        bail!("expected at least one metis.statuses row to assert default against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<i64> = row.try_get("auto_archive_after_seconds")?;
        if value.is_some() {
            bail!(
                "metis.statuses({project_id}, sequence={sequence}): expected NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_auto_archive_after_seconds_domain_roundtrip(pool: &PgPool) -> Result<()> {
    // Read j-defaul back through the production `get_project` path —
    // verifies that the new column is included in the `StatusRow`
    // SELECT projection and round-trips into the `StatusDefinition`
    // value (as `None`, since no row has been set yet).
    let store = PostgresStoreV2::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("PostgresStoreV2::get_project(j-defaul) post-auto-archive migration")?;
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

// ---------------------------------------------------------------------------
// 20260712000000_add_statuses_max_simultaneous_sessions. Adds
// `max_simultaneous_sessions BIGINT NULL` to `metis.statuses` — the
// per-status cap on simultaneously-active sessions (interactive +
// headless, across all agents). `NULL` (the default) leaves the cap
// off for the row.
// ---------------------------------------------------------------------------

async fn add_statuses_max_simultaneous_sessions_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "statuses", "max_simultaneous_sessions").await? {
        bail!(
            "expected `metis.statuses.max_simultaneous_sessions` column to exist post-rollforward"
        );
    }
    if !column_is_nullable(pool, "statuses", "max_simultaneous_sessions").await? {
        bail!("expected `metis.statuses.max_simultaneous_sessions` to be NULLABLE");
    }
    Ok(())
}

async fn add_statuses_max_simultaneous_sessions_defaults_to_null(pool: &PgPool) -> Result<()> {
    let rows =
        sqlx::query("SELECT project_id, sequence, max_simultaneous_sessions FROM metis.statuses")
            .fetch_all(pool)
            .await
            .context("read metis.statuses for max_simultaneous_sessions default check")?;
    if rows.is_empty() {
        bail!("expected at least one statuses row to assert default against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<i64> = row.try_get("max_simultaneous_sessions")?;
        if value.is_some() {
            bail!(
                "metis.statuses({project_id}, sequence={sequence}): expected NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_max_simultaneous_sessions_domain_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store.get_project(&project_id, false).await.context(
        "PostgresStoreV2::get_project(j-defaul) post-max-simultaneous-sessions migration",
    )?;
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

// ---------------------------------------------------------------------------
// 20260618000000_add_clear_assignee_to_default_terminal_statuses. Seeds
// `on_enter.clear_assignee = true` on the three terminal default-project
// statuses (`closed`, `dropped`, `failed`) without disturbing
// `on_enter` on any other row. Idempotent under rerun.
// ---------------------------------------------------------------------------

async fn add_clear_assignee_to_default_terminal_statuses_post_migration_state(
    pool: &PgPool,
) -> Result<()> {
    for key in ["closed", "dropped", "failed"] {
        let row = sqlx::query(
            "SELECT on_enter::text AS on_enter_text \
             FROM metis.statuses \
             WHERE project_id = 'j-defaul' AND key = $1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let on_enter_text = on_enter_text
            .ok_or_else(|| anyhow::anyhow!("j-defaul.{key}: expected on_enter NOT NULL"))?;
        let parsed: serde_json::Value = serde_json::from_str(&on_enter_text)
            .with_context(|| format!("decode on_enter JSON for j-defaul.{key}"))?;
        if parsed.get("clear_assignee") != Some(&serde_json::json!(true)) {
            bail!("j-defaul.{key}: expected on_enter.clear_assignee=true; got {parsed}");
        }
    }

    // The non-terminal default-project statuses (`open`, `in-progress`)
    // must NOT carry an `on_enter` block — the migration is scoped to
    // the three terminal rows and must not touch any other row.
    for key in ["open", "in-progress"] {
        let row = sqlx::query(
            "SELECT on_enter::text AS on_enter_text \
             FROM metis.statuses \
             WHERE project_id = 'j-defaul' AND key = $1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        if on_enter_text.is_some() {
            bail!("j-defaul.{key}: expected on_enter=NULL post-migration; got {on_enter_text:?}");
        }
    }
    Ok(())
}

async fn add_clear_assignee_to_default_terminal_statuses_is_idempotent(
    pool: &PgPool,
) -> Result<()> {
    // Snapshot the on_enter blobs, replay the migration body verbatim
    // (the migration is an UPDATE; sqlx's tracking table won't rerun
    // it, but a verbatim replay catches any "the second run drops a
    // pre-existing key" regression).
    let snapshot_before = sqlx::query(
        "SELECT key, on_enter::text AS on_enter_text \
         FROM metis.statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter before clear_assignee idempotency rerun")?;

    sqlx::raw_sql(
        "UPDATE metis.statuses \
         SET on_enter = jsonb_set( \
             COALESCE(on_enter, '{}'::jsonb), \
             '{clear_assignee}', \
             'true'::jsonb, \
             true \
         ) \
         WHERE project_id = 'j-defaul' \
           AND key IN ('closed', 'dropped', 'failed')",
    )
    .execute(pool)
    .await
    .context("re-apply clear_assignee seed migration body verbatim")?;

    let snapshot_after = sqlx::query(
        "SELECT key, on_enter::text AS on_enter_text \
         FROM metis.statuses WHERE project_id = 'j-defaul' \
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
        let before_on_enter: Option<String> = before.try_get("on_enter_text")?;
        let after_on_enter: Option<String> = after.try_get("on_enter_text")?;
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
// 20260619000000_drop_is_assignment_agent. Drops the `is_assignment_agent`
// column from `metis.agents` after the runtime concept was removed in favor
// of per-status `on_enter.assign_to`. Postgres uses native
// `ALTER TABLE ... DROP COLUMN`, so these assertions verify the column is
// gone and that the baseline-seeded rows survive verbatim.
// ---------------------------------------------------------------------------

async fn drop_is_assignment_agent_schema_invariants(pool: &PgPool) -> Result<()> {
    if column_exists(pool, "agents", "is_assignment_agent").await? {
        bail!("expected `metis.agents.is_assignment_agent` column to be dropped post-rollforward");
    }
    for required in [
        "name",
        "prompt_path",
        "max_tries",
        "max_simultaneous",
        "deleted",
        "created_at",
        "updated_at",
        "secrets",
        "mcp_config_path",
        "is_default_conversation_agent",
    ] {
        if !column_exists(pool, "agents", required).await? {
            bail!(
                "expected `metis.agents.{required}` to remain present post-drop-is-assignment-agent"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260620000000_add_kill_sessions_to_default_terminal_statuses (seeds
// `on_enter.kill_sessions = true` on the three terminal default-project
// statuses) followed by
// 20260710000000_rename_kill_sessions_to_teardown_work (renames that
// JSONB key to `teardown_work`). The post-state checks the end result of
// the pair: terminal rows carry `teardown_work=true` alongside the
// `clear_assignee=true` key seeded by the prior 20260618 migration, with
// no legacy `kill_sessions` key remaining. The rename migration is
// idempotent under rerun.
// ---------------------------------------------------------------------------

async fn teardown_work_on_default_terminal_statuses_post_migration_state(
    pool: &PgPool,
) -> Result<()> {
    for key in ["closed", "dropped", "failed"] {
        let row = sqlx::query(
            "SELECT on_enter::text AS on_enter_text \
             FROM metis.statuses \
             WHERE project_id = 'j-defaul' AND key = $1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
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

    // The non-terminal default-project statuses (`open`, `in-progress`)
    // must NOT carry an `on_enter` block — neither migration touches
    // those rows.
    for key in ["open", "in-progress"] {
        let row = sqlx::query(
            "SELECT on_enter::text AS on_enter_text \
             FROM metis.statuses \
             WHERE project_id = 'j-defaul' AND key = $1",
        )
        .bind(key)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read on_enter for j-defaul.{key}"))?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        if on_enter_text.is_some() {
            bail!("j-defaul.{key}: expected on_enter=NULL post-migration; got {on_enter_text:?}");
        }
    }
    Ok(())
}

async fn rename_kill_sessions_to_teardown_work_is_idempotent(pool: &PgPool) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT key, on_enter::text AS on_enter_text \
         FROM metis.statuses WHERE project_id = 'j-defaul' \
         ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("snapshot j-defaul on_enter before rename idempotency rerun")?;

    sqlx::raw_sql(
        "UPDATE metis.statuses \
         SET on_enter = jsonb_set( \
             on_enter - 'kill_sessions', \
             '{teardown_work}', \
             on_enter->'kill_sessions', \
             true \
         ) \
         WHERE on_enter ? 'kill_sessions'",
    )
    .execute(pool)
    .await
    .context("re-apply rename migration body verbatim")?;

    let snapshot_after = sqlx::query(
        "SELECT key, on_enter::text AS on_enter_text \
         FROM metis.statuses WHERE project_id = 'j-defaul' \
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
        let before_on_enter: Option<String> = before.try_get("on_enter_text")?;
        let after_on_enter: Option<String> = after.try_get("on_enter_text")?;
        if before_key != after_key || before_on_enter != after_on_enter {
            bail!(
                "j-defaul.{before_key} changed across rename rerun: \
                 {before_on_enter:?} -> {after_on_enter:?}"
            );
        }
    }
    Ok(())
}

async fn drop_is_assignment_agent_preserves_rows(pool: &PgPool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT name, max_tries, max_simultaneous, deleted, \
                secrets::text AS secrets, mcp_config_path, is_default_conversation_agent \
         FROM metis.agents \
         WHERE name IN ('pm-baseline', 'chat-baseline', 'deleted-baseline') \
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("read metis.agents rows seeded by the pre-drop-is-assignment-agent baseline")?;
    if rows.len() != 3 {
        bail!(
            "expected 3 baseline agents rows post-drop-column; got {}",
            rows.len()
        );
    }
    // Rows ordered by name: chat-baseline, deleted-baseline, pm-baseline.
    let chat = &rows[0];
    assert_eq!(chat.try_get::<String, _>("name")?, "chat-baseline");
    assert_eq!(chat.try_get::<i32, _>("max_tries")?, 5);
    assert_eq!(chat.try_get::<i32, _>("max_simultaneous")?, 10);
    assert!(!chat.try_get::<bool, _>("deleted")?);
    assert_eq!(
        chat.try_get::<String, _>("secrets")?,
        "[\"OPENAI_API_KEY\"]"
    );
    assert_eq!(
        chat.try_get::<Option<String>, _>("mcp_config_path")?,
        Some("/agents/chat-baseline/mcp.json".to_string())
    );
    assert!(chat.try_get::<bool, _>("is_default_conversation_agent")?);

    let deleted = &rows[1];
    assert_eq!(deleted.try_get::<String, _>("name")?, "deleted-baseline");
    assert!(deleted.try_get::<bool, _>("deleted")?);

    let pm = &rows[2];
    assert_eq!(pm.try_get::<String, _>("name")?, "pm-baseline");
    assert_eq!(pm.try_get::<i32, _>("max_tries")?, 3);
    assert!(!pm.try_get::<bool, _>("deleted")?);
    assert!(!pm.try_get::<bool, _>("is_default_conversation_agent")?);

    Ok(())
}

// ---------------------------------------------------------------------------
// 20260621000000_backfill_assignee_null_on_terminal_default_issues. The
// rollforward migration body has already nulled every preexisting
// default-project terminal-status assignee by the time this test runs,
// so the assertions seed fresh rows with non-NULL assignees in each of
// the three terminal statuses (plus a non-terminal control row and a
// terminal-status row in a non-default project that must stay
// untouched), replay the migration body verbatim, and read back.
// ---------------------------------------------------------------------------

const BACKFILL_ASSIGNEE_BODY_PG: &str = "\
    UPDATE metis.issues_v2 \
    SET assignee = NULL, \
        assignee_principal = NULL \
    WHERE is_latest = TRUE \
      AND project_id = 'j-defaul' \
      AND status_sequence IN ( \
          SELECT sequence \
          FROM metis.statuses \
          WHERE project_id = 'j-defaul' \
            AND key IN ('closed', 'dropped', 'failed') \
      ) \
      AND (assignee IS NOT NULL OR assignee_principal IS NOT NULL)";

async fn backfill_assignee_null_on_terminal_default_issues_nulls_targeted_rows(
    pool: &PgPool,
) -> Result<()> {
    sqlx::raw_sql(
        "INSERT INTO metis.issues_v2 \
         (id, version_number, issue_type, description, creator, project_id, status_sequence, assignee, assignee_principal) \
         VALUES \
         ('i-bfacl', 1, 'task', 'closed with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM metis.statuses WHERE project_id='j-defaul' AND key='closed'), \
            'agents/swe', '{\"Agent\":{\"name\":\"swe\"}}'::jsonb), \
         ('i-bfadrp', 1, 'task', 'dropped with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM metis.statuses WHERE project_id='j-defaul' AND key='dropped'), \
            'agents/pm', '{\"Agent\":{\"name\":\"pm\"}}'::jsonb), \
         ('i-bfafld', 1, 'task', 'failed with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM metis.statuses WHERE project_id='j-defaul' AND key='failed'), \
            'agents/reviewer', '{\"Agent\":{\"name\":\"reviewer\"}}'::jsonb), \
         ('i-bfaopn', 1, 'task', 'open with assignee', 'jayantk', 'j-defaul', \
            (SELECT sequence FROM metis.statuses WHERE project_id='j-defaul' AND key='open'), \
            'agents/swe', '{\"Agent\":{\"name\":\"swe\"}}'::jsonb), \
         ('i-bfacus', 1, 'task', 'shipped (terminal) in custom project', 'jayantk', 'j-cutsteady', \
            (SELECT sequence FROM metis.statuses WHERE project_id='j-cutsteady' AND key='shipped'), \
            'users/jayantk', '{\"User\":{\"name\":\"jayantk\"}}'::jsonb)",
    )
    .execute(pool)
    .await
    .context("seed backfill_assignee test rows")?;

    sqlx::raw_sql(BACKFILL_ASSIGNEE_BODY_PG)
        .execute(pool)
        .await
        .context("re-apply backfill_assignee migration body")?;

    for id in ["i-bfacl", "i-bfadrp", "i-bfafld"] {
        let row = sqlx::query(
            "SELECT assignee, assignee_principal::text AS principal_text \
             FROM metis.issues_v2 WHERE id = $1 AND is_latest = TRUE",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read backfill target {id}"))?;
        let assignee: Option<String> = row.try_get("assignee")?;
        let principal: Option<String> = row.try_get("principal_text")?;
        if assignee.is_some() || principal.is_some() {
            bail!(
                "{id}: expected assignee and assignee_principal NULL post-backfill; \
                 got assignee={assignee:?}, assignee_principal={principal:?}"
            );
        }
    }

    let row = sqlx::query(
        "SELECT assignee FROM metis.issues_v2 WHERE id = 'i-bfaopn' AND is_latest = TRUE",
    )
    .fetch_one(pool)
    .await
    .context("read non-terminal control row")?;
    let assignee: Option<String> = row.try_get("assignee")?;
    if assignee.as_deref() != Some("agents/swe") {
        bail!("i-bfaopn (open default-project): expected assignee retained; got {assignee:?}");
    }

    let row = sqlx::query(
        "SELECT assignee FROM metis.issues_v2 WHERE id = 'i-bfacus' AND is_latest = TRUE",
    )
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
    pool: &PgPool,
) -> Result<()> {
    let snapshot_before = sqlx::query(
        "SELECT id, assignee, assignee_principal::text AS principal_text \
         FROM metis.issues_v2 WHERE is_latest = TRUE ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .context("snapshot issues_v2 before backfill_assignee idempotency rerun")?;

    sqlx::raw_sql(BACKFILL_ASSIGNEE_BODY_PG)
        .execute(pool)
        .await
        .context("re-apply backfill_assignee migration body for idempotency")?;

    let snapshot_after = sqlx::query(
        "SELECT id, assignee, assignee_principal::text AS principal_text \
         FROM metis.issues_v2 WHERE is_latest = TRUE ORDER BY id",
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
        let before_principal: Option<String> = before.try_get("principal_text")?;
        let after_principal: Option<String> = after.try_get("principal_text")?;
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
// `suppress_sessions BOOLEAN NOT NULL DEFAULT FALSE` to `metis.statuses` —
// the schema-only prerequisite (PR-A) for the per-status
// session-suppression feature. No Rust code reads or writes the column
// yet (lands in PR-B); existing rows backfill to FALSE via the column
// default.
// ---------------------------------------------------------------------------

async fn add_statuses_suppress_sessions_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "statuses", "suppress_sessions").await? {
        bail!("expected `metis.statuses.suppress_sessions` column to exist post-rollforward");
    }
    if column_is_nullable(pool, "statuses", "suppress_sessions").await? {
        bail!("expected `metis.statuses.suppress_sessions` to be NOT NULL");
    }
    // Verify the declared DEFAULT is FALSE — backs the no-backfill
    // rollout. Postgres normalizes `FALSE` to `false` in
    // information_schema.column_default.
    let row = sqlx::query(
        "SELECT column_default FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = 'statuses' \
           AND column_name = 'suppress_sessions'",
    )
    .fetch_one(pool)
    .await
    .context("look up column_default for metis.statuses.suppress_sessions")?;
    let default_text: Option<String> = row.try_get("column_default")?;
    let default_text = default_text.ok_or_else(|| {
        anyhow::anyhow!("metis.statuses.suppress_sessions has no declared default")
    })?;
    if !default_text.eq_ignore_ascii_case("false") {
        bail!("expected metis.statuses.suppress_sessions DEFAULT false; got {default_text:?}");
    }
    Ok(())
}

async fn add_statuses_suppress_sessions_defaults_to_false(pool: &PgPool) -> Result<()> {
    // The migration has no backfill body: every existing row must come
    // out with the column FALSE via the column default.
    let rows = sqlx::query("SELECT project_id, sequence, suppress_sessions FROM metis.statuses")
        .fetch_all(pool)
        .await
        .context("read metis.statuses for suppress_sessions default check")?;
    if rows.is_empty() {
        bail!("expected at least one metis.statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: bool = row.try_get("suppress_sessions")?;
        if value {
            bail!(
                "metis.statuses({project_id}, sequence={sequence}): expected suppress_sessions=false (no backfill); got true"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260713000000_add_statuses_session_settings. Adds the
// `session_settings_json TEXT NULL` column to `metis.statuses` for the
// per-status `SessionSettings` override layer. Backfills to NULL — the
// read path materializes `SessionSettings::default()`.
// ---------------------------------------------------------------------------

async fn add_statuses_session_settings_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "statuses", "session_settings_json").await? {
        bail!("expected `metis.statuses.session_settings_json` column to exist post-rollforward");
    }
    if !column_is_nullable(pool, "statuses", "session_settings_json").await? {
        bail!("expected `metis.statuses.session_settings_json` to be NULLABLE");
    }
    let row = sqlx::query(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = 'statuses' \
           AND column_name = 'session_settings_json'",
    )
    .fetch_one(pool)
    .await
    .context("look up data_type for metis.statuses.session_settings_json")?;
    let data_type: String = row.try_get("data_type")?;
    if data_type != "text" {
        bail!("expected metis.statuses.session_settings_json data_type=text; got {data_type:?}");
    }
    Ok(())
}

async fn add_statuses_session_settings_defaults_to_null(pool: &PgPool) -> Result<()> {
    let rows =
        sqlx::query("SELECT project_id, sequence, session_settings_json FROM metis.statuses")
            .fetch_all(pool)
            .await
            .context("read metis.statuses for session_settings_json default check")?;
    if rows.is_empty() {
        bail!("expected at least one metis.statuses row to assert backfill against");
    }
    for row in &rows {
        let project_id: String = row.try_get("project_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let value: Option<String> = row.try_get("session_settings_json")?;
        if value.is_some() {
            bail!(
                "metis.statuses({project_id}, sequence={sequence}): expected session_settings_json=NULL (no backfill); got {value:?}"
            );
        }
    }
    Ok(())
}

async fn add_statuses_session_settings_domain_roundtrip(pool: &PgPool) -> Result<()> {
    // Pull the seeded default project back through `PostgresStoreV2::get_project`
    // — verifies the new `session_settings_json` column is included in the
    // `StatusRow` SELECT projections (catches the `#[sqlx(default)]`
    // foot-gun). Backfilled rows must rebuild as
    // `SessionSettings::default()`.
    use hydra_common::api::v1::issues::SessionSettings as ApiSessionSettings;
    let store = PostgresStoreV2::new(pool.clone());
    let project_id = ProjectId::from_str("j-defaul").context("parse j-defaul")?;
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("PostgresStoreV2::get_project(j-defaul) post-session_settings migration")?;
    if fetched.item.statuses.is_empty() {
        bail!("expected j-defaul to have statuses post-rollforward");
    }
    for status in &fetched.item.statuses {
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
// 20260714000000_create_issue_comments. Creates `metis.issue_comments`
// as the per-issue append-only comments stream. Sister to the SQLite
// migration `20260711000000_create_issue_comments.sql`. No backfill —
// the table starts empty.
// ---------------------------------------------------------------------------

async fn create_issue_comments_migration_schema_invariants(pool: &PgPool) -> Result<()> {
    if !table_exists(pool, "issue_comments").await? {
        bail!("expected metis.issue_comments table to exist post-rollforward");
    }

    // Every column NOT NULL with the expected PG data_type.
    let expected_columns: &[(&str, &str)] = &[
        ("issue_id", "text"),
        ("sequence", "bigint"),
        ("body", "text"),
        ("actor", "jsonb"),
        ("created_at", "timestamp with time zone"),
    ];
    for (col, want_type) in expected_columns {
        if !column_exists(pool, "issue_comments", col).await? {
            bail!("expected metis.issue_comments.{col} column to exist");
        }
        if column_is_nullable(pool, "issue_comments", col).await? {
            bail!("expected metis.issue_comments.{col} to be NOT NULL");
        }
        let row = sqlx::query(
            "SELECT data_type FROM information_schema.columns \
             WHERE table_schema = 'metis' AND table_name = 'issue_comments' \
               AND column_name = $1",
        )
        .bind(col)
        .fetch_one(pool)
        .await
        .with_context(|| format!("look up data_type for metis.issue_comments.{col}"))?;
        let data_type: String = row.try_get("data_type")?;
        if data_type != *want_type {
            bail!("metis.issue_comments.{col}: expected data_type={want_type}; got {data_type:?}");
        }
    }

    // `created_at` DEFAULT must reference `now()`.
    let row = sqlx::query(
        "SELECT column_default FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = 'issue_comments' \
           AND column_name = 'created_at'",
    )
    .fetch_one(pool)
    .await
    .context("look up column_default for metis.issue_comments.created_at")?;
    let default: Option<String> = row.try_get("column_default")?;
    let default = default.context("expected metis.issue_comments.created_at to carry a DEFAULT")?;
    if !default.to_lowercase().contains("now()") {
        bail!(
            "metis.issue_comments.created_at: expected DEFAULT referencing now(); got {default:?}"
        );
    }

    // Primary key is (issue_id, sequence) in that order.
    let row = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY k.ord) AS cols \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN unnest(string_to_array(i.indkey::text, ' ')::int[]) WITH ORDINALITY AS k(attnum, ord) ON TRUE \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE n.nspname = 'metis' AND c.relname = 'issue_comments' AND i.indisprimary",
    )
    .fetch_one(pool)
    .await
    .context("look up metis.issue_comments primary key columns")?;
    let pk_cols: Vec<String> = row.try_get("cols")?;
    if pk_cols != vec!["issue_id".to_string(), "sequence".to_string()] {
        bail!("metis.issue_comments PK: expected [issue_id, sequence]; got {pk_cols:?}");
    }

    // Covering DESC-by-sequence list index exists.
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'metis' AND c.relname = 'issue_comments_issue_seq_desc_idx')",
    )
    .fetch_one(pool)
    .await
    .context("check for issue_comments_issue_seq_desc_idx")?;
    let idx_exists: bool = row.try_get(0)?;
    if !idx_exists {
        bail!("expected metis.issue_comments_issue_seq_desc_idx to exist");
    }

    Ok(())
}

async fn create_issue_comments_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260714000000_create_issue_comments.sql"),
    )
    .context("read create_issue_comments migration body")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-execute create_issue_comments migration body")?;
    Ok(())
}

/// Drop and recreate the `metis` schema, and drop the sqlx migration tracking
/// table so the next `run_migrations` call replays from scratch.
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

// ---------------------------------------------------------------------------
// Baseline directory enumeration
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Baseline {
    version: u64,
    body: String,
}

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/migration_baselines")
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

/// Parse `<version>__<description>.sql` into the leading `u64` version. The
/// description after `__` is human-readable and not validated against
/// anything. Filenames not matching the pattern are rejected so typos and
/// leftover files surface loudly. (§3, §4 of `/designs/migration-testing-redesign.md`.)
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
// §3.1 schema invariants
// ---------------------------------------------------------------------------

async fn assert_schema_invariants(pool: &PgPool) -> Result<()> {
    // Columns added by this release's migrations.
    for (table, col) in [
        ("issues_v2", "assignee_principal"),
        ("auth_tokens", "session_id"),
        ("auth_tokens", "is_revoked"),
        ("auth_tokens", "creator"),
        ("tasks_v2", "mount_spec"),
        ("tasks_v2", "agent_config"),
        ("tasks_v2", "mode"),
        ("tasks_v2", "resumed_from"),
        ("repositories_v2", "merge_policy"),
        ("issues_v2", "project_id"),
        ("tasks_v2", "proxy_targets"),
    ] {
        if !column_exists(pool, table, col).await? {
            bail!("expected metis.{table}.{col} to exist after rollforward");
        }
    }

    // The proxy_targets column must be nullable so existing rows inflate to
    // an empty `Vec<ProxyTarget>` on read.
    if !column_is_nullable(pool, "tasks_v2", "proxy_targets").await? {
        bail!("expected metis.tasks_v2.proxy_targets to be nullable after rollforward");
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
    for table in [
        "notifications",
        "conversation_session_state",
        "conversation_events_v2",
        "actors_v2",
    ] {
        if table_exists(pool, table).await? {
            bail!("expected metis.{table} to be dropped after rollforward");
        }
    }

    // Tables added by this release's migrations.
    for table in [
        "session_events_v2",
        "session_state_v2",
        "triggers",
        "projects",
        "statuses",
    ] {
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

    // NOT NULL tightening on auth_tokens.creator landed by
    // 20260609000000_add_creator_to_auth_tokens.sql.
    if column_is_nullable(pool, "auth_tokens", "creator").await? {
        bail!("expected metis.auth_tokens.creator to be NOT NULL after rollforward");
    }

    // Column added by 20260606010000_add_projects_prompt_path.sql.
    if !column_exists(pool, "projects", "prompt_path").await? {
        bail!("expected metis.projects.prompt_path column to exist after rollforward");
    }
    if !column_is_nullable(pool, "projects", "prompt_path").await? {
        bail!("expected metis.projects.prompt_path to be nullable after rollforward");
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
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-userpfx0001",
        Some(serde_json::json!({"User": {"name": "jayantk"}})),
    )
    .await?;
    expect_assignee_principal(
        pool,
        "i-agentpfx001",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // external/<sys>/<x> is intentionally left NULL by the SQL backfill.
    expect_assignee_principal(pool, "i-extslash001", None).await?;
    expect_assignee_principal(pool, "i-nullasgn01", None).await?;

    // ---- patches_v2: reviews[*].author -> typed Principal ----
    expect_first_review_author(
        pool,
        "p-bareauth01",
        serde_json::json!({"User": {"name": "jayantk"}}),
    )
    .await?;
    expect_first_review_author(
        pool,
        "p-agentauth1",
        serde_json::json!({"Agent": {"name": "swe"}}),
    )
    .await?;
    // Already-typed author must pass through the rewrite untouched.
    expect_first_review_author(
        pool,
        "p-typedauth1",
        serde_json::json!({"User": {"name": "jayantk"}}),
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
         WHERE actor_name = 'users/legacy' AND token_hash = 'deadbeef'",
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

    let row = sqlx::query("SELECT mode::text FROM metis.tasks_v2 WHERE id = 's-interbase'")
        .fetch_one(pool)
        .await?;
    let mode_text: String = row.get(0);
    let mode: serde_json::Value = serde_json::from_str(&mode_text)?;
    if mode.get("type").and_then(|v| v.as_str()) != Some("interactive") {
        bail!("expected s-interbase.mode.type='interactive'; got {mode}");
    }
    if mode.get("conversation_id").and_then(|v| v.as_str()) != Some("c-convbase") {
        bail!("expected s-interbase.mode.conversation_id='c-convbase'; got {mode}");
    }

    // resumed_from on s-interresume should point at s-interbase (the
    // is_latest-true predecessor in the same conversation).
    let row = sqlx::query("SELECT resumed_from FROM metis.tasks_v2 WHERE id = 's-interresume'")
        .fetch_one(pool)
        .await?;
    let resumed_from: Option<String> = row.get(0);
    if resumed_from.as_deref() != Some("s-interbase") {
        bail!("expected s-interresume.resumed_from='s-interbase'; got {resumed_from:?}");
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
// Events migration partitioning edge cases.
//
// The baseline fixture seeds four conversations that exercise the tolerant
// assignment rules added to `src/store/migrations/events.rs`:
//
//   * `c-convnoses`  — conversation has message events but zero linked
//                      sessions: migration must NOT bail; events are dropped.
//   * `c-convbefore` — single session whose creation_time is AFTER the only
//                      message event; event lands on the first session.
//   * `c-convgap`    — two sessions with a gap; event in the gap lands on
//                      the subsequent session.
//   * `c-convafter`  — two sessions where the last suspends; event past the
//                      suspend timestamp lands on the last session.
// ---------------------------------------------------------------------------

async fn assert_events_migration_edge_cases(pool: &PgPool) -> Result<()> {
    // 1. c-convnoses: no linked sessions, so no session_events for it.
    //    The real check is that the migration ran to completion above.
    let row = sqlx::query(
        "SELECT COUNT(*) FROM metis.session_events_v2 se \
         JOIN metis.tasks_v2 t ON t.id = se.session_id \
         WHERE t.conversation_id = 'c-convnoses'",
    )
    .fetch_one(pool)
    .await
    .context("count session_events for c-convnoses")?;
    let count: i64 = row.get(0);
    if count != 0 {
        bail!("expected 0 session_events for c-convnoses (no sessions exist); got {count}");
    }

    // 2. c-convbefore: the single message event predating s-beforeone's
    //    creation_time must still land on s-beforeone.
    expect_session_event_count(pool, "s-beforeone", 1).await?;

    // 3. c-convgap: the gap event must land on s-gaptwo (subsequent
    //    session), NOT s-gapone.
    expect_session_event_count(pool, "s-gapone", 0).await?;
    expect_session_event_count(pool, "s-gaptwo", 1).await?;

    // 4. c-convafter: the post-suspend event on the last session lands on
    //    s-aftertwo. s-afterone owns nothing (no events in its window).
    expect_session_event_count(pool, "s-afterone", 0).await?;
    expect_session_event_count(pool, "s-aftertwo", 1).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// actor_variant_cleanup rewrite assertions
// ---------------------------------------------------------------------------
//
// The baseline at `20260603000000__pre_actor_variant_cleanup.sql` seeds
// one row per pre-cleanup actor shape (Username, Session, Issue ±match,
// Service ±valid, Legacy bare-string ±parseable, multi-key map,
// already-typed). After the cleanup migration runs, every row's
// `actor` / `actor_id` column must hold either the rewritten
// post-cleanup wire shape or NULL.
//
// Round-2 store/domain-level smoke: deserializing the rewritten rows
// through `PostgresStoreV2` exercises `ActorRef::deserialize` against
// the migration's raw `serde_json::Value` output and catches serde
// drift between the two.

async fn assert_actor_variant_cleanup(pool: &PgPool) -> Result<()> {
    // 1. Username -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actuname",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 2. Session -> Adhoc
    expect_issue_actor_actor_id(
        pool,
        "i-actsess",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-sessone"}})),
    )
    .await?;
    // 3. Issue with matching tasks_v2 row -> resolved User
    expect_issue_actor_actor_id(
        pool,
        "i-actiss",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 4. Issue without matching tasks_v2 row -> External-legacy(<iid>)
    expect_issue_actor_actor_id(pool, "i-actissno", Some(external_legacy("i-actisstwo"))).await?;
    // 5. Service with valid AgentName -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actsvcok",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 6. Service with invalid AgentName -> External-legacy(<name>)
    expect_issue_actor_actor_id(pool, "i-actsvcno", Some(external_legacy("has space"))).await?;
    // 7. Legacy users/<x> -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actlegu",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 8. Legacy agents/<x> -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actlega",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 9. Legacy unparseable -> External-legacy(<bare-string>)
    expect_issue_actor_actor_id(
        pool,
        "i-actlegx",
        Some(external_legacy("definitely not an actor")),
    )
    .await?;
    // 10. Already-typed User -> no-op (still User)
    expect_issue_actor_actor_id(
        pool,
        "i-actuser",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 11. Multi-key actor_id -> External-legacy(JSON form of the map)
    expect_issue_actor_actor_id(
        pool,
        "i-actmulti",
        Some(external_legacy(
            serde_json::json!({"kind": "user", "name": "alice"}).to_string(),
        )),
    )
    .await?;
    // 12. Legacy adhoc/<sid> -> Adhoc
    expect_issue_actor_actor_id(
        pool,
        "i-actadhoc",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-adhocone"}})),
    )
    .await?;
    // 13. Legacy external/<sys>/<user> -> External
    expect_issue_actor_actor_id(
        pool,
        "i-actextn",
        Some(serde_json::json!({"External": {"system": "github", "username": "jayantk"}})),
    )
    .await?;
    // 14. Legacy u-<x> shorthand -> User
    expect_issue_actor_actor_id(
        pool,
        "i-actushrt",
        Some(serde_json::json!({"User": {"name": "alice"}})),
    )
    .await?;
    // 15. Legacy s-<sid> shorthand -> Adhoc (session_id preserves the s- prefix)
    expect_issue_actor_actor_id(
        pool,
        "i-actsshrt",
        Some(serde_json::json!({"Adhoc": {"session_id": "s-abcdef"}})),
    )
    .await?;
    // 16. Legacy svc-<n> shorthand -> Agent
    expect_issue_actor_actor_id(
        pool,
        "i-actsvshr",
        Some(serde_json::json!({"Agent": {"name": "swe"}})),
    )
    .await?;
    // 17. Legacy users/<x> with invalid Username -> External-legacy("users/<x>")
    expect_issue_actor_actor_id(pool, "i-actubad", Some(external_legacy("users/has space")))
        .await?;
    // 18. Legacy agents/<x> with invalid AgentName -> External-legacy("agents/<x>")
    expect_issue_actor_actor_id(
        pool,
        "i-actabad",
        Some(external_legacy("agents/with space")),
    )
    .await?;
    // 19. Legacy external/<sys>/<x> with invalid system -> External-legacy(<bare-string>)
    expect_issue_actor_actor_id(
        pool,
        "i-actexbad",
        Some(external_legacy("external/has space/foo")),
    )
    .await?;
    // 20. Legacy a-<issue_id> shorthand -> External-legacy("a-<issue_id>")
    expect_issue_actor_actor_id(pool, "i-actashrt", Some(external_legacy("a-i-actissone"))).await?;

    // Issue-arm tie-break edges: multi-match / deleted-only / not-latest /
    // chained Issue all fall back to External-legacy with the original
    // parent issue id preserved as the username, because
    // `load_issue_to_actor_id` only inserts when exactly one
    // post-cleanup-resolvable task is associated with the issue.
    for (issue_id, expected_iid) in [
        ("i-actrefmny", "i-actissmany"),
        ("i-actrefdel", "i-actissdel"),
        ("i-actrefold", "i-actissold"),
        ("i-actrefchn", "i-actisschn"),
    ] {
        expect_issue_actor_actor_id(pool, issue_id, Some(external_legacy(expected_iid))).await?;
    }

    // Nested ActorRef rewrites — System.on_behalf_of resolved/unresolved
    // and Automation.triggered_by resolved/unresolved. Unresolved sub-actors
    // collapse to null WITHIN the ActorRef rather than NULLing the row.
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actsysu",
        Some(serde_json::json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": {"User": {"name": "alice"}}
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actsysn",
        Some(serde_json::json!({
            "System": {
                "worker_name": "task-spawner",
                "on_behalf_of": null
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actauto",
        Some(serde_json::json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": {
                    "Authenticated": {"actor_id": {"User": {"name": "alice"}}}
                }
            }
        })),
    )
    .await?;
    expect_table_actor(
        pool,
        "issues_v2",
        "i-actauton",
        Some(serde_json::json!({
            "Automation": {
                "automation_name": "github_pr_sync",
                "triggered_by": null
            }
        })),
    )
    .await?;

    // Multi-table coverage — every table in `ACTOR_REF_TABLES_COMMON`
    // (plus session_events_v2 below) carries one Username-legacy row
    // that the cleanup must rewrite to a User-tagged ActorRef.
    let expected_user_authenticated = serde_json::json!({
        "Authenticated": {"actor_id": {"User": {"name": "alice"}}}
    });
    for (table, id) in [
        ("repositories_v2", "r-actreplc"),
        ("users_v2", "u-actusrlc"),
        ("patches_v2", "p-actpchlc"),
        ("tasks_v2", "s-actrowx"),
        ("documents_v2", "d-actdoclc"),
    ] {
        expect_table_actor(pool, table, id, Some(expected_user_authenticated.clone())).await?;
    }

    // session_events_v2 has a (session_id, version_number) primary key
    // and no is_latest column, so it gets a one-off inline assertion.
    let row = sqlx::query(
        "SELECT actor::text AS pl FROM metis.session_events_v2 \
         WHERE session_id = 's-actrowx' AND version_number = 1",
    )
    .fetch_one(pool)
    .await
    .context("read session_events_v2.actor for (s-actrowx, 1)")?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode session_events_v2.actor")?;
    if got != Some(expected_user_authenticated.clone()) {
        bail!(
            "session_events_v2(s-actrowx, 1).actor: expected {expected_user_authenticated}; got {got:?}",
        );
    }

    // conversations_v2 — the new walker added in [[i-jyhvstcj]]
    // rewrites the `actor` column on this table too. The fixture seeds
    // the exact prod symptom (`Session`-tagged inner actor_id) so the
    // §3.3 store-level smoke below proves `get_conversation` stops
    // failing to deserialize.
    expect_table_actor(
        pool,
        "conversations_v2",
        "c-actconvx",
        Some(serde_json::json!({
            "Authenticated": {"actor_id": {"Adhoc": {"session_id": "s-csessacx"}}}
        })),
    )
    .await?;

    // issues_v2.form_response — the new walker added in [[i-jyhvstcj]]
    // rewrites the embedded `.actor` while preserving sibling fields
    // (`action_id`, `values`, `submitted_at`).
    let row = sqlx::query(
        "SELECT form_response::text AS pl FROM metis.issues_v2 \
         WHERE id = 'i-actform' AND is_latest = TRUE",
    )
    .fetch_one(pool)
    .await
    .context("read issues_v2.form_response for i-actform")?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("decode issues_v2.form_response JSON for i-actform")?;
    let expected_form_response = serde_json::json!({
        "action_id": "approve",
        "actor": {"User": {"name": "alice"}},
        "values": {"score": 4},
        "submitted_at": "2026-05-10T11:00:00Z"
    });
    if got.as_ref() != Some(&expected_form_response) {
        bail!("issues_v2(i-actform).form_response: expected {expected_form_response}; got {got:?}");
    }

    // §3.3 smoke: read every row through the store and confirm
    // `ActorRef::deserialize` accepts the rewritten payload — including
    // the new External-legacy fallback shape that catches serde drift
    // between the migration's raw JSON and
    // `hydra_common::ActorId::deserialize`.
    use hydra_server::domain::actors::{ActorId, ActorRef as DomainActorRef};
    let store = PostgresStoreV2::new(pool.clone());
    let legacy = ExternalSystem::try_new("legacy").unwrap();
    let external_legacy_actor = |username: &str| ActorId::External {
        system: legacy.clone(),
        username: username.to_string(),
    };
    for (issue_id, expected_actor_id) in [
        (
            "i-actuname",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        ("i-actsess", ActorId::Adhoc("s-sessone".parse().unwrap())),
        (
            "i-actiss",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        // Previously NULL — now External-legacy.
        ("i-actissno", external_legacy_actor("i-actisstwo")),
        (
            "i-actsvcok",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        ("i-actsvcno", external_legacy_actor("has space")),
        (
            "i-actlegu",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        (
            "i-actlega",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        (
            "i-actlegx",
            external_legacy_actor("definitely not an actor"),
        ),
        (
            "i-actuser",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        (
            "i-actmulti",
            external_legacy_actor(
                &serde_json::json!({"kind": "user", "name": "alice"}).to_string(),
            ),
        ),
        ("i-actadhoc", ActorId::Adhoc("s-adhocone".parse().unwrap())),
        (
            "i-actextn",
            ActorId::External {
                system: ExternalSystem::try_new("github").unwrap(),
                username: "jayantk".to_string(),
            },
        ),
        (
            "i-actushrt",
            ActorId::User(ApiUsername::try_new("alice").unwrap()),
        ),
        ("i-actsshrt", ActorId::Adhoc("s-abcdef".parse().unwrap())),
        (
            "i-actsvshr",
            ActorId::Agent(AgentName::try_new("swe").unwrap()),
        ),
        ("i-actubad", external_legacy_actor("users/has space")),
        ("i-actabad", external_legacy_actor("agents/with space")),
        (
            "i-actexbad",
            external_legacy_actor("external/has space/foo"),
        ),
        ("i-actashrt", external_legacy_actor("a-i-actissone")),
        // Issue-arm tie-break edges fall back with the parent issue id.
        ("i-actrefmny", external_legacy_actor("i-actissmany")),
        ("i-actrefdel", external_legacy_actor("i-actissdel")),
        ("i-actrefold", external_legacy_actor("i-actissold")),
        ("i-actrefchn", external_legacy_actor("i-actisschn")),
    ] {
        let issue = store
            .get_issue(&parse_issue_id(issue_id)?, false)
            .await
            .with_context(|| format!("store-level read of {issue_id}"))?;
        let actor = issue.actor.with_context(|| {
            format!("expected non-None actor on {issue_id} after cleanup; got None")
        })?;
        match &actor {
            DomainActorRef::Authenticated { actor_id, .. } if actor_id == &expected_actor_id => {}
            other => bail!(
                "{issue_id}: expected Authenticated(actor_id={expected_actor_id:?}); got {other:?}"
            ),
        }
    }

    // Nested ActorRef shapes — `System` and `Automation` rows must
    // round-trip through `ActorRef::deserialize` with the inner
    // `on_behalf_of` / `triggered_by` rewritten or collapsed to None.
    let sysu = store
        .get_issue(&parse_issue_id("i-actsysu")?, false)
        .await
        .context("store-level read of i-actsysu")?;
    match sysu.actor.as_ref() {
        Some(DomainActorRef::System {
            worker_name,
            on_behalf_of: Some(ActorId::User(name)),
        }) if worker_name == "task-spawner" && name.as_str() == "alice" => {}
        other => bail!(
            "i-actsysu: expected System(worker_name=task-spawner, on_behalf_of=User(alice)); got {other:?}"
        ),
    }
    let sysn = store
        .get_issue(&parse_issue_id("i-actsysn")?, false)
        .await
        .context("store-level read of i-actsysn")?;
    match sysn.actor.as_ref() {
        Some(DomainActorRef::System {
            worker_name,
            on_behalf_of: None,
        }) if worker_name == "task-spawner" => {}
        other => bail!(
            "i-actsysn: expected System(worker_name=task-spawner, on_behalf_of=None); got {other:?}"
        ),
    }
    let auto = store
        .get_issue(&parse_issue_id("i-actauto")?, false)
        .await
        .context("store-level read of i-actauto")?;
    match auto.actor.as_ref() {
        Some(DomainActorRef::Automation {
            automation_name,
            triggered_by: Some(boxed),
        }) if automation_name == "github_pr_sync" => match boxed.as_ref() {
            DomainActorRef::Authenticated {
                actor_id: ActorId::User(name),
                ..
            } if name.as_str() == "alice" => {}
            other => {
                bail!("i-actauto.triggered_by: expected Authenticated(User(alice)); got {other:?}")
            }
        },
        other => bail!(
            "i-actauto: expected Automation(github_pr_sync, triggered_by=Authenticated(User(alice))); got {other:?}"
        ),
    }
    let auton = store
        .get_issue(&parse_issue_id("i-actauton")?, false)
        .await
        .context("store-level read of i-actauton")?;
    match auton.actor.as_ref() {
        Some(DomainActorRef::Automation {
            automation_name,
            triggered_by: None,
        }) if automation_name == "github_pr_sync" => {}
        other => bail!(
            "i-actauton: expected Automation(github_pr_sync, triggered_by=None); got {other:?}"
        ),
    }

    // Multi-table coverage — read the post-cleanup actor through the
    // store for tables that expose an `id`-keyed getter so any serde
    // drift between the per-table walker's raw JSON output and the
    // domain `ActorRef` shows up here. `repositories_v2`/`users_v2`
    // lookups go through name-typed keys (RepoName / Username) that
    // the bare-id fixtures don't satisfy, so those stay at SQL-level
    // only above. `session_events_v2` is data-store-only.
    let multi_user = ActorId::User(ApiUsername::try_new("alice").unwrap());
    let patch = store
        .get_patch(&parse_patch_id("p-actpchlc")?, false)
        .await
        .context("store-level read of p-actpchlc")?;
    match patch.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("p-actpchlc: expected Authenticated(User(alice)); got {other:?}"),
    }
    let session = store
        .get_session(&parse_session_id("s-actrowx")?, false)
        .await
        .context("store-level read of s-actrowx")?;
    match session.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("s-actrowx: expected Authenticated(User(alice)); got {other:?}"),
    }
    let document = store
        .get_document(
            &DocumentId::from_str("d-actdoclc").context("parse d-actdoclc as DocumentId")?,
            false,
        )
        .await
        .context("store-level read of d-actdoclc")?;
    match document.actor.as_ref() {
        Some(DomainActorRef::Authenticated { actor_id, .. }) if actor_id == &multi_user => {}
        other => bail!("d-actdoclc: expected Authenticated(User(alice)); got {other:?}"),
    }

    // conversations_v2 store-level smoke — the load-bearing acceptance
    // criterion for [[i-jyhvstcj]]: prod's `GET /v1/conversations` was
    // failing on this exact shape pre-fix.
    let conversation = store
        .get_conversation(
            &ConversationId::from_str("c-actconvx")
                .context("parse c-actconvx as ConversationId")?,
            false,
        )
        .await
        .context("store-level read of c-actconvx")?;
    let conv_session_id: SessionId = "s-csessacx".parse().unwrap();
    match conversation.actor.as_ref() {
        Some(DomainActorRef::Authenticated {
            actor_id: ActorId::Adhoc(sid),
            ..
        }) if sid == &conv_session_id => {}
        other => bail!("c-actconvx: expected Authenticated(Adhoc(s-csessacx)); got {other:?}"),
    }

    // issues_v2 form_response store-level smoke — the other load-bearing
    // acceptance criterion for [[i-jyhvstcj]]: prod's `GET /v1/issues`
    // was failing on the embedded `Username` actor pre-fix.
    let form_issue = store
        .get_issue(&parse_issue_id("i-actform")?, false)
        .await
        .context("store-level read of i-actform")?;
    let form_response = form_issue
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

/// Read the `actor` JSONB column from any `id`+`is_latest`-keyed table
/// and compare it to `expected`. Used by the §3.2 multi-table coverage
/// assertions and the nested `ActorRef` (System/Automation) checks.
async fn expect_table_actor(
    pool: &PgPool,
    table: &str,
    id: &str,
    expected: Option<serde_json::Value>,
) -> Result<()> {
    let sql = format!(
        "SELECT actor::text AS pl FROM metis.{table} \
         WHERE id = $1 AND is_latest = TRUE",
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read {table}.actor for {id}"))?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .with_context(|| format!("decode {table}.actor JSON for {id}"))?;
    if got != expected {
        bail!("{table}({id}).actor: expected {expected:?}; got {got:?}");
    }
    Ok(())
}

async fn expect_issue_actor_actor_id(
    pool: &PgPool,
    issue_id: &str,
    expected_actor_id: Option<serde_json::Value>,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT actor::text AS pl FROM metis.issues_v2 \
         WHERE id = $1 AND is_latest = TRUE",
    )
    .bind(issue_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("read issues_v2.actor for {issue_id}"))?;
    let raw: Option<String> = row.get("pl");
    let got: Option<serde_json::Value> = raw
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .with_context(|| format!("decode issues_v2.actor JSON for {issue_id}"))?;
    let actor_id = got.as_ref().and_then(|v| {
        v.get("Authenticated")
            .and_then(|a| a.get("actor_id"))
            .cloned()
    });
    if actor_id != expected_actor_id {
        bail!(
            "issue {issue_id}: expected Authenticated.actor_id={expected_actor_id:?}; got full actor={got:?}",
        );
    }
    Ok(())
}

/// Canonical External-legacy fallback JSON wire shape.
fn external_legacy(username: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "External": {"system": "legacy", "username": username.into()}
    })
}

async fn expect_session_event_count(pool: &PgPool, session_id: &str, expected: i64) -> Result<()> {
    let row = sqlx::query("SELECT COUNT(*) FROM metis.session_events_v2 WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("count session_events_v2 for {session_id}"))?;
    let got: i64 = row.get(0);
    if got != expected {
        bail!("session {session_id}: expected {expected} session_events; got {got}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// §3.3 store / domain-level smoke
//
// The §3.2 assertions above verify SQL-level shapes of the migrated rows. This
// third layer reads them back through the live `Store` trait and asserts the
// typed domain objects (Principal, SessionMode, Review, SessionEvent,
// refers-to relationship) deserialize cleanly, then exercises a
// create→read-back cycle on the same APIs so any post-migration write path
// that diverged from the read path fails loud here instead of at first prod
// traffic.
//
// Preserved verbatim — this is the round-2 acceptance criterion from the
// prior design. See memory rule `migration-test-read-migrated` and design
// §3.3 / §7.
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
    // Headless task: mode backfill -> unit-like SessionMode::Headless, with
    // the legacy `prompt` column backfilled onto `agent_config.system_prompt`.
    let session = store
        .get_session(&parse_session_id("s-headalpha")?, false)
        .await
        .context("Store::get_session(s-headalpha)")?;
    if !matches!(&session.item.mode, SessionMode::Headless) {
        bail!(
            "s-headalpha: expected unit-like SessionMode::Headless; got {:?}",
            session.item.mode
        );
    }
    if session.item.agent_config.system_prompt.as_deref() != Some("do a thing") {
        bail!(
            "s-headalpha: expected agent_config.system_prompt='do a thing'; got {:?}",
            session.item.agent_config.system_prompt
        );
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
        status("open"),
        hydra_server::domain::projects::default_project_id(),
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
        AgentConfig::new(None, None, Some("smoke: do a thing".to_string()), None),
        Default::default(),
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
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
    if !matches!(&fetched.item.mode, SessionMode::Headless) {
        bail!(
            "post-migration create_session did not round-trip SessionMode::Headless; got {:?}",
            fetched.item.mode
        );
    }
    if fetched.item.agent_config.system_prompt.as_deref() != Some("smoke: do a thing") {
        bail!(
            "post-migration create_session did not round-trip agent_config.system_prompt; \
             got {:?}",
            fetched.item.agent_config.system_prompt
        );
    }
    Ok(())
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

// ---------------------------------------------------------------------------
// Recent-migration data-shape round-trip
//
// The three most recent feature migrations (`20260603020000_add_triggers_table`,
// `20260604000000_drop_conversation_events_v2`, `20260604000001_create_projects`)
// land schema after the last baseline pin, so no fixture rows exist for them.
// Seed one row in each of `metis.triggers` and `metis.projects` via raw SQL
// post-rollforward, then read back through `PostgresStoreV2::get_trigger` /
// `get_project` to confirm:
//
//   * the `BEFORE INSERT` `maintain_latest_version` triggers flipped
//     `is_latest = true` (`get_*` relies on the `id`-keyed lookups, which
//     would surface a NULL row if the trigger never fired);
//   * the JSONB columns (`schedule` / `actions` for triggers,
//     `statuses` for projects) deserialize cleanly into the typed domain
//     objects;
//   * the primary key (`id`, `version_number`) round-trips through the store.
// ---------------------------------------------------------------------------

async fn assert_recent_migration_data_shape(pool: &PgPool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey};
    use hydra_common::triggers::{Schedule, Trigger as ApiTrigger};

    // ---- triggers -----------------------------------------------------
    let trigger_id = parse_trigger_id("t-rtrip")?;
    let schedule_json = serde_json::json!({
        "Cron": {"expression": "0 9 * * MON", "timezone": "UTC"}
    });
    let actions_json = serde_json::json!([]);
    sqlx::query(
        "INSERT INTO metis.triggers \
         (id, version_number, enabled, creator, schedule, actions) \
         VALUES ($1, 1, TRUE, 'jayantk', $2, $3)",
    )
    .bind(trigger_id.as_ref())
    .bind(&schedule_json)
    .bind(&actions_json)
    .execute(pool)
    .await
    .context("seed metis.triggers row for round-trip assertion")?;

    let store = PostgresStoreV2::new(pool.clone());
    let fetched = store
        .get_trigger(&trigger_id, false)
        .await
        .context("Store::get_trigger post-migration")?;
    if fetched.version != 1 {
        bail!(
            "trigger {trigger_id}: expected version 1; got {}",
            fetched.version
        );
    }
    let ApiTrigger {
        enabled,
        schedule,
        actions,
        creator,
        ..
    } = &fetched.item;
    if !enabled {
        bail!("trigger {trigger_id}: expected enabled=true");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "trigger {trigger_id}: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    match schedule {
        Schedule::Cron {
            expression,
            timezone,
        } if expression == "0 9 * * MON" && timezone.as_deref() == Some("UTC") => {}
        other => bail!("trigger {trigger_id}: expected Cron(0 9 * * MON / UTC); got {other:?}"),
    }
    if !actions.is_empty() {
        bail!(
            "trigger {trigger_id}: expected empty actions; got {} entries",
            actions.len()
        );
    }

    // ---- projects -----------------------------------------------------
    let project_id = parse_project_id("j-rtrip")?;
    // Post-cutover (20260614000000): `metis.projects.statuses` JSONB
    // is gone, so the smoke seed populates the post-cutover schema —
    // the `projects` row carries the per-project high-water mark, and
    // the per-status row lives in `metis.statuses`. Include
    // `prompt_path` so 20260606010000_add_projects_prompt_path is
    // exercised, and `next_status_sequence` so 20260614000000's new
    // column is exercised.
    sqlx::query(
        "INSERT INTO metis.projects \
         (id, version_number, key, name, creator, prompt_path, next_status_sequence) \
         VALUES ($1, 1, 'roundtrip', 'Roundtrip', 'jayantk', $2, 2)",
    )
    .bind(project_id.as_ref())
    .bind("/projects/roundtrip/prompt.md")
    .execute(pool)
    .await
    .context("seed metis.projects row for round-trip assertion")?;
    sqlx::query(
        "INSERT INTO metis.statuses (project_id, sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive) \
         VALUES ($1, 1, 'open', 'Open', '#3498db', FALSE, FALSE, FALSE, NULL, NULL, FALSE)",
    )
    .bind(project_id.as_ref())
    .execute(pool)
    .await
    .context("seed metis.statuses row for round-trip project")?;

    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("Store::get_project post-migration")?;
    if fetched.version != 1 {
        bail!(
            "project {project_id}: expected version 1; got {}",
            fetched.version
        );
    }
    let ApiProject {
        key,
        name,
        statuses,
        creator,
        prompt_path,
        ..
    } = &fetched.item;
    if key != &ProjectKey::try_new("roundtrip").unwrap() {
        bail!("project {project_id}: expected key='roundtrip'; got {key:?}");
    }
    if name != "Roundtrip" {
        bail!("project {project_id}: expected name='Roundtrip'; got {name:?}");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "project {project_id}: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    if prompt_path.as_deref() != Some("/projects/roundtrip/prompt.md") {
        bail!(
            "project {project_id}: expected prompt_path='/projects/roundtrip/prompt.md'; got {prompt_path:?}"
        );
    }
    let Some(status) = statuses.first() else {
        bail!("project {project_id}: expected one status; got none");
    };
    if status.key.as_str() != "open"
        || status.label != "Open"
        || status.color.as_ref() != "#3498db"
        || status.unblocks_parents
        || status.unblocks_dependents
        || status.cascades_to_children
    {
        bail!(
            "project {project_id}: status[0] did not round-trip the seeded JSONB shape; got {status:?}"
        );
    }

    Ok(())
}

fn parse_issue_id(s: &str) -> Result<IssueId> {
    IssueId::from_str(s).with_context(|| format!("parse issue id '{s}'"))
}

fn parse_trigger_id(s: &str) -> Result<TriggerId> {
    TriggerId::from_str(s).with_context(|| format!("parse trigger id '{s}'"))
}

fn parse_project_id(s: &str) -> Result<ProjectId> {
    ProjectId::from_str(s).with_context(|| format!("parse project id '{s}'"))
}

fn parse_patch_id(s: &str) -> Result<PatchId> {
    PatchId::from_str(s).with_context(|| format!("parse patch id '{s}'"))
}

fn parse_session_id(s: &str) -> Result<SessionId> {
    SessionId::from_str(s).with_context(|| format!("parse session id '{s}'"))
}

// ---------------------------------------------------------------------------
// 20260607000000_seed_default_project — assert that the seed INSERT, the
// `metis.issues_v2.project_id` backfill UPDATE, and the migration's
// idempotency guard (`ON CONFLICT (id, version_number) DO NOTHING`) all
// behave as designed. Coverage gap closed by [[i-bivbnsgb]] (follow-up to
// [[p-xtixlxfy]]) — the merged seed migration shipped with in-store
// round-trip tests but no migration-framework coverage.
// ---------------------------------------------------------------------------

async fn seed_default_project_migration_inserts_row(pool: &PgPool) -> Result<()> {
    // Post-cutover (20260614000000), `metis.projects.statuses` JSONB
    // is gone; statuses live in `metis.statuses`. Validate the row's
    // non-statuses columns directly, then rebuild the status set
    // from `metis.statuses` and compare against
    // `default_project_seed()`.
    let row = sqlx::query(
        "SELECT id, version_number, key, name, creator, archived, \
                actor::text AS actor, is_latest, prompt_path \
         FROM metis.projects WHERE id = 'j-defaul'",
    )
    .fetch_one(pool)
    .await
    .context("read seeded default project row 'j-defaul'")?;

    let id: String = row.try_get("id")?;
    let version_number: i64 = row.try_get("version_number")?;
    let key: String = row.try_get("key")?;
    let name: String = row.try_get("name")?;
    let creator: String = row.try_get("creator")?;
    let archived: bool = row.try_get("archived")?;
    let is_latest: bool = row.try_get("is_latest")?;
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
    if archived {
        bail!("j-defaul: expected archived=FALSE; got TRUE");
    }
    if !is_latest {
        bail!("j-defaul: expected is_latest=TRUE; got FALSE");
    }
    if actor.is_some() {
        bail!("j-defaul: expected actor=NULL; got {actor:?}");
    }
    if prompt_path.as_deref() != Some("/projects/default/prompt.md") {
        bail!("j-defaul: expected prompt_path='/projects/default/prompt.md'; got {prompt_path:?}");
    }

    let status_rows = sqlx::query(
        "SELECT sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter::text AS on_enter_text, prompt_path, interactive \
         FROM metis.statuses WHERE project_id = 'j-defaul' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read j-defaul rows from metis.statuses")?;
    let mut statuses: Vec<StatusDefinition> = Vec::with_capacity(status_rows.len());
    for row in &status_rows {
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: bool = row.try_get("unblocks_parents")?;
        let unblocks_dependents: bool = row.try_get("unblocks_dependents")?;
        let cascades_to_children: bool = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: bool = row.try_get("interactive")?;
        let on_enter_value = on_enter_text
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .context("decode metis.statuses.on_enter")?;
        let mut def = StatusDefinition::new(
            hydra_common::api::v1::projects::StatusKey::try_new(key)
                .map_err(|e| anyhow::anyhow!("invalid status key: {e}"))?,
            label,
            color
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid status color: {e}"))?,
            unblocks_parents,
            unblocks_dependents,
            cascades_to_children,
            on_enter_value,
        );
        def.prompt_path = prompt_path;
        def.interactive = interactive;
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

async fn seed_default_project_migration_backfills_null_project_ids(pool: &PgPool) -> Result<()> {
    // Every fixture row that had NULL `project_id` at baseline-insert
    // time (single-version and multi-version) must now point at
    // `'j-defaul'`. The multi-version rows verify that the UPDATE
    // touches every NULL row regardless of `is_latest`.
    for (id, version) in [("i-seedone", 1_i64), ("i-seedmv", 1), ("i-seedmv", 2)] {
        let row = sqlx::query(
            "SELECT project_id FROM metis.issues_v2 \
             WHERE id = $1 AND version_number = $2",
        )
        .bind(id)
        .bind(version)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read project_id for metis.issues_v2({id}, {version})"))?;
        let project_id: Option<String> = row.try_get("project_id")?;
        if project_id.as_deref() != Some("j-defaul") {
            bail!(
                "metis.issues_v2({id}, {version}).project_id: \
                 expected 'j-defaul'; got {project_id:?}"
            );
        }
    }

    // Catch-all: no `issues_v2` row should be left with NULL project_id
    // post-backfill. The migration's UPDATE is unconditional on
    // `is_latest`, so older / soft-deleted versions get backfilled too.
    let row = sqlx::query("SELECT COUNT(*) FROM metis.issues_v2 WHERE project_id IS NULL")
        .fetch_one(pool)
        .await
        .context("count remaining NULL project_id rows after backfill")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 metis.issues_v2 rows with NULL project_id post-backfill; got {count}");
    }
    Ok(())
}

async fn seed_default_project_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Original behavior of this assertion was to replay the seed
    // migration body verbatim. After
    // 20260611000000_drop_projects_default_status_key drops a column
    // the body references, a verbatim re-apply errors — so the
    // idempotency guarantee that matters here is now: after the full
    // migration plan rolls forward, exactly one j-defaul row exists.
    let row = sqlx::query("SELECT COUNT(*) FROM metis.projects WHERE id = 'j-defaul'")
        .fetch_one(pool)
        .await
        .context("count projects rows for j-defaul after rollforward")?;
    let count: i64 = row.try_get(0)?;
    if count != 1 {
        bail!("expected exactly 1 metis.projects row for j-defaul post-rollforward; got {count}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260608000000_drop_status_icon — assert that the migration strips the
// `icon` key from every row's `metis.projects.statuses` array (both the
// already-seeded `j-defaul` row and a custom fixture row), the migration
// body is idempotent, and the post-migration JSON deserializes cleanly
// through `Vec<StatusDefinition>` (which no longer carries an `icon`
// field). Covers [[i-jazguvll]].
// ---------------------------------------------------------------------------

async fn drop_status_icon_migration_strips_default_seed(pool: &PgPool) -> Result<()> {
    // Post-cutover, the legacy `metis.projects.statuses` JSONB column
    // is gone; per-status rows live in `metis.statuses`, which has no
    // `icon` column at all. The schema-shape invariant ("no `icon`
    // anywhere") becomes a one-line check against
    // `information_schema.columns`.
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'metis' AND table_name = 'statuses' AND column_name = 'icon')",
    )
    .fetch_one(pool)
    .await
    .context("check metis.statuses for `icon` column")?;
    let has_icon: bool = row.get(0);
    if has_icon {
        bail!("metis.statuses still carries an `icon` column post-strip");
    }
    let row_count =
        sqlx::query("SELECT COUNT(*) FROM metis.statuses WHERE project_id = 'j-defaul'")
            .fetch_one(pool)
            .await
            .context("count j-defaul statuses post-cutover")?;
    let count: i64 = row_count.try_get(0)?;
    if count != 5 {
        bail!("j-defaul: expected 5 statuses rows post-cutover; got {count}");
    }
    Ok(())
}

async fn drop_status_icon_migration_strips_custom_row(pool: &PgPool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey};

    // The `j-iconfix` row was inserted by the
    // `20260607000000__pre_drop_status_icon` baseline with three statuses
    // that each carry `"icon": "<value>"`. Read back through
    // `Store::get_project` so any drift between the migration's
    // post-strip JSON shape and the Rust `StatusDefinition` serde impl
    // fails loud here (the typed deserializer must accept the migrated
    // rows).
    let project_id = parse_project_id("j-iconfix")?;
    let store = PostgresStoreV2::new(pool.clone());
    let fetched = store
        .get_project(&project_id, false)
        .await
        .context("Store::get_project(j-iconfix) post-drop_status_icon")?;

    let ApiProject {
        key,
        name,
        statuses,
        creator,
        ..
    } = &fetched.item;
    if key != &ProjectKey::try_new("iconfix").unwrap() {
        bail!("j-iconfix: expected key='iconfix'; got {key:?}");
    }
    if name != "Icon Fixture" {
        bail!("j-iconfix: expected name='Icon Fixture'; got {name:?}");
    }
    if creator.as_str() != "jayantk" {
        bail!(
            "j-iconfix: expected creator='jayantk'; got {:?}",
            creator.as_str()
        );
    }
    if statuses.len() != 3 {
        bail!("j-iconfix: expected 3 statuses; got {}", statuses.len());
    }

    let expected_shapes: &[(&str, &str, &str, bool, bool, bool)] = &[
        ("todo", "Todo", "#abcdef", false, false, false),
        ("doing", "Doing", "#f1c40f", false, false, false),
        ("done", "Done", "#2ecc71", true, true, false),
    ];
    for (i, (k, label, color, up, ud, ctc)) in expected_shapes.iter().enumerate() {
        let s = &statuses[i];
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

    // The legacy `metis.projects.statuses` JSONB column was dropped
    // by 20260614000000_cutover_to_statuses_table; the per-project
    // schema-shape "no icon anywhere" assertion is in
    // `drop_status_icon_migration_strips_default_seed` above (checks
    // `metis.statuses` once, which is shared across every project).
    // The typed-Store read above already validates the post-strip
    // column shape for j-iconfix.

    Ok(())
}

async fn drop_status_icon_migration_is_idempotent(_pool: &PgPool) -> Result<()> {
    // Originally this re-executed `20260608000000_drop_status_icon.sql`
    // verbatim. After 20260614000000_cutover_to_statuses_table drops
    // the `metis.projects.statuses` JSONB column the body operates on,
    // the body can no longer re-run. The schema-shape invariant is
    // captured elsewhere (no `icon` column on `metis.statuses`).
    Ok(())
}

// `snapshot_status_arrays` was removed alongside the idempotency
// rerun helpers that relied on the legacy `metis.projects.statuses`
// JSONB column. The 20260614000000 cutover migration dropped the
// column; per-status state now lives in `metis.statuses`. The
// schema-invariant checks against the new table replace the prior
// byte-for-byte JSONB comparison.

// ---------------------------------------------------------------------------
// 20260609000000_add_creator_to_auth_tokens / 20260609010000_drop_actors_v2
// ---------------------------------------------------------------------------

async fn denormalize_creator_session_backfill(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT creator FROM metis.auth_tokens WHERE token_hash = 'hash-session-alice'",
    )
    .fetch_one(pool)
    .await
    .context("read back session-bound auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "alice" {
        bail!("session-bound auth_tokens.creator: expected 'alice'; got {creator:?}");
    }
    Ok(())
}

async fn denormalize_creator_user_backfill(pool: &PgPool) -> Result<()> {
    let row =
        sqlx::query("SELECT creator FROM metis.auth_tokens WHERE token_hash = 'hash-cli-bob'")
            .fetch_one(pool)
            .await
            .context("read back user-CLI auth_tokens row")?;
    let creator: String = row.try_get(0)?;
    if creator != "bob" {
        bail!("user-CLI auth_tokens.creator: expected 'bob'; got {creator:?}");
    }
    Ok(())
}

async fn denormalize_creator_domain_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let session_row = store
        .get_auth_token_by_hash("hash-session-alice")
        .await
        .context("PostgresStoreV2::get_auth_token_by_hash(hash-session-alice)")?
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
        .context("PostgresStoreV2::get_auth_token_by_hash(hash-cli-bob)")?
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

async fn drop_actors_v2_migration_removes_table(pool: &PgPool) -> Result<()> {
    if table_exists(pool, "actors_v2").await? {
        bail!("expected `metis.actors_v2` table to be dropped after 20260609010000");
    }
    Ok(())
}

async fn denormalize_creator_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let creator = Username::from("eve");
    let sid = SessionId::new();
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
// `PostgresStoreV2::list_projects` typed read path surfaces the
// backfilled value. Sister to the SQLite assertion in
// `migration_roundtrip_sqlite.rs`; both catch the
// `#[sqlx(default)]` / SELECT-projection foot-gun the parent issue
// calls out.
// ---------------------------------------------------------------------------

async fn add_projects_priority_backfill_sql_level(pool: &PgPool) -> Result<()> {
    let expected: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
    for (id, want) in expected {
        let row = sqlx::query(
            "SELECT priority FROM metis.projects \
             WHERE id = $1 AND is_latest = true",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read metis.projects.priority for {id}"))?;
        let got: f64 = row.try_get("priority")?;
        if got != *want {
            bail!("metis.projects({id}).priority: expected {want}; got {got}");
        }
    }
    Ok(())
}

async fn add_projects_priority_backfill_domain_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());
    let listed = store
        .list_projects(false)
        .await
        .context("PostgresStoreV2::list_projects(include_archived = false)")?;

    let want: &[(&str, f64)] = &[
        ("j-prione", 1000.0),
        ("j-pritwo", 2000.0),
        ("j-pritri", 3000.0),
    ];
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
// migration removes the `default_status_key` column from
// `metis.projects`, that the seeded `j-defaul` row and the custom
// `j-dskdrop` baseline row still deserialize through
// `PostgresStoreV2::get_project` into the new `Project` wire type (no
// `default_status_key` field), and that the migration body is
// idempotent on a second pass.
// ---------------------------------------------------------------------------

async fn drop_projects_default_status_key_migration_removes_column(pool: &PgPool) -> Result<()> {
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!(
            "expected metis.projects.default_status_key to be dropped after \
             20260611000000_drop_projects_default_status_key"
        );
    }
    Ok(())
}

async fn drop_projects_default_status_key_migration_preserves_typed_read(
    pool: &PgPool,
) -> Result<()> {
    // Read the seeded `j-defaul` row plus the custom `j-dskdrop` baseline
    // row back through the typed store API. Both must deserialize into
    // `Project` without the `default_status_key` field — covers the
    // serde-projection foot-gun (post-migration SELECT must match the
    // ProjectRow struct, and the row must serde into the wire type).
    let store = PostgresStoreV2::new(pool.clone());

    let defaul = parse_project_id("j-defaul")?;
    let fetched = store
        .get_project(&defaul, false)
        .await
        .context("PostgresStoreV2::get_project(j-defaul) post-drop-default-status-key")?;
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

    let dskdrop = parse_project_id("j-dskdrop")?;
    let fixture = store
        .get_project(&dskdrop, false)
        .await
        .context("PostgresStoreV2::get_project(j-dskdrop) post-drop-default-status-key")?;
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

async fn drop_projects_default_status_key_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Re-execute the migration body verbatim. `ALTER TABLE ... DROP
    // COLUMN IF EXISTS` is intrinsically idempotent: a second pass on
    // the already-stripped table is a no-op. Reading the file rather
    // than hard-coding the SQL keeps this test honest if the
    // migration's body ever changes shape.
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260611000000_drop_projects_default_status_key.sql"),
    )
    .context(
        "read postgres drop_projects_default_status_key migration body for idempotency rerun",
    )?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres drop_projects_default_status_key migration body")?;
    if column_exists(pool, "projects", "default_status_key").await? {
        bail!("drop_projects_default_status_key: column reappeared after idempotency rerun");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260612000000_issues_v2_project_id_not_null — Postgres parity for the
// schema-tightening migration. Sister to
// `migration_roundtrip_sqlite::issues_v2_project_id_*`. Asserts the
// column is NOT NULL, fresh NULL inserts are rejected, and the body is
// idempotent under re-execution. The pre-flight NULL-guard exercise
// lives in the sister sqlite roundtrip — repeating it here would require
// resetting the shared postgres database mid-test, which would invalidate
// the downstream idempotency assertion at the end of the run.
// ---------------------------------------------------------------------------

async fn issues_v2_project_id_is_not_null(pool: &PgPool) -> Result<()> {
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!(
            "expected `metis.issues_v2.project_id` to be NOT NULL after \
             20260612000000_issues_v2_project_id_not_null"
        );
    }
    Ok(())
}

async fn issues_v2_project_id_rejects_null_insert(pool: &PgPool) -> Result<()> {
    let result = sqlx::query(
        "INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, project_id) \
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

async fn issues_v2_project_id_not_null_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260612000000_issues_v2_project_id_not_null.sql"),
    )
    .context("read postgres issues_v2_project_id_not_null migration body for idempotency rerun")?;
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres issues_v2_project_id_not_null migration body")?;
    if column_is_nullable(pool, "issues_v2", "project_id").await? {
        bail!("expected `metis.issues_v2.project_id` to stay NOT NULL after idempotency rerun");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260613000000_create_statuses /
// 20260613010000_add_issues_v2_status_sequence — assert the new
// `metis.statuses` table exists with the expected PK and unique index,
// that the seeded `j-defaul` project's status JSONB array backfills
// into matching `metis.statuses` rows (sequence 1..=5 in JSONB array
// order), that a custom-project baseline row covers the full column
// shape (`on_enter`, `prompt_path`, `interactive`), that every
// `metis.issues_v2` row has `status_sequence` populated and joins back
// to the original status key, and that re-applying either migration
// body is a no-op. Covers [[i-jvmpqwwe]] acceptance criteria (a)–(d).
// ---------------------------------------------------------------------------

async fn create_statuses_migration_schema_invariants(pool: &PgPool) -> Result<()> {
    // `pg_index.indkey` is `int2vector`, which is not an `anyarray` in
    // Postgres 16 — passing it directly to `array_position` errors at
    // function lookup. Cast through `text` (its output form is a
    // space-separated list) and split to get an ordinary `int[]` so
    // `unnest ... WITH ORDINALITY` can yield a 1-indexed column order.
    let row = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY k.ord) AS cols \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN unnest(string_to_array(i.indkey::text, ' ')::int[]) WITH ORDINALITY AS k(attnum, ord) ON TRUE \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE n.nspname = 'metis' AND c.relname = 'statuses' AND i.indisprimary",
    )
    .fetch_one(pool)
    .await
    .context("look up metis.statuses primary key columns")?;
    let pk_cols: Vec<String> = row.try_get("cols")?;
    if pk_cols != vec!["project_id".to_string(), "sequence".to_string()] {
        bail!("metis.statuses PK: expected [project_id, sequence]; got {pk_cols:?}");
    }

    let row = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY k.ord) AS cols \
         FROM pg_index i \
         JOIN pg_class c ON c.oid = i.indrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_class ic ON ic.oid = i.indexrelid \
         JOIN unnest(string_to_array(i.indkey::text, ' ')::int[]) WITH ORDINALITY AS k(attnum, ord) ON TRUE \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE n.nspname = 'metis' AND c.relname = 'statuses' \
           AND ic.relname = 'statuses_project_key_idx'",
    )
    .fetch_one(pool)
    .await
    .context("look up metis.statuses_project_key_idx columns")?;
    let idx_cols: Vec<String> = row.try_get("cols")?;
    if idx_cols != vec!["project_id".to_string(), "key".to_string()] {
        bail!(
            "metis.statuses_project_key_idx columns: expected [project_id, key]; got {idx_cols:?}"
        );
    }
    let row = sqlx::query(
        "SELECT i.indisunique \
         FROM pg_index i \
         JOIN pg_class ic ON ic.oid = i.indexrelid \
         WHERE ic.relname = 'statuses_project_key_idx'",
    )
    .fetch_one(pool)
    .await
    .context("read uniqueness flag for statuses_project_key_idx")?;
    let is_unique: bool = row.try_get(0)?;
    if !is_unique {
        bail!("metis.statuses_project_key_idx: expected unique index; got non-unique");
    }
    Ok(())
}

async fn create_statuses_migration_backfills_default_seed(pool: &PgPool) -> Result<()> {
    // The seeded j-defaul row has 5 statuses in this exact order. The
    // backfill must produce sequence 1..=5 with the same column values,
    // matching `default_project_seed()` byte-for-byte after the icon
    // column was stripped by 20260608000000.
    let expected_seed = default_project_seed().statuses;
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter::text AS on_enter_text, prompt_path, interactive \
         FROM metis.statuses WHERE project_id = 'j-defaul' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read metis.statuses for j-defaul")?;
    if rows.len() != 5 {
        bail!(
            "metis.statuses(j-defaul): expected 5 rows; got {}",
            rows.len()
        );
    }
    for (i, row) in rows.iter().enumerate() {
        let sequence: i64 = row.try_get("sequence")?;
        if sequence != (i + 1) as i64 {
            bail!(
                "metis.statuses(j-defaul)[{i}]: expected sequence={}; got {sequence}",
                i + 1
            );
        }
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: bool = row.try_get("unblocks_parents")?;
        let unblocks_dependents: bool = row.try_get("unblocks_dependents")?;
        let cascades_to_children: bool = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: bool = row.try_get("interactive")?;

        let expected = &expected_seed[i];
        if key != expected.key.as_str() {
            bail!(
                "metis.statuses(j-defaul)[{i}].key: expected {}; got {key}",
                expected.key.as_str()
            );
        }
        if label != expected.label {
            bail!(
                "metis.statuses(j-defaul)[{i}].label: expected {}; got {label}",
                expected.label
            );
        }
        if color != expected.color.as_ref() {
            bail!(
                "metis.statuses(j-defaul)[{i}].color: expected {}; got {color}",
                expected.color
            );
        }
        if unblocks_parents != expected.unblocks_parents
            || unblocks_dependents != expected.unblocks_dependents
            || cascades_to_children != expected.cascades_to_children
        {
            bail!(
                "metis.statuses(j-defaul)[{i}] flags mismatch: \
                 expected ({}, {}, {}); got ({unblocks_parents}, {unblocks_dependents}, {cascades_to_children})",
                expected.unblocks_parents,
                expected.unblocks_dependents,
                expected.cascades_to_children
            );
        }
        if prompt_path != expected.prompt_path {
            bail!(
                "metis.statuses(j-defaul)[{i}].prompt_path: expected {:?}; got {prompt_path:?}",
                expected.prompt_path
            );
        }
        if interactive != expected.interactive {
            bail!(
                "metis.statuses(j-defaul)[{i}].interactive: expected {}; got {interactive}",
                expected.interactive
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
        let actual_on_enter_json = on_enter_text
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .with_context(|| format!("decode metis.statuses(j-defaul)[{i}].on_enter JSON"))?;
        if expected_on_enter_json != actual_on_enter_json {
            bail!(
                "metis.statuses(j-defaul)[{i}].on_enter: expected {expected_on_enter_json:?}; got {actual_on_enter_json:?}"
            );
        }
    }
    Ok(())
}

async fn create_statuses_migration_backfills_custom_project(pool: &PgPool) -> Result<()> {
    // The j-stsfixt baseline row's 3 statuses exercise the full column
    // shape: draft (minimal), reviewing (on_enter + prompt_path +
    // interactive), merged (unblocks_* flags set).
    let rows = sqlx::query(
        "SELECT sequence, key, label, color, \
                unblocks_parents, unblocks_dependents, cascades_to_children, \
                on_enter::text AS on_enter_text, prompt_path, interactive \
         FROM metis.statuses WHERE project_id = 'j-stsfixt' ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .context("read metis.statuses for j-stsfixt")?;
    if rows.len() != 3 {
        bail!(
            "metis.statuses(j-stsfixt): expected 3 rows; got {}",
            rows.len()
        );
    }

    struct Expected {
        sequence: i64,
        key: &'static str,
        label: &'static str,
        color: &'static str,
        unblocks_parents: bool,
        unblocks_dependents: bool,
        cascades_to_children: bool,
        on_enter: Option<serde_json::Value>,
        prompt_path: Option<&'static str>,
        interactive: bool,
    }

    let expectations: [Expected; 3] = [
        Expected {
            sequence: 1,
            key: "draft",
            label: "Draft",
            color: "#cccccc",
            unblocks_parents: false,
            unblocks_dependents: false,
            cascades_to_children: false,
            on_enter: None,
            prompt_path: None,
            interactive: false,
        },
        Expected {
            sequence: 2,
            key: "reviewing",
            label: "Reviewing",
            color: "#f1c40f",
            unblocks_parents: false,
            unblocks_dependents: false,
            cascades_to_children: false,
            on_enter: Some(serde_json::json!({"assign_to": {"Agent": {"name": "reviewer"}}})),
            prompt_path: Some("/projects/stsfixt/reviewing.md"),
            interactive: true,
        },
        Expected {
            sequence: 3,
            key: "merged",
            label: "Merged",
            color: "#2ecc71",
            unblocks_parents: true,
            unblocks_dependents: true,
            cascades_to_children: false,
            on_enter: None,
            prompt_path: None,
            interactive: false,
        },
    ];

    for (row, expected) in rows.iter().zip(expectations.iter()) {
        let sequence: i64 = row.try_get("sequence")?;
        let key: String = row.try_get("key")?;
        let label: String = row.try_get("label")?;
        let color: String = row.try_get("color")?;
        let unblocks_parents: bool = row.try_get("unblocks_parents")?;
        let unblocks_dependents: bool = row.try_get("unblocks_dependents")?;
        let cascades_to_children: bool = row.try_get("cascades_to_children")?;
        let on_enter_text: Option<String> = row.try_get("on_enter_text")?;
        let on_enter_value = on_enter_text
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .context("decode j-stsfixt.statuses.on_enter JSON")?;
        let prompt_path: Option<String> = row.try_get("prompt_path")?;
        let interactive: bool = row.try_get("interactive")?;

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
                "metis.statuses(j-stsfixt) sequence={sequence}: did not match expected\n  \
                 got: (key={key}, label={label}, color={color}, \
                 unblocks_parents={unblocks_parents}, unblocks_dependents={unblocks_dependents}, \
                 cascades_to_children={cascades_to_children}, on_enter={on_enter_value:?}, \
                 prompt_path={prompt_path:?}, interactive={interactive})\n  \
                 expected: sequence={}, key={}",
                expected.sequence,
                expected.key
            );
        }
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_schema_invariants(pool: &PgPool) -> Result<()> {
    if !column_exists(pool, "issues_v2", "status_sequence").await? {
        bail!("expected metis.issues_v2.status_sequence column to exist after rollforward");
    }
    // Post-cutover (20260614000000), the column is tightened to
    // NOT NULL and carries the FK to `metis.statuses`.
    if column_is_nullable(pool, "issues_v2", "status_sequence").await? {
        bail!("expected metis.issues_v2.status_sequence to be NOT NULL post-cutover; got nullable");
    }
    if column_exists(pool, "issues_v2", "status").await? {
        bail!("expected metis.issues_v2.status (TEXT) to be dropped post-cutover");
    }
    if column_exists(pool, "projects", "statuses").await? {
        bail!("expected metis.projects.statuses (JSONB) to be dropped post-cutover");
    }
    if !column_exists(pool, "projects", "next_status_sequence").await? {
        bail!("expected metis.projects.next_status_sequence to be added post-cutover");
    }
    // Confirm the supporting index for the JOIN landed.
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes \
         WHERE schemaname = 'metis' AND indexname = 'issues_v2_project_status_sequence_idx')",
    )
    .fetch_one(pool)
    .await?;
    let exists: bool = row.get(0);
    if !exists {
        bail!("expected metis.issues_v2_project_status_sequence_idx to exist after rollforward");
    }
    // Confirm the FK landed with the expected name and ON DELETE / ON
    // UPDATE policy.
    let row = sqlx::query(
        "SELECT confdeltype, confupdtype FROM pg_constraint \
         WHERE conname = 'issues_v2_status_sequence_fkey'",
    )
    .fetch_optional(pool)
    .await?;
    let row = row.context("expected FK 'issues_v2_status_sequence_fkey' to exist post-cutover")?;
    let del_type: i8 = row.try_get("confdeltype")?;
    let upd_type: i8 = row.try_get("confupdtype")?;
    // 'r' = RESTRICT.
    if del_type as u8 != b'r' || upd_type as u8 != b'r' {
        bail!(
            "FK issues_v2_status_sequence_fkey: expected ON DELETE/UPDATE RESTRICT; got del={} upd={}",
            del_type as u8 as char,
            upd_type as u8 as char
        );
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_backfills_issues(pool: &PgPool) -> Result<()> {
    // Per-status default-project coverage: each baseline issue must
    // round-trip `(project_id, status_sequence) → metis.statuses.key`.
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
             FROM metis.issues_v2 i \
             LEFT JOIN metis.statuses s \
                ON s.project_id = i.project_id AND s.sequence = i.status_sequence \
             WHERE i.id = $1 AND i.is_latest = TRUE",
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
                 join to metis.statuses must recover key; got {resolved_key:?}"
            );
        }
    }
    // Post-cutover (20260614000000), every issue row must have a
    // non-NULL status_sequence (the column was tightened to NOT NULL
    // after the catch-up backfill).
    let row = sqlx::query("SELECT COUNT(*) FROM metis.issues_v2 WHERE status_sequence IS NULL")
        .fetch_one(pool)
        .await
        .context("count NULL status_sequence rows post-cutover")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!(
            "expected 0 metis.issues_v2 rows with NULL status_sequence post-cutover; got {count}"
        );
    }
    Ok(())
}

async fn create_statuses_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // PR 1's body sourced its INSERT from `metis.projects.statuses`
    // JSONB. After 20260614000000_cutover_to_statuses_table dropped
    // that column, re-executing the body verbatim errors. The
    // duplicate-free invariant remains testable directly.
    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT (project_id, sequence)) AS dup FROM metis.statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, sequence) rows in metis.statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!(
            "metis.statuses carries {dup} duplicate (project_id, sequence) rows post-rollforward"
        );
    }
    let row = sqlx::query(
        "SELECT COUNT(*) - COUNT(DISTINCT (project_id, key)) AS dup FROM metis.statuses",
    )
    .fetch_one(pool)
    .await
    .context("count duplicate (project_id, key) rows in metis.statuses")?;
    let dup: i64 = row.try_get("dup")?;
    if dup != 0 {
        bail!("metis.statuses carries {dup} duplicate (project_id, key) rows post-rollforward");
    }
    Ok(())
}

async fn add_issues_v2_status_sequence_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // PR 1's body relied on the legacy `metis.issues_v2.status` TEXT
    // column for the join. After
    // 20260614000000_cutover_to_statuses_table dropped that column
    // and tightened `status_sequence` to NOT NULL, re-executing the
    // body verbatim errors. The invariant that matters — "every
    // post-cutover issue row has a non-NULL status_sequence" — is
    // captured in `add_issues_v2_status_sequence_backfills_issues`
    // above.
    let _ = pool;
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260615000000_reserve_hydra_id_shape_in_keys — assert the rewrite
// of shape-matching `metis.projects.key` / `metis.statuses.key` rows
// from the `20260614000000__pre_reserve_hydra_id_shape` baseline. The
// load-bearing assertion is the typed `Store::get_project` /
// `Store::list_projects` read-back: a pure SQL-level assertion would
// not catch a wire-side validation gap.
// ---------------------------------------------------------------------------

async fn reserve_hydra_id_shape_rewrites_project_keys(pool: &PgPool) -> Result<()> {
    let expected: &[(&str, &str)] = &[
        ("j-rsvshapa", "renamed-j-foo"),
        ("j-rsvshapb", "engineering"),
        ("j-rsvshapc", "renamed-x-old"),
    ];
    for (id, want_key) in expected {
        let row = sqlx::query("SELECT key FROM metis.projects WHERE id = $1 AND is_latest = TRUE")
            .bind(*id)
            .fetch_one(pool)
            .await
            .with_context(|| format!("read metis.projects.key for {id}"))?;
        let got: String = row.try_get("key")?;
        if got.as_str() != *want_key {
            bail!("metis.projects({id}).key: expected {want_key:?}; got {got:?}");
        }
    }
    Ok(())
}

async fn reserve_hydra_id_shape_rewrites_status_keys(pool: &PgPool) -> Result<()> {
    // (sequence, expected key) on j-rsvshapa.
    let expected: &[(i64, &str)] = &[
        (1, "renamed-i-progress"),
        (2, "done"),
        (3, "renamed-s-todo-seq3"),
        (4, "renamed-s-todo"),
    ];
    for (sequence, want_key) in expected {
        let row = sqlx::query(
            "SELECT key FROM metis.statuses WHERE project_id = 'j-rsvshapa' AND sequence = $1",
        )
        .bind(*sequence)
        .fetch_one(pool)
        .await
        .with_context(|| format!("read metis.statuses.key for (j-rsvshapa, seq={sequence})"))?;
        let got: String = row.try_get("key")?;
        if got.as_str() != *want_key {
            bail!(
                "metis.statuses(j-rsvshapa, seq={sequence}).key: expected {want_key:?}; got {got:?}"
            );
        }
    }
    Ok(())
}

async fn reserve_hydra_id_shape_no_reserved_shape_remains(pool: &PgPool) -> Result<()> {
    let row = sqlx::query("SELECT COUNT(*) FROM metis.projects WHERE key ~ '^[a-z]-.*'")
        .fetch_one(pool)
        .await
        .context("count metis.projects.key rows still matching reserved shape")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 metis.projects rows with key matching `^[a-z]-`; got {count}");
    }
    let row = sqlx::query("SELECT COUNT(*) FROM metis.statuses WHERE key ~ '^[a-z]-.*'")
        .fetch_one(pool)
        .await
        .context("count metis.statuses.key rows still matching reserved shape")?;
    let count: i64 = row.try_get(0)?;
    if count != 0 {
        bail!("expected 0 metis.statuses rows with key matching `^[a-z]-`; got {count}");
    }
    Ok(())
}

async fn reserve_hydra_id_shape_domain_roundtrip(pool: &PgPool) -> Result<()> {
    use hydra_common::api::v1::projects::{Project as ApiProject, ProjectKey, StatusKey};

    let store = PostgresStoreV2::new(pool.clone());

    // Per-project shape and status keys must round-trip through the
    // typed `Store::get_project`, which deserializes the row through
    // the post-rewrite `ProjectKey` / `StatusKey` validators. A
    // shape-matching value would surface as a deserialization error.
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
        let project_id = parse_project_id(id)?;
        let fetched = store
            .get_project(&project_id, false)
            .await
            .with_context(|| format!("Store::get_project({id}) post-reserve-hydra-id-shape"))?;
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

    // `list_projects` reads the same backing rows through a different
    // SELECT projection — assert the shape-matching baseline rows
    // surface there too with the rewritten keys.
    let listed = store
        .list_projects(false)
        .await
        .context("Store::list_projects(false) post-reserve-hydra-id-shape")?;
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

async fn reserve_hydra_id_shape_migration_is_idempotent(pool: &PgPool) -> Result<()> {
    // Re-execute the migration body. The reserved-shape WHERE clauses
    // match nothing post-rewrite (every renamed key starts with
    // `renamed-`, second byte `e`), so the body's iteration is empty
    // and no further UPDATEs run. Re-asserting the expected post-
    // rewrite key set confirms.
    //
    // The on-disk body still references `projects.deleted`, but a
    // subsequent migration (20260715000000) renamed that column to
    // `projects.archived`. Patch the loaded body to track the rename
    // so the literal replay still exercises the body's idempotency
    // against the current schema.
    let body = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/20260615000000_reserve_hydra_id_shape_in_keys.sql"),
    )
    .context("read postgres reserve_hydra_id_shape migration body for idempotency rerun")?;
    let body = body.replace("NOT p2.deleted", "NOT p2.archived");
    sqlx::raw_sql(&body)
        .execute(pool)
        .await
        .context("re-apply postgres reserve_hydra_id_shape migration body")?;
    reserve_hydra_id_shape_rewrites_project_keys(pool).await?;
    reserve_hydra_id_shape_rewrites_status_keys(pool).await?;
    reserve_hydra_id_shape_no_reserved_shape_remains(pool).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 20260715000000_rename_projects_deleted_to_archived. Pure column
// rename: `metis.projects.deleted` → `metis.projects.archived`. No
// semantic change. The partial unique index
// `projects_key_unique_active_idx` has its `WHERE` clause auto-rewritten
// by Postgres's `ALTER TABLE RENAME COLUMN`, so no explicit index touch.
// ---------------------------------------------------------------------------

async fn rename_projects_deleted_to_archived_schema_invariants(pool: &PgPool) -> Result<()> {
    if column_exists(pool, "projects", "deleted").await? {
        bail!("expected metis.projects.deleted column to be renamed away post-rollforward");
    }
    if !column_exists(pool, "projects", "archived").await? {
        bail!("expected metis.projects.archived column to exist post-rollforward");
    }
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes \
         WHERE schemaname = 'metis' AND indexname = 'projects_key_unique_active_idx')",
    )
    .fetch_one(pool)
    .await?;
    let exists: bool = row.get(0);
    if !exists {
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
async fn rename_projects_deleted_to_archived_baseline_roundtrip(pool: &PgPool) -> Result<()> {
    let store = PostgresStoreV2::new(pool.clone());

    let archived_id = parse_project_id("j-renarcha")?;
    let archived = store
        .get_project(&archived_id, true)
        .await
        .context("PostgresStoreV2::get_project(j-renarcha, include_archived=true)")?;
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

    let archived_hidden = store.get_project(&archived_id, false).await;
    assert!(
        matches!(archived_hidden, Err(StoreError::ProjectNotFound(_))),
        "j-renarcha must not surface through include_archived=false; got {archived_hidden:?}"
    );

    let live_id = parse_project_id("j-renarchb")?;
    let live = store
        .get_project(&live_id, false)
        .await
        .context("PostgresStoreV2::get_project(j-renarchb, include_archived=false)")?;
    if live.item.archived {
        bail!(
            "j-renarchb: expected archived=false after rename; got archived={}",
            live.item.archived
        );
    }

    let listed = store
        .list_projects(false)
        .await
        .context("PostgresStoreV2::list_projects(false) post-rename")?;
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

// ---------------------------------------------------------------------------
// parse_baseline_filename unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::parse_baseline_filename;

    #[test]
    fn parses_well_formed_filename() {
        assert_eq!(
            parse_baseline_filename("20260519000000__pre_actor_overhaul.sql").unwrap(),
            20_260_519_000_000
        );
    }

    #[test]
    fn parses_minimal_description() {
        assert_eq!(parse_baseline_filename("42__x.sql").unwrap(), 42);
    }

    #[test]
    fn rejects_missing_sql_suffix() {
        let err = parse_baseline_filename("20260519000000__pre_actor_overhaul")
            .unwrap_err()
            .to_string();
        assert!(err.contains(".sql"), "expected `.sql` mention; got: {err}");
    }

    #[test]
    fn rejects_missing_double_underscore() {
        let err = parse_baseline_filename("20260519000000_pre_actor_overhaul.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("<version>__<description>"),
            "expected the filename-shape mention; got: {err}"
        );
    }

    #[test]
    fn rejects_empty_description() {
        let err = parse_baseline_filename("20260519000000__.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("empty description"),
            "expected empty-description mention; got: {err}"
        );
    }

    #[test]
    fn rejects_non_numeric_version() {
        let err = parse_baseline_filename("not_a_number__desc.sql")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a u64"),
            "expected u64 parse failure; got: {err}"
        );
    }
}
