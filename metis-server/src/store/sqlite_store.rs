use crate::domain::{
    actors::{Actor, ActorId, ActorRef, UNKNOWN_CREATOR},
    agents::Agent,
    documents::Document,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
        JobSettings, TodoItem,
    },
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::Patch,
    users::{User, Username},
};
use crate::store::issue_graph::IssueGraphContext;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, TaskId,
    VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    repositories::{Repository, SearchRepositoriesQuery},
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use super::{ReadOnlyStore, Store, StoreError, Task, TaskStatusLog};

const TABLE_REPOSITORIES_V2: &str = "repositories_v2";
const TABLE_ACTORS_V2: &str = "actors_v2";
const TABLE_USERS_V2: &str = "users_v2";
const TABLE_ISSUES_V2: &str = "issues_v2";
const TABLE_LABEL_ASSOCIATIONS: &str = "label_associations";

static MIGRATOR: Migrator = sqlx::migrate!("./sqlite-migrations");

#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: String,
    version_number: i64,
    remote_url: String,
    default_branch: Option<String>,
    default_image: Option<String>,
    deleted: bool,
    patch_workflow: Option<String>,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct ActorRow {
    id: String,
    version_number: i64,
    auth_token_hash: String,
    auth_token_salt: String,
    actor_id: String,
    creator: Option<String>,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    version_number: i64,
    username: String,
    github_user_id: i64,
    github_token: Option<String>,
    github_refresh_token: Option<String>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct IssueRow {
    id: String,
    version_number: i64,
    issue_type: String,
    title: String,
    description: String,
    creator: String,
    progress: String,
    status: String,
    assignee: Option<String>,
    job_settings: String,
    todo_list: String,
    dependencies: String,
    patches: String,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

impl SqliteStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn init_pool(database_url: &str) -> Result<SqlitePool, anyhow::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        // Enable WAL mode for concurrent read access
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;

        Ok(pool)
    }

    pub async fn run_migrations(pool: &SqlitePool) -> Result<(), anyhow::Error> {
        MIGRATOR.run(pool).await?;
        Ok(())
    }

    async fn fetch_latest_version_number(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<VersionNumber>, StoreError> {
        let query = format!(
            "SELECT version_number FROM {table} WHERE id = ?1 ORDER BY version_number DESC LIMIT 1"
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

    async fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), StoreError> {
        let name_str = name.as_str();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_REPOSITORIES_V2} WHERE id = ?1"
        ))
        .bind(name_str)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            Err(StoreError::RepositoryNotFound(name.clone()))
        } else {
            Ok(())
        }
    }

    // ---- Repository helpers ----

    async fn insert_repository(
        &self,
        id: &str,
        version_number: VersionNumber,
        repo: &Repository,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for repository '{id}'"))
        })?;

        let patch_workflow_json = repo
            .patch_workflow
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize patch_workflow: {e}"))
            })?;

        sqlx::query(
            "INSERT INTO repositories_v2 (id, version_number, remote_url, default_branch, default_image, deleted, patch_workflow, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        )
        .bind(id)
        .bind(version_number)
        .bind(&repo.remote_url)
        .bind(repo.default_branch.as_deref())
        .bind(repo.default_image.as_deref())
        .bind(repo.deleted)
        .bind(&patch_workflow_json)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_repository(&self, row: &RepositoryRow) -> Result<Repository, StoreError> {
        let patch_workflow = row
            .patch_workflow
            .as_ref()
            .map(|v| {
                serde_json::from_str(v).map_err(|e| {
                    StoreError::Internal(format!("failed to deserialize patch_workflow: {e}"))
                })
            })
            .transpose()?;

        let mut repo = Repository::new(
            row.remote_url.clone(),
            row.default_branch.clone(),
            row.default_image.clone(),
            patch_workflow,
        );
        repo.deleted = row.deleted;
        Ok(repo)
    }

    // ---- Actor helpers ----

    async fn insert_actor(
        &self,
        id: &str,
        version_number: VersionNumber,
        actor: &Actor,
        acting_as: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for actor '{id}'"))
        })?;

        let actor_id_json = serde_json::to_string(&actor.actor_id)
            .map_err(|e| StoreError::Internal(format!("failed to serialize actor_id: {e}")))?;

        let creator_str = actor.creator.to_string();

        sqlx::query(
            "INSERT INTO actors_v2 (id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        )
        .bind(id)
        .bind(version_number)
        .bind(&actor.auth_token_hash)
        .bind(&actor.auth_token_salt)
        .bind(&actor_id_json)
        .bind(&creator_str)
        .bind(acting_as)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_actor(&self, row: &ActorRow) -> Result<Actor, StoreError> {
        let actor_id: ActorId = serde_json::from_str(&row.actor_id)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize actor_id: {e}")))?;

        Ok(Actor {
            auth_token_hash: row.auth_token_hash.clone(),
            auth_token_salt: row.auth_token_salt.clone(),
            actor_id,
            creator: Username::from(row.creator.as_deref().unwrap_or(UNKNOWN_CREATOR)),
        })
    }

    // ---- User helpers ----

    async fn insert_user(
        &self,
        id: &str,
        version_number: VersionNumber,
        user: &User,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for user '{id}'"))
        })?;

        sqlx::query(
            "INSERT INTO users_v2 (id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        )
        .bind(id)
        .bind(version_number)
        .bind(user.username.as_str())
        .bind(user.github_user_id as i64)
        .bind(&user.github_token)
        .bind(&user.github_refresh_token)
        .bind(user.deleted)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_user(&self, row: &UserRow) -> User {
        User::new(
            Username::from(row.username.clone()),
            row.github_user_id as u64,
            row.github_token.clone().unwrap_or_default(),
            row.github_refresh_token.clone().unwrap_or_default(),
            row.deleted,
        )
    }

    // ---- Issue helpers ----

    async fn ensure_issue_exists(&self, id: &IssueId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ISSUES_V2} WHERE id = ?1"
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

    async fn insert_issue(
        &self,
        id: &IssueId,
        version_number: VersionNumber,
        issue: &Issue,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let job_settings_json = serde_json::to_string(&issue.job_settings)
            .map_err(|e| StoreError::Internal(format!("failed to serialize job_settings: {e}")))?;
        let todo_list_json = serde_json::to_string(&issue.todo_list)
            .map_err(|e| StoreError::Internal(format!("failed to serialize todo_list: {e}")))?;
        let dependencies_json = serde_json::to_string(&issue.dependencies)
            .map_err(|e| StoreError::Internal(format!("failed to serialize dependencies: {e}")))?;
        let patches_json = serde_json::to_string(&issue.patches)
            .map_err(|e| StoreError::Internal(format!("failed to serialize patches: {e}")))?;

        sqlx::query(
            "INSERT INTO issues_v2 (id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(issue.issue_type.as_str())
        .bind(&issue.title)
        .bind(&issue.description)
        .bind(issue.creator.as_str())
        .bind(&issue.progress)
        .bind(issue.status.as_str())
        .bind(issue.assignee.as_deref())
        .bind(&job_settings_json)
        .bind(&todo_list_json)
        .bind(&dependencies_json)
        .bind(&patches_json)
        .bind(issue.deleted)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_issue(&self, row: &IssueRow) -> Result<Issue, StoreError> {
        let issue_type = IssueType::from_str(&row.issue_type)
            .map_err(|e| StoreError::Internal(format!("invalid issue_type: {e}")))?;
        let status = IssueStatus::from_str(&row.status).map_err(StoreError::InvalidIssueStatus)?;
        let job_settings: JobSettings = serde_json::from_str(&row.job_settings).map_err(|e| {
            StoreError::Internal(format!("failed to deserialize job_settings: {e}"))
        })?;
        let todo_list: Vec<TodoItem> = serde_json::from_str(&row.todo_list)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize todo_list: {e}")))?;
        let dependencies: Vec<IssueDependency> =
            serde_json::from_str(&row.dependencies).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize dependencies: {e}"))
            })?;
        let patches: Vec<PatchId> = serde_json::from_str(&row.patches)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize patches: {e}")))?;

        Ok(Issue {
            issue_type,
            title: row.title.clone(),
            description: row.description.clone(),
            creator: Username::from(row.creator.clone()),
            progress: row.progress.clone(),
            status,
            assignee: row.assignee.clone(),
            job_settings,
            todo_list,
            dependencies,
            patches,
            deleted: row.deleted,
        })
    }
}

fn actor_to_json_string(actor: &ActorRef) -> String {
    serde_json::to_string(actor).expect("ActorRef serialization should not fail")
}

fn parse_actor_json_string(value: Option<&str>) -> Result<Option<ActorRef>, StoreError> {
    match value {
        None => Ok(None),
        Some(v) => serde_json::from_str(v).map(Some).map_err(|e| {
            StoreError::Internal(format!("failed to parse actor JSON into ActorRef: {e}"))
        }),
    }
}

fn parse_sqlite_timestamp(s: &str) -> Result<DateTime<Utc>, StoreError> {
    // Try RFC3339/ISO8601 format first
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // Try the SQLite strftime format: "2024-01-15T12:30:45.123+00:00"
            DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%:z").map(|dt| dt.with_timezone(&Utc))
        })
        .or_else(|_| {
            // Try without timezone: "2024-01-15 12:30:45"
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(|ndt| ndt.and_utc())
        })
        .map_err(|e| StoreError::Internal(format!("failed to parse timestamp '{s}': {e}")))
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    StoreError::Internal(err.to_string())
}

#[async_trait]
impl ReadOnlyStore for SqliteStore {
    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        let name_str = name.as_str();
        let row = sqlx::query_as::<_, RepositoryRow>(
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, patch_workflow, actor, created_at, updated_at
             FROM repositories_v2
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        )
        .bind(name_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        if !include_deleted && row.deleted {
            return Err(StoreError::RepositoryNotFound(name.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for repository '{}'",
                row.id
            ))
        })?;
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let repo = self.row_to_repository(&row)?;
        Ok(Versioned::with_optional_actor(
            repo,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            timestamp,
        ))
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        // SQLite doesn't have DISTINCT ON, use a subquery instead
        let rows = sqlx::query_as::<_, RepositoryRow>(
            "SELECT r.id, r.version_number, r.remote_url, r.default_branch, r.default_image, r.deleted, r.patch_workflow, r.actor, r.created_at, r.updated_at
             FROM repositories_v2 r
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM repositories_v2 GROUP BY id) latest
             ON r.id = latest.id AND r.version_number = latest.max_vn
             ORDER BY r.id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            if !include_deleted && row.deleted {
                continue;
            }

            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for repository '{}'",
                    row.id
                ))
            })?;
            let name = RepoName::from_str(&row.id).map_err(|e| {
                StoreError::Internal(format!("invalid repository id stored in database: {e}"))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let repo = self.row_to_repository(&row)?;
            results.push((
                name,
                Versioned::with_optional_actor(
                    repo,
                    version,
                    timestamp,
                    parse_actor_json_string(row.actor.as_deref())?,
                    timestamp,
                ),
            ));
        }

        results.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(results)
    }

    async fn get_issue(
        &self,
        id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        let row = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = ?1) AS creation_time
             FROM {TABLE_ISSUES_V2}
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for issue '{}'",
                row.id
            ))
        })?;
        let issue = self.row_to_issue(&row)?;

        if !include_deleted && issue.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(timestamp);

        Ok(Versioned::with_optional_actor(
            issue,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let rows = sqlx::query_as::<_, IssueRow>(&format!(
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor, created_at, updated_at, NULL AS creation_time
             FROM {TABLE_ISSUES_V2}
             WHERE id = ?1
             ORDER BY version_number"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for issue '{}'",
                    row.id
                ))
            })?;
            let issue = self.row_to_issue(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            results.push(Versioned::with_optional_actor(
                issue,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                timestamp,
            ));
        }

        let creation_time = results.first().map(|r| r.timestamp);
        for r in &mut results {
            r.creation_time = creation_time.unwrap_or(r.timestamp);
        }

        Ok(results)
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        // SQLite doesn't have DISTINCT ON; use a subquery with MAX(version_number) instead
        let subquery = format!(
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, i.progress, i.status, i.assignee, i.job_settings, i.todo_list, i.dependencies, i.patches, i.deleted, i.actor, i.created_at, i.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = i.id) AS creation_time
             FROM {TABLE_ISSUES_V2} i
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_ISSUES_V2} GROUP BY id) latest
             ON i.id = latest.id AND i.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        if let Some(issue_type) = query.issue_type.as_ref() {
            bindings.push(issue_type.as_str().to_string());
            predicates.push(format!("issue_type = ?{}", bindings.len()));
        }

        if let Some(status) = query.status.as_ref() {
            bindings.push(status.as_str().to_string());
            predicates.push(format!("status = ?{}", bindings.len()));
        }

        if let Some(assignee) = query
            .assignee
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            bindings.push(assignee.to_lowercase());
            predicates.push(format!("LOWER(assignee) = ?{}", bindings.len()));
        }

        if let Some(term) = query
            .q
            .as_ref()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty())
        {
            let pattern = format!("%{term}%");
            let start = bindings.len() + 1;
            bindings.push(pattern.clone()); // id
            bindings.push(pattern.clone()); // title
            bindings.push(pattern.clone()); // description
            bindings.push(pattern.clone()); // progress
            bindings.push(term.clone()); // type (exact)
            bindings.push(term.clone()); // status (exact)
            bindings.push(pattern.clone()); // creator
            bindings.push(pattern); // assignee
            predicates.push(format!(
                "(LOWER(id) LIKE ?{s0} OR LOWER(title) LIKE ?{s1} OR LOWER(description) LIKE ?{s2} OR LOWER(progress) LIKE ?{s3} OR issue_type = ?{s4} OR status = ?{s5} OR LOWER(creator) LIKE ?{s6} OR LOWER(COALESCE(assignee,'')) LIKE ?{s7})",
                s0 = start,
                s1 = start + 1,
                s2 = start + 2,
                s3 = start + 3,
                s4 = start + 4,
                s5 = start + 5,
                s6 = start + 6,
                s7 = start + 7,
            ));
        }

        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = 0".to_string());
        }

        if !query.label_ids.is_empty() {
            let label_count = query.label_ids.len();
            let placeholders: Vec<String> = query
                .label_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
                .collect();
            predicates.push(format!(
                "id IN (SELECT la.object_id FROM {TABLE_LABEL_ASSOCIATIONS} la WHERE la.label_id IN ({}) GROUP BY la.object_id HAVING COUNT(DISTINCT la.label_id) = {label_count})",
                placeholders.join(", ")
            ));
            for label_id in &query.label_ids {
                bindings.push(label_id.to_string());
            }
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_as::<_, IssueRow>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut issues = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for issue '{}'",
                    row.id
                ))
            })?;
            let issue = self.row_to_issue(&row)?;
            let issue_id = row.id.parse::<IssueId>().map_err(|err| {
                StoreError::Internal(format!("invalid issue id stored in database: {err}"))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            let versioned = Versioned::with_optional_actor(
                issue,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                creation_time,
            );
            issues.push((issue_id, versioned));
        }

        Ok(issues)
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
        let query = SearchJobsQuery::new(None, Some(issue_id.clone()), None, None);
        let tasks = self.list_tasks(&query).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    async fn get_patch(
        &self,
        id: &PatchId,
        _include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        Err(StoreError::PatchNotFound(id.clone()))
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        Err(StoreError::PatchNotFound(id.clone()))
    }

    async fn list_patches(
        &self,
        _query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_issues_for_patch(&self, _patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_document(
        &self,
        id: &DocumentId,
        _include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        Err(StoreError::DocumentNotFound(id.clone()))
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        Err(StoreError::DocumentNotFound(id.clone()))
    }

    async fn list_documents(
        &self,
        _query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_documents_by_path(
        &self,
        _path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_task(
        &self,
        id: &TaskId,
        _include_deleted: bool,
    ) -> Result<Versioned<Task>, StoreError> {
        Err(StoreError::TaskNotFound(id.clone()))
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        Err(StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(
        &self,
        _query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        Err(StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        _ids: &[TaskId],
    ) -> Result<HashMap<TaskId, TaskStatusLog>, StoreError> {
        Ok(HashMap::new())
    }

    async fn count_distinct_issues(&self) -> Result<u64, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(DISTINCT id) FROM {TABLE_ISSUES_V2}"
        ))
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(count as u64)
    }

    async fn count_distinct_patches(&self) -> Result<u64, StoreError> {
        Ok(0)
    }

    async fn count_distinct_documents(&self) -> Result<u64, StoreError> {
        Ok(0)
    }

    async fn count_distinct_tasks(&self) -> Result<u64, StoreError> {
        Ok(0)
    }

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        super::validate_actor_name(name)?;
        let row = sqlx::query_as::<_, ActorRow>(
            "SELECT id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor, created_at, updated_at
             FROM actors_v2
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::ActorNotFound(name.to_string()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for actor '{}'",
                row.id
            ))
        })?;
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let actor = self.row_to_actor(&row)?;
        Ok(Versioned::with_optional_actor(
            actor,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            timestamp,
        ))
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        let rows = sqlx::query_as::<_, ActorRow>(
            "SELECT a.id, a.version_number, a.auth_token_hash, a.auth_token_salt, a.actor_id, a.creator, a.actor, a.created_at, a.updated_at
             FROM actors_v2 a
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM actors_v2 GROUP BY id) latest
             ON a.id = latest.id AND a.version_number = latest.max_vn
             ORDER BY a.id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let mut actors = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for actor '{}'",
                    row.id
                ))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let actor = self.row_to_actor(&row)?;
            actors.push((
                row.id,
                Versioned::with_optional_actor(
                    actor,
                    version,
                    timestamp,
                    parse_actor_json_string(row.actor.as_deref())?,
                    timestamp,
                ),
            ));
        }

        actors.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(actors)
    }

    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, actor, created_at, updated_at
             FROM users_v2
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::UserNotFound(username.clone()))?;
        if !include_deleted && row.deleted {
            return Err(StoreError::UserNotFound(username.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for user '{}'",
                row.id
            ))
        })?;
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let user = self.row_to_user(&row);
        Ok(Versioned::with_optional_actor(
            user,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            timestamp,
        ))
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);

        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT u.id, u.version_number, u.username, u.github_user_id, u.github_token, u.github_refresh_token, u.deleted, u.actor, u.created_at, u.updated_at
             FROM users_v2 u
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM users_v2 GROUP BY id) latest
             ON u.id = latest.id AND u.version_number = latest.max_vn
             ORDER BY u.id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            if !include_deleted && row.deleted {
                continue;
            }

            if let Some(ref term) = search_term {
                let id_lower = row.id.to_lowercase();
                let username_lower = row.username.to_lowercase();
                if !id_lower.contains(term) && !username_lower.contains(term) {
                    continue;
                }
            }

            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for user '{}'",
                    row.id
                ))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let user = self.row_to_user(&row);
            let username = Username::from(row.id);
            users.push((
                username,
                Versioned::with_optional_actor(
                    user,
                    version,
                    timestamp,
                    parse_actor_json_string(row.actor.as_deref())?,
                    timestamp,
                ),
            ));
        }

        Ok(users)
    }

    async fn get_notification(&self, id: &NotificationId) -> Result<Notification, StoreError> {
        Err(StoreError::NotificationNotFound(id.clone()))
    }

    async fn list_notifications(
        &self,
        _query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError> {
        Ok(Vec::new())
    }

    async fn count_unread_notifications(&self, _recipient: &ActorId) -> Result<u64, StoreError> {
        Ok(0)
    }

    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        Err(StoreError::MessageNotFound(id.clone()))
    }

    async fn list_messages(
        &self,
        _query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        Err(StoreError::AgentNotFound(name.to_string()))
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError> {
        Err(StoreError::LabelNotFound(id.clone()))
    }

    async fn list_labels(
        &self,
        _query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_label_by_name(&self, _name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        Ok(None)
    }

    async fn get_labels_for_object(
        &self,
        _object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_labels_for_objects(
        &self,
        _object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        Ok(HashMap::new())
    }

    async fn get_objects_for_label(&self, _label_id: &LabelId) -> Result<Vec<MetisId>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(None)
    }

    async fn list_user_secret_names(
        &self,
        _username: &Username,
    ) -> Result<Vec<String>, StoreError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn add_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError> {
        let name_str = name.as_str();

        let existing = self.get_repository(&name, true).await;

        match existing {
            Ok(repo) if repo.item.deleted => self.update_repository(name, config, actor).await,
            Ok(_) => Err(StoreError::RepositoryAlreadyExists(name)),
            Err(StoreError::RepositoryNotFound(_)) => {
                let actor_json = actor_to_json_string(actor);
                self.insert_repository(name_str.as_str(), 1, &config, Some(&actor_json))
                    .await
            }
            Err(e) => Err(e),
        }
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError> {
        let name_str = name.as_str();
        self.ensure_repository_exists(&name).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_REPOSITORIES_V2, name_str.as_str())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("repository '{name_str}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!(
                "version number overflow for repository '{name_str}'"
            ))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_repository(name_str.as_str(), next_version, &config, Some(&actor_json))
            .await
    }

    async fn delete_repository(&self, name: &RepoName, actor: &ActorRef) -> Result<(), StoreError> {
        let current = self.get_repository(name, true).await?;
        let mut repo = current.item;
        repo.deleted = true;
        self.update_repository(name.clone(), repo, actor).await
    }

    async fn add_issue(
        &self,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        let count = self.count_distinct_issues().await?;
        let id = IssueId::new_for_count(count);
        let actor_json = actor_to_json_string(actor);
        self.insert_issue(&id, 1, &issue, Some(&actor_json)).await?;
        Ok((id, 1))
    }

    async fn update_issue(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.get_issue(id, true).await?;
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_ISSUES_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("issue '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_issue(id, next_version, &issue, Some(&actor_json))
            .await?;
        Ok(next_version)
    }

    async fn delete_issue(
        &self,
        id: &IssueId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_issue(id, true).await?;
        let mut issue = current.item;
        issue.deleted = true;
        self.update_issue(id, issue, actor).await
    }

    async fn add_patch(
        &self,
        _patch: Patch,
        _actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        Err(StoreError::Internal(
            "SQLite patches not yet implemented".to_string(),
        ))
    }

    async fn update_patch(
        &self,
        id: &PatchId,
        _patch: Patch,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::PatchNotFound(id.clone()))
    }

    async fn delete_patch(
        &self,
        id: &PatchId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::PatchNotFound(id.clone()))
    }

    async fn add_document(
        &self,
        _document: Document,
        _actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        Err(StoreError::Internal(
            "SQLite documents not yet implemented".to_string(),
        ))
    }

    async fn update_document(
        &self,
        id: &DocumentId,
        _document: Document,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::DocumentNotFound(id.clone()))
    }

    async fn delete_document(
        &self,
        id: &DocumentId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::DocumentNotFound(id.clone()))
    }

    async fn add_task(
        &self,
        _task: Task,
        _creation_time: DateTime<Utc>,
        _actor: &ActorRef,
    ) -> Result<(TaskId, VersionNumber), StoreError> {
        Err(StoreError::Internal(
            "SQLite tasks not yet implemented".to_string(),
        ))
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        _task: Task,
        _actor: &ActorRef,
    ) -> Result<Versioned<Task>, StoreError> {
        Err(StoreError::TaskNotFound(metis_id.clone()))
    }

    async fn delete_task(
        &self,
        id: &TaskId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::TaskNotFound(id.clone()))
    }

    async fn add_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS_V2} WHERE id = ?1"
        ))
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists > 0 {
            return Err(StoreError::ActorAlreadyExists(name));
        }

        let acting_as_json = actor_to_json_string(acting_as);
        self.insert_actor(&name, 1, &actor, Some(&acting_as_json))
            .await
    }

    async fn update_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS_V2} WHERE id = ?1"
        ))
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            return Err(StoreError::ActorNotFound(name));
        }

        let latest_version = self
            .fetch_latest_version_number(TABLE_ACTORS_V2, &name)
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("actor '{name}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for actor '{name}'"))
        })?;

        let acting_as_json = actor_to_json_string(acting_as);
        self.insert_actor(&name, next_version, &actor, Some(&acting_as_json))
            .await
    }

    async fn add_user(&self, user: User, actor: &ActorRef) -> Result<(), StoreError> {
        let existing = sqlx::query_as::<_, UserRow>(
            "SELECT id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, actor, created_at, updated_at
             FROM users_v2
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        )
        .bind(user.username.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        match existing {
            Some(row) => {
                if row.deleted {
                    self.update_user(user, actor).await?;
                    Ok(())
                } else {
                    Err(StoreError::UserAlreadyExists(user.username.clone()))
                }
            }
            None => {
                let actor_json = actor_to_json_string(actor);
                self.insert_user(user.username.as_str(), 1, &user, Some(&actor_json))
                    .await
            }
        }
    }

    async fn update_user(
        &self,
        user: User,
        actor: &ActorRef,
    ) -> Result<Versioned<User>, StoreError> {
        let username = user.username.clone();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_USERS_V2} WHERE id = ?1"
        ))
        .bind(user.username.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists == 0 {
            return Err(StoreError::UserNotFound(username));
        }

        let latest_version = self
            .fetch_latest_version_number(TABLE_USERS_V2, user.username.as_str())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!(
                    "user '{}' was missing during update",
                    user.username.as_str()
                ))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!(
                "version number overflow for user '{}'",
                user.username.as_str()
            ))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_user(
            user.username.as_str(),
            next_version,
            &user,
            Some(&actor_json),
        )
        .await?;

        self.get_user(&username, true).await
    }

    async fn delete_user(&self, username: &Username, actor: &ActorRef) -> Result<(), StoreError> {
        let current = self.get_user(username, true).await?;
        let mut user = current.item;
        user.deleted = true;
        self.update_user(user, actor).await?;
        Ok(())
    }

    async fn insert_notification(
        &self,
        _notification: Notification,
    ) -> Result<NotificationId, StoreError> {
        Err(StoreError::Internal(
            "SQLite notifications not yet implemented".to_string(),
        ))
    }

    async fn mark_notification_read(&self, id: &NotificationId) -> Result<(), StoreError> {
        Err(StoreError::NotificationNotFound(id.clone()))
    }

    async fn mark_all_notifications_read(
        &self,
        _recipient: &ActorId,
        _before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError> {
        Ok(0)
    }

    async fn add_message(
        &self,
        _message: Message,
        _actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        Err(StoreError::Internal(
            "SQLite messages not yet implemented".to_string(),
        ))
    }

    async fn update_message(
        &self,
        id: &MessageId,
        _message: Message,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::MessageNotFound(id.clone()))
    }

    async fn add_agent(&self, _agent: Agent) -> Result<(), StoreError> {
        Err(StoreError::Internal(
            "SQLite agents not yet implemented".to_string(),
        ))
    }

    async fn update_agent(&self, _agent: Agent) -> Result<(), StoreError> {
        Err(StoreError::Internal(
            "SQLite agents not yet implemented".to_string(),
        ))
    }

    async fn delete_agent(&self, name: &str) -> Result<(), StoreError> {
        Err(StoreError::AgentNotFound(name.to_string()))
    }

    async fn add_label(&self, _label: Label) -> Result<LabelId, StoreError> {
        Err(StoreError::Internal(
            "SQLite labels not yet implemented".to_string(),
        ))
    }

    async fn update_label(&self, id: &LabelId, _label: Label) -> Result<(), StoreError> {
        Err(StoreError::LabelNotFound(id.clone()))
    }

    async fn delete_label(&self, id: &LabelId) -> Result<(), StoreError> {
        Err(StoreError::LabelNotFound(id.clone()))
    }

    async fn add_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        Err(StoreError::Internal(
            "SQLite label associations not yet implemented".to_string(),
        ))
    }

    async fn remove_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn set_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
        _encrypted_value: &[u8],
    ) -> Result<(), StoreError> {
        Err(StoreError::Internal(
            "SQLite user secrets not yet implemented".to_string(),
        ))
    }

    async fn delete_user_secret(
        &self,
        _username: &Username,
        _secret_name: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::{ActorId, ActorRef};
    use metis_common::TaskId;

    async fn create_test_store() -> SqliteStore {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();
        SqliteStore::new(pool)
    }

    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://github.com/dourolabs/metis".to_string(),
            Some("main".to_string()),
            None,
            None,
        )
    }

    fn assert_versioned<T: PartialEq + std::fmt::Debug>(
        versioned: &Versioned<T>,
        expected_item: &T,
        expected_version: VersionNumber,
    ) {
        assert_eq!(versioned.item, *expected_item);
        assert_eq!(versioned.version, expected_version);
    }

    // ---- Repository tests ----

    #[tokio::test]
    async fn repository_crud_round_trip() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.version, 1);

        let mut updated = config.clone();
        updated.default_branch = Some("develop".to_string());
        store
            .update_repository(name.clone(), updated.clone(), &ActorRef::test())
            .await
            .unwrap();

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

    #[tokio::test]
    async fn add_repository_rejects_duplicates() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/metis").unwrap();

        store
            .add_repository(name.clone(), sample_repository_config(), &ActorRef::test())
            .await
            .unwrap();

        let err = store
            .add_repository(name.clone(), sample_repository_config(), &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::RepositoryAlreadyExists(existing) if existing == name
        ));

        let missing_name = RepoName::from_str("dourolabs/other").unwrap();
        let err = store
            .update_repository(
                missing_name.clone(),
                sample_repository_config(),
                &ActorRef::test(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(existing) if existing == missing_name
        ));
    }

    #[tokio::test]
    async fn delete_repository_soft_deletes() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        let err = store.get_repository(&name, false).await.unwrap_err();
        assert!(matches!(err, StoreError::RepositoryNotFound(_)));

        let fetched = store.get_repository(&name, true).await.unwrap();
        assert!(fetched.item.deleted);
        assert_eq!(fetched.version, 2);

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert!(list.is_empty());

        let query = SearchRepositoriesQuery::new(Some(true));
        let list = store.list_repositories(&query).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_recreates_over_soft_deleted_repo() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        let mut new_config = config.clone();
        new_config.default_branch = Some("develop".to_string());
        new_config.deleted = false;
        store
            .add_repository(name.clone(), new_config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert!(!fetched.item.deleted);
        assert_eq!(fetched.item.default_branch, Some("develop".to_string()));
        assert_eq!(fetched.version, 3);

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_respects_caller_deleted_field() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        let mut new_config = config.clone();
        new_config.default_branch = Some("develop".to_string());
        new_config.deleted = true;
        store
            .add_repository(name.clone(), new_config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, true).await.unwrap();
        assert!(fetched.item.deleted);
        assert_eq!(fetched.item.default_branch, Some("develop".to_string()));
        assert_eq!(fetched.version, 3);

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn delete_repository_not_found_error() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/nonexistent").unwrap();

        let err = store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(n) if n == name
        ));
    }

    // ---- Actor tests ----

    #[tokio::test]
    async fn add_and_get_actor_by_name() {
        let store = create_test_store().await;
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Username(Username::from("ada").into()),
            creator: Username::from("ada"),
        };

        let name = actor.name();
        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_actor(&name).await.unwrap();
        assert_eq!(fetched.item, actor);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn add_actor_rejects_duplicate_name() {
        let store = create_test_store().await;
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Task(TaskId::new()),
            creator: Username::from("creator"),
        };
        let name = actor.name();

        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();
        let err = store.add_actor(actor, &ActorRef::test()).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorAlreadyExists(existing) if existing == name
        ));
    }

    #[tokio::test]
    async fn update_actor_overwrites_existing_entry() {
        let store = create_test_store().await;
        let task_id = TaskId::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Task(task_id),
            creator: Username::from("creator"),
        };
        let mut updated = actor.clone();
        updated.auth_token_hash = "new-hash".to_string();

        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .update_actor(updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_actor(&updated.name()).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);
    }

    #[tokio::test]
    async fn update_actor_missing_returns_not_found() {
        let store = create_test_store().await;
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Username(Username::from("ada").into()),
            creator: Username::from("ada"),
        };

        let err = store
            .update_actor(actor, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorNotFound(name) if name == "u-ada"
        ));
    }

    #[tokio::test]
    async fn get_actor_missing_returns_not_found() {
        let store = create_test_store().await;
        let task_id = TaskId::new();
        let name = format!("w-{task_id}");

        let err = store.get_actor(&name).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorNotFound(missing) if missing == name
        ));
    }

    #[tokio::test]
    async fn get_actor_invalid_name_returns_error() {
        let store = create_test_store().await;

        let err = store.get_actor("u-").await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidActorName(name) if name == "u-"
        ));
    }

    #[tokio::test]
    async fn list_actors_returns_all() {
        let store = create_test_store().await;
        let actor1 = Actor {
            auth_token_hash: "hash1".to_string(),
            auth_token_salt: "salt1".to_string(),
            actor_id: ActorId::Username(Username::from("alice").into()),
            creator: Username::from("alice"),
        };
        let actor2 = Actor {
            auth_token_hash: "hash2".to_string(),
            auth_token_salt: "salt2".to_string(),
            actor_id: ActorId::Username(Username::from("bob").into()),
            creator: Username::from("bob"),
        };

        store
            .add_actor(actor1.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .add_actor(actor2.clone(), &ActorRef::test())
            .await
            .unwrap();

        let actors = store.list_actors().await.unwrap();
        assert_eq!(actors.len(), 2);
        assert_eq!(actors[0].1.item, actor1);
        assert_eq!(actors[1].1.item, actor2);
    }

    // ---- User tests ----

    #[tokio::test]
    async fn user_crud_round_trip() {
        let store = create_test_store().await;
        let username = Username::from("alice");

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 101,
                    github_token: "token".to_string(),
                    github_refresh_token: "refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_user(&username, false).await.unwrap();
        assert_eq!(fetched.item.username, username);
        assert_eq!(fetched.item.github_user_id, 101);
        assert_eq!(fetched.version, 1);

        let users = store
            .list_users(&SearchUsersQuery::default())
            .await
            .unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].0, username);
    }

    #[tokio::test]
    async fn update_user_overwrites_existing_value() {
        let store = create_test_store().await;
        let username = Username::from("alice");

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 101,
                    github_token: "old-token".to_string(),
                    github_refresh_token: "old-refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let updated = store
            .update_user(
                User {
                    username: username.clone(),
                    github_user_id: 202,
                    github_token: "new-token".to_string(),
                    github_refresh_token: "new-refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        assert_eq!(updated.item.github_token, "new-token");
        assert_eq!(updated.item.github_user_id, 202);
        assert_eq!(updated.item.github_refresh_token, "new-refresh");
        assert_eq!(updated.version, 2);

        let user = store.get_user(&username, false).await.unwrap();
        assert_eq!(user.item.github_token, "new-token");
        assert_eq!(user.item.github_user_id, 202);
        assert_eq!(user.item.github_refresh_token, "new-refresh");
        assert_eq!(user.version, 2);
    }

    #[tokio::test]
    async fn get_user_filters_deleted_users() {
        let store = create_test_store().await;
        let username = Username::from("alice");
        let user = User {
            username: username.clone(),
            github_user_id: 101,
            github_token: "token".to_string(),
            github_refresh_token: "refresh".to_string(),
            deleted: false,
        };
        store.add_user(user, &ActorRef::test()).await.unwrap();

        let fetched = store.get_user(&username, false).await.unwrap();
        assert_eq!(fetched.item.username, username);

        store
            .delete_user(&username, &ActorRef::test())
            .await
            .unwrap();

        let err = store.get_user(&username, false).await.unwrap_err();
        assert!(matches!(err, StoreError::UserNotFound(_)));

        let fetched = store.get_user(&username, true).await.unwrap();
        assert_eq!(fetched.item.username, username);
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn add_user_rejects_duplicates() {
        let store = create_test_store().await;
        let username = Username::from("alice");

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 101,
                    github_token: "token".to_string(),
                    github_refresh_token: "refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let err = store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 202,
                    github_token: "token2".to_string(),
                    github_refresh_token: "refresh2".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::UserAlreadyExists(existing) if existing == username
        ));
    }

    #[tokio::test]
    async fn add_user_undeletes_soft_deleted_user() {
        let store = create_test_store().await;
        let username = Username::from("alice");

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 101,
                    github_token: "token".to_string(),
                    github_refresh_token: "refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        store
            .delete_user(&username, &ActorRef::test())
            .await
            .unwrap();

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 303,
                    github_token: "new-token".to_string(),
                    github_refresh_token: "new-refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_user(&username, false).await.unwrap();
        assert!(!fetched.item.deleted);
        assert_eq!(fetched.item.github_user_id, 303);
        assert_eq!(fetched.version, 3);
    }

    #[tokio::test]
    async fn list_users_filters_deleted() {
        let store = create_test_store().await;
        let alice = Username::from("alice");
        let bob = Username::from("bob");

        store
            .add_user(
                User {
                    username: alice.clone(),
                    github_user_id: 101,
                    github_token: "t1".to_string(),
                    github_refresh_token: "r1".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        store
            .add_user(
                User {
                    username: bob.clone(),
                    github_user_id: 202,
                    github_token: "t2".to_string(),
                    github_refresh_token: "r2".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        store.delete_user(&alice, &ActorRef::test()).await.unwrap();

        let users = store
            .list_users(&SearchUsersQuery::default())
            .await
            .unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].0, bob);

        let query = SearchUsersQuery::new(None, Some(true));
        let users = store.list_users(&query).await.unwrap();
        assert_eq!(users.len(), 2);
    }

    // ---- Issue helpers ----

    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "issue details".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    // ---- Issue tests ----

    #[tokio::test]
    async fn issue_crud_round_trip() {
        let store = create_test_store().await;

        let (issue_id, version) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "Test Title");
        assert_eq!(fetched.item.description, "issue details");
        assert_eq!(fetched.version, 1);

        let issues = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, issue_id);
    }

    #[tokio::test]
    async fn issue_versions_increment_and_latest_returned() {
        let store = create_test_store().await;

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_issue(vec![]);
        updated.description = "updated details".to_string();
        let v2 = store
            .update_issue(&issue_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.description, "updated details");
        assert_eq!(fetched.version, 2);

        let versions = store.get_issue_versions(&issue_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[0].item.description, "issue details");
        assert_eq!(versions[1].item.description, "updated details");
    }

    #[tokio::test]
    async fn delete_issue_soft_deletes() {
        let store = create_test_store().await;

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        let err = store.get_issue(&issue_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));

        let fetched = store.get_issue(&issue_id, true).await.unwrap();
        assert!(fetched.item.deleted);
        assert_eq!(fetched.version, 2);

        let list = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert!(list.is_empty());

        let mut query_deleted = SearchIssuesQuery::default();
        query_deleted.include_deleted = Some(true);
        let list = store.list_issues(&query_deleted).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_issue_rejects_missing_dependencies() {
        let store = create_test_store().await;
        let missing_dependency = IssueId::new();

        let err = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    missing_dependency.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidDependency(id) if id == missing_dependency
        ));
    }

    #[tokio::test]
    async fn issue_dependency_indexes_populated_on_create() {
        let store = create_test_store().await;

        let (parent_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (blocker_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let (child_id, _) = store
            .add_issue(
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
                    IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone()),
                ]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        assert_eq!(
            store.get_issue_children(&parent_id).await.unwrap(),
            vec![child_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&blocker_id).await.unwrap(),
            vec![child_id]
        );
    }

    #[tokio::test]
    async fn graph_filter_returns_children() {
        let store = create_test_store().await;

        let (parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (child, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (_grandchild, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    child.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{parent}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([child]));
    }

    #[tokio::test]
    async fn count_distinct_issues_increments() {
        let store = create_test_store().await;

        assert_eq!(store.count_distinct_issues().await.unwrap(), 0);

        store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(store.count_distinct_issues().await.unwrap(), 1);

        store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(store.count_distinct_issues().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn list_issues_filters_by_status() {
        let store = create_test_store().await;

        let (id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut closed_issue = sample_issue(vec![]);
        closed_issue.status = IssueStatus::Closed;
        store
            .add_issue(closed_issue, &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchIssuesQuery::default();
        query.status = Some(IssueStatus::Open.into());
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
    }

    #[tokio::test]
    async fn list_issues_text_search() {
        let store = create_test_store().await;

        let mut special = sample_issue(vec![]);
        special.title = "Special Needle Title".to_string();
        store.add_issue(special, &ActorRef::test()).await.unwrap();

        store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchIssuesQuery::default();
        query.q = Some("needle".to_string());
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.title, "Special Needle Title");
    }

    #[tokio::test]
    async fn get_issue_not_found() {
        let store = create_test_store().await;
        let missing = IssueId::new();
        let err = store.get_issue(&missing, false).await.unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));
    }

    #[tokio::test]
    async fn update_issue_not_found() {
        let store = create_test_store().await;
        let missing = IssueId::new();
        let err = store
            .update_issue(&missing, sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));
    }

    #[tokio::test]
    async fn get_issue_children_not_found() {
        let store = create_test_store().await;
        let missing = IssueId::new();
        let err = store.get_issue_children(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));
    }
}
