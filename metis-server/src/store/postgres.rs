use crate::{
    config::DatabaseSection,
    domain::{
        actors::Actor,
        documents::Document,
        issues::{Issue, IssueDependency, IssueDependencyType, IssueGraphFilter},
        patches::Patch,
        users::{User, Username},
    },
    store::{Status, Store, StoreError, Task, TaskStatusLog},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, PatchId, RepoName, TaskId, VersionNumber, Versioned,
    repositories::{Repository, SearchRepositoriesQuery},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{
    Pool, Postgres,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{collections::HashSet, str::FromStr, time::Duration};
use tracing::info;

use super::issue_graph::IssueGraphContext;

pub type PgStorePool = Pool<Postgres>;

pub const ISSUE_SCHEMA_VERSION: i32 = 1;
pub const PATCH_SCHEMA_VERSION: i32 = 1;
pub const TASK_SCHEMA_VERSION: i32 = 1;
pub const USER_SCHEMA_VERSION: i32 = 3;
pub const REPOSITORY_SCHEMA_VERSION: i32 = 1;
pub const ACTOR_SCHEMA_VERSION: i32 = 3;
pub const DOCUMENT_SCHEMA_VERSION: i32 = 1;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Establish a Postgres connection pool using the provided configuration.
///
/// Returns `Ok(None)` when no database URL is configured, allowing callers to
/// continue using the in-memory store in development environments.
pub async fn init_pool(config: &DatabaseSection) -> Result<Option<PgStorePool>> {
    let Some(database_url) = config.database_url() else {
        return Ok(None);
    };

    let max_connections = config.max_connections.max(1);
    let min_connections = config.min_connections.min(max_connections);

    let mut pool_options = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

    let connect_options = PgConnectOptions::from_str(&database_url)
        .context("failed to parse database URL for Postgres pool")?;

    if let Some(idle_timeout_secs) = config.idle_timeout() {
        pool_options = pool_options.idle_timeout(Duration::from_secs(idle_timeout_secs));
    }

    let pool = pool_options
        .connect_with(connect_options)
        .await
        .context("failed to connect to configured Postgres database")?;

    Ok(Some(pool))
}

/// Run embedded SQLx migrations against the provided pool.
pub async fn run_migrations(pool: &PgStorePool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("failed to apply Postgres migrations")
}

#[derive(Clone, Copy)]
struct PayloadTable {
    object_type: &'static str,
    table: &'static str,
    target_version: i32,
}

const PAYLOAD_TABLES: &[PayloadTable] = &[
    PayloadTable {
        object_type: "issue",
        table: TABLE_ISSUES,
        target_version: ISSUE_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "patch",
        table: TABLE_PATCHES,
        target_version: PATCH_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "task",
        table: TABLE_TASKS,
        target_version: TASK_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "user",
        table: TABLE_USERS,
        target_version: USER_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "repository",
        table: TABLE_REPOSITORIES,
        target_version: REPOSITORY_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "actor",
        table: TABLE_ACTORS,
        target_version: ACTOR_SCHEMA_VERSION,
    },
    PayloadTable {
        object_type: "document",
        table: TABLE_DOCUMENTS,
        target_version: DOCUMENT_SCHEMA_VERSION,
    },
];

/// Migrate any outdated payloads to the current schema versions using the
/// database-level `metis.migrate_payload` helper.
pub async fn migrate_payloads(pool: &PgStorePool) -> Result<()> {
    for table in PAYLOAD_TABLES {
        let rows = migrate_table_payloads(pool, *table).await?;
        if rows > 0 {
            info!(
                object_type = table.object_type,
                rows_migrated = rows,
                target_version = table.target_version,
                "updated Postgres payloads to current schema version"
            );
        }
    }

    Ok(())
}

async fn migrate_table_payloads(pool: &PgStorePool, table: PayloadTable) -> Result<u64> {
    let query = format!(
        "UPDATE {table_name}
         SET payload = metis.migrate_payload($1, schema_version, $2, payload),
             schema_version = $2
         WHERE schema_version < $2",
        table_name = table.table
    );

    let result = sqlx::query(&query)
        .bind(table.object_type)
        .bind(table.target_version)
        .execute(pool)
        .await
        .with_context(|| format!("failed to migrate payloads for {}", table.object_type))?;

    Ok(result.rows_affected())
}

const TABLE_ISSUES: &str = "metis.issues";
const TABLE_PATCHES: &str = "metis.patches";
const TABLE_TASKS: &str = "metis.tasks";
const TABLE_USERS: &str = "metis.users";
const TABLE_REPOSITORIES: &str = "metis.repositories";
const TABLE_ACTORS: &str = "metis.actors";
const TABLE_DOCUMENTS: &str = "metis.documents";

#[derive(Clone)]
pub struct PostgresStore {
    pool: PgStorePool,
}

impl PostgresStore {
    pub fn new(pool: PgStorePool) -> Self {
        Self { pool }
    }

    async fn ensure_issue_exists(&self, id: &IssueId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ISSUES} WHERE id = $1"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            Err(StoreError::IssueNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_patch_exists(&self, id: &PatchId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_PATCHES} WHERE id = $1"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            Err(StoreError::PatchNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_task_exists(&self, id: &TaskId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_TASKS} WHERE id = $1"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            Err(StoreError::TaskNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), StoreError> {
        let name_str = name.as_str();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_REPOSITORIES} WHERE id = $1"
        ))
        .bind(name_str.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            Err(StoreError::RepositoryNotFound(name.clone()))
        } else {
            Ok(())
        }
    }

    async fn validate_issue_dependencies(
        &self,
        dependencies: &[IssueDependency],
    ) -> Result<(), StoreError> {
        for dependency in dependencies {
            if let Err(err) = self.ensure_issue_exists(&dependency.issue_id).await {
                if matches!(err, StoreError::IssueNotFound(_)) {
                    return Err(StoreError::InvalidDependency(dependency.issue_id.clone()));
                }
                return Err(err);
            }
        }
        Ok(())
    }

    async fn fetch_versioned_payload<T: DeserializeOwned>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        target_version: i32,
    ) -> Result<Option<Versioned<T>>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct VersionedPayloadRow {
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        let query = format!(
            "SELECT schema_version, payload, version_number, created_at FROM {table} WHERE id = $1 ORDER BY version_number DESC LIMIT 1"
        );
        let row = sqlx::query_as::<_, VersionedPayloadRow>(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let Some(row) = row else {
            return Ok(None);
        };

        ensure_schema_version(object_type, row.schema_version, target_version)?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for {object_type} '{id}'"
            ))
        })?;
        let item = serde_json::from_value(row.payload).map_err(map_serde_error(object_type))?;
        Ok(Some(Versioned::new(item, version, row.created_at)))
    }

    async fn fetch_versioned_payloads<T: DeserializeOwned>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        target_version: i32,
    ) -> Result<Vec<Versioned<T>>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct VersionedPayloadRow {
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        let query = format!(
            "SELECT schema_version, payload, version_number, created_at FROM {table} WHERE id = $1 ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, VersionedPayloadRow>(&query)
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version(object_type, row.schema_version, target_version)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for {object_type} '{id}'"
                ))
            })?;
            let item = serde_json::from_value(row.payload).map_err(map_serde_error(object_type))?;
            results.push(Versioned::new(item, version, row.created_at));
        }

        Ok(results)
    }

    async fn fetch_versioned_payloads_with_ids<T: DeserializeOwned>(
        &self,
        table: &str,
        object_type: &str,
        target_version: i32,
    ) -> Result<Vec<(String, Versioned<T>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct VersionedPayloadWithId {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        let query = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at FROM {table} ORDER BY id, version_number DESC"
        );
        let rows = sqlx::query_as::<_, VersionedPayloadWithId>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version(object_type, row.schema_version, target_version)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for {object_type} '{}'",
                    row.id
                ))
            })?;
            let value: T =
                serde_json::from_value(row.payload).map_err(map_serde_error(object_type))?;
            results.push((row.id, Versioned::new(value, version, row.created_at)));
        }

        Ok(results)
    }

    async fn fetch_latest_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct DocumentRow {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        // Use a subquery to get the latest version of each document first,
        // then apply filters. This ensures we filter on the current state
        // of each document, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at \
             FROM {TABLE_DOCUMENTS} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        if let Some(path) = query.path_prefix.as_ref() {
            if query.path_is_exact.unwrap_or(false) {
                predicates.push(format!(
                    "COALESCE(payload->>'path','') = ${}",
                    bindings.len() + 1
                ));
                bindings.push(path.clone());
            } else {
                predicates.push(format!(
                    "COALESCE(payload->>'path','') LIKE ${}",
                    bindings.len() + 1
                ));
                bindings.push(format!("{path}%"));
            }
        }

        if let Some(created_by) = query.created_by.as_ref() {
            predicates.push(format!("payload->>'created_by' = ${}", bindings.len() + 1));
            bindings.push(created_by.as_ref().to_string());
        }

        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            let idx_title = bindings.len() + 1;
            let idx_body = bindings.len() + 2;
            let idx_path = bindings.len() + 3;
            predicates.push(format!(
                "(LOWER(payload->>'title') LIKE ${idx_title} \
                 OR LOWER(payload->>'body_markdown') LIKE ${idx_body} \
                 OR LOWER(COALESCE(payload->>'path','')) LIKE ${idx_path})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone());
            bindings.push(pattern.clone());
            bindings.push(pattern);
        }

        // Filter deleted documents by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("COALESCE((payload->>'deleted')::boolean, false) = false".to_string());
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, DocumentRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut documents = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version("document", row.schema_version, DOCUMENT_SCHEMA_VERSION)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for document '{}'",
                    row.id
                ))
            })?;
            let document: Document =
                serde_json::from_value(row.payload).map_err(map_serde_error("document"))?;
            let document_id = row.id.parse::<DocumentId>().map_err(|err| {
                StoreError::Internal(format!("invalid document id stored in database: {err}"))
            })?;
            documents.push((
                document_id,
                Versioned::new(document, version, row.created_at),
            ));
        }

        Ok(documents)
    }

    async fn fetch_latest_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct TaskRow {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        // Use a subquery to get the latest version of each task first,
        // then apply filters. This ensures we filter on the current state
        // of each task, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at \
             FROM {TABLE_TASKS} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        // Filter by spawned_from
        if let Some(spawned_from) = query.spawned_from.as_ref() {
            predicates.push(format!(
                "payload->>'spawned_from' = ${}",
                bindings.len() + 1
            ));
            bindings.push(spawned_from.as_ref().to_string());
        }

        // Filter by search term (q) - matches task ID, prompt, status (NOT notes)
        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            let idx_id = bindings.len() + 1;
            let idx_prompt = bindings.len() + 2;
            let idx_status = bindings.len() + 3;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} \
                 OR LOWER(payload->>'prompt') LIKE ${idx_prompt} \
                 OR LOWER(payload->>'status') LIKE ${idx_status})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone()); // id
            bindings.push(pattern.clone()); // prompt
            bindings.push(pattern); // status
        }

        // Filter deleted tasks by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("COALESCE((payload->>'deleted')::boolean, false) = false".to_string());
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, TaskRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut tasks = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version("task", row.schema_version, TASK_SCHEMA_VERSION)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for task '{}'",
                    row.id
                ))
            })?;
            let task: Task =
                serde_json::from_value(row.payload).map_err(map_serde_error("task"))?;
            let task_id = row.id.parse::<TaskId>().map_err(|err| {
                StoreError::Internal(format!("invalid task id stored in database: {err}"))
            })?;
            tasks.push((task_id, Versioned::new(task, version, row.created_at)));
        }

        Ok(tasks)
    }

    async fn fetch_latest_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct IssueRow {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        // Use a subquery to get the latest version of each issue first,
        // then apply filters. This ensures we filter on the current state
        // of each issue, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at \
             FROM {TABLE_ISSUES} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        // Filter by issue_type
        if let Some(issue_type) = query.issue_type.as_ref() {
            predicates.push(format!("payload->>'type' = ${}", bindings.len() + 1));
            bindings.push(issue_type.as_str().to_string());
        }

        // Filter by status
        if let Some(status) = query.status.as_ref() {
            predicates.push(format!("payload->>'status' = ${}", bindings.len() + 1));
            bindings.push(status.as_str().to_string());
        }

        // Filter by assignee (case-insensitive)
        if let Some(assignee) = query
            .assignee
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            predicates.push(format!(
                "LOWER(payload->>'assignee') = ${}",
                bindings.len() + 1
            ));
            bindings.push(assignee.to_lowercase());
        }

        // Filter by search term (q)
        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            let idx_id = bindings.len() + 1;
            let idx_desc = bindings.len() + 2;
            let idx_progress = bindings.len() + 3;
            let idx_type = bindings.len() + 4;
            let idx_status = bindings.len() + 5;
            let idx_creator = bindings.len() + 6;
            let idx_assignee = bindings.len() + 7;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} \
                 OR LOWER(payload->>'description') LIKE ${idx_desc} \
                 OR LOWER(COALESCE(payload->>'progress','')) LIKE ${idx_progress} \
                 OR payload->>'type' = ${idx_type} \
                 OR payload->>'status' = ${idx_status} \
                 OR LOWER(payload->>'creator') LIKE ${idx_creator} \
                 OR LOWER(COALESCE(payload->>'assignee','')) LIKE ${idx_assignee})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone()); // id
            bindings.push(pattern.clone()); // description
            bindings.push(pattern.clone()); // progress
            bindings.push(term.clone()); // type (exact match)
            bindings.push(term.clone()); // status (exact match)
            bindings.push(pattern.clone()); // creator
            bindings.push(pattern); // assignee
        }

        // Filter deleted issues by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("COALESCE((payload->>'deleted')::boolean, false) = false".to_string());
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, IssueRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut issues = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version("issue", row.schema_version, ISSUE_SCHEMA_VERSION)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for issue '{}'",
                    row.id
                ))
            })?;
            let issue: Issue =
                serde_json::from_value(row.payload).map_err(map_serde_error("issue"))?;
            let issue_id = row.id.parse::<IssueId>().map_err(|err| {
                StoreError::Internal(format!("invalid issue id stored in database: {err}"))
            })?;
            issues.push((issue_id, Versioned::new(issue, version, row.created_at)));
        }

        Ok(issues)
    }

    async fn fetch_latest_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct PatchRow {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        // Use a subquery to get the latest version of each patch first,
        // then apply filters. This ensures we filter on the current state
        // of each patch, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at \
             FROM {TABLE_PATCHES} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        // Filter deleted patches by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("COALESCE((payload->>'deleted')::boolean, false) = false".to_string());
        }

        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            // Search across multiple fields: id, title, description, status, service_repo_name, diff, github fields
            let idx_start = bindings.len() + 1;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} \
                 OR LOWER(payload->>'title') LIKE ${idx_title} \
                 OR LOWER(payload->>'description') LIKE ${idx_desc} \
                 OR LOWER(payload->>'status') LIKE ${idx_status} \
                 OR LOWER(payload->>'service_repo_name') LIKE ${idx_repo} \
                 OR LOWER(payload->>'diff') LIKE ${idx_diff} \
                 OR LOWER(payload->'github'->>'owner') LIKE ${idx_gh_owner} \
                 OR LOWER(payload->'github'->>'repo') LIKE ${idx_gh_repo} \
                 OR (payload->'github'->>'number') LIKE ${idx_gh_number} \
                 OR LOWER(COALESCE(payload->'github'->>'head_ref','')) LIKE ${idx_gh_head} \
                 OR LOWER(COALESCE(payload->'github'->>'base_ref','')) LIKE ${idx_gh_base})",
                idx_id = idx_start,
                idx_title = idx_start + 1,
                idx_desc = idx_start + 2,
                idx_status = idx_start + 3,
                idx_repo = idx_start + 4,
                idx_diff = idx_start + 5,
                idx_gh_owner = idx_start + 6,
                idx_gh_repo = idx_start + 7,
                idx_gh_number = idx_start + 8,
                idx_gh_head = idx_start + 9,
                idx_gh_base = idx_start + 10,
            ));
            let pattern = format!("%{term}%");
            for _ in 0..11 {
                bindings.push(pattern.clone());
            }
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, PatchRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut patches = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version("patch", row.schema_version, PATCH_SCHEMA_VERSION)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for patch '{}'",
                    row.id
                ))
            })?;
            let patch: Patch =
                serde_json::from_value(row.payload).map_err(map_serde_error("patch"))?;
            let patch_id = row.id.parse::<PatchId>().map_err(|err| {
                StoreError::Internal(format!("invalid patch id stored in database: {err}"))
            })?;
            patches.push((patch_id, Versioned::new(patch, version, row.created_at)));
        }

        Ok(patches)
    }

    async fn fetch_latest_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct UserRow {
            id: String,
            schema_version: i32,
            payload: Value,
            version_number: i64,
            created_at: DateTime<Utc>,
        }

        // Use a subquery to get the latest version of each user first,
        // then apply filters. This ensures we filter on the current state
        // of each user, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, schema_version, payload, version_number, created_at \
             FROM {TABLE_USERS} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        // Filter deleted users by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("COALESCE((payload->>'deleted')::boolean, false) = false".to_string());
        }

        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            // Search across multiple fields: id, username
            let idx_start = bindings.len() + 1;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} \
                 OR LOWER(payload->>'username') LIKE ${idx_username})",
                idx_id = idx_start,
                idx_username = idx_start + 1,
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone());
            bindings.push(pattern);
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, UserRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            ensure_schema_version("user", row.schema_version, USER_SCHEMA_VERSION)?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for user '{}'",
                    row.id
                ))
            })?;
            let user: User =
                serde_json::from_value(row.payload).map_err(map_serde_error("user"))?;
            let username = Username::from(row.id);
            users.push((username, Versioned::new(user, version, row.created_at)));
        }

        Ok(users)
    }

    async fn insert_payload<T: Serialize>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        schema_version: i32,
        version_number: VersionNumber,
        payload: &T,
    ) -> Result<(), StoreError> {
        let payload_value = serde_json::to_value(payload).map_err(map_serde_error(object_type))?;
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for {object_type} '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {table} (id, version_number, schema_version, payload) VALUES ($1, $2, $3, $4)"
        );
        sqlx::query(&query)
            .bind(id)
            .bind(version_number)
            .bind(schema_version)
            .bind(payload_value)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn update_payload<T: Serialize>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        schema_version: i32,
        payload: &T,
    ) -> Result<(), StoreError> {
        let latest_version = self
            .fetch_latest_version_number(table, id)
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("{object_type} '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for {object_type} '{id}'"))
        })?;

        self.insert_payload(
            table,
            object_type,
            id,
            schema_version,
            next_version,
            payload,
        )
        .await
    }

    async fn fetch_latest_version_number(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<VersionNumber>, StoreError> {
        let query = format!(
            "SELECT version_number FROM {table} WHERE id = $1 ORDER BY version_number DESC LIMIT 1"
        );
        let version = sqlx::query_scalar::<_, i64>(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        match version {
            Some(value) => VersionNumber::try_from(value).map(Some).map_err(|_| {
                StoreError::Internal(format!("invalid version number stored for {table} '{id}'"))
            }),
            None => Ok(None),
        }
    }
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    StoreError::Internal(err.to_string())
}

fn map_serde_error(object_type: &str) -> impl FnOnce(serde_json::Error) -> StoreError + '_ {
    move |err| StoreError::Internal(format!("failed to encode/decode {object_type}: {err}"))
}

fn ensure_schema_version(
    object_type: &str,
    schema_version: i32,
    target_version: i32,
) -> Result<(), StoreError> {
    if schema_version != target_version {
        return Err(StoreError::Internal(format!(
            "unexpected {object_type} schema version {schema_version} (expected {target_version})"
        )));
    }

    Ok(())
}

#[async_trait]
impl Store for PostgresStore {
    async fn add_repository(&self, name: RepoName, config: Repository) -> Result<(), StoreError> {
        let name_str = name.as_str();

        // Check if repository exists (including deleted)
        let existing = self
            .fetch_versioned_payload::<Repository>(
                TABLE_REPOSITORIES,
                "repository",
                name_str.as_str(),
                REPOSITORY_SCHEMA_VERSION,
            )
            .await?;

        match existing {
            Some(repo) if repo.item.deleted => {
                // Re-create over deleted: use caller's config as-is
                self.update_payload(
                    TABLE_REPOSITORIES,
                    "repository",
                    name_str.as_str(),
                    REPOSITORY_SCHEMA_VERSION,
                    &config,
                )
                .await
            }
            Some(_) => Err(StoreError::RepositoryAlreadyExists(name)),
            None => {
                self.insert_payload(
                    TABLE_REPOSITORIES,
                    "repository",
                    name_str.as_str(),
                    REPOSITORY_SCHEMA_VERSION,
                    1,
                    &config,
                )
                .await
            }
        }
    }

    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        let name_str = name.as_str();
        let versioned: Versioned<Repository> = self
            .fetch_versioned_payload(
                TABLE_REPOSITORIES,
                "repository",
                name_str.as_str(),
                REPOSITORY_SCHEMA_VERSION,
            )
            .await?
            .ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::RepositoryNotFound(name.clone()));
        }
        Ok(versioned)
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
    ) -> Result<(), StoreError> {
        let name_str = name.as_str();
        self.ensure_repository_exists(&name).await?;

        self.update_payload(
            TABLE_REPOSITORIES,
            "repository",
            name_str.as_str(),
            REPOSITORY_SCHEMA_VERSION,
            &config,
        )
        .await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let mut rows = self
            .fetch_versioned_payloads_with_ids::<Repository>(
                TABLE_REPOSITORIES,
                "repository",
                REPOSITORY_SCHEMA_VERSION,
            )
            .await?
            .into_iter()
            .filter(|(_, repo)| include_deleted || !repo.item.deleted)
            .map(|(id, repo)| {
                RepoName::from_str(&id)
                    .map(|name| (name, repo))
                    .map_err(|err| {
                        StoreError::Internal(format!(
                            "invalid repository id stored in database: {err}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        rows.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(rows)
    }

    async fn delete_repository(&self, name: &RepoName) -> Result<(), StoreError> {
        // Use include_deleted: true since we need to access the repository to mark it as deleted
        let current = self.get_repository(name, true).await?;
        let mut repo = current.item;
        repo.deleted = true;
        self.update_repository(name.clone(), repo).await
    }

    async fn add_issue(&self, issue: Issue) -> Result<IssueId, StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        let id = IssueId::new();

        self.insert_payload(
            TABLE_ISSUES,
            "issue",
            id.as_ref(),
            ISSUE_SCHEMA_VERSION,
            1,
            &issue,
        )
        .await?;

        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Versioned<Issue>, StoreError> {
        self.fetch_versioned_payload(TABLE_ISSUES, "issue", id.as_ref(), ISSUE_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let versions = self
            .fetch_versioned_payloads(TABLE_ISSUES, "issue", id.as_ref(), ISSUE_SCHEMA_VERSION)
            .await?;
        if versions.is_empty() {
            return Err(StoreError::IssueNotFound(id.clone()));
        }
        Ok(versions)
    }

    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        self.get_issue(id).await?;

        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        self.update_payload(
            TABLE_ISSUES,
            "issue",
            id.as_ref(),
            ISSUE_SCHEMA_VERSION,
            &issue,
        )
        .await
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        self.fetch_latest_issues(query).await
    }

    async fn delete_issue(&self, id: &IssueId) -> Result<(), StoreError> {
        let current = self.get_issue(id).await?;
        let mut issue = current.item;
        issue.deleted = true;
        self.update_issue(id, issue).await
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let issues = self.list_issues(&SearchIssuesQuery::default()).await?;
        let issue_values: Vec<(IssueId, Issue)> = issues
            .into_iter()
            .map(|(id, issue)| (id, issue.item))
            .collect();
        let context = IssueGraphContext::from_issues(&issue_values);
        context.apply_filters(filters)
    }

    async fn add_patch(&self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.insert_payload(
            TABLE_PATCHES,
            "patch",
            id.as_ref(),
            PATCH_SCHEMA_VERSION,
            1,
            &patch,
        )
        .await?;
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Versioned<Patch>, StoreError> {
        self.fetch_versioned_payload(TABLE_PATCHES, "patch", id.as_ref(), PATCH_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let versions = self
            .fetch_versioned_payloads(TABLE_PATCHES, "patch", id.as_ref(), PATCH_SCHEMA_VERSION)
            .await?;
        if versions.is_empty() {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        Ok(versions)
    }

    async fn update_patch(&self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        self.get_patch(id).await?;

        self.update_payload(
            TABLE_PATCHES,
            "patch",
            id.as_ref(),
            PATCH_SCHEMA_VERSION,
            &patch,
        )
        .await
    }

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        self.fetch_latest_patches(query).await
    }

    async fn delete_patch(&self, id: &PatchId) -> Result<(), StoreError> {
        let current = self.get_patch(id).await?;
        let mut patch = current.item;
        patch.deleted = true;
        self.update_patch(id, patch).await
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_patch_exists(patch_id).await?;
        let issues = self.list_issues(&SearchIssuesQuery::default()).await?;

        Ok(issues
            .into_iter()
            .filter(|(_, issue)| issue.item.patches.contains(patch_id))
            .map(|(id, _)| id)
            .collect())
    }

    async fn add_document(&self, document: Document) -> Result<DocumentId, StoreError> {
        let id = DocumentId::new();
        self.insert_payload(
            TABLE_DOCUMENTS,
            "document",
            id.as_ref(),
            DOCUMENT_SCHEMA_VERSION,
            1,
            &document,
        )
        .await?;
        Ok(id)
    }

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let versioned: Versioned<Document> = self
            .fetch_versioned_payload(
                TABLE_DOCUMENTS,
                "document",
                id.as_ref(),
                DOCUMENT_SCHEMA_VERSION,
            )
            .await?
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }
        Ok(versioned)
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let versions = self
            .fetch_versioned_payloads(
                TABLE_DOCUMENTS,
                "document",
                id.as_ref(),
                DOCUMENT_SCHEMA_VERSION,
            )
            .await?;
        if versions.is_empty() {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }
        Ok(versions)
    }

    async fn update_document(&self, id: &DocumentId, document: Document) -> Result<(), StoreError> {
        self.get_document(id, true).await?;
        self.update_payload(
            TABLE_DOCUMENTS,
            "document",
            id.as_ref(),
            DOCUMENT_SCHEMA_VERSION,
            &document,
        )
        .await
    }

    async fn delete_document(&self, id: &DocumentId) -> Result<(), StoreError> {
        let current = self.get_document(id, true).await?;
        let mut document = current.item;
        document.deleted = true;
        self.update_document(id, document).await
    }

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        self.fetch_latest_documents(query).await
    }

    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        self.list_documents(&SearchDocumentsQuery::new(
            None,
            Some(path_prefix.to_string()),
            None,
            None,
            None,
        ))
        .await
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let issues = self.list_issues(&SearchIssuesQuery::default()).await?;
        Ok(issues
            .into_iter()
            .filter_map(|(id, issue)| {
                issue
                    .item
                    .dependencies
                    .iter()
                    .any(|dep| {
                        dep.dependency_type == IssueDependencyType::ChildOf
                            && dep.issue_id == *issue_id
                    })
                    .then_some(id)
            })
            .collect())
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let issues = self.list_issues(&SearchIssuesQuery::default()).await?;
        Ok(issues
            .into_iter()
            .filter_map(|(id, issue)| {
                issue
                    .item
                    .dependencies
                    .iter()
                    .any(|dep| {
                        dep.dependency_type == IssueDependencyType::BlockedOn
                            && dep.issue_id == *issue_id
                    })
                    .then_some(id)
            })
            .collect())
    }

    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        // Use spawned_from filter at the database level for efficiency
        let query = SearchJobsQuery::new(None, Some(issue_id.clone()), None);
        let tasks = self.list_tasks(&query).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    async fn add_task(
        &self,
        task: Task,
        creation_time: chrono::DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        let id = TaskId::new();
        self.add_task_with_id(id.clone(), task, creation_time)
            .await?;
        Ok(id)
    }

    async fn add_task_with_id(
        &self,
        metis_id: TaskId,
        task: Task,
        _creation_time: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut task = task;
        task.status = Status::Created;
        task.last_message = None;
        task.error = None;
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_TASKS} WHERE id = $1"
        ))
        .bind(metis_id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists > 0 {
            return Err(StoreError::Internal(format!(
                "Task already exists: {metis_id}"
            )));
        }

        if let Some(issue_id) = task.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        self.insert_payload(
            TABLE_TASKS,
            "task",
            metis_id.as_ref(),
            TASK_SCHEMA_VERSION,
            1,
            &task,
        )
        .await?;

        Ok(())
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
    ) -> Result<Versioned<Task>, StoreError> {
        self.ensure_task_exists(metis_id).await?;
        if let Some(issue_id) = task.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        self.update_payload(
            TABLE_TASKS,
            "task",
            metis_id.as_ref(),
            TASK_SCHEMA_VERSION,
            &task,
        )
        .await?;

        self.fetch_versioned_payload(TABLE_TASKS, "task", metis_id.as_ref(), TASK_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::TaskNotFound(metis_id.clone()))
    }

    async fn get_task(&self, id: &TaskId) -> Result<Versioned<Task>, StoreError> {
        self.fetch_versioned_payload(TABLE_TASKS, "task", id.as_ref(), TASK_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        let versions = self
            .fetch_versioned_payloads(TABLE_TASKS, "task", id.as_ref(), TASK_SCHEMA_VERSION)
            .await?;
        if versions.is_empty() {
            return Err(StoreError::TaskNotFound(id.clone()));
        }
        Ok(versions)
    }

    async fn list_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        self.fetch_latest_tasks(query).await
    }

    async fn delete_task(&self, id: &TaskId) -> Result<(), StoreError> {
        let current = self.get_task(id).await?;
        let mut task = current.item;
        task.deleted = true;
        self.update_task(id, task).await?;
        Ok(())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        let rows = self
            .fetch_versioned_payloads_with_ids::<Task>(TABLE_TASKS, "task", TASK_SCHEMA_VERSION)
            .await?;

        let mut matches = Vec::new();
        for (id, task) in rows {
            if task.item.status == status {
                matches.push(id.parse::<TaskId>().map_err(|err| {
                    StoreError::Internal(format!("invalid task id stored in database: {err}"))
                })?);
            }
        }

        Ok(matches)
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        let versions = self
            .fetch_versioned_payloads::<Task>(TABLE_TASKS, "task", id.as_ref(), TASK_SCHEMA_VERSION)
            .await?;
        super::task_status_log_from_versions(&versions)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn add_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS} WHERE id = $1"
        ))
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists > 0 {
            return Err(StoreError::ActorAlreadyExists(name));
        }

        self.insert_payload(
            TABLE_ACTORS,
            "actor",
            &name,
            ACTOR_SCHEMA_VERSION,
            1,
            &actor,
        )
        .await
    }

    async fn update_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS} WHERE id = $1"
        ))
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            return Err(StoreError::ActorNotFound(name));
        }

        self.update_payload(TABLE_ACTORS, "actor", &name, ACTOR_SCHEMA_VERSION, &actor)
            .await
    }

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        super::validate_actor_name(name)?;
        self.fetch_versioned_payload(TABLE_ACTORS, "actor", name, ACTOR_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::ActorNotFound(name.to_string()))
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        let mut actors = self
            .fetch_versioned_payloads_with_ids::<Actor>(TABLE_ACTORS, "actor", ACTOR_SCHEMA_VERSION)
            .await?;
        actors.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(actors)
    }

    async fn add_user(&self, user: User) -> Result<(), StoreError> {
        // Check if user already exists
        let existing = self
            .fetch_versioned_payload::<User>(
                TABLE_USERS,
                "user",
                user.username.as_str(),
                USER_SCHEMA_VERSION,
            )
            .await?;

        match existing {
            Some(versioned) => {
                // If user exists but is deleted, allow re-creation with the provided user
                if versioned.item.deleted {
                    self.update_user(user).await?;
                    Ok(())
                } else {
                    Err(StoreError::UserAlreadyExists(user.username.clone()))
                }
            }
            None => {
                // User doesn't exist, insert new
                self.insert_payload(
                    TABLE_USERS,
                    "user",
                    user.username.as_str(),
                    USER_SCHEMA_VERSION,
                    1,
                    &user,
                )
                .await
            }
        }
    }

    async fn update_user(&self, user: User) -> Result<Versioned<User>, StoreError> {
        let username = user.username.clone();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_USERS} WHERE id = $1"
        ))
        .bind(user.username.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            return Err(StoreError::UserNotFound(username));
        }

        self.update_payload(
            TABLE_USERS,
            "user",
            user.username.as_str(),
            USER_SCHEMA_VERSION,
            &user,
        )
        .await?;

        self.fetch_versioned_payload(
            TABLE_USERS,
            "user",
            user.username.as_str(),
            USER_SCHEMA_VERSION,
        )
        .await?
        .ok_or_else(|| {
            StoreError::Internal(format!(
                "user '{}' missing after update",
                user.username.as_str()
            ))
        })
    }

    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        let versioned: Versioned<User> = self
            .fetch_versioned_payload(TABLE_USERS, "user", username.as_str(), USER_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::UserNotFound(username.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::UserNotFound(username.clone()));
        }
        Ok(versioned)
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        self.fetch_latest_users(query).await
    }

    async fn delete_user(&self, username: &Username) -> Result<(), StoreError> {
        let current = self.get_user(username, true).await?;
        let mut user = current.item;
        user.deleted = true;
        self.update_user(user).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            documents::Document,
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, TodoItem,
            },
            jobs::BundleSpec,
            patches::{Patch, PatchStatus},
            users::{User, Username},
        },
        test_utils::test_state_with_store,
    };
    use metis_common::{
        RepoName, TaskId, VersionNumber, Versioned,
        repositories::{Repository, SearchRepositoriesQuery},
    };
    use std::{collections::HashSet, str::FromStr, sync::Arc};

    fn assert_versioned<T: std::fmt::Debug + PartialEq>(
        actual: &Versioned<T>,
        expected_item: &T,
        expected_version: VersionNumber,
    ) {
        assert_eq!(&actual.item, expected_item);
        assert_eq!(actual.version, expected_version);
    }

    #[allow(dead_code)]
    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "details".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            vec![TodoItem::new("todo".to_string(), false)],
            dependencies,
            Vec::new(),
        )
    }

    #[allow(dead_code)]
    fn sample_patch() -> Patch {
        Patch::new(
            "patch title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        )
    }

    #[allow(dead_code)]
    fn sample_document(path: &str, created_by: Option<TaskId>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: Some(path.to_string()),
            created_by,
            deleted: false,
        }
    }

    #[allow(dead_code)]
    fn sample_task() -> Task {
        Task::new(
            "prompt".to_string(),
            BundleSpec::None,
            None,
            Some("metis-worker:latest".to_string()),
            None,
            Default::default(),
            None,
            None,
            None,
        )
    }

    #[allow(dead_code)]
    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("image:latest".to_string()),
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_round_trip(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.version, 1);

        let mut updated = config.clone();
        updated.default_image = Some("other:latest".to_string());
        store
            .update_repository(name.clone(), updated.clone())
            .await
            .unwrap();

        let versions: Vec<i64> = sqlx::query_scalar(&format!(
            "SELECT version_number FROM {TABLE_REPOSITORIES} WHERE id = $1 ORDER BY version_number"
        ))
        .bind(name.as_str())
        .fetch_all(&store.pool)
        .await
        .unwrap();
        assert_eq!(versions, vec![1, 2]);

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, name);
        assert_versioned(&list[0].1, &updated, 2);

        let fetched_again = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched_again.item, updated);
        assert_eq!(fetched_again.version, 2);
        assert!(fetched_again.timestamp >= fetched.timestamp);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_add_rejects_duplicate(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let name = RepoName::from_str("dourolabs/metis").unwrap();

        store
            .add_repository(name.clone(), sample_repository_config())
            .await
            .unwrap();

        let err = store
            .add_repository(name.clone(), sample_repository_config())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryAlreadyExists(existing) if existing == name
        ));

        let missing = RepoName::from_str("dourolabs/missing").unwrap();
        let err = store
            .update_repository(missing.clone(), sample_repository_config())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(existing) if existing == missing
        ));
    }

    #[test]
    fn init_migration_drops_triggers_before_create() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let migration = std::fs::read_to_string(format!(
            "{manifest_dir}/migrations/20241115000000_init_store.sql"
        ))
        .unwrap();
        let triggers = [
            "set_timestamp_payload_schema_versions",
            "set_timestamp_issues",
            "set_timestamp_patches",
            "set_timestamp_tasks",
            "set_timestamp_task_status_logs",
            "set_timestamp_users",
            "set_timestamp_repositories",
        ];

        for trigger in triggers {
            let drop_stmt = format!("DROP TRIGGER IF EXISTS {trigger} ON metis.");
            let create_stmt = format!("CREATE TRIGGER {trigger}");
            let drop_pos = migration
                .find(&drop_stmt)
                .unwrap_or_else(|| panic!("missing drop for {trigger}"));
            let create_pos = migration
                .find(&create_stmt)
                .unwrap_or_else(|| panic!("missing create for {trigger}"));
            assert!(
                drop_pos < create_pos,
                "drop should precede create for {trigger}"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_round_trip(pool: PgStorePool) {
        let store = PostgresStore::new(pool);

        let parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let issue = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )]))
            .await
            .unwrap();

        let fetched = store.get_issue(&issue).await.unwrap();
        assert_eq!(fetched.item.dependencies.len(), 1);
        assert_eq!(fetched.version, 1);

        let issues: HashSet<_> = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(issues.contains(&issue));

        let children = store.get_issue_children(&parent).await.unwrap();
        assert_eq!(children, vec![issue.clone()]);

        let new_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let mut updated_issue = sample_issue(vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            new_parent.clone(),
        )]);
        updated_issue.patches = Vec::new();
        store.update_issue(&issue, updated_issue).await.unwrap();

        let fetched_after_update = store.get_issue(&issue).await.unwrap();
        assert_eq!(fetched_after_update.version, 2);

        assert!(store.get_issue_children(&parent).await.unwrap().is_empty());
        assert_eq!(
            store.get_issue_children(&new_parent).await.unwrap(),
            vec![issue]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_issue_rejects_missing_dependency(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let missing = IssueId::new();

        let err = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                missing.clone(),
            )]))
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::InvalidDependency(id) if id == missing));

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let err = store
            .update_issue(
                &issue_id,
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    missing.clone(),
                )]),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidDependency(id) if id == missing));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn errors_on_schema_mismatch(pool: PgStorePool) {
        let pool_for_update = pool.clone();
        let store = PostgresStore::new(pool);

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();

        sqlx::query(&format!(
            "UPDATE {TABLE_ISSUES} SET schema_version = $1 WHERE id = $2"
        ))
        .bind(ISSUE_SCHEMA_VERSION + 1)
        .bind(issue_id.as_ref())
        .execute(&pool_for_update)
        .await
        .unwrap();

        let err = store.get_issue(&issue_id).await.unwrap_err();
        assert!(matches!(err, StoreError::Internal(message) if message.contains("schema version")));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migrates_outdated_payloads(pool: PgStorePool) {
        let store = PostgresStore::new(pool.clone());
        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();

        let migration = PayloadTable {
            object_type: "issue",
            table: TABLE_ISSUES,
            target_version: ISSUE_SCHEMA_VERSION + 1,
        };

        let updated = migrate_table_payloads(&pool, migration).await.unwrap();
        assert_eq!(updated, 1);

        let version: i32 = sqlx::query_scalar(&format!(
            "SELECT schema_version FROM {TABLE_ISSUES} WHERE id = $1"
        ))
        .bind(issue_id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(version, ISSUE_SCHEMA_VERSION + 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_graph_searches_blockers(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let blocker = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocked = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                blocker.clone(),
            )]))
            .await
            .unwrap();

        let blocked_list = store.get_issue_blocked_on(&blocker).await.unwrap();
        assert_eq!(blocked_list, vec![blocked.clone()]);

        let filter: IssueGraphFilter = format!("*:blocked-on:{blocker}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();
        assert_eq!(matches, HashSet::from([blocked]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn patch_associations_round_trip(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let patch_id = store.add_patch(sample_patch()).await.unwrap();
        let mut issue = sample_issue(vec![]);
        issue.patches = vec![patch_id.clone()];
        let issue_id = store.add_issue(issue).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        assert_eq!(issues, vec![issue_id]);

        let mut updated = sample_patch();
        updated.title = "updated".to_string();
        store
            .update_patch(&patch_id, updated.clone())
            .await
            .unwrap();
        let fetched = store.get_patch(&patch_id).await.unwrap();
        assert_eq!(fetched.item.title, "updated");
        assert_eq!(fetched.version, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_lifecycle_updates_status(pool: PgStorePool) {
        let store = Arc::new(PostgresStore::new(pool));
        let handles = test_state_with_store(store.clone());
        let issue_id = handles.store.add_issue(sample_issue(vec![])).await.unwrap();

        let mut task = sample_task();
        task.spawned_from = Some(issue_id.clone());
        let task_id = handles
            .store
            .add_task(task.clone(), Utc::now())
            .await
            .unwrap();
        assert_eq!(
            handles.store.get_task(&task_id).await.unwrap().item.status,
            Status::Created
        );

        handles
            .state
            .transition_task_to_pending(&task_id)
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_running(&task_id)
            .await
            .unwrap();
        assert_eq!(
            handles.store.get_task(&task_id).await.unwrap().item.status,
            Status::Running
        );

        handles
            .state
            .transition_task_to_completion(&task_id, Ok(()), Some("done".into()))
            .await
            .unwrap();
        assert_eq!(
            handles.store.get_task(&task_id).await.unwrap().item.status,
            Status::Complete
        );

        let tasks = handles.store.get_tasks_for_issue(&issue_id).await.unwrap();
        assert_eq!(tasks, vec![task_id.clone()]);

        let mut updated_task = handles.store.get_task(&task_id).await.unwrap().item;
        updated_task.spawned_from = None;
        let updated_version = handles
            .store
            .update_task(&task_id, updated_task.clone())
            .await
            .unwrap();
        assert_eq!(updated_version.item, updated_task);
        let fetched = handles.store.get_task(&task_id).await.unwrap();
        assert_eq!(fetched.item, updated_task);
        assert!(
            handles
                .store
                .get_tasks_for_issue(&issue_id)
                .await
                .unwrap()
                .is_empty()
        );

        let complete = handles
            .store
            .list_tasks_with_status(Status::Complete)
            .await
            .unwrap();
        assert_eq!(complete, vec![task_id]);

        let explicit_id = TaskId::new();
        store
            .add_task_with_id(explicit_id.clone(), sample_task(), Utc::now())
            .await
            .unwrap();
        let all_tasks: HashSet<_> = store
            .list_tasks(&SearchJobsQuery::default())
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert!(all_tasks.contains(&explicit_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn documents_round_trip(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let doc_id = store
            .add_document(sample_document("docs/guide.md", None))
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "Doc");
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.title = "Updated Doc".to_string();
        store
            .update_document(&doc_id, updated.clone())
            .await
            .unwrap();

        let versions = store.get_document_versions(&doc_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].item.title, "Updated Doc");

        let list = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, doc_id);

        let by_path = store.get_documents_by_path("docs/").await.unwrap();
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].0, doc_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_filters_apply_query(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let task_id = TaskId::new();
        let doc_id = store
            .add_document(sample_document("docs/howto.md", Some(task_id.clone())))
            .await
            .unwrap();
        store
            .add_document(sample_document("notes/todo.md", Some(TaskId::new())))
            .await
            .unwrap();

        let query = SearchDocumentsQuery::new(
            Some("howto".to_string()),
            Some("docs/".to_string()),
            None,
            Some(task_id),
            None,
        );

        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, doc_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn user_management_round_trip(pool: PgStorePool) {
        let store = PostgresStore::new(pool);
        let user = User {
            username: Username::from("alice"),
            github_user_id: 101,
            github_token: "token".to_string(),
            github_refresh_token: "refresh-token".to_string(),
            deleted: false,
        };
        store.add_user(user.clone()).await.unwrap();

        let fetched = store
            .get_user(&Username::from("alice"), false)
            .await
            .unwrap();
        assert_eq!(fetched.item, user);
        assert_eq!(fetched.version, 1);

        let updated = store
            .update_user(User {
                username: Username::from("alice"),
                github_user_id: 202,
                github_token: "new-token".to_string(),
                github_refresh_token: "new-refresh".to_string(),
                deleted: false,
            })
            .await
            .unwrap();
        assert_eq!(updated.item.github_token, "new-token");
        assert_eq!(updated.item.github_user_id, 202);
        assert_eq!(updated.version, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_search_only_matches_latest_version(pool: PgStorePool) {
        let store = PostgresStore::new(pool);

        // Create a document with title "original_title"
        let doc = Document {
            title: "original_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".to_string()),
            created_by: None,
            deleted: false,
        };
        let doc_id = store.add_document(doc).await.unwrap();

        // Update the document to change the title to "changed_title"
        let updated_doc = Document {
            title: "changed_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".to_string()),
            created_by: None,
            deleted: false,
        };
        store.update_document(&doc_id, updated_doc).await.unwrap();

        // Search for the old title - should return NO results
        let old_query =
            SearchDocumentsQuery::new(Some("original_title".to_string()), None, None, None, None);
        let old_results = store.list_documents(&old_query).await.unwrap();
        assert!(
            old_results.is_empty(),
            "Search for old title should return no results, but got {:?}",
            old_results.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );

        // Search for the new title - should return the document
        let new_query =
            SearchDocumentsQuery::new(Some("changed_title".to_string()), None, None, None, None);
        let new_results = store.list_documents(&new_query).await.unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].0, doc_id);
        assert_eq!(new_results[0].1.item.title, "changed_title");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_search_only_matches_latest_version(pool: PgStorePool) {
        let store = PostgresStore::new(pool);

        // Create an issue with a unique description
        let issue = Issue::new(
            IssueType::Task,
            "original_unique_description_abc123".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            vec![],
            vec![],
            vec![],
        );
        let issue_id = store.add_issue(issue).await.unwrap();

        // Update the issue to change the description
        let updated_issue = Issue::new(
            IssueType::Task,
            "changed_unique_description_xyz789".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            vec![],
            vec![],
            vec![],
        );
        store.update_issue(&issue_id, updated_issue).await.unwrap();

        // Search for the old description - should return NO results
        let old_query = SearchIssuesQuery::new(
            None,
            None,
            None,
            Some("original_unique_description_abc123".to_string()),
            Vec::new(),
            None,
        );
        let old_results = store.list_issues(&old_query).await.unwrap();
        assert!(
            old_results.is_empty(),
            "Search for old description should return no results, but got {:?}",
            old_results.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );

        // Search for the new description - should return the issue
        let new_query = SearchIssuesQuery::new(
            None,
            None,
            None,
            Some("changed_unique_description_xyz789".to_string()),
            Vec::new(),
            None,
        );
        let new_results = store.list_issues(&new_query).await.unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].0, issue_id);
        assert!(
            new_results[0]
                .1
                .item
                .description
                .contains("changed_unique_description_xyz789")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn patch_search_only_matches_latest_version(pool: PgStorePool) {
        let store = PostgresStore::new(pool);

        // Create a patch with a unique title
        let patch = Patch::new(
            "original_unique_patch_title_abc123".to_string(),
            "desc".to_string(),
            "diff content".to_string(),
            PatchStatus::Open,
            false,
            None,
            vec![],
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        );
        let patch_id = store.add_patch(patch).await.unwrap();

        // Update the patch to change the title
        let updated_patch = Patch::new(
            "changed_unique_patch_title_xyz789".to_string(),
            "desc".to_string(),
            "diff content".to_string(),
            PatchStatus::Open,
            false,
            None,
            vec![],
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        );
        store.update_patch(&patch_id, updated_patch).await.unwrap();

        // Search for the old title - should return NO results
        let old_query =
            SearchPatchesQuery::new(Some("original_unique_patch_title_abc123".to_string()), None);
        let old_results = store.list_patches(&old_query).await.unwrap();
        assert!(
            old_results.is_empty(),
            "Search for old title should return no results, but got {:?}",
            old_results.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );

        // Search for the new title - should return the patch
        let new_query =
            SearchPatchesQuery::new(Some("changed_unique_patch_title_xyz789".to_string()), None);
        let new_results = store.list_patches(&new_query).await.unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].0, patch_id);
        assert_eq!(
            new_results[0].1.item.title,
            "changed_unique_patch_title_xyz789"
        );
    }
}
