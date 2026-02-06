//! Migration module for transferring data from v1 JSONB tables to v2 column-based tables.
//!
//! This module provides a one-off migration function that reads all versions of all objects
//! from the v1 tables and writes them to the v2 tables, preserving complete history.

use crate::store::postgres::PgStorePool;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{info, warn};

/// Batch size for migration operations to avoid memory issues with large datasets.
const BATCH_SIZE: i64 = 1000;

/// Migration status values.
const STATUS_IN_PROGRESS: &str = "in_progress";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";

/// Table names for v1 (JSONB) tables.
const V1_TABLE_ISSUES: &str = "metis.issues";
const V1_TABLE_PATCHES: &str = "metis.patches";
const V1_TABLE_TASKS: &str = "metis.tasks";
const V1_TABLE_USERS: &str = "metis.users";
const V1_TABLE_ACTORS: &str = "metis.actors";
const V1_TABLE_REPOSITORIES: &str = "metis.repositories";
const V1_TABLE_DOCUMENTS: &str = "metis.documents";

/// Table names for v2 (column-based) tables.
const V2_TABLE_ISSUES: &str = "metis.issues_v2";
const V2_TABLE_PATCHES: &str = "metis.patches_v2";
const V2_TABLE_TASKS: &str = "metis.tasks_v2";
const V2_TABLE_USERS: &str = "metis.users_v2";
const V2_TABLE_ACTORS: &str = "metis.actors_v2";
const V2_TABLE_REPOSITORIES: &str = "metis.repositories_v2";
const V2_TABLE_DOCUMENTS: &str = "metis.documents_v2";

/// Migration status table.
const MIGRATION_STATUS_TABLE: &str = "metis.migration_status";

/// Result of a migration operation.
#[derive(Debug)]
pub struct MigrationResult {
    pub issues_migrated: u64,
    pub patches_migrated: u64,
    pub tasks_migrated: u64,
    pub users_migrated: u64,
    pub actors_migrated: u64,
    pub repositories_migrated: u64,
    pub documents_migrated: u64,
}

impl MigrationResult {
    fn new() -> Self {
        Self {
            issues_migrated: 0,
            patches_migrated: 0,
            tasks_migrated: 0,
            users_migrated: 0,
            actors_migrated: 0,
            repositories_migrated: 0,
            documents_migrated: 0,
        }
    }

    /// Total number of records migrated across all tables.
    pub fn total(&self) -> u64 {
        self.issues_migrated
            + self.patches_migrated
            + self.tasks_migrated
            + self.users_migrated
            + self.actors_migrated
            + self.repositories_migrated
            + self.documents_migrated
    }
}

/// Check if a migration has already been completed.
async fn is_migration_completed(pool: &PgStorePool, migration_id: &str) -> Result<bool> {
    let result = sqlx::query_scalar::<_, String>(&format!(
        "SELECT status FROM {MIGRATION_STATUS_TABLE} WHERE id = $1"
    ))
    .bind(migration_id)
    .fetch_optional(pool)
    .await
    .context("failed to check migration status")?;

    Ok(result.as_deref() == Some(STATUS_COMPLETED))
}

/// Start a migration by recording it in the status table.
/// Returns false if the migration is already in progress (concurrent migration prevention).
async fn start_migration(pool: &PgStorePool, migration_id: &str) -> Result<bool> {
    // Use INSERT ... ON CONFLICT to handle concurrent migration attempts.
    // If another process already started the migration, we'll see a conflict.
    let result = sqlx::query(&format!(
        "INSERT INTO {MIGRATION_STATUS_TABLE} (id, status, started_at, migrated_count)
         VALUES ($1, $2, NOW(), 0)
         ON CONFLICT (id) DO NOTHING"
    ))
    .bind(migration_id)
    .bind(STATUS_IN_PROGRESS)
    .execute(pool)
    .await
    .context("failed to start migration")?;

    Ok(result.rows_affected() > 0)
}

/// Mark a migration as completed.
async fn complete_migration(pool: &PgStorePool, migration_id: &str, count: u64) -> Result<()> {
    sqlx::query(&format!(
        "UPDATE {MIGRATION_STATUS_TABLE}
         SET status = $1, completed_at = NOW(), migrated_count = $2
         WHERE id = $3"
    ))
    .bind(STATUS_COMPLETED)
    .bind(count as i64)
    .bind(migration_id)
    .execute(pool)
    .await
    .context("failed to complete migration")?;

    Ok(())
}

/// Mark a migration as failed.
async fn fail_migration(pool: &PgStorePool, migration_id: &str) -> Result<()> {
    sqlx::query(&format!(
        "UPDATE {MIGRATION_STATUS_TABLE}
         SET status = $1
         WHERE id = $2"
    ))
    .bind(STATUS_FAILED)
    .bind(migration_id)
    .execute(pool)
    .await
    .context("failed to mark migration as failed")?;

    Ok(())
}

/// Migrate all data from v1 tables to v2 tables.
///
/// This function is idempotent - it checks migration status before running
/// and skips tables that have already been migrated. It also handles
/// concurrent migration attempts by using database-level locking.
///
/// # Arguments
/// * `pool` - Database connection pool
///
/// # Returns
/// A `MigrationResult` containing counts of migrated records, or an error.
pub async fn migrate_v1_to_v2(pool: &PgStorePool) -> Result<MigrationResult> {
    let mut result = MigrationResult::new();

    info!("starting v1 to v2 migration");

    // Migrate each table type
    result.issues_migrated = migrate_issues(pool).await?;
    result.patches_migrated = migrate_patches(pool).await?;
    result.tasks_migrated = migrate_tasks(pool).await?;
    result.users_migrated = migrate_users(pool).await?;
    result.actors_migrated = migrate_actors(pool).await?;
    result.repositories_migrated = migrate_repositories(pool).await?;
    result.documents_migrated = migrate_documents(pool).await?;

    info!(
        total = result.total(),
        issues = result.issues_migrated,
        patches = result.patches_migrated,
        tasks = result.tasks_migrated,
        users = result.users_migrated,
        actors = result.actors_migrated,
        repositories = result.repositories_migrated,
        documents = result.documents_migrated,
        "v1 to v2 migration complete"
    );

    Ok(result)
}

/// Row structure for v1 payloads.
#[derive(sqlx::FromRow)]
struct V1Row {
    id: String,
    version_number: i64,
    payload: Value,
    created_at: DateTime<Utc>,
}

/// Migrate issues from v1 to v2.
async fn migrate_issues(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_issues";

    if is_migration_completed(pool, migration_id).await? {
        info!("issues migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("issues migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_issues_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "issues migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "issues migration failed");
        }
    }

    result
}

async fn migrate_issues_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_ISSUES}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch issues from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            // Extract fields from payload
            let issue_type = row
                .payload
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("task");
            let description = row
                .payload
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let creator = row
                .payload
                .get("creator")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let progress = row
                .payload
                .get("progress")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let status = row
                .payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("open");
            let assignee = row.payload.get("assignee").and_then(|v| v.as_str());
            let job_settings = row
                .payload
                .get("job_settings")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let todo_list = row
                .payload
                .get("todo_list")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            let dependencies = row
                .payload
                .get("dependencies")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            let patches = row
                .payload
                .get("patches")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            let deleted = row
                .payload
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Insert into v2 table, using ON CONFLICT to skip existing records
            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_ISSUES}
                 (id, version_number, issue_type, description, creator, progress, status, assignee,
                  job_settings, todo_list, dependencies, patches, deleted, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(issue_type)
            .bind(description)
            .bind(creator)
            .bind(progress)
            .bind(status)
            .bind(assignee)
            .bind(&job_settings)
            .bind(&todo_list)
            .bind(&dependencies)
            .bind(&patches)
            .bind(deleted)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| {
                format!("failed to insert issue {} v{}", row.id, row.version_number)
            })?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate patches from v1 to v2.
async fn migrate_patches(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_patches";

    if is_migration_completed(pool, migration_id).await? {
        info!("patches migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("patches migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_patches_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "patches migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "patches migration failed");
        }
    }

    result
}

async fn migrate_patches_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_PATCHES}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch patches from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let title = row
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let description = row
                .payload
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let diff = row
                .payload
                .get("diff")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let status = row
                .payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("open");
            let is_automatic_backup = row
                .payload
                .get("is_automatic_backup")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let created_by = row.payload.get("created_by").and_then(|v| v.as_str());
            let reviews = row
                .payload
                .get("reviews")
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            let service_repo_name = row
                .payload
                .get("service_repo_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let github = row.payload.get("github").cloned();
            let deleted = row
                .payload
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_PATCHES}
                 (id, version_number, title, description, diff, status, is_automatic_backup,
                  created_by, reviews, service_repo_name, github, deleted, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(title)
            .bind(description)
            .bind(diff)
            .bind(status)
            .bind(is_automatic_backup)
            .bind(created_by)
            .bind(&reviews)
            .bind(service_repo_name)
            .bind(&github)
            .bind(deleted)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| {
                format!("failed to insert patch {} v{}", row.id, row.version_number)
            })?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate tasks from v1 to v2.
async fn migrate_tasks(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_tasks";

    if is_migration_completed(pool, migration_id).await? {
        info!("tasks migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("tasks migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_tasks_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "tasks migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "tasks migration failed");
        }
    }

    result
}

async fn migrate_tasks_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_TASKS}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch tasks from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let prompt = row
                .payload
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let context = row.payload.get("context").cloned().unwrap_or(Value::Null);
            let spawned_from = row.payload.get("spawned_from").and_then(|v| v.as_str());
            let image = row.payload.get("image").and_then(|v| v.as_str());
            let model = row.payload.get("model").and_then(|v| v.as_str());
            let env_vars = row
                .payload
                .get("env_vars")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            let cpu_limit = row.payload.get("cpu_limit").and_then(|v| v.as_str());
            let memory_limit = row.payload.get("memory_limit").and_then(|v| v.as_str());
            let status = row
                .payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("complete");
            let last_message = row.payload.get("last_message").and_then(|v| v.as_str());
            let error = row.payload.get("error").cloned();
            let deleted = row
                .payload
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_TASKS}
                 (id, version_number, prompt, context, spawned_from, image, model, env_vars,
                  cpu_limit, memory_limit, status, last_message, error, deleted, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(prompt)
            .bind(&context)
            .bind(spawned_from)
            .bind(image)
            .bind(model)
            .bind(&env_vars)
            .bind(cpu_limit)
            .bind(memory_limit)
            .bind(status)
            .bind(last_message)
            .bind(&error)
            .bind(deleted)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| format!("failed to insert task {} v{}", row.id, row.version_number))?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate users from v1 to v2.
async fn migrate_users(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_users";

    if is_migration_completed(pool, migration_id).await? {
        info!("users migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("users migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_users_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "users migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "users migration failed");
        }
    }

    result
}

async fn migrate_users_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_USERS}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch users from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let username = row
                .payload
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let github_user_id = row
                .payload
                .get("github_user_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let github_token = row
                .payload
                .get("github_token")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let github_refresh_token = row
                .payload
                .get("github_refresh_token")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_USERS}
                 (id, version_number, username, github_user_id, github_token, github_refresh_token, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(username)
            .bind(github_user_id)
            .bind(github_token)
            .bind(github_refresh_token)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| format!("failed to insert user {} v{}", row.id, row.version_number))?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate actors from v1 to v2.
async fn migrate_actors(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_actors";

    if is_migration_completed(pool, migration_id).await? {
        info!("actors migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("actors migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_actors_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "actors migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "actors migration failed");
        }
    }

    result
}

async fn migrate_actors_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_ACTORS}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch actors from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let auth_token_hash = row
                .payload
                .get("auth_token_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let auth_token_salt = row
                .payload
                .get("auth_token_salt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let user_or_worker = row
                .payload
                .get("user_or_worker")
                .cloned()
                .unwrap_or(Value::Null);

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_ACTORS}
                 (id, version_number, auth_token_hash, auth_token_salt, user_or_worker, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(auth_token_hash)
            .bind(auth_token_salt)
            .bind(&user_or_worker)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| {
                format!("failed to insert actor {} v{}", row.id, row.version_number)
            })?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate repositories from v1 to v2.
async fn migrate_repositories(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_repositories";

    if is_migration_completed(pool, migration_id).await? {
        info!("repositories migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("repositories migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_repositories_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "repositories migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "repositories migration failed");
        }
    }

    result
}

async fn migrate_repositories_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_REPOSITORIES}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch repositories from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let remote_url = row
                .payload
                .get("remote_url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let default_branch = row.payload.get("default_branch").and_then(|v| v.as_str());
            let default_image = row.payload.get("default_image").and_then(|v| v.as_str());

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_REPOSITORIES}
                 (id, version_number, remote_url, default_branch, default_image, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(remote_url)
            .bind(default_branch)
            .bind(default_image)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| {
                format!(
                    "failed to insert repository {} v{}",
                    row.id, row.version_number
                )
            })?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

/// Migrate documents from v1 to v2.
async fn migrate_documents(pool: &PgStorePool) -> Result<u64> {
    let migration_id = "v1_to_v2_documents";

    if is_migration_completed(pool, migration_id).await? {
        info!("documents migration already completed, skipping");
        return Ok(0);
    }

    if !start_migration(pool, migration_id).await? {
        warn!("documents migration already in progress by another process, skipping");
        return Ok(0);
    }

    let result = migrate_documents_internal(pool).await;

    match &result {
        Ok(count) => {
            complete_migration(pool, migration_id, *count).await?;
            info!(count = *count, "documents migration completed");
        }
        Err(e) => {
            fail_migration(pool, migration_id).await?;
            warn!(error = %e, "documents migration failed");
        }
    }

    result
}

async fn migrate_documents_internal(pool: &PgStorePool) -> Result<u64> {
    let mut offset = 0i64;
    let mut total_migrated = 0u64;

    loop {
        let rows = sqlx::query_as::<_, V1Row>(&format!(
            "SELECT id, version_number, payload, created_at
             FROM {V1_TABLE_DOCUMENTS}
             ORDER BY id, version_number
             LIMIT $1 OFFSET $2"
        ))
        .bind(BATCH_SIZE)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("failed to fetch documents from v1")?;

        if rows.is_empty() {
            break;
        }

        let batch_size = rows.len();

        for row in rows {
            let title = row
                .payload
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body_markdown = row
                .payload
                .get("body_markdown")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = row.payload.get("path").and_then(|v| v.as_str());
            let created_by = row.payload.get("created_by").and_then(|v| v.as_str());
            let deleted = row
                .payload
                .get("deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            sqlx::query(&format!(
                "INSERT INTO {V2_TABLE_DOCUMENTS}
                 (id, version_number, title, body_markdown, path, created_by, deleted, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (id, version_number) DO NOTHING"
            ))
            .bind(&row.id)
            .bind(row.version_number)
            .bind(title)
            .bind(body_markdown)
            .bind(path)
            .bind(created_by)
            .bind(deleted)
            .bind(row.created_at)
            .execute(pool)
            .await
            .with_context(|| {
                format!(
                    "failed to insert document {} v{}",
                    row.id, row.version_number
                )
            })?;

            total_migrated += 1;
        }

        offset += batch_size as i64;
    }

    Ok(total_migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::store::Store;
    use crate::store::postgres::{PgStorePool, PostgresStore};
    use crate::store::postgres_v2::PostgresStoreV2;
    use metis_common::api::v1::issues::SearchIssuesQuery;

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migration_is_idempotent(pool: PgStorePool) {
        // First run should complete normally
        let result1 = migrate_v1_to_v2(&pool).await.unwrap();

        // Second run should skip (all tables already migrated)
        let result2 = migrate_v1_to_v2(&pool).await.unwrap();

        // Second run should have 0 migrations since everything was already migrated
        assert_eq!(result2.total(), 0);

        // Both should succeed (first run results are available)
        let _ = result1;
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migration_handles_empty_database(pool: PgStorePool) {
        // Empty database should complete without errors
        let result = migrate_v1_to_v2(&pool).await.unwrap();
        assert_eq!(result.total(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migration_transfers_data_from_v1_to_v2(pool: PgStorePool) {
        // Create data in v1 store
        let v1_store = PostgresStore::new(pool.clone());
        let issue = Issue {
            issue_type: IssueType::Task,
            description: "Test issue for migration".to_string(),
            creator: Username::from("test-user"),
            progress: "In progress".to_string(),
            status: IssueStatus::Open,
            assignee: Some("assignee".to_string()),
            job_settings: Default::default(),
            todo_list: vec![],
            dependencies: vec![],
            patches: vec![],
            deleted: false,
        };
        let issue_id = v1_store.add_issue(issue.clone()).await.unwrap();

        // Run migration
        let result = migrate_v1_to_v2(&pool).await.unwrap();
        assert_eq!(result.issues_migrated, 1);
        assert!(result.total() >= 1);

        // Verify data is readable from v2 store
        let v2_store = PostgresStoreV2::new(pool.clone());
        let migrated_issue = v2_store.get_issue(&issue_id).await.unwrap();
        assert_eq!(migrated_issue.item.description, issue.description);
        assert_eq!(migrated_issue.item.creator, issue.creator);
        assert_eq!(migrated_issue.item.progress, issue.progress);
        assert_eq!(migrated_issue.item.status, issue.status);
        assert_eq!(migrated_issue.item.assignee, issue.assignee);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn v2_store_works_after_migration(pool: PgStorePool) {
        // Run migration on empty database
        let _ = migrate_v1_to_v2(&pool).await.unwrap();

        // Create new data directly in v2 store
        let v2_store = PostgresStoreV2::new(pool.clone());
        let issue = Issue {
            issue_type: IssueType::Task,
            description: "New issue created in v2".to_string(),
            creator: Username::from("v2-user"),
            progress: "".to_string(),
            status: IssueStatus::Open,
            assignee: None,
            job_settings: Default::default(),
            todo_list: vec![],
            dependencies: vec![],
            patches: vec![],
            deleted: false,
        };
        let issue_id = v2_store.add_issue(issue.clone()).await.unwrap();

        // Verify data can be read back
        let retrieved = v2_store.get_issue(&issue_id).await.unwrap();
        assert_eq!(retrieved.item.description, issue.description);
        assert_eq!(retrieved.item.creator, issue.creator);

        // Verify listing works
        let issues = v2_store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, issue_id);
    }
}
