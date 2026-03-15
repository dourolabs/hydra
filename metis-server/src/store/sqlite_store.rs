use crate::domain::{
    actors::{Actor, ActorId, ActorRef, UNKNOWN_CREATOR},
    agents::Agent,
    documents::Document,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
        SessionSettings, TodoItem,
    },
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::{CommitRange, GithubPr, Patch, PatchStatus, Review},
    secrets::SecretRef,
    users::{User, Username},
};
use crate::store::issue_graph::IssueGraphContext;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::pagination::{DecodedCursor, MAX_LIMIT as PAGINATION_MAX_LIMIT};
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::sessions::SearchSessionsQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, SessionId,
    VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    repositories::{Repository, SearchRepositoriesQuery},
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use super::{ReadOnlyStore, Session, Status, Store, StoreError, TaskError, TaskStatusLog};

const TABLE_REPOSITORIES_V2: &str = "repositories_v2";
const TABLE_ACTORS_V2: &str = "actors_v2";
const TABLE_USERS_V2: &str = "users_v2";
const TABLE_ISSUES_V2: &str = "issues_v2";
const TABLE_PATCHES_V2: &str = "patches_v2";
const TABLE_DOCUMENTS_V2: &str = "documents_v2";
const TABLE_TASKS_V2: &str = "tasks_v2";
const TABLE_AGENTS: &str = "agents";
const TABLE_LABELS: &str = "labels";
const TABLE_LABEL_ASSOCIATIONS: &str = "label_associations";
const TABLE_NOTIFICATIONS: &str = "notifications";
const TABLE_MESSAGES_V2: &str = "messages_v2";
const TABLE_USER_SECRETS: &str = "user_secrets";
const TABLE_OBJECT_RELATIONSHIPS: &str = "object_relationships";

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
    github_user_id: Option<i64>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct ObjectRelationshipRow {
    source_id: String,
    source_kind: String,
    target_id: String,
    target_kind: String,
    rel_type: String,
}

fn parse_relationship_row(
    r: ObjectRelationshipRow,
) -> Result<super::ObjectRelationship, StoreError> {
    let source_id: MetisId = r.source_id.parse().map_err(|_| {
        StoreError::Internal("invalid source_id in object_relationships".to_string())
    })?;
    let target_id: MetisId = r.target_id.parse().map_err(|_| {
        StoreError::Internal("invalid target_id in object_relationships".to_string())
    })?;
    let source_kind = super::ObjectKind::from_str(&r.source_kind).map_err(|e| {
        StoreError::Internal(format!("invalid source_kind in object_relationships: {e}"))
    })?;
    let target_kind = super::ObjectKind::from_str(&r.target_kind).map_err(|e| {
        StoreError::Internal(format!("invalid target_kind in object_relationships: {e}"))
    })?;
    let rel_type = super::RelationshipType::from_str(&r.rel_type).map_err(|e| {
        StoreError::Internal(format!("invalid rel_type in object_relationships: {e}"))
    })?;
    Ok(super::ObjectRelationship {
        source_id,
        source_kind,
        target_id,
        target_kind,
        rel_type,
    })
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
    #[sqlx(rename = "job_settings")]
    session_settings: String,
    todo_list: String,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PatchRow {
    id: String,
    version_number: i64,
    title: String,
    description: String,
    diff: String,
    status: String,
    is_automatic_backup: bool,
    created_by: Option<String>,
    creator: Option<String>,
    base_branch: Option<String>,
    branch_name: Option<String>,
    commit_range: Option<String>,
    reviews: String,
    service_repo_name: String,
    github: Option<String>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    version_number: i64,
    title: String,
    body_markdown: String,
    path: Option<String>,
    created_by: Option<String>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: String,
    version_number: i64,
    prompt: String,
    context: String,
    spawned_from: Option<String>,
    image: Option<String>,
    model: Option<String>,
    env_vars: String,
    cpu_limit: Option<String>,
    memory_limit: Option<String>,
    status: String,
    last_message: Option<String>,
    error: Option<String>,
    secrets: Option<String>,
    creator: Option<String>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
    #[sqlx(default)]
    start_time: Option<String>,
    #[sqlx(default)]
    end_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    name: String,
    prompt_path: String,
    max_tries: i32,
    max_simultaneous: i32,
    is_assignment_agent: bool,
    deleted: bool,
    created_at: String,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct LabelRow {
    id: String,
    name: String,
    color: String,
    deleted: bool,
    recurse: bool,
    hidden: bool,
    created_at: String,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct NotificationRow {
    id: String,
    recipient: String,
    source_actor: Option<String>,
    object_kind: String,
    object_id: String,
    object_version: i64,
    event_type: String,
    summary: String,
    source_issue_id: Option<String>,
    policy: String,
    is_read: bool,
    created_at: String,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: String,
    version_number: i64,
    sender: Option<String>,
    recipient: String,
    body: String,
    is_read: bool,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

fn row_to_agent(row: AgentRow) -> Result<Agent, StoreError> {
    let created_at = parse_sqlite_timestamp(&row.created_at)?;
    let updated_at = parse_sqlite_timestamp(&row.updated_at)?;
    Ok(Agent {
        name: row.name,
        prompt_path: row.prompt_path,
        max_tries: row.max_tries,
        max_simultaneous: row.max_simultaneous,
        is_assignment_agent: row.is_assignment_agent,
        deleted: row.deleted,
        created_at,
        updated_at,
    })
}

fn row_to_label(row: &LabelRow) -> Result<Label, StoreError> {
    let color = row
        .color
        .parse()
        .map_err(|err| StoreError::Internal(format!("invalid label color in database: {err}")))?;
    let created_at = parse_sqlite_timestamp(&row.created_at)?;
    let updated_at = parse_sqlite_timestamp(&row.updated_at)?;
    Ok(Label {
        name: row.name.clone(),
        color,
        deleted: row.deleted,
        recurse: row.recurse,
        hidden: row.hidden,
        created_at,
        updated_at,
    })
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
            "INSERT INTO users_v2 (id, version_number, username, github_user_id, deleted, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(version_number)
        .bind(user.username.as_str())
        .bind(user.github_user_id.map(|id| id as i64))
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
            row.github_user_id.map(|id| id as u64),
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

    async fn insert_issue_in_tx<'e, E>(
        executor: E,
        id: &IssueId,
        version_number: VersionNumber,
        issue: &Issue,
        actor: Option<&str>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let session_settings_json =
            serde_json::to_string(&issue.session_settings).map_err(|e| {
                StoreError::Internal(format!("failed to serialize session_settings: {e}"))
            })?;
        let todo_list_json = serde_json::to_string(&issue.todo_list)
            .map_err(|e| StoreError::Internal(format!("failed to serialize todo_list: {e}")))?;
        sqlx::query(
            "INSERT INTO issues_v2 (id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, deleted, actor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
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
        .bind(&session_settings_json)
        .bind(&todo_list_json)
        .bind(issue.deleted)
        .bind(actor)
        .execute(executor)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    /// Syncs the object_relationships table for the given issue within a transaction.
    async fn sync_issue_relationships_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> Result<(), StoreError> {
        // Delete all existing relationships for this issue
        let delete_sql = format!("DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} WHERE source_id = ?1");
        sqlx::query(&delete_sql)
            .bind(issue_id.as_ref())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx_error)?;

        // Insert dependency relationships
        for dep in &issue.dependencies {
            let rel_type = super::RelationshipType::from(dep.dependency_type);
            sqlx::query(
                "INSERT OR IGNORE INTO object_relationships \
                 (source_id, source_kind, target_id, target_kind, rel_type) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(issue_id.as_ref())
            .bind(super::ObjectKind::Issue.as_str())
            .bind(dep.issue_id.as_ref())
            .bind(super::ObjectKind::Issue.as_str())
            .bind(rel_type.as_str())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx_error)?;
        }

        // Insert patch relationships
        for patch_id in &issue.patches {
            sqlx::query(
                "INSERT OR IGNORE INTO object_relationships \
                 (source_id, source_kind, target_id, target_kind, rel_type) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(issue_id.as_ref())
            .bind(super::ObjectKind::Issue.as_str())
            .bind(patch_id.as_ref())
            .bind(super::ObjectKind::Patch.as_str())
            .bind(super::RelationshipType::HasPatch.as_str())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx_error)?;
        }

        Ok(())
    }

    // ---- Patch helpers ----

    async fn ensure_patch_exists(&self, id: &PatchId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_PATCHES_V2} WHERE id = ?1"
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

    async fn insert_patch(
        &self,
        id: &PatchId,
        version_number: VersionNumber,
        patch: &Patch,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for patch '{id}'"))
        })?;

        let reviews_json = serde_json::to_string(&patch.reviews)
            .map_err(|e| StoreError::Internal(format!("failed to serialize reviews: {e}")))?;
        let github_json = patch
            .github
            .as_ref()
            .map(|g| {
                serde_json::to_string(g)
                    .map_err(|e| StoreError::Internal(format!("failed to serialize github: {e}")))
            })
            .transpose()?;
        let commit_range_json = patch
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::to_string(cr).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize commit_range: {e}"))
                })
            })
            .transpose()?;

        sqlx::query(
            &format!(
                "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, created_by, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"
            )
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&patch.title)
        .bind(&patch.description)
        .bind(&patch.diff)
        .bind(patch.status.as_str())
        .bind(patch.is_automatic_backup)
        .bind(patch.created_by.as_ref().map(|t| t.as_ref()))
        .bind(patch.creator.as_str())
        .bind(patch.base_branch.as_deref())
        .bind(patch.branch_name.as_deref())
        .bind(&commit_range_json)
        .bind(&reviews_json)
        .bind(patch.service_repo_name.as_str())
        .bind(&github_json)
        .bind(patch.deleted)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_patch(&self, row: &PatchRow) -> Result<Patch, StoreError> {
        let status = PatchStatus::from_str(&row.status)
            .map_err(|e| StoreError::Internal(format!("invalid patch status: {e}")))?;
        let reviews: Vec<Review> = serde_json::from_str(&row.reviews)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize reviews: {e}")))?;
        let github: Option<GithubPr> = row
            .github
            .as_ref()
            .map(|g| {
                serde_json::from_str(g)
                    .map_err(|e| StoreError::Internal(format!("failed to deserialize github: {e}")))
            })
            .transpose()?;
        let service_repo_name = RepoName::from_str(&row.service_repo_name)
            .map_err(|e| StoreError::Internal(format!("invalid service_repo_name: {e}")))?;
        let created_by = row
            .created_by
            .as_ref()
            .map(|s| {
                SessionId::from_str(s)
                    .map_err(|e| StoreError::Internal(format!("invalid created_by task id: {e}")))
            })
            .transpose()?;
        let commit_range: Option<CommitRange> = row
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::from_str(cr).map_err(|e| {
                    StoreError::Internal(format!("failed to deserialize commit_range: {e}"))
                })
            })
            .transpose()?;
        let creator = Username::from(row.creator.as_deref().unwrap_or(UNKNOWN_CREATOR));

        Ok(Patch {
            title: row.title.clone(),
            description: row.description.clone(),
            diff: row.diff.clone(),
            status,
            is_automatic_backup: row.is_automatic_backup,
            created_by,
            creator,
            reviews,
            service_repo_name,
            github,
            deleted: row.deleted,
            branch_name: row.branch_name.clone(),
            commit_range,
            base_branch: row.base_branch.clone(),
        })
    }

    // ---- Document helpers ----

    async fn insert_document(
        &self,
        id: &DocumentId,
        version_number: VersionNumber,
        document: &Document,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        sqlx::query(
            &format!(
                "INSERT INTO {TABLE_DOCUMENTS_V2} (id, version_number, title, body_markdown, path, created_by, deleted, actor)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            )
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&document.title)
        .bind(&document.body_markdown)
        .bind(document.path.as_ref().map(|p| p.as_str()))
        .bind(document.created_by.as_ref().map(|t| t.as_ref()))
        .bind(document.deleted)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_document(&self, row: &DocumentRow) -> Result<Document, StoreError> {
        let created_by = row
            .created_by
            .as_ref()
            .map(|s| {
                SessionId::from_str(s)
                    .map_err(|e| StoreError::Internal(format!("invalid created_by task id: {e}")))
            })
            .transpose()?;
        let path = row
            .path
            .as_ref()
            .map(|s| {
                s.parse()
                    .map_err(|e| StoreError::Internal(format!("invalid document path: {e}")))
            })
            .transpose()?;

        Ok(Document {
            title: row.title.clone(),
            body_markdown: row.body_markdown.clone(),
            path,
            created_by,
            deleted: row.deleted,
        })
    }

    // ---- Task helpers ----

    async fn insert_task(
        &self,
        id: &SessionId,
        version_number: VersionNumber,
        session: &Session,
        actor: Option<&str>,
        created_at: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for task '{id}'"))
        })?;

        let context_json = serde_json::to_string(&session.context)
            .map_err(|e| StoreError::Internal(format!("failed to serialize context: {e}")))?;
        let env_vars_json = serde_json::to_string(&session.env_vars)
            .map_err(|e| StoreError::Internal(format!("failed to serialize env_vars: {e}")))?;
        let error_json = session
            .error
            .as_ref()
            .map(|e| {
                serde_json::to_string(e).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize error: {err}"))
                })
            })
            .transpose()?;
        let secrets_json = session
            .secrets
            .as_ref()
            .map(|s| {
                serde_json::to_string(s).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize secrets: {err}"))
                })
            })
            .transpose()?;
        let status_str = super::status_to_db_str(session.status);
        let creation_time_str = session.creation_time.map(|t| t.to_rfc3339());
        let start_time_str = session.start_time.map(|t| t.to_rfc3339());
        let end_time_str = session.end_time.map(|t| t.to_rfc3339());

        if let Some(ts) = created_at {
            sqlx::query(
                &format!(
                    "INSERT INTO {TABLE_TASKS_V2} (id, version_number, prompt, context, spawned_from, creator, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets, created_at, creation_time, start_time, end_time)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)"
                )
            )
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&session.prompt)
            .bind(&context_json)
            .bind(session.spawned_from.as_ref().map(|i| i.as_ref()))
            .bind(session.creator.as_str())
            .bind(session.image.as_deref())
            .bind(session.model.as_deref())
            .bind(&env_vars_json)
            .bind(session.cpu_limit.as_deref())
            .bind(session.memory_limit.as_deref())
            .bind(status_str)
            .bind(session.last_message.as_deref())
            .bind(&error_json)
            .bind(session.deleted)
            .bind(actor)
            .bind(&secrets_json)
            .bind(ts)
            .bind(creation_time_str.as_deref())
            .bind(start_time_str.as_deref())
            .bind(end_time_str.as_deref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        } else {
            sqlx::query(
                &format!(
                    "INSERT INTO {TABLE_TASKS_V2} (id, version_number, prompt, context, spawned_from, creator, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets, creation_time, start_time, end_time)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)"
                )
            )
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&session.prompt)
            .bind(&context_json)
            .bind(session.spawned_from.as_ref().map(|i| i.as_ref()))
            .bind(session.creator.as_str())
            .bind(session.image.as_deref())
            .bind(session.model.as_deref())
            .bind(&env_vars_json)
            .bind(session.cpu_limit.as_deref())
            .bind(session.memory_limit.as_deref())
            .bind(status_str)
            .bind(session.last_message.as_deref())
            .bind(&error_json)
            .bind(session.deleted)
            .bind(actor)
            .bind(&secrets_json)
            .bind(creation_time_str.as_deref())
            .bind(start_time_str.as_deref())
            .bind(end_time_str.as_deref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        }

        Ok(())
    }

    fn row_to_session(&self, row: &TaskRow) -> Result<Session, StoreError> {
        let context = serde_json::from_str(&row.context)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize context: {e}")))?;
        let env_vars: HashMap<String, String> = serde_json::from_str(&row.env_vars)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize env_vars: {e}")))?;
        let error: Option<TaskError> = row
            .error
            .as_ref()
            .map(|e| {
                serde_json::from_str(e).map_err(|err| {
                    StoreError::Internal(format!("failed to deserialize error: {err}"))
                })
            })
            .transpose()?;
        let secrets: Option<Vec<String>> = row
            .secrets
            .as_ref()
            .map(|s| {
                serde_json::from_str(s).map_err(|err| {
                    StoreError::Internal(format!("failed to deserialize secrets: {err}"))
                })
            })
            .transpose()?;
        let spawned_from = row
            .spawned_from
            .as_ref()
            .map(|s| {
                IssueId::from_str(s).map_err(|e| {
                    StoreError::Internal(format!("invalid spawned_from issue id: {e}"))
                })
            })
            .transpose()?;
        let status = match row.status.as_str() {
            "created" => Status::Created,
            "pending" => Status::Pending,
            "running" => Status::Running,
            "complete" => Status::Complete,
            "failed" => Status::Failed,
            other => {
                return Err(StoreError::Internal(format!(
                    "invalid task status: {other}"
                )));
            }
        };
        let creator = Username::from(row.creator.as_deref().unwrap_or(UNKNOWN_CREATOR));

        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?;
        let start_time = row
            .start_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?;
        let end_time = row
            .end_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?;

        Ok(Session {
            prompt: row.prompt.clone(),
            context,
            spawned_from,
            creator,
            image: row.image.clone(),
            model: row.model.clone(),
            env_vars,
            cpu_limit: row.cpu_limit.clone(),
            memory_limit: row.memory_limit.clone(),
            secrets,
            status,
            last_message: row.last_message.clone(),
            error,
            deleted: row.deleted,
            creation_time,
            start_time,
            end_time,
        })
    }

    fn row_to_session_id(id: &str) -> Result<SessionId, StoreError> {
        id.parse::<SessionId>().map_err(|err| {
            StoreError::Internal(format!("invalid session id stored in database: {err}"))
        })
    }

    fn row_to_versioned_session(&self, row: &TaskRow) -> Result<Versioned<Session>, StoreError> {
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for session '{}'",
                row.id
            ))
        })?;
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(timestamp);
        let task = self.row_to_session(row)?;
        Ok(Versioned::with_optional_actor(
            task,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    fn row_to_issue(&self, row: &IssueRow) -> Result<Issue, StoreError> {
        let issue_type = IssueType::from_str(&row.issue_type)
            .map_err(|e| StoreError::Internal(format!("invalid issue_type: {e}")))?;
        let status = IssueStatus::from_str(&row.status).map_err(StoreError::InvalidIssueStatus)?;
        let session_settings: SessionSettings = serde_json::from_str(&row.session_settings)
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize session_settings: {e}"))
            })?;
        let todo_list: Vec<TodoItem> = serde_json::from_str(&row.todo_list)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize todo_list: {e}")))?;
        Ok(Issue {
            issue_type,
            title: row.title.clone(),
            description: row.description.clone(),
            creator: Username::from(row.creator.clone()),
            progress: row.progress.clone(),
            status,
            assignee: row.assignee.clone(),
            session_settings,
            todo_list,
            dependencies: vec![],
            patches: vec![],
            deleted: row.deleted,
        })
    }

    /// Populates Issue.dependencies and Issue.patches from the object_relationships
    /// table for a single issue.
    async fn populate_issue_relationships(
        &self,
        issue_id: &IssueId,
        issue: &mut Issue,
    ) -> Result<(), StoreError> {
        let sql = format!(
            "SELECT target_id, rel_type FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = ?1 AND source_kind = 'issue'"
        );
        let rows = sqlx::query_as::<_, (String, String)>(&sql)
            .bind(issue_id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut dependencies = Vec::new();
        let mut patches = Vec::new();

        for (target_id, rel_type) in rows {
            match rel_type.as_str() {
                "child-of" => {
                    let id = target_id.parse::<IssueId>().map_err(|err| {
                        StoreError::Internal(format!(
                            "invalid issue id in object_relationships: {err}"
                        ))
                    })?;
                    dependencies.push(IssueDependency::new(IssueDependencyType::ChildOf, id));
                }
                "blocked-on" => {
                    let id = target_id.parse::<IssueId>().map_err(|err| {
                        StoreError::Internal(format!(
                            "invalid issue id in object_relationships: {err}"
                        ))
                    })?;
                    dependencies.push(IssueDependency::new(IssueDependencyType::BlockedOn, id));
                }
                "has-patch" => {
                    let id = target_id.parse::<PatchId>().map_err(|err| {
                        StoreError::Internal(format!(
                            "invalid patch id in object_relationships: {err}"
                        ))
                    })?;
                    patches.push(id);
                }
                _ => {}
            }
        }

        issue.dependencies = dependencies;
        issue.patches = patches;
        Ok(())
    }

    /// Populates Issue.dependencies and Issue.patches from the object_relationships
    /// table for a batch of issues.
    async fn populate_issues_relationships(
        &self,
        issues: &mut [(IssueId, Versioned<Issue>)],
    ) -> Result<(), StoreError> {
        if issues.is_empty() {
            return Ok(());
        }

        let ids: Vec<&str> = issues.iter().map(|(id, _)| id.as_ref()).collect();
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT source_id, target_id, rel_type FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id IN ({}) AND source_kind = 'issue'",
            placeholders.join(", ")
        );

        let mut query = sqlx::query_as::<_, (String, String, String)>(&sql);
        for id in &ids {
            query = query.bind(*id);
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;

        let mut deps_map: HashMap<String, Vec<IssueDependency>> = HashMap::new();
        let mut patches_map: HashMap<String, Vec<PatchId>> = HashMap::new();

        for (source_id, target_id, rel_type) in rows {
            match rel_type.as_str() {
                "child-of" => {
                    if let Ok(id) = target_id.parse::<IssueId>() {
                        deps_map
                            .entry(source_id)
                            .or_default()
                            .push(IssueDependency::new(IssueDependencyType::ChildOf, id));
                    }
                }
                "blocked-on" => {
                    if let Ok(id) = target_id.parse::<IssueId>() {
                        deps_map
                            .entry(source_id)
                            .or_default()
                            .push(IssueDependency::new(IssueDependencyType::BlockedOn, id));
                    }
                }
                "has-patch" => {
                    if let Ok(id) = target_id.parse::<PatchId>() {
                        patches_map.entry(source_id).or_default().push(id);
                    }
                }
                _ => {}
            }
        }

        for (issue_id, versioned) in issues.iter_mut() {
            let id_str = issue_id.as_ref().to_string();
            versioned.item.dependencies = deps_map.remove(&id_str).unwrap_or_default();
            versioned.item.patches = patches_map.remove(&id_str).unwrap_or_default();
        }

        Ok(())
    }

    /// Builds an IssueGraphContext from the object_relationships table
    /// and known issue IDs, avoiding the need to load all issue data.
    async fn build_issue_graph_from_relationships(&self) -> Result<IssueGraphContext, StoreError> {
        let issue_ids_sql = format!("SELECT DISTINCT id FROM {TABLE_ISSUES_V2} WHERE deleted = 0");
        let known_ids: Vec<String> = sqlx::query_scalar(&issue_ids_sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        let known_issues: HashSet<IssueId> = known_ids
            .into_iter()
            .filter_map(|id| id.parse::<IssueId>().ok())
            .collect();

        let rels_sql = format!(
            "SELECT source_id, target_id, rel_type FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_kind = 'issue' AND target_kind = 'issue' \
             AND rel_type IN ('child-of', 'blocked-on')"
        );
        let rows = sqlx::query_as::<_, (String, String, String)>(&rels_sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();

        for (source_id, target_id, rel_type) in rows {
            let Ok(source) = source_id.parse::<IssueId>() else {
                continue;
            };
            let Ok(target) = target_id.parse::<IssueId>() else {
                continue;
            };
            let dep_type = match rel_type.as_str() {
                "child-of" => IssueDependencyType::ChildOf,
                "blocked-on" => IssueDependencyType::BlockedOn,
                _ => continue,
            };

            forward
                .entry(dep_type)
                .or_default()
                .entry(target.clone())
                .or_default()
                .push(source.clone());

            reverse
                .entry(dep_type)
                .or_default()
                .entry(source)
                .or_default()
                .push(target);
        }

        Ok(IssueGraphContext::from_dependency_maps(
            known_issues,
            forward,
            reverse,
        ))
    }

    // ---- Notification helpers ----

    fn row_to_notification(&self, row: &NotificationRow) -> Result<Notification, StoreError> {
        let recipient = Actor::parse_name(&row.recipient).map_err(|_| {
            StoreError::Internal(format!(
                "invalid recipient '{}' stored for notification '{}'",
                row.recipient, row.id
            ))
        })?;
        let source_actor = row
            .source_actor
            .as_deref()
            .map(|s| {
                Actor::parse_name(s).map_err(|_| {
                    StoreError::Internal(format!(
                        "invalid source_actor '{}' stored for notification '{}'",
                        s, row.id
                    ))
                })
            })
            .transpose()?;
        let object_id = MetisId::from_str(&row.object_id).map_err(|_| {
            StoreError::Internal(format!(
                "invalid object_id '{}' stored for notification '{}'",
                row.object_id, row.id
            ))
        })?;
        let source_issue_id = row
            .source_issue_id
            .as_deref()
            .map(|s| {
                IssueId::from_str(s).map_err(|_| {
                    StoreError::Internal(format!(
                        "invalid source_issue_id '{}' stored for notification '{}'",
                        s, row.id
                    ))
                })
            })
            .transpose()?;
        let object_version = VersionNumber::try_from(row.object_version).map_err(|_| {
            StoreError::Internal(format!(
                "invalid object_version stored for notification '{}'",
                row.id
            ))
        })?;
        let created_at = parse_sqlite_timestamp(&row.created_at)?;

        Ok(Notification {
            recipient,
            source_actor,
            object_kind: row.object_kind.clone(),
            object_id,
            object_version,
            event_type: row.event_type.clone(),
            summary: row.summary.clone(),
            source_issue_id,
            policy: row.policy.clone(),
            is_read: row.is_read,
            created_at,
        })
    }

    async fn insert_notification_row(
        &self,
        id: &NotificationId,
        notification: &Notification,
    ) -> Result<(), StoreError> {
        let recipient_name = notification.recipient.to_string();
        let source_actor_name = notification.source_actor.as_ref().map(|a| a.to_string());
        let object_id_str = notification.object_id.to_string();
        let source_issue_str = notification.source_issue_id.as_ref().map(|i| i.to_string());
        let object_version = i64::try_from(notification.object_version).map_err(|_| {
            StoreError::Internal(format!("object_version overflow for notification '{id}'"))
        })?;
        let created_at = notification.created_at.to_rfc3339();

        sqlx::query(&format!(
            "INSERT INTO {TABLE_NOTIFICATIONS} \
             (id, recipient, source_actor, object_kind, object_id, object_version, \
              event_type, summary, source_issue_id, policy, is_read, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ))
        .bind(id.as_ref())
        .bind(&recipient_name)
        .bind(&source_actor_name)
        .bind(&notification.object_kind)
        .bind(&object_id_str)
        .bind(object_version)
        .bind(&notification.event_type)
        .bind(&notification.summary)
        .bind(&source_issue_str)
        .bind(&notification.policy)
        .bind(notification.is_read)
        .bind(&created_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    // ---- Message helpers ----

    fn row_to_message(&self, row: &MessageRow) -> Result<Message, StoreError> {
        let sender = row
            .sender
            .as_deref()
            .map(|s| {
                Actor::parse_name(s).map_err(|_| {
                    StoreError::Internal(format!(
                        "invalid sender '{}' stored for message '{}'",
                        s, row.id
                    ))
                })
            })
            .transpose()?;
        let recipient = Actor::parse_name(&row.recipient).map_err(|_| {
            StoreError::Internal(format!(
                "invalid recipient '{}' stored for message '{}'",
                row.recipient, row.id
            ))
        })?;

        Ok(Message {
            sender,
            recipient,
            body: row.body.clone(),
            deleted: row.deleted,
            is_read: row.is_read,
        })
    }

    async fn insert_message_row(
        &self,
        id: &MessageId,
        version_number: VersionNumber,
        message: &Message,
        actor: Option<&str>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for message '{id}'"))
        })?;
        let sender_name = message.sender.as_ref().map(|s| s.to_string());
        let recipient_name = message.recipient.to_string();

        sqlx::query(&format!(
            "INSERT INTO {TABLE_MESSAGES_V2} \
             (id, version_number, sender, recipient, body, is_read, deleted, actor) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        ))
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&sender_name)
        .bind(&recipient_name)
        .bind(&message.body)
        .bind(message.is_read)
        .bind(message.deleted)
        .bind(actor)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }
}

/// Build WHERE predicates and bindings for issues queries (SQLite `?N` placeholders).
fn build_issues_predicates_sqlite(query: &SearchIssuesQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    // When `ids` is provided, filter by ID and skip other content filters.
    if !query.ids.is_empty() {
        let placeholders: Vec<String> = query
            .ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("id IN ({})", placeholders.join(", ")));
        for id in &query.ids {
            bindings.push(id.as_ref().to_string());
        }
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = 0".to_string());
        }
        return (predicates, bindings);
    }

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

    if let Some(creator) = query
        .creator
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        bindings.push(creator.to_lowercase());
        predicates.push(format!("LOWER(creator) = ?{}", bindings.len()));
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

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for patches queries (SQLite `?N` placeholders).
fn build_patches_predicates_sqlite(query: &SearchPatchesQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = 0".to_string());
    }

    if !query.status.is_empty() {
        let status_strings: Vec<String> = query
            .status
            .iter()
            .map(|s| {
                let domain: crate::domain::patches::PatchStatus = (*s).into();
                domain.as_str().to_string()
            })
            .collect();
        let placeholders: Vec<String> = status_strings
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("status IN ({})", placeholders.join(", ")));
        for s in status_strings {
            bindings.push(s);
        }
    }

    if let Some(ref branch) = query.branch_name {
        bindings.push(branch.clone());
        predicates.push(format!("branch_name = ?{}", bindings.len()));
    }

    if let Some(term) = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{term}%");
        let start = bindings.len() + 1;
        for _ in 0..12 {
            bindings.push(pattern.clone());
        }
        predicates.push(format!(
            "(LOWER(id) LIKE ?{s0} \
             OR LOWER(title) LIKE ?{s1} \
             OR LOWER(description) LIKE ?{s2} \
             OR LOWER(status) LIKE ?{s3} \
             OR LOWER(service_repo_name) LIKE ?{s4} \
             OR LOWER(diff) LIKE ?{s5} \
             OR LOWER(COALESCE(branch_name,'')) LIKE ?{s6} \
             OR LOWER(COALESCE(json_extract(github,'$.owner'),'')) LIKE ?{s7} \
             OR LOWER(COALESCE(json_extract(github,'$.repo'),'')) LIKE ?{s8} \
             OR CAST(json_extract(github,'$.number') AS TEXT) LIKE ?{s9} \
             OR LOWER(COALESCE(json_extract(github,'$.head_ref'),'')) LIKE ?{s10} \
             OR LOWER(COALESCE(json_extract(github,'$.base_ref'),'')) LIKE ?{s11})",
            s0 = start,
            s1 = start + 1,
            s2 = start + 2,
            s3 = start + 3,
            s4 = start + 4,
            s5 = start + 5,
            s6 = start + 6,
            s7 = start + 7,
            s8 = start + 8,
            s9 = start + 9,
            s10 = start + 10,
            s11 = start + 11,
        ));
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for documents queries (SQLite `?N` placeholders).
fn build_documents_predicates_sqlite(query: &SearchDocumentsQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(path) = query.path_prefix.as_ref() {
        if query.path_is_exact.unwrap_or(false) {
            bindings.push(path.clone());
            predicates.push(format!("COALESCE(path,'') = ?{}", bindings.len()));
        } else {
            bindings.push(format!("{path}%"));
            predicates.push(format!("COALESCE(path,'') LIKE ?{}", bindings.len()));
        }
    }

    if let Some(created_by) = query.created_by.as_ref() {
        bindings.push(created_by.as_ref().to_string());
        predicates.push(format!("created_by = ?{}", bindings.len()));
    }

    if let Some(term) = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{term}%");
        let start = bindings.len() + 1;
        bindings.push(pattern.clone());
        bindings.push(pattern.clone());
        bindings.push(pattern);
        predicates.push(format!(
            "(LOWER(title) LIKE ?{s0} \
             OR LOWER(body_markdown) LIKE ?{s1} \
             OR LOWER(COALESCE(path,'')) LIKE ?{s2})",
            s0 = start,
            s1 = start + 1,
            s2 = start + 2,
        ));
    }

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = 0".to_string());
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for tasks queries (SQLite `?N` placeholders).
/// Uses `t.` column prefix since tasks queries join against the table alias `t`.
fn build_tasks_predicates_sqlite(query: &SearchSessionsQuery) -> (Vec<String>, Vec<String>) {
    use crate::domain::task_status::Status;

    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(spawned_from) = query.spawned_from.as_ref() {
        bindings.push(spawned_from.as_ref().to_string());
        predicates.push(format!("t.spawned_from = ?{}", bindings.len()));
    }

    if !query.spawned_from_ids.is_empty() {
        let placeholders: Vec<String> = query
            .spawned_from_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("t.spawned_from IN ({})", placeholders.join(", ")));
        for id in &query.spawned_from_ids {
            bindings.push(id.as_ref().to_string());
        }
    }

    if let Some(term) = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
    {
        let pattern = format!("%{term}%");
        bindings.push(pattern.clone());
        let idx_id = bindings.len();
        bindings.push(pattern.clone());
        let idx_prompt = bindings.len();
        bindings.push(pattern);
        let idx_status = bindings.len();
        predicates.push(format!(
            "(LOWER(t.id) LIKE ?{idx_id} \
             OR LOWER(t.prompt) LIKE ?{idx_prompt} \
             OR LOWER(t.status) LIKE ?{idx_status})"
        ));
    }

    if !query.status.is_empty() {
        let status_strings: Vec<String> = query
            .status
            .iter()
            .map(|s| {
                let server_status: Status = (*s).into();
                super::status_to_db_str(server_status).to_string()
            })
            .collect();
        let placeholders: Vec<String> = status_strings
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("t.status IN ({})", placeholders.join(", ")));
        for s in status_strings {
            bindings.push(s);
        }
    }

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("t.deleted = 0".to_string());
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for labels queries (SQLite `?N` placeholders).
fn build_labels_predicates_sqlite(query: &SearchLabelsQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = 0".to_string());
    }

    if let Some(ref q) = query.q {
        bindings.push(format!("%{}%", q.to_lowercase()));
        predicates.push(format!("LOWER(name) LIKE ?{}", bindings.len()));
    }

    (predicates, bindings)
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
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, deleted, actor, created_at, updated_at,
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
        let mut issue = self.row_to_issue(&row)?;

        if !include_deleted && issue.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        self.populate_issue_relationships(id, &mut issue).await?;

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
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, deleted, actor, created_at, updated_at, NULL AS creation_time
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
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, i.progress, i.status, i.assignee, i.job_settings, i.todo_list, i.deleted, i.actor, i.created_at, i.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = i.id) AS creation_time
             FROM {TABLE_ISSUES_V2} i
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_ISSUES_V2} GROUP BY id) latest
             ON i.id = latest.id AND i.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let (mut predicates, mut bindings) = build_issues_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

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

        self.populate_issues_relationships(&mut issues).await?;

        Ok(issues)
    }

    async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        let subquery = format!(
            "SELECT i.id, i.issue_type, i.title, i.description, i.creator, i.progress, i.status, i.assignee, i.deleted, i.created_at
             FROM {TABLE_ISSUES_V2} i
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_ISSUES_V2} GROUP BY id) latest
             ON i.id = latest.id AND i.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT COUNT(*) FROM ({subquery}) AS latest");
        let (predicates, bindings) = build_issues_predicates_sqlite(query);

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let context = self.build_issue_graph_from_relationships().await?;
        context.apply_filters(filters)
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let sql = format!(
            "SELECT source_id FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE target_id = ?1 AND rel_type = ?2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(issue_id.as_ref())
            .bind(super::RelationshipType::ChildOf.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter()
            .map(|id| {
                id.parse::<IssueId>().map_err(|err| {
                    StoreError::Internal(format!("invalid issue id in object_relationships: {err}"))
                })
            })
            .collect()
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let sql = format!(
            "SELECT source_id FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE target_id = ?1 AND rel_type = ?2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(issue_id.as_ref())
            .bind(super::RelationshipType::BlockedOn.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter()
            .map(|id| {
                id.parse::<IssueId>().map_err(|err| {
                    StoreError::Internal(format!("invalid issue id in object_relationships: {err}"))
                })
            })
            .collect()
    }

    async fn get_sessions_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<SessionId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let query = SearchSessionsQuery::new(None, Some(issue_id.clone()), None, vec![]);
        let tasks = self.list_sessions(&query).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    async fn get_patch(
        &self,
        id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        let row = sqlx::query_as::<_, PatchRow>(&format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_PATCHES_V2} WHERE id = ?1) AS creation_time
             FROM {TABLE_PATCHES_V2}
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for patch '{}'",
                row.id
            ))
        })?;
        let patch = self.row_to_patch(&row)?;
        if !include_deleted && patch.deleted {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(timestamp);
        Ok(Versioned::with_optional_actor(
            patch,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let rows = sqlx::query_as::<_, PatchRow>(&format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor, created_at, updated_at, NULL AS creation_time
             FROM {TABLE_PATCHES_V2}
             WHERE id = ?1
             ORDER BY version_number"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::PatchNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for patch '{}'",
                    row.id
                ))
            })?;
            let patch = self.row_to_patch(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            results.push(Versioned::with_optional_actor(
                patch,
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

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let subquery = format!(
            "SELECT p.id, p.version_number, p.title, p.description, p.diff, p.status, p.is_automatic_backup, p.created_by, p.creator, p.base_branch, p.branch_name, p.commit_range, p.reviews, p.service_repo_name, p.github, p.deleted, p.actor, p.created_at, p.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_PATCHES_V2} WHERE id = p.id) AS creation_time
             FROM {TABLE_PATCHES_V2} p
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_PATCHES_V2} GROUP BY id) latest
             ON p.id = latest.id AND p.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let (mut predicates, mut bindings) = build_patches_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

        let mut query_builder = sqlx::query_as::<_, PatchRow>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut patches = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for patch '{}'",
                    row.id
                ))
            })?;
            let patch = self.row_to_patch(&row)?;
            let patch_id = row.id.parse::<PatchId>().map_err(|err| {
                StoreError::Internal(format!("invalid patch id stored in database: {err}"))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            let versioned = Versioned::with_optional_actor(
                patch,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                creation_time,
            );
            patches.push((patch_id, versioned));
        }

        Ok(patches)
    }

    async fn count_patches(&self, query: &SearchPatchesQuery) -> Result<u64, StoreError> {
        let subquery = format!(
            "SELECT p.id, p.status, p.is_automatic_backup, p.branch_name, p.service_repo_name, p.github, p.title, p.description, p.diff, p.deleted
             FROM {TABLE_PATCHES_V2} p
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_PATCHES_V2} GROUP BY id) latest
             ON p.id = latest.id AND p.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT COUNT(*) FROM ({subquery}) AS latest");
        let (predicates, bindings) = build_patches_predicates_sqlite(query);

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_patch_exists(patch_id).await?;
        let sql = format!(
            "SELECT source_id FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE target_id = ?1 AND rel_type = ?2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(patch_id.as_ref())
            .bind(super::RelationshipType::HasPatch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter()
            .map(|id| {
                id.parse::<IssueId>().map_err(|err| {
                    StoreError::Internal(format!("invalid issue id in object_relationships: {err}"))
                })
            })
            .collect()
    }

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let row = sqlx::query_as::<_, DocumentRow>(&format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = ?1) AS creation_time
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        if !include_deleted && row.deleted {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for document '{}'",
                row.id
            ))
        })?;
        let document = self.row_to_document(&row)?;
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(timestamp);
        Ok(Versioned::with_optional_actor(
            document,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let rows = sqlx::query_as::<_, DocumentRow>(&format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, actor, created_at, updated_at, NULL AS creation_time
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = ?1
             ORDER BY version_number"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for document '{}'",
                    row.id
                ))
            })?;
            let document = self.row_to_document(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            results.push(Versioned::with_optional_actor(
                document,
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

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let subquery = format!(
            "SELECT d.id, d.version_number, d.title, d.body_markdown, d.path, d.created_by, d.deleted, d.actor, d.created_at, d.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = d.id) AS creation_time
             FROM {TABLE_DOCUMENTS_V2} d
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_DOCUMENTS_V2} GROUP BY id) latest
             ON d.id = latest.id AND d.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let (mut predicates, mut bindings) = build_documents_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

        let mut query_builder = sqlx::query_as::<_, DocumentRow>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut documents = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for document '{}'",
                    row.id
                ))
            })?;
            let document = self.row_to_document(&row)?;
            let document_id = row.id.parse::<DocumentId>().map_err(|err| {
                StoreError::Internal(format!("invalid document id stored in database: {err}"))
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            let versioned = Versioned::with_optional_actor(
                document,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                creation_time,
            );
            documents.push((document_id, versioned));
        }

        Ok(documents)
    }

    async fn count_documents(&self, query: &SearchDocumentsQuery) -> Result<u64, StoreError> {
        let subquery = format!(
            "SELECT d.id, d.title, d.body_markdown, d.path, d.created_by, d.deleted
             FROM {TABLE_DOCUMENTS_V2} d
             INNER JOIN (SELECT id, MAX(version_number) AS max_vn FROM {TABLE_DOCUMENTS_V2} GROUP BY id) latest
             ON d.id = latest.id AND d.version_number = latest.max_vn"
        );
        let mut sql = format!("SELECT COUNT(*) FROM ({subquery}) AS latest");
        let (predicates, bindings) = build_documents_predicates_sqlite(query);

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
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

    async fn get_session(
        &self,
        id: &SessionId,
        include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError> {
        let row = sqlx::query_as::<_, TaskRow>(
            &format!(
                "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at,
                 creation_time, start_time, end_time
                 FROM {TABLE_TASKS_V2}
                 WHERE id = ?1
                 ORDER BY version_number DESC
                 LIMIT 1"
            )
        )
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::SessionNotFound(id.clone()))?;
        if !include_deleted && row.deleted {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        self.row_to_versioned_session(&row)
    }

    async fn get_session_versions(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError> {
        let rows = sqlx::query_as::<_, TaskRow>(
            &format!(
                "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at, creation_time, start_time, end_time
                 FROM {TABLE_TASKS_V2}
                 WHERE id = ?1
                 ORDER BY version_number"
            )
        )
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::SessionNotFound(id.clone()));
        }

        let mut results: Vec<Versioned<Session>> = rows
            .iter()
            .map(|row| self.row_to_versioned_session(row))
            .collect::<Result<Vec<_>, _>>()?;

        let creation_time = results.first().map(|r| r.timestamp);
        for r in &mut results {
            r.creation_time = creation_time.unwrap_or(r.timestamp);
        }

        Ok(results)
    }

    async fn list_sessions(
        &self,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError> {
        let mut sql = format!(
            "SELECT t.id, t.version_number, t.prompt, t.context, t.spawned_from, t.image, t.model, t.env_vars, t.cpu_limit, t.memory_limit, t.status, t.last_message, t.error, t.secrets, t.creator, t.deleted, t.actor, t.created_at, t.updated_at, \
             t.creation_time, t.start_time, t.end_time \
             FROM {TABLE_TASKS_V2} t \
             INNER JOIN (SELECT id, MAX(version_number) AS max_version FROM {TABLE_TASKS_V2} GROUP BY id) latest \
             ON t.id = latest.id AND t.version_number = latest.max_version"
        );
        let (mut predicates, mut bindings) = build_tasks_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "t.created_at",
            "t.id",
        )?;

        let mut query_builder = sqlx::query_as::<_, TaskRow>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut tasks = Vec::with_capacity(rows.len());
        for row in &rows {
            let task_id = Self::row_to_session_id(&row.id)?;
            let versioned = self.row_to_versioned_session(row)?;
            tasks.push((task_id, versioned));
        }

        Ok(tasks)
    }

    async fn count_sessions(&self, query: &SearchSessionsQuery) -> Result<u64, StoreError> {
        let mut sql = format!(
            "SELECT COUNT(*) \
             FROM {TABLE_TASKS_V2} t \
             INNER JOIN (SELECT id, MAX(version_number) AS max_version FROM {TABLE_TASKS_V2} GROUP BY id) latest \
             ON t.id = latest.id AND t.version_number = latest.max_version"
        );
        let (predicates, bindings) = build_tasks_predicates_sqlite(query);

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn get_status_log(&self, id: &SessionId) -> Result<TaskStatusLog, StoreError> {
        let versions = self.get_session_versions(id).await?;
        super::session_status_log_from_versions(&versions)
            .ok_or_else(|| StoreError::SessionNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        // SQLite doesn't support ANY($1), so we build a query with placeholders
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at, creation_time, start_time, end_time \
             FROM {TABLE_TASKS_V2} \
             WHERE id IN ({}) \
             ORDER BY id, version_number",
            placeholders.join(", ")
        );
        let mut query_builder = sqlx::query_as::<_, TaskRow>(&sql);
        for id in ids {
            query_builder = query_builder.bind(id.as_ref());
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut grouped: HashMap<SessionId, Vec<Versioned<Session>>> = HashMap::new();
        for row in &rows {
            let task_id = Self::row_to_session_id(&row.id)?;
            let versioned = self.row_to_versioned_session(row)?;
            grouped.entry(task_id).or_default().push(versioned);
        }

        let mut result = HashMap::new();
        for (task_id, versions) in grouped {
            if let Some(log) = super::session_status_log_from_versions(&versions) {
                result.insert(task_id, log);
            }
        }

        Ok(result)
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
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
            "SELECT u.id, u.version_number, u.username, u.github_user_id, u.deleted, u.actor, u.created_at, u.updated_at
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
        let sql = format!(
            "SELECT id, recipient, source_actor, object_kind, object_id, object_version, \
             event_type, summary, source_issue_id, policy, is_read, created_at \
             FROM {TABLE_NOTIFICATIONS} WHERE id = ?1"
        );
        let row = sqlx::query_as::<_, NotificationRow>(&sql)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?
            .ok_or_else(|| StoreError::NotificationNotFound(id.clone()))?;
        self.row_to_notification(&row)
    }

    async fn list_notifications(
        &self,
        query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError> {
        let limit = i64::from(query.limit.unwrap_or(50));
        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if let Some(ref recipient) = query.recipient {
            conditions.push(format!("recipient = ?{}", bind_values.len() + 1));
            bind_values.push(recipient.clone());
        }
        if let Some(is_read) = query.is_read {
            conditions.push(format!("is_read = ?{}", bind_values.len() + 1));
            bind_values.push(if is_read {
                "1".to_string()
            } else {
                "0".to_string()
            });
        }
        if let Some(before) = query.before {
            conditions.push(format!("created_at < ?{}", bind_values.len() + 1));
            bind_values.push(before.to_rfc3339());
        }
        if let Some(after) = query.after {
            conditions.push(format!("created_at > ?{}", bind_values.len() + 1));
            bind_values.push(after.to_rfc3339());
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let limit_param = bind_values.len() + 1;
        let sql = format!(
            "SELECT id, recipient, source_actor, object_kind, object_id, object_version, \
             event_type, summary, source_issue_id, policy, is_read, created_at \
             FROM {TABLE_NOTIFICATIONS}{where_clause} \
             ORDER BY created_at DESC LIMIT ?{limit_param}"
        );

        let mut qb = sqlx::query_as::<_, NotificationRow>(&sql);
        for val in &bind_values {
            qb = qb.bind(val);
        }
        qb = qb.bind(limit);

        let rows = qb.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        let mut notifications = Vec::with_capacity(rows.len());
        for row in &rows {
            let notification_id = row.id.parse::<NotificationId>().map_err(|err| {
                StoreError::Internal(format!("invalid notification id stored in database: {err}"))
            })?;
            let notification = self.row_to_notification(row)?;
            notifications.push((notification_id, notification));
        }
        Ok(notifications)
    }

    async fn count_unread_notifications(&self, recipient: &ActorId) -> Result<u64, StoreError> {
        let recipient_name = recipient.to_string();
        let count = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(*) FROM {TABLE_NOTIFICATIONS} WHERE recipient = ?1 AND is_read = 0"
        ))
        .bind(&recipient_name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        let sql = format!(
            "SELECT id, version_number, sender, recipient, body, is_read, deleted, actor, \
             created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_MESSAGES_V2} WHERE id = ?1) AS creation_time \
             FROM {TABLE_MESSAGES_V2} \
             WHERE id = ?1 \
             ORDER BY version_number DESC \
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, MessageRow>(&sql)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?
            .ok_or_else(|| StoreError::MessageNotFound(id.clone()))?;

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for message '{}'",
                row.id
            ))
        })?;
        let created_at = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(created_at);
        let actor = parse_actor_json_string(row.actor.as_deref())?;
        let message = self.row_to_message(&row)?;
        Ok(Versioned::with_optional_actor(
            message,
            version,
            created_at,
            actor,
            creation_time,
        ))
    }

    async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        let limit = i64::from(query.limit.unwrap_or(50));
        let include_deleted = query.include_deleted.unwrap_or(false);

        // SQLite doesn't have DISTINCT ON, so use a subquery with GROUP BY to get the latest version
        let subquery = format!(
            "SELECT m.id, m.version_number, m.sender, m.recipient, m.body, m.is_read, \
             m.deleted, m.actor, m.created_at, m.updated_at, \
             (SELECT MIN(m2.created_at) FROM {TABLE_MESSAGES_V2} m2 WHERE m2.id = m.id) AS creation_time \
             FROM {TABLE_MESSAGES_V2} m \
             INNER JOIN (SELECT id, MAX(version_number) AS max_ver FROM {TABLE_MESSAGES_V2} GROUP BY id) latest \
             ON m.id = latest.id AND m.version_number = latest.max_ver"
        );

        let mut conditions = Vec::new();
        let mut bind_values: Vec<String> = Vec::new();

        if !include_deleted {
            conditions.push("deleted = 0".to_string());
        }
        if let Some(ref sender) = query.sender {
            conditions.push(format!("sender = ?{}", bind_values.len() + 1));
            bind_values.push(sender.clone());
        }
        if let Some(ref recipient) = query.recipient {
            conditions.push(format!("recipient = ?{}", bind_values.len() + 1));
            bind_values.push(recipient.clone());
        }
        if let Some(after) = query.after {
            conditions.push(format!("created_at > ?{}", bind_values.len() + 1));
            bind_values.push(after.to_rfc3339());
        }
        if let Some(before) = query.before {
            conditions.push(format!("created_at < ?{}", bind_values.len() + 1));
            bind_values.push(before.to_rfc3339());
        }
        if let Some(is_read) = query.is_read {
            conditions.push(format!("is_read = ?{}", bind_values.len() + 1));
            bind_values.push(if is_read {
                "1".to_string()
            } else {
                "0".to_string()
            });
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let limit_param = bind_values.len() + 1;
        let sql = format!(
            "SELECT * FROM ({subquery}) AS latest{where_clause} \
             ORDER BY created_at DESC LIMIT ?{limit_param}"
        );

        let mut qb = sqlx::query_as::<_, MessageRow>(&sql);
        for val in &bind_values {
            qb = qb.bind(val);
        }
        qb = qb.bind(limit);

        let rows = qb.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        let mut messages = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for message '{}'",
                    row.id
                ))
            })?;
            let message_id = row.id.parse::<MessageId>().map_err(|err| {
                StoreError::Internal(format!("invalid message id stored in database: {err}"))
            })?;
            let created_at = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(created_at);
            let actor = parse_actor_json_string(row.actor.as_deref())?;
            let message = self.row_to_message(&row)?;
            let versioned =
                Versioned::with_optional_actor(message, version, created_at, actor, creation_time);
            messages.push((message_id, versioned));
        }
        Ok(messages)
    }

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, max_tries, max_simultaneous, \
                    is_assignment_agent, deleted, created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE name = ?1"
        );
        let row = sqlx::query_as::<_, AgentRow>(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?
            .ok_or_else(|| StoreError::AgentNotFound(name.to_string()))?;
        let agent = row_to_agent(row)?;
        if agent.deleted {
            return Err(StoreError::AgentNotFound(name.to_string()));
        }
        Ok(agent)
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, max_tries, max_simultaneous, \
                    is_assignment_agent, deleted, created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE deleted = 0 ORDER BY name"
        );
        let rows = sqlx::query_as::<_, AgentRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(row_to_agent).collect()
    }

    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError> {
        let sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS} WHERE id = ?1"
        );
        let row = sqlx::query_as::<_, LabelRow>(&sql)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?
            .ok_or_else(|| StoreError::LabelNotFound(id.clone()))?;
        let label = row_to_label(&row)?;
        if label.deleted {
            return Err(StoreError::LabelNotFound(id.clone()));
        }
        Ok(label)
    }

    async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        let (mut predicates, mut bindings) = build_labels_predicates_sqlite(query);

        let mut sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS}"
        );

        if query.limit.is_some() || query.cursor.is_some() {
            apply_pagination_sql_sqlite(
                &mut sql,
                &mut predicates,
                &mut bindings,
                &query.cursor,
                query.limit,
                "updated_at",
                "id",
            )?;
        } else {
            if !predicates.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&predicates.join(" AND "));
            }
            sql.push_str(" ORDER BY name");
        }

        let mut qb = sqlx::query_as::<_, LabelRow>(&sql);
        for value in &bindings {
            qb = qb.bind(value);
        }

        let rows = qb.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;

        let mut labels = Vec::with_capacity(rows.len());
        for row in &rows {
            let label_id = row.id.parse::<LabelId>().map_err(|err| {
                StoreError::Internal(format!("invalid label id stored in database: {err}"))
            })?;
            let label = row_to_label(row)?;
            labels.push((label_id, label));
        }

        Ok(labels)
    }

    async fn count_labels(&self, query: &SearchLabelsQuery) -> Result<u64, StoreError> {
        let (predicates, bindings) = build_labels_predicates_sqlite(query);

        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_LABELS}");

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut qb = sqlx::query_scalar::<_, i64>(&sql);
        for value in &bindings {
            qb = qb.bind(value);
        }

        let count = qb.fetch_one(&self.pool).await.map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn get_label_by_name(&self, name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        let sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS} WHERE LOWER(name) = LOWER(?1) AND deleted = 0"
        );
        let row = sqlx::query_as::<_, LabelRow>(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        match row {
            Some(row) => {
                let label_id = row.id.parse::<LabelId>().map_err(|err| {
                    StoreError::Internal(format!("invalid label id stored in database: {err}"))
                })?;
                Ok(Some((label_id, row_to_label(&row)?)))
            }
            None => Ok(None),
        }
    }

    async fn get_labels_for_object(
        &self,
        object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        let sql = format!(
            "SELECT l.id, l.name, l.color, l.recurse, l.hidden \
             FROM {TABLE_LABELS} l \
             INNER JOIN {TABLE_LABEL_ASSOCIATIONS} la ON l.id = la.label_id \
             WHERE la.object_id = ?1 AND l.deleted = 0 \
             ORDER BY l.name"
        );
        let rows = sqlx::query_as::<_, (String, String, String, bool, bool)>(&sql)
            .bind(object_id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|(id, name, color, recurse, hidden)| {
                let label_id = id.parse::<LabelId>().map_err(|err| {
                    StoreError::Internal(format!("invalid label id stored in database: {err}"))
                })?;
                let color = color.parse().map_err(|err| {
                    StoreError::Internal(format!("invalid color stored in database: {err}"))
                })?;
                Ok(LabelSummary::new(label_id, name, color, recurse, hidden))
            })
            .collect()
    }

    async fn get_labels_for_objects(
        &self,
        object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        if object_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // SQLite doesn't support ANY($1), so build individual placeholders
        let placeholders: Vec<String> = (1..=object_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT la.object_id, l.id, l.name, l.color, l.recurse, l.hidden \
             FROM {TABLE_LABELS} l \
             INNER JOIN {TABLE_LABEL_ASSOCIATIONS} la ON l.id = la.label_id \
             WHERE la.object_id IN ({}) AND l.deleted = 0 \
             ORDER BY l.name",
            placeholders.join(", ")
        );
        let mut qb = sqlx::query_as::<_, (String, String, String, String, bool, bool)>(&sql);
        for oid in object_ids {
            qb = qb.bind(oid.as_ref());
        }
        let rows = qb.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;

        let mut result: HashMap<MetisId, Vec<LabelSummary>> = HashMap::new();
        for (obj_id_str, label_id_str, name, color, recurse, hidden) in rows {
            let obj_id = obj_id_str.parse::<MetisId>().map_err(|err| {
                StoreError::Internal(format!("invalid object id stored in database: {err}"))
            })?;
            let label_id = label_id_str.parse::<LabelId>().map_err(|err| {
                StoreError::Internal(format!("invalid label id stored in database: {err}"))
            })?;
            let color = color.parse().map_err(|err| {
                StoreError::Internal(format!("invalid color stored in database: {err}"))
            })?;
            result
                .entry(obj_id)
                .or_default()
                .push(LabelSummary::new(label_id, name, color, recurse, hidden));
        }
        Ok(result)
    }

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<MetisId>, StoreError> {
        let sql = format!("SELECT object_id FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = ?1");
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(label_id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|id| {
                id.parse::<MetisId>().map_err(|err| {
                    StoreError::Internal(format!("invalid object id stored in database: {err}"))
                })
            })
            .collect()
    }

    // ---- Object relationships (read-only) ----

    async fn get_relationships(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: Option<super::RelationshipType>,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let mut conditions = Vec::new();
        let mut bind_index = 1u32;

        if source_id.is_some() {
            conditions.push(format!("source_id = ?{bind_index}"));
            bind_index += 1;
        }
        if target_id.is_some() {
            conditions.push(format!("target_id = ?{bind_index}"));
            bind_index += 1;
        }
        if rel_type.is_some() {
            conditions.push(format!("rel_type = ?{bind_index}"));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT source_id, source_kind, target_id, target_kind, rel_type \
             FROM {TABLE_OBJECT_RELATIONSHIPS}{where_clause} \
             ORDER BY created_at"
        );

        let mut query = sqlx::query_as::<_, ObjectRelationshipRow>(&sql);
        if let Some(id) = source_id {
            query = query.bind(id.as_ref());
        }
        if let Some(id) = target_id {
            query = query.bind(id.as_ref());
        }
        if let Some(rt) = rel_type {
            query = query.bind(rt.as_str());
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    async fn get_relationships_batch(
        &self,
        source_ids: Option<&[MetisId]>,
        target_ids: Option<&[MetisId]>,
        rel_type: Option<super::RelationshipType>,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let mut conditions = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        if let Some(sids) = source_ids {
            if sids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders: Vec<String> = sids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", binds.len() + i + 1))
                .collect();
            conditions.push(format!("source_id IN ({})", placeholders.join(", ")));
            for sid in sids {
                binds.push(sid.as_ref().to_string());
            }
        }
        if let Some(tids) = target_ids {
            if tids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders: Vec<String> = tids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", binds.len() + i + 1))
                .collect();
            conditions.push(format!("target_id IN ({})", placeholders.join(", ")));
            for tid in tids {
                binds.push(tid.as_ref().to_string());
            }
        }
        if let Some(rt) = rel_type {
            binds.push(rt.as_str().to_string());
            conditions.push(format!("rel_type = ?{}", binds.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT source_id, source_kind, target_id, target_kind, rel_type \
             FROM {TABLE_OBJECT_RELATIONSHIPS}{where_clause} \
             ORDER BY created_at"
        );

        let mut query = sqlx::query_as::<_, ObjectRelationshipRow>(&sql);
        for b in &binds {
            query = query.bind(b);
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    async fn get_relationships_transitive(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: super::RelationshipType,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let (sql, start_id) = if let Some(sid) = source_id {
            let sql = format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE source_id = ?1 AND rel_type = ?2 \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.source_id = tr.target_id \
                     WHERE r.rel_type = ?2 \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type \
                 FROM transitive_rels"
            );
            (sql, sid.as_ref())
        } else if let Some(tid) = target_id {
            let sql = format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE target_id = ?1 AND rel_type = ?2 \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.target_id = tr.source_id \
                     WHERE r.rel_type = ?2 \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type \
                 FROM transitive_rels"
            );
            (sql, tid.as_ref())
        } else {
            return Ok(Vec::new());
        };

        let rows = sqlx::query_as::<_, ObjectRelationshipRow>(&sql)
            .bind(start_id)
            .bind(rel_type.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let sql = format!(
            "SELECT encrypted_value FROM {TABLE_USER_SECRETS} WHERE username = ?1 AND secret_name = ?2"
        );
        let row = sqlx::query_scalar::<_, Vec<u8>>(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(row)
    }

    async fn list_user_secret_names(
        &self,
        username: &Username,
    ) -> Result<Vec<SecretRef>, StoreError> {
        let sql = format!(
            "SELECT secret_name, internal FROM {TABLE_USER_SECRETS} WHERE username = ?1 ORDER BY secret_name"
        );
        let rows = sqlx::query_as::<_, (String, bool)>(&sql)
            .bind(username.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(|(name, internal)| SecretRef { name, internal })
            .collect())
    }

    async fn is_secret_internal(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<bool, StoreError> {
        let sql = format!(
            "SELECT internal FROM {TABLE_USER_SECRETS} WHERE username = ?1 AND secret_name = ?2"
        );
        let row = sqlx::query_scalar::<_, bool>(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(row.unwrap_or(false))
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
        let id = IssueId::new();
        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        Self::insert_issue_in_tx(&mut *tx, &id, 1, &issue, Some(&actor_json)).await?;
        Self::sync_issue_relationships_in_tx(&mut tx, &id, &issue).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        Ok((id, 1))
    }

    async fn update_issue(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_ISSUES_V2, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        Self::insert_issue_in_tx(&mut *tx, id, next_version, &issue, Some(&actor_json)).await?;
        Self::sync_issue_relationships_in_tx(&mut tx, id, &issue).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

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
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        let id = PatchId::new();
        let actor_json = actor_to_json_string(actor);
        self.insert_patch(&id, 1, &patch, Some(&actor_json)).await?;
        Ok((id, 1))
    }

    async fn update_patch(
        &self,
        id: &PatchId,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let latest_version = self
            .fetch_latest_version_number(TABLE_PATCHES_V2, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for patch '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_patch(id, next_version, &patch, Some(&actor_json))
            .await?;
        Ok(next_version)
    }

    async fn delete_patch(
        &self,
        id: &PatchId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_patch(id, true).await?;
        let mut patch = current.item;
        patch.deleted = true;
        self.update_patch(id, patch, actor).await
    }

    async fn add_document(
        &self,
        document: Document,
        actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        let id = DocumentId::new();
        let actor_json = actor_to_json_string(actor);
        self.insert_document(&id, 1, &document, Some(&actor_json))
            .await?;
        Ok((id, 1))
    }

    async fn update_document(
        &self,
        id: &DocumentId,
        document: Document,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let latest_version = self
            .fetch_latest_version_number(TABLE_DOCUMENTS_V2, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_document(id, next_version, &document, Some(&actor_json))
            .await?;
        Ok(next_version)
    }

    async fn delete_document(
        &self,
        id: &DocumentId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_document(id, true).await?;
        let mut document = current.item;
        document.deleted = true;
        self.update_document(id, document, actor).await
    }

    async fn add_session(
        &self,
        mut session: Session,
        creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(SessionId, VersionNumber), StoreError> {
        let id = SessionId::new();

        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        session.creation_time = Some(creation_time);
        let actor_json = actor_to_json_string(actor);
        let created_at = creation_time.to_rfc3339();
        self.insert_task(&id, 1, &session, Some(&actor_json), Some(&created_at))
            .await?;
        Ok((id, 1))
    }

    async fn update_session(
        &self,
        metis_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        let latest_version = self
            .fetch_latest_version_number(TABLE_TASKS_V2, metis_id.as_ref())
            .await?
            .ok_or_else(|| StoreError::SessionNotFound(metis_id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for task '{metis_id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_task(metis_id, next_version, &session, Some(&actor_json), None)
            .await?;
        self.get_session(metis_id, true).await
    }

    async fn delete_session(
        &self,
        id: &SessionId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_session(id, true).await?;
        let mut task = current.item;
        task.deleted = true;
        let versioned = self.update_session(id, task, actor).await?;
        Ok(versioned.version)
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
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
        notification: Notification,
    ) -> Result<NotificationId, StoreError> {
        let id = NotificationId::new();
        self.insert_notification_row(&id, &notification).await?;
        Ok(id)
    }

    async fn mark_notification_read(&self, id: &NotificationId) -> Result<(), StoreError> {
        let result = sqlx::query(&format!(
            "UPDATE {TABLE_NOTIFICATIONS} SET is_read = 1 WHERE id = ?1"
        ))
        .bind(id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(StoreError::NotificationNotFound(id.clone()));
        }
        Ok(())
    }

    async fn mark_all_notifications_read(
        &self,
        recipient: &ActorId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError> {
        let recipient_name = recipient.to_string();
        let result = if let Some(before_ts) = before {
            let before_str = before_ts.to_rfc3339();
            sqlx::query(&format!(
                "UPDATE {TABLE_NOTIFICATIONS} SET is_read = 1 \
                 WHERE recipient = ?1 AND is_read = 0 AND created_at < ?2"
            ))
            .bind(&recipient_name)
            .bind(&before_str)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?
        } else {
            sqlx::query(&format!(
                "UPDATE {TABLE_NOTIFICATIONS} SET is_read = 1 \
                 WHERE recipient = ?1 AND is_read = 0"
            ))
            .bind(&recipient_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?
        };
        Ok(result.rows_affected())
    }

    async fn add_message(
        &self,
        message: Message,
        actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        let id = MessageId::new();
        let actor_json = actor_to_json_string(actor);
        self.insert_message_row(&id, 1, &message, Some(&actor_json))
            .await?;
        Ok((id, 1))
    }

    async fn update_message(
        &self,
        id: &MessageId,
        message: Message,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let latest_version = self
            .fetch_latest_version_number(TABLE_MESSAGES_V2, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::MessageNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for message '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_message_row(id, next_version, &message, Some(&actor_json))
            .await?;
        Ok(next_version)
    }

    async fn add_agent(&self, agent: Agent) -> Result<(), StoreError> {
        let existing_deleted = sqlx::query_scalar::<_, bool>(&format!(
            "SELECT deleted FROM {TABLE_AGENTS} WHERE name = ?1"
        ))
        .bind(&agent.name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        match existing_deleted {
            Some(false) => {
                return Err(StoreError::AgentAlreadyExists(agent.name));
            }
            Some(true) => {
                if agent.is_assignment_agent {
                    let has_assignment = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_assignment_agent = 1 AND deleted = 0)"
                    ))
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                    if has_assignment {
                        return Err(StoreError::AssignmentAgentAlreadyExists);
                    }
                }

                let now = Utc::now().to_rfc3339();
                let sql = format!(
                    "UPDATE {TABLE_AGENTS} \
                     SET prompt_path = ?1, max_tries = ?2, max_simultaneous = ?3, \
                         is_assignment_agent = ?4, deleted = 0, \
                         created_at = ?5, updated_at = ?6 \
                     WHERE name = ?7"
                );
                sqlx::query(&sql)
                    .bind(&agent.prompt_path)
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(&now)
                    .bind(&now)
                    .bind(&agent.name)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;

                Ok(())
            }
            None => {
                if agent.is_assignment_agent {
                    let has_assignment = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_assignment_agent = 1 AND deleted = 0)"
                    ))
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                    if has_assignment {
                        return Err(StoreError::AssignmentAgentAlreadyExists);
                    }
                }

                let sql = format!(
                    "INSERT INTO {TABLE_AGENTS} \
                     (name, prompt_path, max_tries, max_simultaneous, is_assignment_agent, \
                      deleted, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
                );
                sqlx::query(&sql)
                    .bind(&agent.name)
                    .bind(&agent.prompt_path)
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(agent.deleted)
                    .bind(agent.created_at.to_rfc3339())
                    .bind(agent.updated_at.to_rfc3339())
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;

                Ok(())
            }
        }
    }

    async fn update_agent(&self, agent: Agent) -> Result<(), StoreError> {
        let _ = self.get_agent(&agent.name).await?;

        if agent.is_assignment_agent {
            let conflict = sqlx::query_scalar::<_, bool>(&format!(
                "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                 WHERE is_assignment_agent = 1 AND deleted = 0 AND name != ?1)"
            ))
            .bind(&agent.name)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
            if conflict {
                return Err(StoreError::AssignmentAgentAlreadyExists);
            }
        }

        let sql = format!(
            "UPDATE {TABLE_AGENTS} \
             SET prompt_path = ?1, max_tries = ?2, max_simultaneous = ?3, \
                 is_assignment_agent = ?4, updated_at = ?5 \
             WHERE name = ?6"
        );
        sqlx::query(&sql)
            .bind(&agent.prompt_path)
            .bind(agent.max_tries)
            .bind(agent.max_simultaneous)
            .bind(agent.is_assignment_agent)
            .bind(Utc::now().to_rfc3339())
            .bind(&agent.name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn delete_agent(&self, name: &str) -> Result<(), StoreError> {
        let _ = self.get_agent(name).await?;

        let sql = format!("UPDATE {TABLE_AGENTS} SET deleted = 1, updated_at = ?1 WHERE name = ?2");
        sqlx::query(&sql)
            .bind(Utc::now().to_rfc3339())
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn add_label(&self, label: Label) -> Result<LabelId, StoreError> {
        if self.get_label_by_name(&label.name).await?.is_some() {
            return Err(StoreError::LabelAlreadyExists(label.name.clone()));
        }

        let id = LabelId::new();

        let sql = format!(
            "INSERT INTO {TABLE_LABELS} (id, name, color, deleted, recurse, hidden, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        );
        sqlx::query(&sql)
            .bind(id.as_ref())
            .bind(&label.name)
            .bind(label.color.as_ref())
            .bind(label.deleted)
            .bind(label.recurse)
            .bind(label.hidden)
            .bind(label.created_at.to_rfc3339())
            .bind(label.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(id)
    }

    async fn update_label(&self, id: &LabelId, label: Label) -> Result<(), StoreError> {
        let _ = self.get_label(id).await?;

        if let Some((existing_id, _)) = self.get_label_by_name(&label.name).await? {
            if existing_id != *id {
                return Err(StoreError::LabelAlreadyExists(label.name.clone()));
            }
        }

        let sql = format!(
            "UPDATE {TABLE_LABELS} SET name = ?1, color = ?2, recurse = ?3, hidden = ?4, updated_at = ?5 WHERE id = ?6"
        );
        sqlx::query(&sql)
            .bind(&label.name)
            .bind(label.color.as_ref())
            .bind(label.recurse)
            .bind(label.hidden)
            .bind(Utc::now().to_rfc3339())
            .bind(id.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn delete_label(&self, id: &LabelId) -> Result<(), StoreError> {
        let _ = self.get_label(id).await?;

        let sql = format!("UPDATE {TABLE_LABELS} SET deleted = 1, updated_at = ?1 WHERE id = ?2");
        sqlx::query(&sql)
            .bind(Utc::now().to_rfc3339())
            .bind(id.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let object_kind = super::object_kind_from_id(object_id)?;
        let sql = format!(
            "INSERT INTO {TABLE_LABEL_ASSOCIATIONS} (label_id, object_id, object_kind) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT (label_id, object_id) DO NOTHING"
        );
        let result = sqlx::query(&sql)
            .bind(label_id.as_ref())
            .bind(object_id.as_ref())
            .bind(object_kind.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = ?1 AND object_id = ?2"
        );
        let result = sqlx::query(&sql)
            .bind(label_id.as_ref())
            .bind(object_id.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    // ---- Object relationship mutations ----

    async fn add_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: super::RelationshipType,
    ) -> Result<bool, StoreError> {
        let source_kind = super::object_kind_from_id(source_id)?;
        let target_kind = super::object_kind_from_id(target_id)?;
        let sql = format!(
            "INSERT OR IGNORE INTO {TABLE_OBJECT_RELATIONSHIPS} \
             (source_id, source_kind, target_id, target_kind, rel_type) \
             VALUES (?1, ?2, ?3, ?4, ?5)"
        );
        let result = sqlx::query(&sql)
            .bind(source_id.as_ref())
            .bind(source_kind.as_str())
            .bind(target_id.as_ref())
            .bind(target_kind.as_str())
            .bind(rel_type.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    async fn remove_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: super::RelationshipType,
    ) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = ?1 AND target_id = ?2 AND rel_type = ?3"
        );
        let result = sqlx::query(&sql)
            .bind(source_id.as_ref())
            .bind(target_id.as_ref())
            .bind(rel_type.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    // ---- User secret mutations ----

    async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
        internal: bool,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        let sql = format!(
            "INSERT INTO {TABLE_USER_SECRETS} (username, secret_name, encrypted_value, internal, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
             ON CONFLICT (username, secret_name) \
             DO UPDATE SET encrypted_value = ?3, internal = ?4, updated_at = ?5"
        );
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .bind(encrypted_value)
            .bind(internal)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn delete_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<(), StoreError> {
        let sql =
            format!("DELETE FROM {TABLE_USER_SECRETS} WHERE username = ?1 AND secret_name = ?2");
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}

/// Appends cursor-based keyset pagination to a SQL query (SQLite dialect).
///
/// Same as `apply_pagination_sql_pg` but uses `?` placeholders.
fn apply_pagination_sql_sqlite(
    sql: &mut String,
    predicates: &mut Vec<String>,
    bindings: &mut Vec<String>,
    cursor: &Option<String>,
    limit: Option<u32>,
    timestamp_col: &str,
    id_col: &str,
) -> Result<Option<u32>, StoreError> {
    if let Some(cursor_str) = cursor {
        let decoded = DecodedCursor::decode(cursor_str)
            .map_err(|e| StoreError::Internal(format!("invalid cursor: {e}")))?;
        predicates.push(format!("({timestamp_col}, {id_col}) < (?, ?)"));
        bindings.push(decoded.timestamp.to_rfc3339());
        bindings.push(decoded.id);
    }

    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }

    sql.push_str(&format!(" ORDER BY {timestamp_col} DESC, {id_col} DESC"));

    let effective_limit = limit.map(|l| l.min(PAGINATION_MAX_LIMIT));
    if let Some(limit) = effective_limit {
        sql.push_str(&format!(" LIMIT {}", limit + 1));
    }

    Ok(effective_limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::{ActorId, ActorRef};
    use crate::domain::sessions::BundleSpec;
    use chrono::Duration;
    use metis_common::SessionId;

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
            actor_id: ActorId::Session(SessionId::new()),
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
        let task_id = SessionId::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Session(task_id),
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
        let task_id = SessionId::new();
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
                    github_user_id: Some(101),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_user(&username, false).await.unwrap();
        assert_eq!(fetched.item.username, username);
        assert_eq!(fetched.item.github_user_id, Some(101));
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
                    github_user_id: Some(101),
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
                    github_user_id: Some(202),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        assert_eq!(updated.item.github_user_id, Some(202));
        assert_eq!(updated.version, 2);

        let user = store.get_user(&username, false).await.unwrap();
        assert_eq!(user.item.github_user_id, Some(202));
        assert_eq!(user.version, 2);
    }

    #[tokio::test]
    async fn get_user_filters_deleted_users() {
        let store = create_test_store().await;
        let username = Username::from("alice");
        let user = User {
            username: username.clone(),
            github_user_id: Some(101),
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
                    github_user_id: Some(101),
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
                    github_user_id: Some(202),
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
                    github_user_id: Some(101),
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
                    github_user_id: Some(303),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_user(&username, false).await.unwrap();
        assert!(!fetched.item.deleted);
        assert_eq!(fetched.item.github_user_id, Some(303));
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
                    github_user_id: Some(101),
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
                    github_user_id: Some(202),
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

    // ---- Patch tests ----

    fn dummy_diff() -> String {
        "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
    }

    fn sample_patch() -> Patch {
        Patch::new(
            "sample patch".to_string(),
            "sample patch".to_string(),
            dummy_diff(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn sample_document(path: Option<&str>, created_by: Option<SessionId>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: path.map(|p| p.parse().unwrap()),
            created_by,
            deleted: false,
        }
    }

    #[tokio::test]
    async fn add_and_get_patch_assigns_id() {
        let store = create_test_store().await;

        let patch = sample_patch();
        let (id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&id, false).await.unwrap();
        assert_eq!(fetched.item, patch);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn update_patch_overwrites_existing_value() {
        let store = create_test_store().await;

        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        let updated = Patch::new(
            "new title".to_string(),
            "updated patch".to_string(),
            dummy_diff(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        );

        store
            .update_patch(&id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&id, false).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);
    }

    #[tokio::test]
    async fn patch_versions_return_ordered_entries() {
        let store = create_test_store().await;

        let mut patch = sample_patch();
        patch.title = "v1".to_string();
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let mut v2 = sample_patch();
        v2.title = "v2".to_string();
        store
            .update_patch(&patch_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_patch_versions(&patch_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[0].item.title, "v1");
        assert_eq!(versions[1].item.title, "v2");
    }

    #[tokio::test]
    async fn delete_patch_sets_deleted_flag_and_filters_from_list() {
        let store = create_test_store().await;
        let (patch_id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();

        let patches = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(!patches[0].1.item.deleted);

        store
            .delete_patch(&patch_id, &ActorRef::test())
            .await
            .unwrap();

        let patches = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap();
        assert!(patches.is_empty());

        let patches = store
            .list_patches(&SearchPatchesQuery::new(None, Some(true), vec![], None))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].1.item.deleted);

        let patch = store.get_patch(&patch_id, true).await.unwrap();
        assert!(patch.item.deleted);

        let err = store.get_patch(&patch_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::PatchNotFound(_)));
    }

    #[tokio::test]
    async fn get_issues_for_patch_returns_correct_issues() {
        let store = create_test_store().await;

        let (patch_id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();

        let mut issue = sample_issue(vec![]);
        issue.patches = vec![patch_id.clone()];
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        let issue_ids = store.get_issues_for_patch(&patch_id).await.unwrap();
        assert_eq!(issue_ids, vec![issue_id]);
    }

    #[tokio::test]
    async fn list_patches_filters_by_status() {
        let store = create_test_store().await;

        store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();

        let mut closed_patch = sample_patch();
        closed_patch.status = PatchStatus::Closed;
        store
            .add_patch(closed_patch, &ActorRef::test())
            .await
            .unwrap();

        let query = SearchPatchesQuery::new(
            None,
            None,
            vec![metis_common::api::v1::patches::PatchStatus::Open],
            None,
        );
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].1.item.status, PatchStatus::Open);
    }

    /// Patch with every optional field set so serialization round-trip can assert full equality.
    fn sample_patch_all_fields(created_by: Option<SessionId>) -> Patch {
        use crate::domain::patches::GitOid;
        use std::str::FromStr;

        let base_oid = GitOid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let head_oid = GitOid::from_str("fedcba9876543210fedcba9876543210fedcba98").unwrap();
        let mut patch = Patch::new(
            "full title".to_string(),
            "full desc".to_string(),
            "full diff".to_string(),
            PatchStatus::Open,
            true,
            created_by,
            Username::from("test-creator"),
            vec![Review::new(
                "looks good".to_string(),
                true,
                "alice".to_string(),
                None,
            )],
            RepoName::from_str("org/repo").unwrap(),
            Some(GithubPr::new(
                "owner".to_string(),
                "repo".to_string(),
                42,
                Some("feature".to_string()),
                Some("main".to_string()),
                Some("https://github.com/owner/repo/pull/42".to_string()),
                None,
            )),
            Some("feature/xyz".to_string()),
            Some(CommitRange::new(base_oid, head_oid)),
            Some("main".to_string()),
        );
        patch.creator = Username::from("patch-creator");
        patch
    }

    #[tokio::test]
    async fn patch_serialization_round_trip_all_fields() {
        let store = create_test_store().await;
        let task_id = SessionId::new();
        let patch = sample_patch_all_fields(Some(task_id));

        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&patch_id, false).await.unwrap();
        assert_eq!(
            fetched.item, patch,
            "Patch must round-trip all fields (creator, base_branch, branch_name, commit_range, github, reviews)"
        );
    }

    #[tokio::test]
    async fn list_patches_text_search_matches_github_fields() {
        let store = create_test_store().await;

        let patch = sample_patch_all_fields(None);
        store.add_patch(patch, &ActorRef::test()).await.unwrap();

        // Search by github owner
        let mut query = SearchPatchesQuery::default();
        query.q = Some("owner".to_string());
        let results = store.list_patches(&query).await.unwrap();
        assert_eq!(results.len(), 1, "should match github owner field");

        // Search by github head_ref
        query.q = Some("feature".to_string());
        let results = store.list_patches(&query).await.unwrap();
        assert_eq!(results.len(), 1, "should match github head_ref field");

        // Search that doesn't match anything
        query.q = Some("zzz_nonexistent_zzz".to_string());
        let results = store.list_patches(&query).await.unwrap();
        assert!(results.is_empty(), "should not match anything");
    }

    // ---- Document tests ----

    #[tokio::test]
    async fn documents_round_trip() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(
                sample_document(Some("docs/guides/intro.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "Doc");
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.body_markdown = "Updated body".to_string();
        store
            .update_document(&doc_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_document_versions(&doc_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[1].item.body_markdown, "Updated body");

        let documents = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].0, doc_id);
    }

    #[tokio::test]
    async fn document_path_prefix_query() {
        let store = create_test_store().await;
        let (doc1, _) = store
            .add_document(
                sample_document(Some("docs/howto.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("notes/todo.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let by_path = store.get_documents_by_path("/docs/").await.unwrap();
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].0, doc1);
    }

    #[tokio::test]
    async fn document_filters_apply_query() {
        let store = create_test_store().await;
        let task_id = SessionId::new();
        let other_task = SessionId::new();

        let (first, _) = store
            .add_document(
                sample_document(Some("docs/howto.md"), Some(task_id.clone())),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("notes/todo.md"), Some(other_task.clone())),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let query = SearchDocumentsQuery::new(
            Some("how".to_string()),
            Some("/docs/".to_string()),
            None,
            Some(task_id.clone()),
            None,
        );

        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, first);

        let created_by_filtered = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                None,
                None,
                Some(other_task),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(created_by_filtered.len(), 1);
    }

    #[tokio::test]
    async fn delete_document_sets_deleted_flag_and_filters_from_list() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(None, None), &ActorRef::test())
            .await
            .unwrap();

        let documents = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(documents.len(), 1);

        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        let documents = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert!(documents.is_empty());

        let documents = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                None,
                None,
                None,
                Some(true),
            ))
            .await
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert!(documents[0].1.item.deleted);

        let doc = store.get_document(&doc_id, true).await.unwrap();
        assert!(doc.item.deleted);

        let err = store.get_document(&doc_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::DocumentNotFound(_)));
    }

    #[tokio::test]
    async fn document_serialization_round_trip_all_fields() {
        let store = create_test_store().await;
        let task_id = SessionId::new();
        let doc = sample_document(Some("docs/roundtrip.md"), Some(task_id));

        let (doc_id, _) = store
            .add_document(doc.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(
            fetched.item, doc,
            "Document must round-trip all fields (path, created_by)"
        );
    }

    // ---- Task helpers ----

    fn spawn_task() -> Session {
        Session::new(
            "test prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            Some("metis-worker:latest".to_string()),
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        )
    }

    // ---- Task tests ----

    #[tokio::test]
    async fn task_add_and_get() {
        let store = create_test_store().await;
        let task = spawn_task();
        let now = Utc::now();

        let (task_id, version) = store
            .add_session(task.clone(), now, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // add_task sets creation_time on the stored task
        let mut expected = task.clone();
        expected.creation_time = Some(now);
        assert_versioned(&fetched, &expected, 1);
        assert_eq!(fetched.item.status, Status::Created);
    }

    #[tokio::test]
    async fn task_not_found() {
        let store = create_test_store().await;
        let missing_id = SessionId::new();
        let err = store.get_session(&missing_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn task_versions_increment_and_latest_returned() {
        let store = create_test_store().await;

        let mut task = spawn_task();
        task.prompt = "v1".to_string();
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = spawn_task();
        updated.prompt = "v2".to_string();
        store
            .update_session(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        assert_versioned(&fetched, &updated, 2);
    }

    #[tokio::test]
    async fn task_get_versions_returns_ordered_entries() {
        let store = create_test_store().await;

        let mut task = spawn_task();
        task.prompt = "v1".to_string();
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut v2 = spawn_task();
        v2.prompt = "v2".to_string();
        store
            .update_session(&task_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_session_versions(&task_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[0].item.prompt, "v1");
        assert_eq!(versions[1].item.prompt, "v2");
    }

    #[tokio::test]
    async fn task_list_returns_all_tasks() {
        let store = create_test_store().await;

        let (id1, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (id2, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let tasks = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        let ids: HashSet<_> = tasks.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, HashSet::from([id1, id2]));
    }

    #[tokio::test]
    async fn task_list_filters_by_text_search() {
        let store = create_test_store().await;

        let mut task1 = spawn_task();
        task1.prompt = "deploy to production".to_string();
        store
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task2 = spawn_task();
        task2.prompt = "run tests".to_string();
        store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let query = SearchSessionsQuery::new(Some("deploy".to_string()), None, None, vec![]);
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].1.item.prompt, "deploy to production");
    }

    #[tokio::test]
    async fn task_list_filters_by_status() {
        let store = create_test_store().await;

        let (task_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut running = spawn_task();
        running.status = Status::Running;
        store
            .update_session(&task_id, running, &ActorRef::test())
            .await
            .unwrap();

        // Search for running tasks
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![metis_common::task_status::Status::Running],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);

        // Search for created tasks - should be empty since task is now running
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![metis_common::task_status::Status::Created],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[tokio::test]
    async fn task_list_filters_by_multiple_statuses() {
        let store = create_test_store().await;

        // Create three tasks - they all start as Created
        let (task1_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (task2_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (task3_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (task4_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Transition task2 to Running
        let mut running = spawn_task();
        running.status = Status::Running;
        store
            .update_session(&task2_id, running, &ActorRef::test())
            .await
            .unwrap();

        // Transition task3 to Complete
        let mut complete = spawn_task();
        complete.status = Status::Complete;
        store
            .update_session(&task3_id, complete, &ActorRef::test())
            .await
            .unwrap();

        // Transition task4 to Failed
        let mut failed = spawn_task();
        failed.status = Status::Failed;
        store
            .update_session(&task4_id, failed, &ActorRef::test())
            .await
            .unwrap();

        // Filter by multiple statuses: Created and Running
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![
                metis_common::task_status::Status::Created,
                metis_common::task_status::Status::Running,
            ],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 2);
        let ids: Vec<_> = tasks.iter().map(|(id, _)| id.clone()).collect();
        assert!(ids.contains(&task1_id));
        assert!(ids.contains(&task2_id));

        // Filter by single status: Complete
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![metis_common::task_status::Status::Complete],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, task3_id);

        // Empty status vec returns all tasks (no filter)
        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 4);

        // Filter by three statuses: Running, Complete, Failed
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![
                metis_common::task_status::Status::Running,
                metis_common::task_status::Status::Complete,
                metis_common::task_status::Status::Failed,
            ],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 3);
        let ids: Vec<_> = tasks.iter().map(|(id, _)| id.clone()).collect();
        assert!(ids.contains(&task2_id));
        assert!(ids.contains(&task3_id));
        assert!(ids.contains(&task4_id));
    }

    #[tokio::test]
    async fn task_soft_delete_and_list_filtering() {
        let store = create_test_store().await;

        let (task_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_session(&task_id, &ActorRef::test())
            .await
            .unwrap();

        // Should not appear in default list
        let tasks = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert!(tasks.is_empty());

        // Should appear when include_deleted is true
        let query = SearchSessionsQuery::new(None, None, Some(true), vec![]);
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].1.item.deleted);

        // get_task with include_deleted=false should fail
        let err = store.get_session(&task_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));

        // get_task with include_deleted=true should succeed
        let fetched = store.get_session(&task_id, true).await.unwrap();
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn status_log_derived_from_task_versions() {
        let store = create_test_store().await;
        let created_at = Utc::now() - Duration::seconds(60);
        let task = spawn_task();
        let (task_id, _) = store
            .add_session(task.clone(), created_at, &ActorRef::test())
            .await
            .unwrap();

        // Initial status log should have one Created event
        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.current_status(), Status::Created);

        // Update to Pending
        let mut pending = task.clone();
        pending.status = Status::Pending;
        store
            .update_session(&task_id, pending, &ActorRef::test())
            .await
            .unwrap();

        // Update to Running
        let mut running = task.clone();
        running.status = Status::Running;
        store
            .update_session(&task_id, running, &ActorRef::test())
            .await
            .unwrap();

        // Update to Complete
        let mut complete = task.clone();
        complete.status = Status::Complete;
        complete.last_message = Some("done".to_string());
        store
            .update_session(&task_id, complete, &ActorRef::test())
            .await
            .unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.current_status(), Status::Complete);
        // Created, Pending (Created event), Running (Started), Complete (Completed)
        assert_eq!(log.events.len(), 4);
    }

    #[tokio::test]
    async fn batch_get_status_logs_with_missing_tasks() {
        let store = create_test_store().await;

        let (task_id1, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let (task_id2, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let missing_id = SessionId::new();

        let logs = store
            .get_status_logs(&[task_id1.clone(), task_id2.clone(), missing_id.clone()])
            .await
            .unwrap();

        // Should have logs for both existing tasks
        assert!(logs.contains_key(&task_id1));
        assert!(logs.contains_key(&task_id2));
        // Missing task should be silently omitted
        assert!(!logs.contains_key(&missing_id));
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn task_serialization_round_trip_all_fields() {
        let store = create_test_store().await;
        let task = Session::new(
            "full test".to_string(),
            BundleSpec::None,
            None,
            Username::from("alice"),
            Some("my-image:v1".to_string()),
            Some("claude-3".to_string()),
            HashMap::from([("KEY".to_string(), "VALUE".to_string())]),
            Some("2".to_string()),
            Some("4Gi".to_string()),
            Some(vec!["secret1".to_string(), "secret2".to_string()]),
            Status::Pending,
            Some("last msg".to_string()),
            Some(TaskError::JobEngineError {
                reason: "test error".to_string(),
            }),
        );

        let now = Utc::now();
        let (task_id, _) = store
            .add_session(task.clone(), now, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // add_task sets creation_time on the stored task
        let mut expected = task.clone();
        expected.creation_time = Some(now);
        assert_eq!(fetched.item, expected, "Task must round-trip all fields");
    }

    #[tokio::test]
    async fn task_creation_time_is_preserved() {
        let store = create_test_store().await;
        let creation_time = Utc::now() - Duration::hours(2);
        let task = spawn_task();

        let (task_id, _) = store
            .add_session(task, creation_time, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // Timestamps may lose sub-millisecond precision, so check within 1 second
        let diff = (fetched.timestamp - creation_time).num_seconds().abs();
        assert!(
            diff <= 1,
            "Creation time should be preserved; got diff={diff}s"
        );
    }

    #[tokio::test]
    async fn status_log_failed_task() {
        let store = create_test_store().await;
        let task = spawn_task();
        let (task_id, _) = store
            .add_session(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut running = task.clone();
        running.status = Status::Running;
        store
            .update_session(&task_id, running, &ActorRef::test())
            .await
            .unwrap();

        let mut failed = task.clone();
        failed.status = Status::Failed;
        failed.error = Some(TaskError::JobEngineError {
            reason: "OOM killed".to_string(),
        });
        store
            .update_session(&task_id, failed, &ActorRef::test())
            .await
            .unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.current_status(), Status::Failed);
        assert_eq!(log.events.len(), 3); // Created, Started, Failed
    }

    // ---- Agent helpers ----

    fn sample_agent(name: &str) -> Agent {
        Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            3,
            i32::MAX,
            false,
        )
    }

    // ---- Agent tests ----

    #[tokio::test]
    async fn add_and_get_agent() {
        let store = create_test_store().await;
        let agent = sample_agent("swe");

        store.add_agent(agent.clone()).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.name, "swe");
        assert_eq!(fetched.prompt_path, "/agents/swe/prompt.md");
        assert_eq!(fetched.max_tries, 3);
        assert!(!fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn add_agent_duplicate_returns_error() {
        let store = create_test_store().await;
        store.add_agent(sample_agent("swe")).await.unwrap();

        let err = store.add_agent(sample_agent("swe")).await.unwrap_err();
        assert!(matches!(err, StoreError::AgentAlreadyExists(_)));
    }

    #[tokio::test]
    async fn list_agents_excludes_deleted() {
        let store = create_test_store().await;
        store.add_agent(sample_agent("alpha")).await.unwrap();
        store.add_agent(sample_agent("beta")).await.unwrap();
        store.delete_agent("alpha").await.unwrap();

        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "beta");
    }

    #[tokio::test]
    async fn list_agents_sorted_by_name() {
        let store = create_test_store().await;
        store.add_agent(sample_agent("zebra")).await.unwrap();
        store.add_agent(sample_agent("alpha")).await.unwrap();

        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents[0].name, "alpha");
        assert_eq!(agents[1].name, "zebra");
    }

    #[tokio::test]
    async fn update_agent_changes_fields() {
        let store = create_test_store().await;
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut updated = sample_agent("swe");
        updated.max_tries = 5;
        updated.prompt_path = "/agents/swe/v2.md".to_string();
        store.update_agent(updated).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.max_tries, 5);
        assert_eq!(fetched.prompt_path, "/agents/swe/v2.md");
    }

    #[tokio::test]
    async fn update_nonexistent_agent_returns_error() {
        let store = create_test_store().await;
        let err = store
            .update_agent(sample_agent("missing"))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn delete_agent_soft_deletes() {
        let store = create_test_store().await;
        store.add_agent(sample_agent("swe")).await.unwrap();
        store.delete_agent("swe").await.unwrap();

        let err = store.get_agent("swe").await.unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn delete_nonexistent_agent_returns_error() {
        let store = create_test_store().await;
        let err = store.delete_agent("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn assignment_agent_uniqueness_on_add() {
        let store = create_test_store().await;
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        let err = store.add_agent(pm2).await.unwrap_err();
        assert!(matches!(err, StoreError::AssignmentAgentAlreadyExists));
    }

    #[tokio::test]
    async fn assignment_agent_uniqueness_on_update() {
        let store = create_test_store().await;
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_assignment_agent = true;
        let err = store.update_agent(swe_updated).await.unwrap_err();
        assert!(matches!(err, StoreError::AssignmentAgentAlreadyExists));
    }

    #[tokio::test]
    async fn assignment_agent_can_update_itself() {
        let store = create_test_store().await;
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        let mut pm_updated = sample_agent("pm");
        pm_updated.is_assignment_agent = true;
        pm_updated.max_tries = 10;
        store.update_agent(pm_updated).await.unwrap();

        let fetched = store.get_agent("pm").await.unwrap();
        assert_eq!(fetched.max_tries, 10);
        assert!(fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn deleted_assignment_agent_allows_new_one() {
        let store = create_test_store().await;
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.delete_agent("pm").await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        store.add_agent(pm2).await.unwrap();

        let fetched = store.get_agent("pm2").await.unwrap();
        assert!(fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn add_agent_after_soft_deletion_same_name() {
        let store = create_test_store().await;
        let agent = sample_agent("swe");
        store.add_agent(agent).await.unwrap();
        store.delete_agent("swe").await.unwrap();

        let mut agent2 = sample_agent("swe");
        agent2.prompt_path = "new/path".to_string();
        store.add_agent(agent2).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.prompt_path, "new/path");
        assert!(!fetched.deleted);
    }

    // ---- Label helpers ----

    fn sample_label(name: &str, color: &str) -> Label {
        Label::new(name.to_string(), color.parse().unwrap(), true, false)
    }

    // ---- Label tests ----

    #[tokio::test]
    async fn label_crud_round_trip() {
        let store = create_test_store().await;

        let label = sample_label("bug", "#e74c3c");
        let label_id = store.add_label(label.clone()).await.unwrap();

        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "bug");
        assert_eq!(fetched.color.as_ref(), "#e74c3c");
        assert!(!fetched.deleted);

        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, label_id);
        assert_eq!(results[0].1.name, "bug");

        let found = store.get_label_by_name("bug").await.unwrap();
        assert!(found.is_some());
        let (found_id, found_label) = found.unwrap();
        assert_eq!(found_id, label_id);
        assert_eq!(found_label.name, "bug");
    }

    #[tokio::test]
    async fn add_label_rejects_duplicates() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let err = store
            .add_label(sample_label("bug", "#3498db"))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::LabelAlreadyExists(name) if name == "bug"
        ));
    }

    #[tokio::test]
    async fn delete_label_soft_deletes() {
        let store = create_test_store().await;

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        store.delete_label(&label_id).await.unwrap();

        let err = store.get_label(&label_id).await.unwrap_err();
        assert!(matches!(err, StoreError::LabelNotFound(_)));

        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert!(results.is_empty());

        let mut query = SearchLabelsQuery::default();
        query.include_deleted = Some(true);
        let results = store.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.deleted);
    }

    #[tokio::test]
    async fn update_label_changes_name_and_color() {
        let store = create_test_store().await;

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let mut updated = store.get_label(&label_id).await.unwrap();
        updated.name = "defect".to_string();
        updated.color = "#3498db".parse().unwrap();
        updated.updated_at = Utc::now();
        store.update_label(&label_id, updated).await.unwrap();

        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "defect");
        assert_eq!(fetched.color.as_ref(), "#3498db");
    }

    #[tokio::test]
    async fn update_label_rejects_name_collision() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();
        let feature_id = store
            .add_label(sample_label("feature", "#3498db"))
            .await
            .unwrap();

        let mut updated = store.get_label(&feature_id).await.unwrap();
        updated.name = "bug".to_string();
        let err = store.update_label(&feature_id, updated).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::LabelAlreadyExists(name) if name == "bug"
        ));
    }

    #[tokio::test]
    async fn update_label_allows_same_name() {
        let store = create_test_store().await;

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let mut updated = store.get_label(&label_id).await.unwrap();
        updated.color = "#3498db".parse().unwrap();
        store.update_label(&label_id, updated).await.unwrap();

        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "bug");
        assert_eq!(fetched.color.as_ref(), "#3498db");
    }

    #[tokio::test]
    async fn get_label_by_name_case_insensitive() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let found = store.get_label_by_name("BUG").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().1.name, "bug");
    }

    #[tokio::test]
    async fn list_labels_filters_by_query() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();
        store
            .add_label(sample_label("feature", "#3498db"))
            .await
            .unwrap();
        store
            .add_label(sample_label("bugfix", "#2ecc71"))
            .await
            .unwrap();

        let mut query = SearchLabelsQuery::default();
        query.q = Some("bug".to_string());
        let results = store.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        // Results sorted by name (no pagination params)
        assert_eq!(results[0].1.name, "bug");
        assert_eq!(results[1].1.name, "bugfix");
    }

    #[tokio::test]
    async fn list_labels_sorted_by_name() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("zebra", "#000000"))
            .await
            .unwrap();
        store
            .add_label(sample_label("alpha", "#111111"))
            .await
            .unwrap();
        store
            .add_label(sample_label("middle", "#222222"))
            .await
            .unwrap();

        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        // Without pagination params, sorted alphabetically by name
        assert_eq!(results[0].1.name, "alpha");
        assert_eq!(results[1].1.name, "middle");
        assert_eq!(results[2].1.name, "zebra");
    }

    // ---- Label association tests ----

    #[tokio::test]
    async fn label_association_add_and_query() {
        let store = create_test_store().await;

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let issue_id: MetisId = IssueId::new().into();

        let added = store
            .add_label_association(&label_id, &issue_id)
            .await
            .unwrap();
        assert!(added);

        // Adding again should be a no-op
        let added_again = store
            .add_label_association(&label_id, &issue_id)
            .await
            .unwrap();
        assert!(!added_again);

        // Query labels for object
        let labels = store.get_labels_for_object(&issue_id).await.unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name, "bug");

        // Query objects for label
        let objects = store.get_objects_for_label(&label_id).await.unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0], issue_id);
    }

    #[tokio::test]
    async fn label_association_remove() {
        let store = create_test_store().await;

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let issue_id: MetisId = IssueId::new().into();

        store
            .add_label_association(&label_id, &issue_id)
            .await
            .unwrap();

        let removed = store
            .remove_label_association(&label_id, &issue_id)
            .await
            .unwrap();
        assert!(removed);

        // Removing again should return false
        let removed_again = store
            .remove_label_association(&label_id, &issue_id)
            .await
            .unwrap();
        assert!(!removed_again);

        let labels = store.get_labels_for_object(&issue_id).await.unwrap();
        assert!(labels.is_empty());
    }

    #[tokio::test]
    async fn get_labels_for_objects_batch() {
        let store = create_test_store().await;

        let label1_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();
        let label2_id = store
            .add_label(sample_label("feature", "#3498db"))
            .await
            .unwrap();

        let issue1: MetisId = IssueId::new().into();
        let issue2: MetisId = IssueId::new().into();
        let issue3: MetisId = IssueId::new().into();

        store
            .add_label_association(&label1_id, &issue1)
            .await
            .unwrap();
        store
            .add_label_association(&label2_id, &issue1)
            .await
            .unwrap();
        store
            .add_label_association(&label1_id, &issue2)
            .await
            .unwrap();

        let result = store
            .get_labels_for_objects(&[issue1.clone(), issue2.clone(), issue3.clone()])
            .await
            .unwrap();

        // issue1 has 2 labels
        assert_eq!(result.get(&issue1).map(|v| v.len()).unwrap_or(0), 2);
        // issue2 has 1 label
        assert_eq!(result.get(&issue2).map(|v| v.len()).unwrap_or(0), 1);
        // issue3 has no labels (may or may not be in map)
        assert_eq!(result.get(&issue3).map(|v| v.len()).unwrap_or(0), 0);
    }

    #[tokio::test]
    async fn get_labels_for_objects_empty_input() {
        let store = create_test_store().await;
        let result = store.get_labels_for_objects(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    // ---- Notification tests ----

    fn sample_notification(recipient: &ActorId) -> Notification {
        Notification {
            recipient: recipient.clone(),
            source_actor: None,
            object_kind: "issue".to_string(),
            object_id: IssueId::new().into(),
            object_version: 1,
            event_type: "created".to_string(),
            summary: "A test notification".to_string(),
            source_issue_id: None,
            policy: "walk_up".to_string(),
            is_read: false,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn insert_and_get_notification() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();
        let notif = sample_notification(&recipient);

        let id = store.insert_notification(notif.clone()).await.unwrap();
        let fetched = store.get_notification(&id).await.unwrap();

        assert_eq!(fetched.recipient, notif.recipient);
        assert_eq!(fetched.object_kind, "issue");
        assert_eq!(fetched.event_type, "created");
        assert_eq!(fetched.summary, "A test notification");
        assert!(!fetched.is_read);
    }

    #[tokio::test]
    async fn get_notification_not_found() {
        let store = create_test_store().await;
        let id = NotificationId::from_str("nf-nonexistent").unwrap();
        let err = store.get_notification(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::NotificationNotFound(_)));
    }

    #[tokio::test]
    async fn list_notifications_returns_inserted() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        let id = store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();

        let mut query = ListNotificationsQuery::default();
        query.recipient = Some("u-alice".to_string());
        let results = store.list_notifications(&query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
    }

    #[tokio::test]
    async fn list_notifications_filters_by_is_read() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        let id1 = store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();

        store.mark_notification_read(&id1).await.unwrap();

        let mut query = ListNotificationsQuery::default();
        query.recipient = Some("u-alice".to_string());
        query.is_read = Some(false);
        let unread = store.list_notifications(&query).await.unwrap();
        assert_eq!(unread.len(), 1);

        let mut query = ListNotificationsQuery::default();
        query.recipient = Some("u-alice".to_string());
        query.is_read = Some(true);
        let read = store.list_notifications(&query).await.unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].0, id1);
    }

    #[tokio::test]
    async fn count_unread_notifications_returns_correct_count() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );

        let id1 = store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            2
        );

        store.mark_notification_read(&id1).await.unwrap();
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn mark_notification_read_not_found() {
        let store = create_test_store().await;
        let id = NotificationId::from_str("nf-nonexistent").unwrap();
        let err = store.mark_notification_read(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::NotificationNotFound(_)));
    }

    #[tokio::test]
    async fn mark_all_notifications_read_marks_all() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(&recipient))
            .await
            .unwrap();

        let count = store
            .mark_all_notifications_read(&recipient, None)
            .await
            .unwrap();
        assert_eq!(count, 2);

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn mark_all_notifications_read_respects_before() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        let mut older = sample_notification(&recipient);
        older.created_at = Utc::now() - Duration::hours(2);
        store.insert_notification(older).await.unwrap();

        let mut newer = sample_notification(&recipient);
        newer.created_at = Utc::now() + Duration::hours(2);
        store.insert_notification(newer).await.unwrap();

        let cutoff = Utc::now();
        let count = store
            .mark_all_notifications_read(&recipient, Some(cutoff))
            .await
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_notifications_respects_limit() {
        let store = create_test_store().await;
        let recipient = ActorId::from_str("alice").unwrap();

        for _ in 0..5 {
            store
                .insert_notification(sample_notification(&recipient))
                .await
                .unwrap();
        }

        let mut query = ListNotificationsQuery::default();
        query.recipient = Some("u-alice".to_string());
        query.limit = Some(3);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    // ---- Message tests ----

    fn sample_message() -> Message {
        Message {
            sender: Some(ActorId::from_str("alice").unwrap()),
            recipient: ActorId::from_str("bob").unwrap(),
            body: "Hello Bob!".to_string(),
            deleted: false,
            is_read: false,
        }
    }

    #[tokio::test]
    async fn add_and_get_message() {
        let store = create_test_store().await;
        let msg = sample_message();

        let (id, version) = store
            .add_message(msg.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_message(&id).await.unwrap();
        assert_eq!(fetched.item.body, "Hello Bob!");
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.sender, msg.sender);
        assert_eq!(fetched.item.recipient, msg.recipient);
    }

    #[tokio::test]
    async fn get_message_not_found() {
        let store = create_test_store().await;
        let id = MessageId::from_str("m-nonexistent").unwrap();
        let err = store.get_message(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::MessageNotFound(_)));
    }

    #[tokio::test]
    async fn update_message_creates_new_version() {
        let store = create_test_store().await;
        let msg = sample_message();

        let (id, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let mut updated = sample_message();
        updated.body = "Updated body".to_string();
        let v2 = store
            .update_message(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let fetched = store.get_message(&id).await.unwrap();
        assert_eq!(fetched.item.body, "Updated body");
        assert_eq!(fetched.version, 2);
    }

    #[tokio::test]
    async fn list_messages_returns_latest_version() {
        let store = create_test_store().await;
        let msg = sample_message();

        let (id, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let mut updated = sample_message();
        updated.body = "Updated".to_string();
        store
            .update_message(&id, updated, &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchMessagesQuery::default();
        query.recipient = Some("u-bob".to_string());
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.body, "Updated");
        assert_eq!(results[0].1.version, 2);
    }

    #[tokio::test]
    async fn list_messages_filters_deleted() {
        let store = create_test_store().await;

        let (id, _) = store
            .add_message(sample_message(), &ActorRef::test())
            .await
            .unwrap();

        let mut deleted = sample_message();
        deleted.deleted = true;
        store
            .update_message(&id, deleted, &ActorRef::test())
            .await
            .unwrap();

        let results = store
            .list_messages(&SearchMessagesQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 0);

        let mut query = SearchMessagesQuery::default();
        query.include_deleted = Some(true);
        let results_with_deleted = store.list_messages(&query).await.unwrap();
        assert_eq!(results_with_deleted.len(), 1);
    }

    // ---- User secret tests ----

    #[tokio::test]
    async fn set_and_get_user_secret() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());
        let secret = b"supersecret";

        store
            .set_user_secret(&username, "api_key", secret, false)
            .await
            .unwrap();

        let fetched = store.get_user_secret(&username, "api_key").await.unwrap();
        assert_eq!(fetched, Some(secret.to_vec()));
    }

    #[tokio::test]
    async fn get_user_secret_returns_none_when_missing() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        let fetched = store
            .get_user_secret(&username, "nonexistent")
            .await
            .unwrap();
        assert_eq!(fetched, None);
    }

    #[tokio::test]
    async fn set_user_secret_overwrites() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "api_key", b"first", false)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "api_key", b"second", false)
            .await
            .unwrap();

        let fetched = store.get_user_secret(&username, "api_key").await.unwrap();
        assert_eq!(fetched, Some(b"second".to_vec()));
    }

    #[tokio::test]
    async fn list_user_secret_names_returns_sorted() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "zebra", b"z", false)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "alpha", b"a", false)
            .await
            .unwrap();

        let refs = store.list_user_secret_names(&username).await.unwrap();
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zebra"]);
        assert!(refs.iter().all(|r| !r.internal));
    }

    #[tokio::test]
    async fn delete_user_secret_removes_entry() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "api_key", b"secret", false)
            .await
            .unwrap();

        store
            .delete_user_secret(&username, "api_key")
            .await
            .unwrap();

        let fetched = store.get_user_secret(&username, "api_key").await.unwrap();
        assert_eq!(fetched, None);
    }

    #[tokio::test]
    async fn delete_user_secret_noop_when_missing() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        // Should not error even if secret doesn't exist
        store
            .delete_user_secret(&username, "nonexistent")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_user_secret_names_isolated_by_user() {
        let store = create_test_store().await;
        let alice = Username::from("alice".to_string());
        let bob = Username::from("bob".to_string());

        store
            .set_user_secret(&alice, "key_a", b"a", false)
            .await
            .unwrap();
        store
            .set_user_secret(&bob, "key_b", b"b", false)
            .await
            .unwrap();

        let alice_refs = store.list_user_secret_names(&alice).await.unwrap();
        let alice_names: Vec<&str> = alice_refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(alice_names, vec!["key_a"]);

        let bob_refs = store.list_user_secret_names(&bob).await.unwrap();
        let bob_names: Vec<&str> = bob_refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(bob_names, vec!["key_b"]);
    }

    #[tokio::test]
    async fn is_secret_internal_returns_true_for_internal() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "INTERNAL_KEY", b"val", true)
            .await
            .unwrap();

        assert!(
            store
                .is_secret_internal(&username, "INTERNAL_KEY")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn is_secret_internal_returns_false_for_non_internal() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "USER_KEY", b"val", false)
            .await
            .unwrap();

        assert!(
            !store
                .is_secret_internal(&username, "USER_KEY")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn is_secret_internal_returns_false_for_nonexistent() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        assert!(
            !store
                .is_secret_internal(&username, "MISSING")
                .await
                .unwrap()
        );
    }

    // ---- Count tests ----

    #[tokio::test]
    async fn count_issues_returns_total_matching() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        // Create 5 issues: 3 open tasks, 1 open bug, 1 closed task
        for _ in 0..3 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        }
        let bug = Issue::new(
            IssueType::Bug,
            "Bug Title".to_string(),
            "a bug".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        store.add_issue(bug, &actor).await.unwrap();

        let closed = Issue::new(
            IssueType::Task,
            "Closed".to_string(),
            "closed task".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Closed,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        store.add_issue(closed, &actor).await.unwrap();

        // Count all issues
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);

        // Count only bugs
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Bug),
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);

        // Count only closed
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            Some(metis_common::api::v1::issues::IssueStatus::Closed),
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn count_patches_returns_total_matching() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        for _ in 0..3 {
            store.add_patch(sample_patch(), &actor).await.unwrap();
        }

        let query =
            metis_common::api::v1::patches::SearchPatchesQuery::new(None, None, Vec::new(), None);
        assert_eq!(store.count_patches(&query).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn count_documents_returns_total_matching() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        store
            .add_document(sample_document(Some("docs/a.md"), None), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("docs/b.md"), None), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("other/c.md"), None), &actor)
            .await
            .unwrap();

        // Count all
        let query = metis_common::api::v1::documents::SearchDocumentsQuery::new(
            None, None, None, None, None,
        );
        assert_eq!(store.count_documents(&query).await.unwrap(), 3);

        // Count with path prefix filter
        let query = metis_common::api::v1::documents::SearchDocumentsQuery::new(
            Some("docs/".to_string()),
            None,
            None,
            None,
            None,
        );
        assert_eq!(store.count_documents(&query).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn count_tasks_returns_total_matching() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        for _ in 0..4 {
            store
                .add_session(spawn_task(), Utc::now(), &actor)
                .await
                .unwrap();
        }

        let query =
            metis_common::api::v1::sessions::SearchSessionsQuery::new(None, None, None, vec![]);
        assert_eq!(store.count_sessions(&query).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn count_labels_returns_total_matching() {
        let store = create_test_store().await;

        store
            .add_label(sample_label("bug", "#000000"))
            .await
            .unwrap();
        store
            .add_label(sample_label("feature", "#000000"))
            .await
            .unwrap();
        store
            .add_label(sample_label("bugfix", "#000000"))
            .await
            .unwrap();

        // Count all
        let query = SearchLabelsQuery::default();
        assert_eq!(store.count_labels(&query).await.unwrap(), 3);

        // Count with search filter
        let mut query = SearchLabelsQuery::default();
        query.q = Some("bug".to_string());
        assert_eq!(store.count_labels(&query).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn count_issues_ignores_pagination() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        for _ in 0..5 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Count should return 5 even when limit is set
        let mut query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        query.limit = Some(2);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);
    }

    #[tokio::test]
    async fn has_document_relationship_round_trip() {
        use crate::store::RelationshipType;

        let store = create_test_store().await;
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (doc_id, _) = store
            .add_document(sample_document(None, None), &actor)
            .await
            .unwrap();

        let source = MetisId::from(issue_id.clone());
        let target = MetisId::from(doc_id.clone());

        store
            .add_relationship(&source, &target, RelationshipType::HasDocument)
            .await
            .unwrap();

        let rels = store
            .get_relationships(Some(&source), None, Some(RelationshipType::HasDocument))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, source);
        assert_eq!(rels[0].target_id, target);
        assert_eq!(rels[0].rel_type, RelationshipType::HasDocument);
    }

    #[tokio::test]
    async fn get_relationships_batch_filters_by_multiple_sources() {
        use crate::store::RelationshipType;

        let store = create_test_store().await;
        let actor = ActorRef::test();

        let (id1, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id2, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id3, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let sid1 = MetisId::from(id1.clone());
        let sid2 = MetisId::from(id2.clone());
        let sid3 = MetisId::from(id3.clone());
        let tpid = MetisId::from(pid.clone());

        store
            .add_relationship(&sid1, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();
        store
            .add_relationship(&sid2, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();
        store
            .add_relationship(&sid3, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();

        // Batch query for id1 and id2 only
        let results = store
            .get_relationships_batch(
                Some(&[sid1.clone(), sid2.clone()]),
                None,
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Empty source_ids returns empty
        let results = store
            .get_relationships_batch(Some(&[]), None, None)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn get_relationships_transitive_follows_same_type_only() {
        use crate::store::RelationshipType;

        let store = create_test_store().await;
        let actor = ActorRef::test();

        // Create 3 issues: A -> B -> C (child-of chain)
        // Also B -> patch (has-patch, should NOT be followed)
        let (id_a, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id_b, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id_c, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let a = MetisId::from(id_a.clone());
        let b = MetisId::from(id_b.clone());
        let c = MetisId::from(id_c.clone());
        let p = MetisId::from(pid.clone());

        // A is child-of B, B is child-of C
        store
            .add_relationship(&a, &b, RelationshipType::ChildOf)
            .await
            .unwrap();
        store
            .add_relationship(&b, &c, RelationshipType::ChildOf)
            .await
            .unwrap();
        // B has-patch P (different rel_type)
        store
            .add_relationship(&b, &p, RelationshipType::HasPatch)
            .await
            .unwrap();

        // Forward transitive from A following child-of
        let results = store
            .get_relationships_transitive(Some(&a), None, RelationshipType::ChildOf)
            .await
            .unwrap();
        // Should find A->B and B->C, but NOT B->P
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| r.rel_type == RelationshipType::ChildOf)
        );

        // Backward transitive from C following child-of
        let results = store
            .get_relationships_transitive(None, Some(&c), RelationshipType::ChildOf)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Transitive has-patch from B should only find B->P
        let results = store
            .get_relationships_transitive(Some(&b), None, RelationshipType::HasPatch)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, p);
    }
}
