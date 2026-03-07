use crate::domain::{
    actors::{Actor, ActorId, ActorRef, UNKNOWN_CREATOR},
    agents::Agent,
    documents::Document,
    issues::{Issue, IssueGraphFilter},
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::Patch,
    users::{User, Username},
};
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
        _include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        Err(StoreError::IssueNotFound(id.clone()))
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        Err(StoreError::IssueNotFound(id.clone()))
    }

    async fn list_issues(
        &self,
        _query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        Ok(Vec::new())
    }

    async fn search_issue_graph(
        &self,
        _filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        Ok(HashSet::new())
    }

    async fn get_issue_children(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_issue_blocked_on(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        Ok(Vec::new())
    }

    async fn get_tasks_for_issue(&self, _issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        Ok(Vec::new())
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
        Ok(0)
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
        _issue: Issue,
        _actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        Err(StoreError::Internal(
            "SQLite issues not yet implemented".to_string(),
        ))
    }

    async fn update_issue(
        &self,
        id: &IssueId,
        _issue: Issue,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::IssueNotFound(id.clone()))
    }

    async fn delete_issue(
        &self,
        id: &IssueId,
        _actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        Err(StoreError::IssueNotFound(id.clone()))
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
    ) -> Result<(), StoreError> {
        Err(StoreError::Internal(
            "SQLite label associations not yet implemented".to_string(),
        ))
    }

    async fn remove_label_association(
        &self,
        _label_id: &LabelId,
        _object_id: &MetisId,
    ) -> Result<(), StoreError> {
        Ok(())
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
}
