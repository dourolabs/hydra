//! PostgresStoreV2 implementation using column-based v2 tables.
//!
//! This store implementation uses the v2 tables with proper column definitions
//! instead of JSONB payloads, providing better query performance and type safety.

use crate::{
    domain::{
        actors::{Actor, UserOrWorker},
        documents::Document,
        issues::{
            Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
            JobSettings, TodoItem,
        },
        jobs::{BundleSpec, Task},
        patches::{GithubPr, Patch, PatchStatus, Review},
        task_status::{Status, TaskError},
        users::{User, Username},
    },
    store::{Store, StoreError, TaskStatusLog},
};
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
use serde_json::Value;
use std::{collections::HashMap, collections::HashSet, str::FromStr};

use super::issue_graph::IssueGraphContext;
use super::postgres::PgStorePool;

const TABLE_ISSUES_V2: &str = "metis.issues_v2";
const TABLE_PATCHES_V2: &str = "metis.patches_v2";
const TABLE_TASKS_V2: &str = "metis.tasks_v2";
const TABLE_USERS_V2: &str = "metis.users_v2";
const TABLE_REPOSITORIES_V2: &str = "metis.repositories_v2";
const TABLE_ACTORS_V2: &str = "metis.actors_v2";
const TABLE_DOCUMENTS_V2: &str = "metis.documents_v2";

/// PostgresStoreV2 uses the v2 tables with proper column definitions.
#[derive(Clone)]
pub struct PostgresStoreV2 {
    pool: PgStorePool,
}

impl PostgresStoreV2 {
    pub fn new(pool: PgStorePool) -> Self {
        Self { pool }
    }

    async fn ensure_issue_exists(&self, id: &IssueId) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ISSUES_V2} WHERE id = $1"
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
            "SELECT COUNT(1) FROM {TABLE_PATCHES_V2} WHERE id = $1"
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
            "SELECT COUNT(1) FROM {TABLE_TASKS_V2} WHERE id = $1"
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
            "SELECT COUNT(1) FROM {TABLE_REPOSITORIES_V2} WHERE id = $1"
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

    // -------------------------------------------------------------------------
    // Issue helpers
    // -------------------------------------------------------------------------

    async fn insert_issue(
        &self,
        id: &IssueId,
        version_number: VersionNumber,
        issue: &Issue,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let job_settings_json = serde_json::to_value(&issue.job_settings)
            .map_err(|e| StoreError::Internal(format!("failed to serialize job_settings: {e}")))?;
        let todo_list_json = serde_json::to_value(&issue.todo_list)
            .map_err(|e| StoreError::Internal(format!("failed to serialize todo_list: {e}")))?;
        let dependencies_json = serde_json::to_value(&issue.dependencies)
            .map_err(|e| StoreError::Internal(format!("failed to serialize dependencies: {e}")))?;
        let patches_json = serde_json::to_value(&issue.patches)
            .map_err(|e| StoreError::Internal(format!("failed to serialize patches: {e}")))?;

        let query = format!(
            "INSERT INTO {TABLE_ISSUES_V2} (id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(issue.issue_type.as_str())
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
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_issue(&self, row: &IssueRow) -> Result<Issue, StoreError> {
        let issue_type = IssueType::from_str(&row.issue_type)
            .map_err(|e| StoreError::Internal(format!("invalid issue_type: {e}")))?;
        let status = IssueStatus::from_str(&row.status).map_err(StoreError::InvalidIssueStatus)?;
        let job_settings: JobSettings =
            serde_json::from_value(row.job_settings.clone()).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize job_settings: {e}"))
            })?;
        let todo_list: Vec<TodoItem> = serde_json::from_value(row.todo_list.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize todo_list: {e}")))?;
        let dependencies: Vec<IssueDependency> = serde_json::from_value(row.dependencies.clone())
            .map_err(|e| {
            StoreError::Internal(format!("failed to deserialize dependencies: {e}"))
        })?;
        let patches: Vec<PatchId> = serde_json::from_value(row.patches.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize patches: {e}")))?;

        Ok(Issue {
            issue_type,
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

    // -------------------------------------------------------------------------
    // Patch helpers
    // -------------------------------------------------------------------------

    async fn insert_patch(
        &self,
        id: &PatchId,
        version_number: VersionNumber,
        patch: &Patch,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for patch '{id}'"))
        })?;

        let reviews_json = serde_json::to_value(&patch.reviews)
            .map_err(|e| StoreError::Internal(format!("failed to serialize reviews: {e}")))?;
        let github_json = patch
            .github
            .as_ref()
            .map(|g| {
                serde_json::to_value(g)
                    .map_err(|e| StoreError::Internal(format!("failed to serialize github: {e}")))
            })
            .transpose()?;

        let query = format!(
            "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&patch.title)
            .bind(&patch.description)
            .bind(&patch.diff)
            .bind(patch.status.as_str())
            .bind(patch.is_automatic_backup)
            .bind(patch.created_by.as_ref().map(|t| t.as_ref()))
            .bind(&reviews_json)
            .bind(patch.service_repo_name.as_str())
            .bind(&github_json)
            .bind(patch.deleted)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_patch(&self, row: &PatchRow) -> Result<Patch, StoreError> {
        let status = PatchStatus::from_str(&row.status)
            .map_err(|e| StoreError::Internal(format!("invalid patch status: {e}")))?;
        let reviews: Vec<Review> = serde_json::from_value(row.reviews.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize reviews: {e}")))?;
        let github: Option<GithubPr> = row
            .github
            .as_ref()
            .map(|g| {
                serde_json::from_value(g.clone())
                    .map_err(|e| StoreError::Internal(format!("failed to deserialize github: {e}")))
            })
            .transpose()?;
        let service_repo_name = RepoName::from_str(&row.service_repo_name)
            .map_err(|e| StoreError::Internal(format!("invalid service_repo_name: {e}")))?;
        let created_by = row
            .created_by
            .as_ref()
            .map(|s| {
                TaskId::from_str(s)
                    .map_err(|e| StoreError::Internal(format!("invalid created_by task id: {e}")))
            })
            .transpose()?;

        Ok(Patch {
            title: row.title.clone(),
            description: row.description.clone(),
            diff: row.diff.clone(),
            status,
            is_automatic_backup: row.is_automatic_backup,
            created_by,
            reviews,
            service_repo_name,
            github,
            deleted: row.deleted,
        })
    }

    // -------------------------------------------------------------------------
    // Task helpers
    // -------------------------------------------------------------------------

    async fn insert_task(
        &self,
        id: &TaskId,
        version_number: VersionNumber,
        task: &Task,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for task '{id}'"))
        })?;

        let context_json = serde_json::to_value(&task.context)
            .map_err(|e| StoreError::Internal(format!("failed to serialize context: {e}")))?;
        let env_vars_json = serde_json::to_value(&task.env_vars)
            .map_err(|e| StoreError::Internal(format!("failed to serialize env_vars: {e}")))?;
        let error_json = task
            .error
            .as_ref()
            .map(|e| {
                serde_json::to_value(e).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize error: {err}"))
                })
            })
            .transpose()?;

        let status_str = match task.status {
            Status::Created => "created",
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Complete => "complete",
            Status::Failed => "failed",
        };

        let query = format!(
            "INSERT INTO {TABLE_TASKS_V2} (id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&task.prompt)
            .bind(&context_json)
            .bind(task.spawned_from.as_ref().map(|i| i.as_ref()))
            .bind(task.image.as_deref())
            .bind(task.model.as_deref())
            .bind(&env_vars_json)
            .bind(task.cpu_limit.as_deref())
            .bind(task.memory_limit.as_deref())
            .bind(status_str)
            .bind(task.last_message.as_deref())
            .bind(&error_json)
            .bind(task.deleted)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_task(&self, row: &TaskRow) -> Result<Task, StoreError> {
        let context: BundleSpec = serde_json::from_value(row.context.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize context: {e}")))?;
        let env_vars: HashMap<String, String> = serde_json::from_value(row.env_vars.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize env_vars: {e}")))?;
        let error: Option<TaskError> = row
            .error
            .as_ref()
            .map(|e| {
                serde_json::from_value(e.clone()).map_err(|err| {
                    StoreError::Internal(format!("failed to deserialize error: {err}"))
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

        Ok(Task {
            prompt: row.prompt.clone(),
            context,
            spawned_from,
            image: row.image.clone(),
            model: row.model.clone(),
            env_vars,
            cpu_limit: row.cpu_limit.clone(),
            memory_limit: row.memory_limit.clone(),
            status,
            last_message: row.last_message.clone(),
            error,
            deleted: row.deleted,
        })
    }

    // -------------------------------------------------------------------------
    // Document helpers
    // -------------------------------------------------------------------------

    async fn insert_document(
        &self,
        id: &DocumentId,
        version_number: VersionNumber,
        document: &Document,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_DOCUMENTS_V2} (id, version_number, title, body_markdown, path, created_by, deleted)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&document.title)
            .bind(&document.body_markdown)
            .bind(document.path.as_deref())
            .bind(document.created_by.as_ref().map(|t| t.as_ref()))
            .bind(document.deleted)
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
                TaskId::from_str(s)
                    .map_err(|e| StoreError::Internal(format!("invalid created_by task id: {e}")))
            })
            .transpose()?;

        Ok(Document {
            title: row.title.clone(),
            body_markdown: row.body_markdown.clone(),
            path: row.path.clone(),
            created_by,
            deleted: row.deleted,
        })
    }

    // -------------------------------------------------------------------------
    // Repository helpers
    // -------------------------------------------------------------------------

    async fn insert_repository(
        &self,
        id: &str,
        version_number: VersionNumber,
        repo: &Repository,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for repository '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_REPOSITORIES_V2} (id, version_number, remote_url, default_branch, default_image, deleted)
             VALUES ($1, $2, $3, $4, $5, $6)"
        );
        sqlx::query(&query)
            .bind(id)
            .bind(version_number)
            .bind(&repo.remote_url)
            .bind(repo.default_branch.as_deref())
            .bind(repo.default_image.as_deref())
            .bind(repo.deleted)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_repository(&self, row: &RepositoryRow) -> Repository {
        let mut repo = Repository::new(
            row.remote_url.clone(),
            row.default_branch.clone(),
            row.default_image.clone(),
        );
        repo.deleted = row.deleted;
        repo
    }

    // -------------------------------------------------------------------------
    // User helpers
    // -------------------------------------------------------------------------

    async fn insert_user(
        &self,
        id: &str,
        version_number: VersionNumber,
        user: &User,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for user '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_USERS_V2} (id, version_number, username, github_user_id, github_token, github_refresh_token, deleted)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        );
        sqlx::query(&query)
            .bind(id)
            .bind(version_number)
            .bind(user.username.as_str())
            .bind(user.github_user_id as i64)
            .bind(&user.github_token)
            .bind(&user.github_refresh_token)
            .bind(user.deleted)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_user(&self, row: &UserRow) -> User {
        User::new(
            Username::from(row.username.clone()),
            row.github_user_id as u64,
            row.github_token.clone(),
            row.github_refresh_token.clone(),
        )
        .with_deleted(row.deleted)
    }

    async fn fetch_latest_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        // Build query with filtering on latest version of each user
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, created_at, updated_at
             FROM {TABLE_USERS_V2}
             ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        // Filter deleted users by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("NOT deleted".to_string());
        }

        if let Some(term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            // Search across id (username) field
            let idx_start = bindings.len() + 1;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} OR LOWER(username) LIKE ${idx_username})",
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
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for user '{}'",
                    row.id
                ))
            })?;
            let user = self.row_to_user(&row);
            let username = Username::from(row.id);
            users.push((username, Versioned::new(user, version, row.created_at)));
        }

        Ok(users)
    }

    // -------------------------------------------------------------------------
    // Actor helpers
    // -------------------------------------------------------------------------

    async fn insert_actor(
        &self,
        id: &str,
        version_number: VersionNumber,
        actor: &Actor,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for actor '{id}'"))
        })?;

        let user_or_worker_json = serde_json::to_value(&actor.user_or_worker).map_err(|e| {
            StoreError::Internal(format!("failed to serialize user_or_worker: {e}"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_ACTORS_V2} (id, version_number, auth_token_hash, auth_token_salt, user_or_worker)
             VALUES ($1, $2, $3, $4, $5)"
        );
        sqlx::query(&query)
            .bind(id)
            .bind(version_number)
            .bind(&actor.auth_token_hash)
            .bind(&actor.auth_token_salt)
            .bind(&user_or_worker_json)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_actor(&self, row: &ActorRow) -> Result<Actor, StoreError> {
        let user_or_worker: UserOrWorker = serde_json::from_value(row.user_or_worker.clone())
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize user_or_worker: {e}"))
            })?;

        Ok(Actor {
            auth_token_hash: row.auth_token_hash.clone(),
            auth_token_salt: row.auth_token_salt.clone(),
            user_or_worker,
        })
    }
}

// -----------------------------------------------------------------------------
// Row structs for sqlx queries
// -----------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct IssueRow {
    id: String,
    version_number: i64,
    issue_type: String,
    description: String,
    creator: String,
    progress: String,
    status: String,
    assignee: Option<String>,
    job_settings: Value,
    todo_list: Value,
    dependencies: Value,
    patches: Value,
    deleted: bool,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
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
    reviews: Value,
    service_repo_name: String,
    github: Option<Value>,
    deleted: bool,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct TaskRow {
    id: String,
    version_number: i64,
    prompt: String,
    context: Value,
    spawned_from: Option<String>,
    image: Option<String>,
    model: Option<String>,
    env_vars: Value,
    cpu_limit: Option<String>,
    memory_limit: Option<String>,
    status: String,
    last_message: Option<String>,
    error: Option<Value>,
    deleted: bool,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
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
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: String,
    version_number: i64,
    remote_url: String,
    default_branch: Option<String>,
    default_image: Option<String>,
    deleted: bool,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    version_number: i64,
    username: String,
    github_user_id: i64,
    github_token: String,
    github_refresh_token: String,
    deleted: bool,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ActorRow {
    id: String,
    version_number: i64,
    auth_token_hash: String,
    auth_token_salt: String,
    user_or_worker: Value,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    StoreError::Internal(err.to_string())
}

#[async_trait]
impl Store for PostgresStoreV2 {
    // -------------------------------------------------------------------------
    // Repository methods
    // -------------------------------------------------------------------------

    async fn add_repository(&self, name: RepoName, config: Repository) -> Result<(), StoreError> {
        let name_str = name.as_str();

        // Check if repository exists (including deleted)
        let existing = self.get_repository(&name).await;

        match existing {
            Ok(repo) if repo.item.deleted => {
                // Re-create over deleted: use caller's config as-is
                self.update_repository(name, config).await
            }
            Ok(_) => Err(StoreError::RepositoryAlreadyExists(name)),
            Err(StoreError::RepositoryNotFound(_)) => {
                self.insert_repository(name_str.as_str(), 1, &config).await
            }
            Err(e) => Err(e),
        }
    }

    async fn get_repository(&self, name: &RepoName) -> Result<Versioned<Repository>, StoreError> {
        let name_str = name.as_str();
        let query = format!(
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, created_at, updated_at
             FROM {TABLE_REPOSITORIES_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, RepositoryRow>(&query)
            .bind(name_str.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for repository '{}'",
                row.id
            ))
        })?;
        let repo = self.row_to_repository(&row);
        Ok(Versioned::new(repo, version, row.created_at))
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
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

        self.insert_repository(name_str.as_str(), next_version, &config)
            .await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let sql = format!(
            "SELECT DISTINCT ON (id) id, version_number, remote_url, default_branch, default_image, deleted, created_at, updated_at
             FROM {TABLE_REPOSITORIES_V2}
             ORDER BY id, version_number DESC"
        );
        let rows = sqlx::query_as::<_, RepositoryRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            // Skip deleted repositories unless include_deleted is true
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
            let repo = self.row_to_repository(&row);
            results.push((name, Versioned::new(repo, version, row.created_at)));
        }

        results.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(results)
    }

    async fn delete_repository(&self, name: &RepoName) -> Result<(), StoreError> {
        let current = self.get_repository(name).await?;
        let mut repo = current.item;
        repo.deleted = true;
        self.update_repository(name.clone(), repo).await
    }

    // -------------------------------------------------------------------------
    // Issue methods
    // -------------------------------------------------------------------------

    async fn add_issue(&self, issue: Issue) -> Result<IssueId, StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        let id = IssueId::new();
        self.insert_issue(&id, 1, &issue).await?;
        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Versioned<Issue>, StoreError> {
        let query = format!(
            "SELECT id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, created_at, updated_at
             FROM {TABLE_ISSUES_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, IssueRow>(&query)
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
        Ok(Versioned::new(issue, version, row.created_at))
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, created_at, updated_at
             FROM {TABLE_ISSUES_V2}
             WHERE id = $1
             ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, IssueRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for issue '{}'",
                    row.id
                ))
            })?;
            let issue = self.row_to_issue(&row)?;
            results.push(Versioned::new(issue, version, row.created_at));
        }

        Ok(results)
    }

    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        self.get_issue(id).await?;
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

        self.insert_issue(id, next_version, &issue).await
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        // Use a subquery to get the latest version of each issue first,
        // then apply filters. This ensures we filter on the current state
        // of each issue, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, issue_type, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, created_at, updated_at \
             FROM {TABLE_ISSUES_V2} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        // Filter by issue_type
        if let Some(issue_type) = query.issue_type.as_ref() {
            predicates.push(format!("issue_type = ${}", bindings.len() + 1));
            bindings.push(issue_type.as_str().to_string());
        }

        // Filter by status
        if let Some(status) = query.status.as_ref() {
            predicates.push(format!("status = ${}", bindings.len() + 1));
            bindings.push(status.as_str().to_string());
        }

        // Filter by assignee (case-insensitive)
        if let Some(assignee) = query
            .assignee
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            predicates.push(format!("LOWER(assignee) = ${}", bindings.len() + 1));
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
                 OR LOWER(description) LIKE ${idx_desc} \
                 OR LOWER(progress) LIKE ${idx_progress} \
                 OR issue_type = ${idx_type} \
                 OR status = ${idx_status} \
                 OR LOWER(creator) LIKE ${idx_creator} \
                 OR LOWER(COALESCE(assignee,'')) LIKE ${idx_assignee})"
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
            predicates.push("deleted = false".to_string());
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
            issues.push((issue_id, Versioned::new(issue, version, row.created_at)));
        }

        Ok(issues)
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

    // -------------------------------------------------------------------------
    // Patch methods
    // -------------------------------------------------------------------------

    async fn add_patch(&self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.insert_patch(&id, 1, &patch).await?;
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Versioned<Patch>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, created_at, updated_at
             FROM {TABLE_PATCHES_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, PatchRow>(&query)
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
        Ok(Versioned::new(patch, version, row.created_at))
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, created_at, updated_at
             FROM {TABLE_PATCHES_V2}
             WHERE id = $1
             ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, PatchRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::PatchNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for patch '{}'",
                    row.id
                ))
            })?;
            let patch = self.row_to_patch(&row)?;
            results.push(Versioned::new(patch, version, row.created_at));
        }

        Ok(results)
    }

    async fn update_patch(&self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        self.get_patch(id).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_PATCHES_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("patch '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for patch '{id}'"))
        })?;

        self.insert_patch(id, next_version, &patch).await
    }

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        // Use a subquery to get the latest version of each patch first,
        // then apply filters. This ensures we filter on the current state
        // of each patch, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, created_at, updated_at \
             FROM {TABLE_PATCHES_V2} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        // Filter deleted patches by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = false".to_string());
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
                 OR LOWER(title) LIKE ${idx_title} \
                 OR LOWER(description) LIKE ${idx_desc} \
                 OR LOWER(status) LIKE ${idx_status} \
                 OR LOWER(service_repo_name) LIKE ${idx_repo} \
                 OR LOWER(diff) LIKE ${idx_diff} \
                 OR LOWER(github->>'owner') LIKE ${idx_gh_owner} \
                 OR LOWER(github->>'repo') LIKE ${idx_gh_repo} \
                 OR (github->>'number') LIKE ${idx_gh_number} \
                 OR LOWER(COALESCE(github->>'head_ref','')) LIKE ${idx_gh_head} \
                 OR LOWER(COALESCE(github->>'base_ref','')) LIKE ${idx_gh_base})",
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
            patches.push((patch_id, Versioned::new(patch, version, row.created_at)));
        }

        Ok(patches)
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

    // -------------------------------------------------------------------------
    // Document methods
    // -------------------------------------------------------------------------

    async fn add_document(&self, document: Document) -> Result<DocumentId, StoreError> {
        let id = DocumentId::new();
        self.insert_document(&id, 1, &document).await?;
        Ok(id)
    }

    async fn get_document(&self, id: &DocumentId) -> Result<Versioned<Document>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, created_at, updated_at
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, DocumentRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;

        if row.deleted {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for document '{}'",
                row.id
            ))
        })?;
        let document = self.row_to_document(&row)?;
        Ok(Versioned::new(document, version, row.created_at))
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, created_at, updated_at
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = $1
             ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, DocumentRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for document '{}'",
                    row.id
                ))
            })?;
            let document = self.row_to_document(&row)?;
            results.push(Versioned::new(document, version, row.created_at));
        }

        Ok(results)
    }

    async fn update_document(&self, id: &DocumentId, document: Document) -> Result<(), StoreError> {
        self.get_document(id).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_DOCUMENTS_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("document '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        self.insert_document(id, next_version, &document).await
    }

    async fn delete_document(&self, id: &DocumentId) -> Result<(), StoreError> {
        let current = self.get_document(id).await?;
        let mut document = current.item;
        document.deleted = true;
        self.update_document(id, document).await
    }

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        // Use a subquery to get the latest version of each document first,
        // then apply filters. This ensures we filter on the current state
        // of each document, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, title, body_markdown, path, created_by, deleted, created_at, updated_at \
             FROM {TABLE_DOCUMENTS_V2} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        if let Some(path) = query.path_prefix.as_ref() {
            if query.path_is_exact.unwrap_or(false) {
                predicates.push(format!("COALESCE(path,'') = ${}", bindings.len() + 1));
                bindings.push(path.clone());
            } else {
                predicates.push(format!("COALESCE(path,'') LIKE ${}", bindings.len() + 1));
                bindings.push(format!("{path}%"));
            }
        }

        if let Some(created_by) = query.created_by.as_ref() {
            predicates.push(format!("created_by = ${}", bindings.len() + 1));
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
                "(LOWER(title) LIKE ${idx_title} \
                 OR LOWER(body_markdown) LIKE ${idx_body} \
                 OR LOWER(COALESCE(path,'')) LIKE ${idx_path})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone());
            bindings.push(pattern.clone());
            bindings.push(pattern);
        }

        // Filter deleted documents by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = false".to_string());
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
            documents.push((
                document_id,
                Versioned::new(document, version, row.created_at),
            ));
        }

        Ok(documents)
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

    // -------------------------------------------------------------------------
    // Task methods
    // -------------------------------------------------------------------------

    async fn add_task(
        &self,
        task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        let id = TaskId::new();
        self.add_task_with_id(id.clone(), task, _creation_time)
            .await?;
        Ok(id)
    }

    async fn add_task_with_id(
        &self,
        metis_id: TaskId,
        task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut task = task;
        task.status = Status::Created;
        task.last_message = None;
        task.error = None;
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_TASKS_V2} WHERE id = $1"
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

        self.insert_task(&metis_id, 1, &task).await
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

        let latest_version = self
            .fetch_latest_version_number(TABLE_TASKS_V2, metis_id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("task '{metis_id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for task '{metis_id}'"))
        })?;

        self.insert_task(metis_id, next_version, &task).await?;
        self.get_task(metis_id).await
    }

    async fn get_task(&self, id: &TaskId) -> Result<Versioned<Task>, StoreError> {
        let query = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at
             FROM {TABLE_TASKS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, TaskRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for task '{}'",
                row.id
            ))
        })?;
        let task = self.row_to_task(&row)?;
        Ok(Versioned::new(task, version, row.created_at))
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at
             FROM {TABLE_TASKS_V2}
             WHERE id = $1
             ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, TaskRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for task '{}'",
                    row.id
                ))
            })?;
            let task = self.row_to_task(&row)?;
            results.push(Versioned::new(task, version, row.created_at));
        }

        Ok(results)
    }

    async fn list_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        // Use a subquery to get the latest version of each task first,
        // then apply filters. This ensures we filter on the current state
        // of each task, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at \
             FROM {TABLE_TASKS_V2} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings: Vec<String> = Vec::new();

        // Filter by spawned_from
        if let Some(spawned_from) = query.spawned_from.as_ref() {
            predicates.push(format!("spawned_from = ${}", bindings.len() + 1));
            bindings.push(spawned_from.as_ref().to_string());
        }

        // Filter by search term (q) - matches task ID, prompt, status
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
                 OR LOWER(prompt) LIKE ${idx_prompt} \
                 OR LOWER(status) LIKE ${idx_status})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone()); // id
            bindings.push(pattern.clone()); // prompt
            bindings.push(pattern); // status
        }

        // Filter deleted tasks by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = false".to_string());
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
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for task '{}'",
                    row.id
                ))
            })?;
            let task = self.row_to_task(&row)?;
            let task_id = row.id.parse::<TaskId>().map_err(|err| {
                StoreError::Internal(format!("invalid task id stored in database: {err}"))
            })?;
            tasks.push((task_id, Versioned::new(task, version, row.created_at)));
        }

        Ok(tasks)
    }

    async fn delete_task(&self, id: &TaskId) -> Result<(), StoreError> {
        let current = self.get_task(id).await?;
        let mut task = current.item;
        task.deleted = true;
        self.update_task(id, task).await?;
        Ok(())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        let status_str = match status {
            Status::Created => "created",
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Complete => "complete",
            Status::Failed => "failed",
        };

        let query = format!(
            "SELECT DISTINCT ON (id) id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, created_at, updated_at
             FROM {TABLE_TASKS_V2}
             ORDER BY id, version_number DESC"
        );
        let rows = sqlx::query_as::<_, TaskRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut matches = Vec::new();
        for row in rows {
            if row.status == status_str {
                matches.push(row.id.parse::<TaskId>().map_err(|err| {
                    StoreError::Internal(format!("invalid task id stored in database: {err}"))
                })?);
            }
        }

        Ok(matches)
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        let versions = self.get_task_versions(id).await?;
        super::task_status_log_from_versions(&versions)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    // -------------------------------------------------------------------------
    // Actor methods
    // -------------------------------------------------------------------------

    async fn add_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS_V2} WHERE id = $1"
        ))
        .bind(&name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists > 0 {
            return Err(StoreError::ActorAlreadyExists(name));
        }

        self.insert_actor(&name, 1, &actor).await
    }

    async fn update_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_ACTORS_V2} WHERE id = $1"
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

        self.insert_actor(&name, next_version, &actor).await
    }

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        super::validate_actor_name(name)?;
        let query = format!(
            "SELECT id, version_number, auth_token_hash, auth_token_salt, user_or_worker, created_at, updated_at
             FROM {TABLE_ACTORS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, ActorRow>(&query)
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
        let actor = self.row_to_actor(&row)?;
        Ok(Versioned::new(actor, version, row.created_at))
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        let query = format!(
            "SELECT DISTINCT ON (id) id, version_number, auth_token_hash, auth_token_salt, user_or_worker, created_at, updated_at
             FROM {TABLE_ACTORS_V2}
             ORDER BY id, version_number DESC"
        );
        let rows = sqlx::query_as::<_, ActorRow>(&query)
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
            let actor = self.row_to_actor(&row)?;
            actors.push((row.id, Versioned::new(actor, version, row.created_at)));
        }

        actors.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(actors)
    }

    // -------------------------------------------------------------------------
    // User methods
    // -------------------------------------------------------------------------

    async fn add_user(&self, user: User) -> Result<(), StoreError> {
        // Check if user already exists by fetching the latest version
        let query = format!(
            "SELECT id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, created_at, updated_at
             FROM {TABLE_USERS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let existing = sqlx::query_as::<_, UserRow>(&query)
            .bind(user.username.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        match existing {
            Some(row) => {
                // If user exists but is deleted, allow re-creation with the provided user
                if row.deleted {
                    self.update_user(user).await?;
                    Ok(())
                } else {
                    Err(StoreError::UserAlreadyExists(user.username.clone()))
                }
            }
            None => {
                // User doesn't exist, insert new
                self.insert_user(user.username.as_str(), 1, &user).await
            }
        }
    }

    async fn update_user(&self, user: User) -> Result<Versioned<User>, StoreError> {
        let username = user.username.clone();
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_USERS_V2} WHERE id = $1"
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

        self.insert_user(user.username.as_str(), next_version, &user)
            .await?;

        // Fetch and return the updated user
        let query = format!(
            "SELECT id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, created_at, updated_at
             FROM {TABLE_USERS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, UserRow>(&query)
            .bind(username.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| {
            StoreError::Internal(format!("user '{}' missing after update", username.as_str()))
        })?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for user '{}'",
                row.id
            ))
        })?;
        let user = self.row_to_user(&row);
        Ok(Versioned::new(user, version, row.created_at))
    }

    async fn get_user(&self, username: &Username) -> Result<Versioned<User>, StoreError> {
        let query = format!(
            "SELECT id, version_number, username, github_user_id, github_token, github_refresh_token, deleted, created_at, updated_at
             FROM {TABLE_USERS_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, UserRow>(&query)
            .bind(username.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::UserNotFound(username.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for user '{}'",
                row.id
            ))
        })?;
        let user = self.row_to_user(&row);
        Ok(Versioned::new(user, version, row.created_at))
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        self.fetch_latest_users(query).await
    }

    async fn delete_user(&self, username: &Username) -> Result<(), StoreError> {
        let current = self.get_user(username).await?;
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
    async fn repository_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone())
            .await
            .unwrap();

        let fetched = store.get_repository(&name).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.version, 1);

        let mut updated = config.clone();
        updated.default_image = Some("other:latest".to_string());
        store
            .update_repository(name.clone(), updated.clone())
            .await
            .unwrap();

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, name);
        assert_versioned(&list[0].1, &updated, 2);

        let fetched_again = store.get_repository(&name).await.unwrap();
        assert_eq!(fetched_again.item, updated);
        assert_eq!(fetched_again.version, 2);
        assert!(fetched_again.timestamp >= fetched.timestamp);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

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
    async fn patch_associations_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
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
    async fn task_lifecycle_updates_status_v2(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
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

        let complete = handles
            .store
            .list_tasks_with_status(Status::Complete)
            .await
            .unwrap();
        assert_eq!(complete, vec![task_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn documents_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let doc_id = store
            .add_document(sample_document("docs/guide.md", None))
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id).await.unwrap();
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
    async fn user_management_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let user = User {
            username: Username::from("alice"),
            github_user_id: 101,
            github_token: "token".to_string(),
            github_refresh_token: "refresh-token".to_string(),
            deleted: false,
        };
        store.add_user(user.clone()).await.unwrap();

        let fetched = store.get_user(&Username::from("alice")).await.unwrap();
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
    async fn document_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

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
    async fn issue_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

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
    async fn patch_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

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
