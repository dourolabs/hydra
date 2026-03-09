//! PostgresStoreV2 implementation using column-based v2 tables.
//!
//! This store implementation uses the v2 tables with proper column definitions
//! instead of JSONB payloads, providing better query performance and type safety.

use crate::{
    domain::{
        actors::{Actor, ActorId, ActorRef, UNKNOWN_CREATOR},
        agents::Agent,
        documents::Document,
        issues::{
            Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
            JobSettings, TodoItem,
        },
        jobs::{BundleSpec, Task},
        labels::Label,
        messages::Message,
        notifications::Notification,
        patches::{CommitRange, GithubPr, Patch, PatchStatus, Review},
        task_status::{Status, TaskError},
        users::{User, Username},
    },
    store::{ReadOnlyStore, Store, StoreError, TaskStatusLog},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, Rgb,
    TaskId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    repositories::{Repository, SearchRepositoriesQuery},
};
use serde_json::Value;
use sqlx::{
    Pool, Postgres,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{collections::HashMap, collections::HashSet, str::FromStr, time::Duration};

use super::issue_graph::IssueGraphContext;

use crate::config::DatabaseSection;

pub type PgStorePool = Pool<Postgres>;

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

const TABLE_ISSUES_V2: &str = "metis.issues_v2";
const TABLE_PATCHES_V2: &str = "metis.patches_v2";
const TABLE_TASKS_V2: &str = "metis.tasks_v2";
const TABLE_USERS_V2: &str = "metis.users_v2";
const TABLE_REPOSITORIES_V2: &str = "metis.repositories_v2";
const TABLE_ACTORS_V2: &str = "metis.actors_v2";
const TABLE_DOCUMENTS_V2: &str = "metis.documents_v2";
const TABLE_MESSAGES_V2: &str = "metis.messages_v2";
const TABLE_NOTIFICATIONS: &str = "metis.notifications";
const TABLE_AGENTS: &str = "metis.agents";
const TABLE_LABELS: &str = "metis.labels";
const TABLE_LABEL_ASSOCIATIONS: &str = "metis.label_associations";
const TABLE_USER_SECRETS: &str = "metis.user_secrets";
const TABLE_OBJECT_RELATIONSHIPS: &str = "metis.object_relationships";

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

    async fn insert_issue_in_tx<'e, E>(
        executor: E,
        id: &IssueId,
        version_number: VersionNumber,
        issue: &Issue,
        actor: Option<&Value>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
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
            "INSERT INTO {TABLE_ISSUES_V2} (id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"
        );
        sqlx::query(&query)
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
            .execute(executor)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    /// Syncs the object_relationships table for the given issue.
    /// Deletes all existing relationships where this issue is the source,
    /// then inserts the current set of dependencies and patch links.
    async fn sync_issue_relationships_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> Result<(), StoreError> {
        // Delete all existing relationships for this issue
        let delete_sql = format!("DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} WHERE source_id = $1");
        sqlx::query(&delete_sql)
            .bind(issue_id.as_ref())
            .execute(&mut **tx)
            .await
            .map_err(map_sqlx_error)?;

        // Insert dependency relationships
        let insert_sql = format!(
            "INSERT INTO {TABLE_OBJECT_RELATIONSHIPS} \
             (source_id, source_kind, target_id, target_kind, rel_type) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (source_id, rel_type, target_id) DO NOTHING"
        );
        for dep in &issue.dependencies {
            sqlx::query(&insert_sql)
                .bind(issue_id.as_ref())
                .bind("issue")
                .bind(dep.issue_id.as_ref())
                .bind("issue")
                .bind(dep.dependency_type.as_str())
                .execute(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        }

        // Insert patch relationships
        for patch_id in &issue.patches {
            sqlx::query(&insert_sql)
                .bind(issue_id.as_ref())
                .bind("issue")
                .bind(patch_id.as_ref())
                .bind("patch")
                .bind("has-patch")
                .execute(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        }

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

    // -------------------------------------------------------------------------
    // Patch helpers
    // -------------------------------------------------------------------------

    async fn insert_patch(
        &self,
        id: &PatchId,
        version_number: VersionNumber,
        patch: &Patch,
        actor: Option<&Value>,
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

        let commit_range_json = patch
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::to_value(cr).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize commit_range: {e}"))
                })
            })
            .transpose()?;

        let query = format!(
            "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)"
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
            .bind(&patch.branch_name)
            .bind(&commit_range_json)
            .bind(patch.creator.as_str())
            .bind(patch.base_branch.as_deref())
            .bind(actor)
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

        let commit_range: Option<CommitRange> = row
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::from_value(cr.clone()).map_err(|e| {
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

    // -------------------------------------------------------------------------
    // Task helpers
    // -------------------------------------------------------------------------

    async fn insert_task(
        &self,
        id: &TaskId,
        version_number: VersionNumber,
        task: &Task,
        actor: Option<&Value>,
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

        let secrets_json = task
            .secrets
            .as_ref()
            .map(|s| {
                serde_json::to_value(s).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize secrets: {err}"))
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
            "INSERT INTO {TABLE_TASKS_V2} (id, version_number, prompt, context, spawned_from, creator, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&task.prompt)
            .bind(&context_json)
            .bind(task.spawned_from.as_ref().map(|i| i.as_ref()))
            .bind(task.creator.as_str())
            .bind(task.image.as_deref())
            .bind(task.model.as_deref())
            .bind(&env_vars_json)
            .bind(task.cpu_limit.as_deref())
            .bind(task.memory_limit.as_deref())
            .bind(status_str)
            .bind(task.last_message.as_deref())
            .bind(&error_json)
            .bind(task.deleted)
            .bind(actor)
            .bind(&secrets_json)
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
        let secrets: Option<Vec<String>> = row
            .secrets
            .as_ref()
            .map(|s| {
                serde_json::from_value(s.clone()).map_err(|err| {
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

        Ok(Task {
            prompt: row.prompt.clone(),
            context,
            spawned_from,
            creator: Username::from(row.creator.as_deref().unwrap_or(UNKNOWN_CREATOR)),
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
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_DOCUMENTS_V2} (id, version_number, title, body_markdown, path, created_by, deleted, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        );
        sqlx::query(&query)
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
                TaskId::from_str(s)
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

    // -------------------------------------------------------------------------
    // Repository helpers
    // -------------------------------------------------------------------------

    async fn insert_repository(
        &self,
        id: &str,
        version_number: VersionNumber,
        repo: &Repository,
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for repository '{id}'"))
        })?;

        let patch_workflow_json = repo
            .patch_workflow
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize patch_workflow: {e}"))
            })?;

        let query = format!(
            "INSERT INTO {TABLE_REPOSITORIES_V2} (id, version_number, remote_url, default_branch, default_image, deleted, patch_workflow, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        );
        sqlx::query(&query)
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
                serde_json::from_value(v.clone()).map_err(|e| {
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

    // -------------------------------------------------------------------------
    // Message helpers
    // -------------------------------------------------------------------------

    async fn insert_message(
        &self,
        id: &MessageId,
        version_number: VersionNumber,
        message: &Message,
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for message '{id}'"))
        })?;

        let sender_name = message.sender.as_ref().map(|s| s.to_string());
        let recipient_name = message.recipient.to_string();

        let query = format!(
            "INSERT INTO {TABLE_MESSAGES_V2} (id, version_number, sender, recipient, body, deleted, is_read, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&sender_name)
            .bind(&recipient_name)
            .bind(&message.body)
            .bind(message.deleted)
            .bind(message.is_read)
            .bind(actor)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_message(&self, row: &MessageRow) -> Result<Message, StoreError> {
        let sender = row
            .sender
            .as_deref()
            .map(|s| {
                crate::domain::actors::Actor::parse_name(s).map_err(|_| {
                    StoreError::Internal(format!(
                        "invalid sender '{}' stored for message '{}'",
                        s, row.id
                    ))
                })
            })
            .transpose()?;

        let recipient = crate::domain::actors::Actor::parse_name(&row.recipient).map_err(|_| {
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

    // -------------------------------------------------------------------------
    // Notification helpers
    // -------------------------------------------------------------------------

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

        let query = format!(
            "INSERT INTO {TABLE_NOTIFICATIONS} \
             (id, recipient, source_actor, object_kind, object_id, object_version, \
              event_type, summary, source_issue_id, policy, is_read, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
        );
        sqlx::query(&query)
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
            .bind(notification.created_at)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_notification(&self, row: &NotificationRow) -> Result<Notification, StoreError> {
        let recipient = crate::domain::actors::Actor::parse_name(&row.recipient).map_err(|_| {
            StoreError::Internal(format!(
                "invalid recipient '{}' stored for notification '{}'",
                row.recipient, row.id
            ))
        })?;

        let source_actor = row
            .source_actor
            .as_deref()
            .map(|s| {
                crate::domain::actors::Actor::parse_name(s).map_err(|_| {
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
            created_at: row.created_at,
        })
    }

    // -------------------------------------------------------------------------
    // User helpers
    // -------------------------------------------------------------------------

    async fn insert_user(
        &self,
        id: &str,
        version_number: VersionNumber,
        user: &User,
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for user '{id}'"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_USERS_V2} (id, version_number, username, github_user_id, deleted, actor)
             VALUES ($1, $2, $3, $4, $5, $6)"
        );
        sqlx::query(&query)
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

    async fn fetch_latest_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        // Build query with filtering on latest version of each user
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
            users.push((
                username,
                Versioned::with_optional_actor(
                    user,
                    version,
                    row.created_at,
                    parse_actor_json(row.actor)?,
                    row.created_at,
                ),
            ));
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
        acting_as: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for actor '{id}'"))
        })?;

        let actor_id_json = serde_json::to_value(&actor.actor_id)
            .map_err(|e| StoreError::Internal(format!("failed to serialize actor_id: {e}")))?;

        let creator_str = actor.creator.to_string();

        let query = format!(
            "INSERT INTO {TABLE_ACTORS_V2} (id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        );
        sqlx::query(&query)
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
        let actor_id: ActorId = serde_json::from_value(row.actor_id.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize actor_id: {e}")))?;

        Ok(Actor {
            auth_token_hash: row.auth_token_hash.clone(),
            auth_token_salt: row.auth_token_salt.clone(),
            actor_id,
            creator: Username::from(row.creator.as_deref().unwrap_or(UNKNOWN_CREATOR)),
        })
    }
}

// -----------------------------------------------------------------------------
// Row structs for sqlx queries
// -----------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct ObjectRelationshipRow {
    source_id: String,
    source_kind: String,
    target_id: String,
    target_kind: String,
    rel_type: String,
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
    job_settings: Value,
    todo_list: Value,
    dependencies: Value,
    patches: Value,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
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
    branch_name: Option<String>,
    commit_range: Option<Value>,
    creator: Option<String>,
    base_branch: Option<String>,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
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
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    creator: Option<String>,
    secrets: Option<Value>,
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
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: String,
    version_number: i64,
    remote_url: String,
    default_branch: Option<String>,
    default_image: Option<String>,
    deleted: bool,
    patch_workflow: Option<Value>,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    version_number: i64,
    username: String,
    github_user_id: Option<i64>,
    deleted: bool,
    actor: Option<Value>,
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
    actor_id: Value,
    creator: Option<String>,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: String,
    version_number: i64,
    sender: Option<String>,
    recipient: String,
    body: String,
    deleted: bool,
    is_read: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
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
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct LabelRow {
    id: String,
    name: String,
    color: String,
    deleted: bool,
    recurse: bool,
    hidden: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AgentRow {
    name: String,
    prompt_path: String,
    max_tries: i32,
    max_simultaneous: i32,
    is_assignment_agent: bool,
    deleted: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db_err) = err {
        if db_err.code().as_deref() == Some("23505")
            && db_err.constraint() == Some("labels_name_idx")
        {
            let name = db_err
                .message()
                .split("=(")
                .nth(1)
                .and_then(|s| s.split(')').next())
                .unwrap_or("unknown")
                .to_string();
            return StoreError::LabelAlreadyExists(name);
        }
    }
    StoreError::Internal(err.to_string())
}

fn actor_to_json(actor: &ActorRef) -> Value {
    serde_json::to_value(actor).expect("ActorRef serialization should not fail")
}

fn parse_actor_json(value: Option<Value>) -> Result<Option<ActorRef>, StoreError> {
    match value {
        None => Ok(None),
        Some(v) => serde_json::from_value(v).map(Some).map_err(|e| {
            StoreError::Internal(format!("failed to parse actor JSON into ActorRef: {e}"))
        }),
    }
}

#[async_trait]
impl ReadOnlyStore for PostgresStoreV2 {
    // -------------------------------------------------------------------------
    // Repository methods
    // -------------------------------------------------------------------------

    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        let name_str = name.as_str();
        let query = format!(
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, patch_workflow, actor, created_at, updated_at
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
        if !include_deleted && row.deleted {
            return Err(StoreError::RepositoryNotFound(name.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for repository '{}'",
                row.id
            ))
        })?;
        let repo = self.row_to_repository(&row)?;
        Ok(Versioned::with_optional_actor(
            repo,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let sql = format!(
            "SELECT DISTINCT ON (id) id, version_number, remote_url, default_branch, default_image, deleted, patch_workflow, actor, created_at, updated_at
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
            let repo = self.row_to_repository(&row)?;
            results.push((
                name,
                Versioned::with_optional_actor(
                    repo,
                    version,
                    row.created_at,
                    parse_actor_json(row.actor)?,
                    row.created_at,
                ),
            ));
        }

        results.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(results)
    }

    // -------------------------------------------------------------------------
    // Issue methods
    // -------------------------------------------------------------------------

    async fn get_issue(
        &self,
        id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        let query = format!(
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = $1) AS creation_time
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

        if !include_deleted && issue.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let versioned = Versioned::with_optional_actor(
            issue,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.creation_time.unwrap_or(row.created_at),
        );
        Ok(versioned)
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor, created_at, updated_at
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
            results.push(Versioned::with_optional_actor(
                issue,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.created_at,
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
        // Use a subquery to get the latest version of each issue first,
        // then apply filters. This ensures we filter on the current state
        // of each issue, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, issue_type, title, description, creator, progress, status, assignee, job_settings, todo_list, dependencies, patches, deleted, actor, created_at, updated_at, \
             MIN(created_at) OVER (PARTITION BY id) AS creation_time \
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
            let idx_title = bindings.len() + 2;
            let idx_desc = bindings.len() + 3;
            let idx_progress = bindings.len() + 4;
            let idx_type = bindings.len() + 5;
            let idx_status = bindings.len() + 6;
            let idx_creator = bindings.len() + 7;
            let idx_assignee = bindings.len() + 8;
            predicates.push(format!(
                "(LOWER(id) LIKE ${idx_id} \
                 OR LOWER(title) LIKE ${idx_title} \
                 OR LOWER(description) LIKE ${idx_desc} \
                 OR LOWER(progress) LIKE ${idx_progress} \
                 OR issue_type = ${idx_type} \
                 OR status = ${idx_status} \
                 OR LOWER(creator) LIKE ${idx_creator} \
                 OR LOWER(COALESCE(assignee,'')) LIKE ${idx_assignee})"
            ));
            let pattern = format!("%{term}%");
            bindings.push(pattern.clone()); // id
            bindings.push(pattern.clone()); // title
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

        // Filter by label IDs (AND semantics: issue must have ALL specified labels)
        if !query.label_ids.is_empty() {
            let label_count = query.label_ids.len();
            let placeholders: Vec<String> = query
                .label_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("${}", bindings.len() + i + 1))
                .collect();
            predicates.push(format!(
                "id IN (SELECT la.object_id FROM {TABLE_LABEL_ASSOCIATIONS} la \
                 WHERE la.label_id IN ({}) \
                 GROUP BY la.object_id \
                 HAVING COUNT(DISTINCT la.label_id) = {label_count})",
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
            let versioned = Versioned::with_optional_actor(
                issue,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
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
        // Use spawned_from filter at the database level for efficiency
        let query = SearchJobsQuery::new(None, Some(issue_id.clone()), None, None);
        let tasks = self.list_tasks(&query).await?;
        Ok(tasks.into_iter().map(|(id, _)| id).collect())
    }

    // -------------------------------------------------------------------------
    // Patch methods
    // -------------------------------------------------------------------------

    async fn get_patch(
        &self,
        id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PATCHES_V2} WHERE id = $1) AS creation_time
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
        if !include_deleted && patch.deleted {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        let versioned = Versioned::with_optional_actor(
            patch,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.creation_time.unwrap_or(row.created_at),
        );
        Ok(versioned)
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor, created_at, updated_at
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
            results.push(Versioned::with_optional_actor(
                patch,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.created_at,
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
        // Use a subquery to get the latest version of each patch first,
        // then apply filters. This ensures we filter on the current state
        // of each patch, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, title, description, diff, status, is_automatic_backup, created_by, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor, created_at, updated_at, \
             MIN(created_at) OVER (PARTITION BY id) AS creation_time \
             FROM {TABLE_PATCHES_V2} ORDER BY id, version_number DESC"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        // Filter deleted patches by default
        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = false".to_string());
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
                .map(|(i, _)| format!("${}", bindings.len() + i + 1))
                .collect();
            predicates.push(format!("status IN ({})", placeholders.join(", ")));
            for s in status_strings {
                bindings.push(s);
            }
        }

        if let Some(ref branch) = query.branch_name {
            let idx = bindings.len() + 1;
            predicates.push(format!("branch_name = ${idx}"));
            bindings.push(branch.clone());
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
                 OR LOWER(COALESCE(branch_name,'')) LIKE ${idx_branch} \
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
                idx_branch = idx_start + 6,
                idx_gh_owner = idx_start + 7,
                idx_gh_repo = idx_start + 8,
                idx_gh_number = idx_start + 9,
                idx_gh_head = idx_start + 10,
                idx_gh_base = idx_start + 11,
            ));
            let pattern = format!("%{term}%");
            for _ in 0..12 {
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
            let versioned = Versioned::with_optional_actor(
                patch,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
            );
            patches.push((patch_id, versioned));
        }

        Ok(patches)
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

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = $1) AS creation_time
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
        let versioned = Versioned::with_optional_actor(
            document,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.creation_time.unwrap_or(row.created_at),
        );
        Ok(versioned)
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, body_markdown, path, created_by, deleted, actor, created_at, updated_at
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
            results.push(Versioned::with_optional_actor(
                document,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.created_at,
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
        // Use a subquery to get the latest version of each document first,
        // then apply filters. This ensures we filter on the current state
        // of each document, not historical versions.
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, title, body_markdown, path, created_by, deleted, actor, created_at, updated_at, \
             MIN(created_at) OVER (PARTITION BY id) AS creation_time \
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
            let versioned = Versioned::with_optional_actor(
                document,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
            );
            documents.push((document_id, versioned));
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

    // -------------------------------------------------------------------------
    // Task methods
    // -------------------------------------------------------------------------

    async fn get_task(
        &self,
        id: &TaskId,
        include_deleted: bool,
    ) -> Result<Versioned<Task>, StoreError> {
        let query = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets
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
        if !include_deleted && row.deleted {
            return Err(StoreError::TaskNotFound(id.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for task '{}'",
                row.id
            ))
        })?;
        let task = self.row_to_task(&row)?;
        Ok(Versioned::with_optional_actor(
            task,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets
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
            results.push(Versioned::with_optional_actor(
                task,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.created_at,
            ));
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
            "SELECT DISTINCT ON (id) id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets \
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

        // Filter by status
        if let Some(status) = query.status {
            let server_status: Status = status.into();
            predicates.push(format!("status = ${}", bindings.len() + 1));
            bindings.push(super::status_to_db_str(server_status).to_string());
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
            tasks.push((
                task_id,
                Versioned::with_optional_actor(
                    task,
                    version,
                    row.created_at,
                    parse_actor_json(row.actor)?,
                    row.created_at,
                ),
            ));
        }

        Ok(tasks)
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        let versions = self.get_task_versions(id).await?;
        super::task_status_log_from_versions(&versions)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        ids: &[TaskId],
    ) -> Result<HashMap<TaskId, TaskStatusLog>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let id_strings: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();
        let query = format!(
            "SELECT id, version_number, prompt, context, spawned_from, image, model, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets
             FROM {TABLE_TASKS_V2}
             WHERE id = ANY($1)
             ORDER BY id, version_number"
        );
        let rows = sqlx::query_as::<_, TaskRow>(&query)
            .bind(&id_strings)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut grouped: HashMap<TaskId, Vec<Versioned<Task>>> = HashMap::new();
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
            grouped
                .entry(task_id)
                .or_default()
                .push(Versioned::with_optional_actor(
                    task,
                    version,
                    row.created_at,
                    parse_actor_json(row.actor)?,
                    row.created_at,
                ));
        }

        let mut result = HashMap::new();
        for (task_id, versions) in grouped {
            if let Some(log) = super::task_status_log_from_versions(&versions) {
                result.insert(task_id, log);
            }
        }

        Ok(result)
    }

    // -------------------------------------------------------------------------
    // Actor methods
    // -------------------------------------------------------------------------

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        super::validate_actor_name(name)?;
        let query = format!(
            "SELECT id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor, created_at, updated_at
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
        Ok(Versioned::with_optional_actor(
            actor,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        let query = format!(
            "SELECT DISTINCT ON (id) id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor, created_at, updated_at
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
            actors.push((
                row.id,
                Versioned::with_optional_actor(
                    actor,
                    version,
                    row.created_at,
                    parse_actor_json(row.actor)?,
                    row.created_at,
                ),
            ));
        }

        actors.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(actors)
    }

    // -------------------------------------------------------------------------
    // User methods
    // -------------------------------------------------------------------------

    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        let query = format!(
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
        if !include_deleted && row.deleted {
            return Err(StoreError::UserNotFound(username.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for user '{}'",
                row.id
            ))
        })?;
        let user = self.row_to_user(&row);
        Ok(Versioned::with_optional_actor(
            user,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        self.fetch_latest_users(query).await
    }

    // -------------------------------------------------------------------------
    // Message methods (read-only)
    // -------------------------------------------------------------------------

    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        let query = format!(
            "SELECT id, version_number, sender, recipient, body, deleted, is_read, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_MESSAGES_V2} WHERE id = $1) AS creation_time
             FROM {TABLE_MESSAGES_V2}
             WHERE id = $1
             ORDER BY version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, MessageRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::MessageNotFound(id.clone()))?;
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for message '{}'",
                row.id
            ))
        })?;
        let message = self.row_to_message(&row)?;
        Ok(Versioned::with_optional_actor(
            message,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.creation_time.unwrap_or(row.created_at),
        ))
    }

    async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        let limit = i64::from(query.limit.unwrap_or(50));
        let include_deleted = query.include_deleted.unwrap_or(false);

        // Build the base subquery that gets the latest version of each message
        let subquery = format!(
            "SELECT DISTINCT ON (id) id, version_number, sender, recipient, body, deleted, is_read, actor, created_at, updated_at, \
             MIN(created_at) OVER (PARTITION BY id) AS creation_time \
             FROM {TABLE_MESSAGES_V2} ORDER BY id, version_number DESC"
        );

        let mut conditions = Vec::new();
        let mut param_idx = 1u32;

        if !include_deleted {
            conditions.push("deleted = false".to_string());
        }

        let sender_val = query.sender.clone();
        if sender_val.is_some() {
            conditions.push(format!("sender = ${param_idx}"));
            param_idx += 1;
        }

        let recipient_val = query.recipient.clone();
        if recipient_val.is_some() {
            conditions.push(format!("recipient = ${param_idx}"));
            param_idx += 1;
        }

        let after_val = query.after;
        if after_val.is_some() {
            conditions.push(format!("created_at > ${param_idx}"));
            param_idx += 1;
        }

        let before_val = query.before;
        if before_val.is_some() {
            conditions.push(format!("created_at < ${param_idx}"));
            param_idx += 1;
        }

        let is_read_val = query.is_read;
        if is_read_val.is_some() {
            conditions.push(format!("is_read = ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT * FROM ({subquery}) AS latest{where_clause} ORDER BY created_at DESC LIMIT ${param_idx}"
        );

        let mut qb = sqlx::query_as::<_, MessageRow>(&sql);
        if let Some(ref s) = sender_val {
            qb = qb.bind(s);
        }
        if let Some(ref r) = recipient_val {
            qb = qb.bind(r);
        }
        if let Some(ref a) = after_val {
            qb = qb.bind(a);
        }
        if let Some(ref b) = before_val {
            qb = qb.bind(b);
        }
        if let Some(ref ir) = is_read_val {
            qb = qb.bind(ir);
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
            let message = self.row_to_message(&row)?;
            let versioned = Versioned::with_optional_actor(
                message,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
            );
            messages.push((message_id, versioned));
        }

        Ok(messages)
    }

    // -------------------------------------------------------------------------
    // Notification methods (read-only)
    // -------------------------------------------------------------------------

    async fn get_notification(&self, id: &NotificationId) -> Result<Notification, StoreError> {
        let sql = format!(
            "SELECT id, recipient, source_actor, object_kind, object_id, object_version, \
             event_type, summary, source_issue_id, policy, is_read, created_at \
             FROM {TABLE_NOTIFICATIONS} WHERE id = $1"
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
        let mut param_idx = 1u32;

        let recipient_val = query.recipient.clone();
        if recipient_val.is_some() {
            conditions.push(format!("recipient = ${param_idx}"));
            param_idx += 1;
        }

        let is_read_val = query.is_read;
        if is_read_val.is_some() {
            conditions.push(format!("is_read = ${param_idx}"));
            param_idx += 1;
        }

        let before_val = query.before;
        if before_val.is_some() {
            conditions.push(format!("created_at < ${param_idx}"));
            param_idx += 1;
        }

        let after_val = query.after;
        if after_val.is_some() {
            conditions.push(format!("created_at > ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, recipient, source_actor, object_kind, object_id, object_version, \
             event_type, summary, source_issue_id, policy, is_read, created_at \
             FROM {TABLE_NOTIFICATIONS}{where_clause} \
             ORDER BY created_at DESC LIMIT ${param_idx}"
        );

        let mut qb = sqlx::query_as::<_, NotificationRow>(&sql);
        if let Some(ref r) = recipient_val {
            qb = qb.bind(r);
        }
        if let Some(ref ir) = is_read_val {
            qb = qb.bind(ir);
        }
        if let Some(ref b) = before_val {
            qb = qb.bind(b);
        }
        if let Some(ref a) = after_val {
            qb = qb.bind(a);
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
            "SELECT COUNT(*) FROM {TABLE_NOTIFICATIONS} WHERE recipient = $1 AND is_read = false"
        ))
        .bind(&recipient_name)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(u64::try_from(count).unwrap_or(0))
    }

    // ---- Agent (read-only) ----

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, max_tries, max_simultaneous, \
                    is_assignment_agent, deleted, created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE name = $1"
        );
        let row = sqlx::query_as::<_, AgentRow>(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?
            .ok_or_else(|| StoreError::AgentNotFound(name.to_string()))?;
        let agent = row_to_agent(row);
        if agent.deleted {
            return Err(StoreError::AgentNotFound(name.to_string()));
        }
        Ok(agent)
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, max_tries, max_simultaneous, \
                    is_assignment_agent, deleted, created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE deleted = false ORDER BY name"
        );
        let rows = sqlx::query_as::<_, AgentRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(row_to_agent).collect())
    }

    // ---- Label (read-only) ----

    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError> {
        let sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS} WHERE id = $1"
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
        let include_deleted = query.include_deleted.unwrap_or(false);
        let mut conditions = Vec::new();

        if !include_deleted {
            conditions.push("deleted = false".to_string());
        }

        let q_val = query.q.clone();
        if q_val.is_some() {
            conditions.push("LOWER(name) LIKE $1".to_string());
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS}{where_clause} ORDER BY name"
        );

        let mut qb = sqlx::query_as::<_, LabelRow>(&sql);
        if let Some(ref q) = q_val {
            qb = qb.bind(format!("%{}%", q.to_lowercase()));
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

    async fn get_label_by_name(&self, name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        let sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS} WHERE name = $1 AND deleted = false"
        );
        let row = sqlx::query_as::<_, LabelRow>(&sql)
            .bind(name.to_lowercase())
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
             WHERE la.object_id = $1 AND l.deleted = false \
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
                let color: Rgb = color.parse().map_err(|err| {
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

        let ids: Vec<&str> = object_ids.iter().map(|id| id.as_ref()).collect();
        let sql = format!(
            "SELECT la.object_id, l.id, l.name, l.color, l.recurse, l.hidden \
             FROM {TABLE_LABELS} l \
             INNER JOIN {TABLE_LABEL_ASSOCIATIONS} la ON l.id = la.label_id \
             WHERE la.object_id = ANY($1) AND l.deleted = false \
             ORDER BY l.name"
        );
        let rows = sqlx::query_as::<_, (String, String, String, String, bool, bool)>(&sql)
            .bind(&ids)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut result: HashMap<MetisId, Vec<LabelSummary>> = HashMap::new();
        for (obj_id_str, label_id_str, name, color, recurse, hidden) in rows {
            let obj_id = obj_id_str.parse::<MetisId>().map_err(|err| {
                StoreError::Internal(format!("invalid object id stored in database: {err}"))
            })?;
            let label_id = label_id_str.parse::<LabelId>().map_err(|err| {
                StoreError::Internal(format!("invalid label id stored in database: {err}"))
            })?;
            let color: Rgb = color.parse().map_err(|err| {
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
        let sql = format!("SELECT object_id FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = $1");
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
        rel_type: Option<&str>,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let mut conditions = Vec::new();
        let mut bind_index = 1u32;

        if source_id.is_some() {
            conditions.push(format!("source_id = ${bind_index}"));
            bind_index += 1;
        }
        if target_id.is_some() {
            conditions.push(format!("target_id = ${bind_index}"));
            bind_index += 1;
        }
        if rel_type.is_some() {
            conditions.push(format!("rel_type = ${bind_index}"));
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
            query = query.bind(rt);
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        let mut result = Vec::with_capacity(rows.len());
        for r in rows {
            let source_id: MetisId = r.source_id.parse().map_err(|_| {
                StoreError::Internal("invalid source_id in object_relationships".to_string())
            })?;
            let target_id: MetisId = r.target_id.parse().map_err(|_| {
                StoreError::Internal("invalid target_id in object_relationships".to_string())
            })?;
            result.push(super::ObjectRelationship {
                source_id,
                source_kind: r.source_kind,
                target_id,
                target_kind: r.target_kind,
                rel_type: r.rel_type,
            });
        }
        Ok(result)
    }

    // ---- User secrets (read-only) ----

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let sql = format!(
            "SELECT encrypted_value FROM {TABLE_USER_SECRETS} WHERE username = $1 AND secret_name = $2"
        );
        let row = sqlx::query_scalar::<_, Vec<u8>>(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(row)
    }

    async fn list_user_secret_names(&self, username: &Username) -> Result<Vec<String>, StoreError> {
        let sql = format!(
            "SELECT secret_name FROM {TABLE_USER_SECRETS} WHERE username = $1 ORDER BY secret_name"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(username.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows)
    }
}

#[async_trait]
impl Store for PostgresStoreV2 {
    // -------------------------------------------------------------------------
    // Repository methods
    // -------------------------------------------------------------------------

    async fn add_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError> {
        let name_str = name.as_str();

        // Check if repository exists (including deleted)
        let existing = self.get_repository(&name, true).await;

        match existing {
            Ok(repo) if repo.item.deleted => {
                // Re-create over deleted: use caller's config as-is
                self.update_repository(name, config, actor).await
            }
            Ok(_) => Err(StoreError::RepositoryAlreadyExists(name)),
            Err(StoreError::RepositoryNotFound(_)) => {
                let actor_json = actor_to_json(actor);
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

        let actor_json = actor_to_json(actor);
        self.insert_repository(name_str.as_str(), next_version, &config, Some(&actor_json))
            .await
    }

    async fn delete_repository(&self, name: &RepoName, actor: &ActorRef) -> Result<(), StoreError> {
        // Use include_deleted: true since we need to access the repository to mark it as deleted
        let current = self.get_repository(name, true).await?;
        let mut repo = current.item;
        repo.deleted = true;
        self.update_repository(name.clone(), repo, actor).await
    }

    // -------------------------------------------------------------------------
    // Issue methods
    // -------------------------------------------------------------------------

    async fn add_issue(
        &self,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        let id = IssueId::new();
        let actor_json = actor_to_json(actor);

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

        let actor_json = actor_to_json(actor);

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

    // -------------------------------------------------------------------------
    // Patch methods
    // -------------------------------------------------------------------------

    async fn add_patch(
        &self,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        let id = PatchId::new();
        let actor_json = actor_to_json(actor);
        self.insert_patch(&id, 1, &patch, Some(&actor_json)).await?;
        Ok((id, 1))
    }

    async fn update_patch(
        &self,
        id: &PatchId,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.get_patch(id, true).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_PATCHES_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("patch '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for patch '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
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

    // -------------------------------------------------------------------------
    // Document methods
    // -------------------------------------------------------------------------

    async fn add_document(
        &self,
        document: Document,
        actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        let id = DocumentId::new();
        let actor_json = actor_to_json(actor);
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
        self.get_document(id, true).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_DOCUMENTS_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("document '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for document '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
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

    // -------------------------------------------------------------------------
    // Task methods
    // -------------------------------------------------------------------------

    async fn add_task(
        &self,
        task: Task,
        _creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(TaskId, VersionNumber), StoreError> {
        let id = TaskId::new();

        if let Some(issue_id) = task.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        let actor_json = actor_to_json(actor);
        self.insert_task(&id, 1, &task, Some(&actor_json)).await?;
        Ok((id, 1))
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
        actor: &ActorRef,
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

        let actor_json = actor_to_json(actor);
        self.insert_task(metis_id, next_version, &task, Some(&actor_json))
            .await?;
        self.get_task(metis_id, true).await
    }

    async fn delete_task(
        &self,
        id: &TaskId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_task(id, true).await?;
        let mut task = current.item;
        task.deleted = true;
        let versioned = self.update_task(id, task, actor).await?;
        Ok(versioned.version)
    }

    // -------------------------------------------------------------------------
    // Actor methods
    // -------------------------------------------------------------------------

    async fn add_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
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

        let acting_as_json = actor_to_json(acting_as);
        self.insert_actor(&name, 1, &actor, Some(&acting_as_json))
            .await
    }

    async fn update_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
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

        let acting_as_json = actor_to_json(acting_as);
        self.insert_actor(&name, next_version, &actor, Some(&acting_as_json))
            .await
    }

    // -------------------------------------------------------------------------
    // User methods
    // -------------------------------------------------------------------------

    async fn add_user(&self, user: User, actor: &ActorRef) -> Result<(), StoreError> {
        // Check if user already exists by fetching the latest version
        let query = format!(
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
                    self.update_user(user, actor).await?;
                    Ok(())
                } else {
                    Err(StoreError::UserAlreadyExists(user.username.clone()))
                }
            }
            None => {
                // User doesn't exist, insert new
                let actor_json = actor_to_json(actor);
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

        let actor_json = actor_to_json(actor);
        self.insert_user(
            user.username.as_str(),
            next_version,
            &user,
            Some(&actor_json),
        )
        .await?;

        // Fetch and return the updated user
        let query = format!(
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
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
        Ok(Versioned::with_optional_actor(
            user,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn delete_user(&self, username: &Username, actor: &ActorRef) -> Result<(), StoreError> {
        let current = self.get_user(username, true).await?;
        let mut user = current.item;
        user.deleted = true;
        self.update_user(user, actor).await?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Notification methods
    // -------------------------------------------------------------------------

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
            "UPDATE {TABLE_NOTIFICATIONS} SET is_read = true WHERE id = $1"
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
            sqlx::query(&format!(
                "UPDATE {TABLE_NOTIFICATIONS} SET is_read = true \
                 WHERE recipient = $1 AND is_read = false AND created_at < $2"
            ))
            .bind(&recipient_name)
            .bind(before_ts)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?
        } else {
            sqlx::query(&format!(
                "UPDATE {TABLE_NOTIFICATIONS} SET is_read = true \
                 WHERE recipient = $1 AND is_read = false"
            ))
            .bind(&recipient_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?
        };

        Ok(result.rows_affected())
    }

    // -------------------------------------------------------------------------
    // Message methods
    // -------------------------------------------------------------------------

    async fn add_message(
        &self,
        message: Message,
        actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        let id = MessageId::new();
        let actor_json = actor_to_json(actor);
        self.insert_message(&id, 1, &message, Some(&actor_json))
            .await?;
        Ok((id, 1))
    }

    async fn update_message(
        &self,
        id: &MessageId,
        message: Message,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.get_message(id).await?;

        let latest_version = self
            .fetch_latest_version_number(TABLE_MESSAGES_V2, id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("message '{id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for message '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        self.insert_message(id, next_version, &message, Some(&actor_json))
            .await?;
        Ok(next_version)
    }

    // ---- Agent mutations ----

    async fn add_agent(&self, agent: Agent) -> Result<(), StoreError> {
        // Check if an agent with this name already exists (including soft-deleted).
        let existing_deleted = sqlx::query_scalar::<_, bool>(&format!(
            "SELECT deleted FROM {TABLE_AGENTS} WHERE name = $1"
        ))
        .bind(&agent.name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        match existing_deleted {
            Some(false) => {
                // Active agent exists — conflict.
                return Err(StoreError::AgentAlreadyExists(agent.name));
            }
            Some(true) => {
                // Soft-deleted agent exists — validate assignment uniqueness, then reactivate.
                if agent.is_assignment_agent {
                    let has_assignment = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_assignment_agent = true AND NOT deleted)"
                    ))
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                    if has_assignment {
                        return Err(StoreError::AssignmentAgentAlreadyExists);
                    }
                }

                let now = Utc::now();
                let sql = format!(
                    "UPDATE {TABLE_AGENTS} \
                     SET prompt_path = $1, max_tries = $2, max_simultaneous = $3, \
                         is_assignment_agent = $4, deleted = false, \
                         created_at = $5, updated_at = $6 \
                     WHERE name = $7"
                );
                sqlx::query(&sql)
                    .bind(&agent.prompt_path)
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(now)
                    .bind(now)
                    .bind(&agent.name)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;

                Ok(())
            }
            None => {
                // No existing row — validate assignment uniqueness, then insert.
                if agent.is_assignment_agent {
                    let has_assignment = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_assignment_agent = true AND NOT deleted)"
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
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
                );
                sqlx::query(&sql)
                    .bind(&agent.name)
                    .bind(&agent.prompt_path)
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(agent.deleted)
                    .bind(agent.created_at)
                    .bind(agent.updated_at)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;

                Ok(())
            }
        }
    }

    async fn update_agent(&self, agent: Agent) -> Result<(), StoreError> {
        // Check it exists (and is not deleted).
        let _ = self.get_agent(&agent.name).await?;

        // Validate assignment agent uniqueness (exclude self).
        if agent.is_assignment_agent {
            let conflict = sqlx::query_scalar::<_, bool>(&format!(
                "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                 WHERE is_assignment_agent = true AND NOT deleted AND name != $1)"
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
             SET prompt_path = $1, max_tries = $2, max_simultaneous = $3, \
                 is_assignment_agent = $4, updated_at = $5 \
             WHERE name = $6"
        );
        sqlx::query(&sql)
            .bind(&agent.prompt_path)
            .bind(agent.max_tries)
            .bind(agent.max_simultaneous)
            .bind(agent.is_assignment_agent)
            .bind(Utc::now())
            .bind(&agent.name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn delete_agent(&self, name: &str) -> Result<(), StoreError> {
        // Check it exists (and is not deleted).
        let _ = self.get_agent(name).await?;

        let sql =
            format!("UPDATE {TABLE_AGENTS} SET deleted = true, updated_at = $1 WHERE name = $2");
        sqlx::query(&sql)
            .bind(Utc::now())
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    // ---- Label mutations ----

    async fn add_label(&self, label: Label) -> Result<LabelId, StoreError> {
        // Check uniqueness by name
        if self.get_label_by_name(&label.name).await?.is_some() {
            return Err(StoreError::LabelAlreadyExists(label.name.clone()));
        }

        let id = LabelId::new();

        let sql = format!(
            "INSERT INTO {TABLE_LABELS} (id, name, color, deleted, recurse, hidden, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        );
        sqlx::query(&sql)
            .bind(id.as_ref())
            .bind(&label.name)
            .bind(label.color.as_ref())
            .bind(label.deleted)
            .bind(label.recurse)
            .bind(label.hidden)
            .bind(label.created_at)
            .bind(label.updated_at)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(id)
    }

    async fn update_label(&self, id: &LabelId, label: Label) -> Result<(), StoreError> {
        // Check it exists
        let _ = self.get_label(id).await?;

        // Check name uniqueness (exclude self)
        if let Some((existing_id, _)) = self.get_label_by_name(&label.name).await? {
            if existing_id != *id {
                return Err(StoreError::LabelAlreadyExists(label.name.clone()));
            }
        }

        let sql = format!(
            "UPDATE {TABLE_LABELS} SET name = $1, color = $2, recurse = $3, hidden = $4, updated_at = $5 WHERE id = $6"
        );
        sqlx::query(&sql)
            .bind(&label.name)
            .bind(label.color.as_ref())
            .bind(label.recurse)
            .bind(label.hidden)
            .bind(Utc::now())
            .bind(id.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    async fn delete_label(&self, id: &LabelId) -> Result<(), StoreError> {
        // Check it exists
        let _ = self.get_label(id).await?;

        let sql =
            format!("UPDATE {TABLE_LABELS} SET deleted = true, updated_at = $1 WHERE id = $2");
        sqlx::query(&sql)
            .bind(Utc::now())
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
             VALUES ($1, $2, $3) \
             ON CONFLICT (label_id, object_id) DO NOTHING"
        );
        let result = sqlx::query(&sql)
            .bind(label_id.as_ref())
            .bind(object_id.as_ref())
            .bind(object_kind)
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
            "DELETE FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = $1 AND object_id = $2"
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
        rel_type: &str,
    ) -> Result<bool, StoreError> {
        let source_kind = super::object_kind_from_id(source_id)?;
        let target_kind = super::object_kind_from_id(target_id)?;
        let sql = format!(
            "INSERT INTO {TABLE_OBJECT_RELATIONSHIPS} \
             (source_id, source_kind, target_id, target_kind, rel_type) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (source_id, rel_type, target_id) DO NOTHING"
        );
        let result = sqlx::query(&sql)
            .bind(source_id.as_ref())
            .bind(source_kind)
            .bind(target_id.as_ref())
            .bind(target_kind)
            .bind(rel_type)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(result.rows_affected() > 0)
    }

    async fn remove_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: &str,
    ) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = $1 AND target_id = $2 AND rel_type = $3"
        );
        let result = sqlx::query(&sql)
            .bind(source_id.as_ref())
            .bind(target_id.as_ref())
            .bind(rel_type)
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
    ) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO {TABLE_USER_SECRETS} (username, secret_name, encrypted_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $4) \
             ON CONFLICT (username, secret_name) \
             DO UPDATE SET encrypted_value = $3, updated_at = $4"
        );
        let now = chrono::Utc::now();
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .bind(encrypted_value)
            .bind(now)
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
            format!("DELETE FROM {TABLE_USER_SECRETS} WHERE username = $1 AND secret_name = $2");
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }
}

fn row_to_label(row: &LabelRow) -> Result<Label, StoreError> {
    let color = row
        .color
        .parse()
        .map_err(|err| StoreError::Internal(format!("invalid label color in database: {err}")))?;
    Ok(Label {
        name: row.name.clone(),
        color,
        deleted: row.deleted,
        recurse: row.recurse,
        hidden: row.hidden,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn row_to_agent(row: AgentRow) -> Agent {
    Agent {
        name: row.name,
        prompt_path: row.prompt_path,
        max_tries: row.max_tries,
        max_simultaneous: row.max_simultaneous,
        is_assignment_agent: row.is_assignment_agent,
        deleted: row.deleted,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            actors::Actor,
            documents::Document,
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, JobSettings,
                TodoItem,
            },
            jobs::BundleSpec,
            patches::{CommitRange, GitOid, GithubPr, Patch, PatchStatus, Review},
            users::{User, Username},
        },
        test_utils::test_state_with_store,
    };
    use metis_common::{
        PatchId, RepoName, TaskId, VersionNumber, Versioned,
        repositories::{
            MergeRequestConfig, RepoWorkflowConfig, Repository, ReviewRequestConfig,
            SearchRepositoriesQuery,
        },
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

    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
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

    fn sample_patch() -> Patch {
        Patch::new(
            "patch title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
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

    fn sample_document(path: &str, created_by: Option<TaskId>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: Some(path.parse().unwrap()),
            created_by,
            deleted: false,
        }
    }

    fn sample_task() -> Task {
        Task::new(
            "prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            Some("metis-worker:latest".to_string()),
            None,
            Default::default(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        )
    }

    /// Task with creator and other fields set for round-trip tests.
    fn task_with_creator_for_round_trip() -> Task {
        Task::new(
            "round-trip prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("alice"),
            Some("metis-worker:latest".to_string()),
            Some("model-v1".to_string()),
            Default::default(),
            None,
            None,
            None,
            Status::Created,
            None,
            None,
        )
    }

    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("image:latest".to_string()),
            Some(RepoWorkflowConfig {
                review_requests: vec![
                    ReviewRequestConfig {
                        assignee: "alice".to_string(),
                    },
                    ReviewRequestConfig {
                        assignee: "bob".to_string(),
                    },
                ],
                merge_request: Some(MergeRequestConfig {
                    assignee: Some("charlie".to_string()),
                }),
            }),
        )
    }

    /// Task with every optional field set so serialization round-trip can assert full equality.
    fn sample_task_all_fields() -> Task {
        Task::new(
            "full prompt".to_string(),
            BundleSpec::None,
            None,
            Username::from("bob"),
            Some("img:tag".to_string()),
            Some("model-x".to_string()),
            [("K".to_string(), "V".to_string())].into_iter().collect(),
            Some("1000m".to_string()),
            Some("512Mi".to_string()),
            Some(vec!["secret-a".to_string(), "secret-b".to_string()]),
            Status::Created,
            Some("last message".to_string()),
            None,
        )
    }

    /// Patch with every optional field set so serialization round-trip can assert full equality.
    fn sample_patch_all_fields(created_by: Option<TaskId>) -> Patch {
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

    /// Issue with every optional field set so serialization round-trip can assert full equality.
    fn sample_issue_all_fields(dependencies: Vec<IssueDependency>, patches: Vec<PatchId>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "full description".to_string(),
            Username::from("issue-creator"),
            "50%".to_string(),
            IssueStatus::Open,
            Some("assignee".to_string()),
            Some(JobSettings {
                repo_name: Some(RepoName::from_str("org/proj").unwrap()),
                remote_url: Some("https://git.example.com/org/proj.git".to_string()),
                image: Some("img:v1".to_string()),
                model: Some("claude-3".to_string()),
                branch: Some("main".to_string()),
                max_retries: Some(3),
                cpu_limit: Some("2".to_string()),
                memory_limit: Some("4Gi".to_string()),
                secrets: Some(vec!["job-secret".to_string()]),
            }),
            vec![
                TodoItem::new("todo one".to_string(), false),
                TodoItem::new("todo two".to_string(), true),
            ],
            dependencies,
            patches,
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
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
        updated.default_image = Some("other:latest".to_string());
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

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let task = task_with_creator_for_round_trip();

        let (task_id, version) = store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_task(&task_id, false).await.unwrap();
        assert_eq!(
            fetched.item.creator, task.creator,
            "creator must round-trip"
        );
        assert_eq!(fetched.item.prompt, task.prompt);
        assert_eq!(fetched.item.image, task.image);
        assert_eq!(fetched.item.model, task.model);
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.prompt = "updated prompt".to_string();
        store
            .update_task(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched2 = store.get_task(&task_id, false).await.unwrap();
        assert_eq!(
            fetched2.item.creator, task.creator,
            "creator must persist across updates"
        );
        assert_eq!(fetched2.item.prompt, "updated prompt");
        assert_eq!(fetched2.version, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let (parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_issue(&issue, false).await.unwrap();
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

        let (new_parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let mut updated_issue = sample_issue(vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            new_parent.clone(),
        )]);
        updated_issue.patches = Vec::new();
        store
            .update_issue(&issue, updated_issue, &ActorRef::test())
            .await
            .unwrap();

        let fetched_after_update = store.get_issue(&issue, false).await.unwrap();
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
        let (patch_id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        let mut issue = sample_issue(vec![]);
        issue.patches = vec![patch_id.clone()];
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        let issues = store.get_issues_for_patch(&patch_id).await.unwrap();
        assert_eq!(issues, vec![issue_id]);

        let mut updated = sample_patch();
        updated.title = "updated".to_string();
        store
            .update_patch(&patch_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();
        let fetched = store.get_patch(&patch_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "updated");
        assert_eq!(fetched.version, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_lifecycle_updates_status_v2(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
        let handles = test_state_with_store(store.clone());
        let (issue_id, _) = handles
            .store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut task = sample_task();
        task.spawned_from = Some(issue_id.clone());
        let (task_id, _) = handles
            .store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            handles
                .store
                .get_task(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Created
        );

        handles
            .state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_running(&task_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            handles
                .store
                .get_task(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Running
        );

        handles
            .state
            .transition_task_to_completion(&task_id, Ok(()), Some("done".into()), ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            handles
                .store
                .get_task(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Complete
        );

        let tasks = handles.store.get_tasks_for_issue(&issue_id).await.unwrap();
        assert_eq!(tasks, vec![task_id.clone()]);

        let query = SearchJobsQuery::new(None, None, None, Some(Status::Complete.into()));
        let complete: Vec<_> = handles
            .store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(complete, vec![task_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn documents_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (doc_id, _) = store
            .add_document(sample_document("docs/guide.md", None), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "Doc");
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.title = "Updated Doc".to_string();
        store
            .update_document(&doc_id, updated.clone(), &ActorRef::test())
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

        let by_path = store.get_documents_by_path("/docs/").await.unwrap();
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].0, doc_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn user_management_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let user = User {
            username: Username::from("alice"),
            github_user_id: Some(101),
            deleted: false,
        };
        store
            .add_user(user.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store
            .get_user(&Username::from("alice"), false)
            .await
            .unwrap();
        assert_eq!(fetched.item, user);
        assert_eq!(fetched.version, 1);

        let updated = store
            .update_user(
                User {
                    username: Username::from("alice"),
                    github_user_id: Some(202),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(updated.item.github_user_id, Some(202));
        assert_eq!(updated.version, 2);
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    /// Catches missing persistence/read of any field.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let name = RepoName::from_str("roundtrip/repo").unwrap();
        let repo = sample_repository_config();

        store
            .add_repository(name.clone(), repo.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item, repo, "Repository must round-trip all fields");
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let task = sample_task_all_fields();

        let (task_id, _) = store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_task(&task_id, false).await.unwrap();
        assert_eq!(
            fetched.item, task,
            "Task must round-trip all fields (creator, secrets, image, model, etc.)"
        );
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn patch_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (task_id, _) = store
            .add_task(sample_task_all_fields(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let patch = sample_patch_all_fields(Some(task_id));

        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&patch_id, false).await.unwrap();
        assert_eq!(
            fetched.item, patch,
            "Patch must round-trip all fields (creator, base_branch, branch_name, commit_range, github, etc.)"
        );
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (parent_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (patch_id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        let issue = sample_issue_all_fields(
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id,
            )],
            vec![patch_id],
        );

        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item, issue,
            "Issue must round-trip all fields (assignee, job_settings, todo_list, dependencies, patches)"
        );
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (task_id, _) = store
            .add_task(sample_task_all_fields(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let doc = sample_document("docs/roundtrip.md", Some(task_id));

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

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn user_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let user = User {
            username: Username::from("roundtrip_user"),
            github_user_id: Some(999),
            deleted: false,
        };

        store
            .add_user(user.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store
            .get_user(&Username::from("roundtrip_user"), false)
            .await
            .unwrap();
        assert_eq!(fetched.item, user, "User must round-trip all fields");
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn actor_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (actor, _token) = Actor::new_for_user(Username::from("actor_creator"));

        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_actor(&actor.name()).await.unwrap();
        assert_eq!(
            fetched.item, actor,
            "Actor must round-trip all fields (creator, actor_id, etc.)"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create a document with title "original_title"
        let doc = Document {
            title: "original_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        let (doc_id, _) = store.add_document(doc, &ActorRef::test()).await.unwrap();

        // Update the document to change the title to "changed_title"
        let updated_doc = Document {
            title: "changed_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        store
            .update_document(&doc_id, updated_doc, &ActorRef::test())
            .await
            .unwrap();

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
            "Original Title".to_string(),
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
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        // Update the issue to change the description
        let updated_issue = Issue::new(
            IssueType::Task,
            "Updated Title".to_string(),
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
        store
            .update_issue(&issue_id, updated_issue, &ActorRef::test())
            .await
            .unwrap();

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
            Username::from("test-creator"),
            vec![],
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        );
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        // Update the patch to change the title
        let updated_patch = Patch::new(
            "changed_unique_patch_title_xyz789".to_string(),
            "desc".to_string(),
            "diff content".to_string(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            vec![],
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        );
        store
            .update_patch(&patch_id, updated_patch, &ActorRef::test())
            .await
            .unwrap();

        // Search for the old title - should return NO results
        let old_query = SearchPatchesQuery::new(
            Some("original_unique_patch_title_abc123".to_string()),
            None,
            vec![],
            None,
        );
        let old_results = store.list_patches(&old_query).await.unwrap();
        assert!(
            old_results.is_empty(),
            "Search for old title should return no results, but got {:?}",
            old_results.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );

        // Search for the new title - should return the patch
        let new_query = SearchPatchesQuery::new(
            Some("changed_unique_patch_title_xyz789".to_string()),
            None,
            vec![],
            None,
        );
        let new_results = store.list_patches(&new_query).await.unwrap();
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].0, patch_id);
        assert_eq!(
            new_results[0].1.item.title,
            "changed_unique_patch_title_xyz789"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_by_sender_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let alice = ActorId::Username(Username::from("alice").into());
        let bob = ActorId::Username(Username::from("bob").into());
        let recipient = ActorId::Username(Username::from("recipient").into());

        let msg_alice = Message::new(Some(alice.clone()), recipient.clone(), "from alice".into());
        let msg_bob = Message::new(Some(bob.clone()), recipient.clone(), "from bob".into());

        let (id_alice, _) = store
            .add_message(msg_alice, &ActorRef::test())
            .await
            .unwrap();
        let (_id_bob, _) = store.add_message(msg_bob, &ActorRef::test()).await.unwrap();

        let mut query = SearchMessagesQuery::default();
        query.sender = Some(alice.to_string());
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id_alice);
        assert_eq!(results[0].1.item.body, "from alice");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_by_recipient_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("sender").into());
        let bob = ActorId::Username(Username::from("bob").into());
        let carol = ActorId::Username(Username::from("carol").into());

        let msg_to_bob = Message::new(Some(sender.clone()), bob.clone(), "to bob".into());
        let msg_to_carol = Message::new(Some(sender.clone()), carol.clone(), "to carol".into());

        let (id_bob, _) = store
            .add_message(msg_to_bob, &ActorRef::test())
            .await
            .unwrap();
        let (_id_carol, _) = store
            .add_message(msg_to_carol, &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchMessagesQuery::default();
        query.recipient = Some(bob.to_string());
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id_bob);
        assert_eq!(results[0].1.item.body, "to bob");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_by_before_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        let msg_early = Message::new(Some(sender.clone()), recipient.clone(), "early".into());
        store
            .add_message(msg_early, &ActorRef::test())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mid_time = Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msg_late = Message::new(Some(sender.clone()), recipient.clone(), "late".into());
        store
            .add_message(msg_late, &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchMessagesQuery::default();
        query.before = Some(mid_time);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.body, "early");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_by_after_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        let msg_early = Message::new(Some(sender.clone()), recipient.clone(), "early".into());
        store
            .add_message(msg_early, &ActorRef::test())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mid_time = Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let msg_late = Message::new(Some(sender.clone()), recipient.clone(), "late".into());
        store
            .add_message(msg_late, &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchMessagesQuery::default();
        query.after = Some(mid_time);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.body, "late");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_exclude_deleted_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        let msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "will be deleted".into(),
        );
        let (msg_id, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let alive_msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "still alive".into(),
        );
        let (alive_id, _) = store
            .add_message(alive_msg, &ActorRef::test())
            .await
            .unwrap();

        // Delete the first message by updating with deleted=true
        let deleted = Message {
            sender: Some(sender.clone()),
            recipient: recipient.clone(),
            body: "will be deleted".into(),
            deleted: true,
            is_read: false,
        };
        store
            .update_message(&msg_id, deleted, &ActorRef::test())
            .await
            .unwrap();

        // Default query should exclude deleted messages
        let query = SearchMessagesQuery::default();
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, alive_id);
        assert_eq!(results[0].1.item.body, "still alive");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_include_deleted_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        let msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "will be deleted".into(),
        );
        let (msg_id, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let alive_msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "still alive".into(),
        );
        store
            .add_message(alive_msg, &ActorRef::test())
            .await
            .unwrap();

        // Delete the first message
        let deleted = Message {
            sender: Some(sender.clone()),
            recipient: recipient.clone(),
            body: "will be deleted".into(),
            deleted: true,
            is_read: false,
        };
        store
            .update_message(&msg_id, deleted, &ActorRef::test())
            .await
            .unwrap();

        // Query with include_deleted should return both
        let mut query = SearchMessagesQuery::default();
        query.include_deleted = Some(true);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_limit_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        for i in 0..5 {
            let msg = Message::new(
                Some(sender.clone()),
                recipient.clone(),
                format!("message {i}"),
            );
            store.add_message(msg, &ActorRef::test()).await.unwrap();
        }

        let mut query = SearchMessagesQuery::default();
        query.limit = Some(2);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        let msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "original_body_abc123".into(),
        );
        let (msg_id, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        // Update the message body
        let updated = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "changed_body_xyz789".into(),
        );
        store
            .update_message(&msg_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Search should only return the latest version
        let query = SearchMessagesQuery::default();
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, msg_id);
        assert_eq!(results[0].1.item.body, "changed_body_xyz789");
        assert_eq!(results[0].1.version, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn message_filter_by_is_read_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        // Create an unread message (default)
        let msg_unread = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "unread message".into(),
        );
        let (_id_unread, _) = store
            .add_message(msg_unread, &ActorRef::test())
            .await
            .unwrap();

        // Create a message and mark it as read
        let msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "read message".into(),
        );
        let (id_read, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let read_msg = Message {
            sender: Some(sender.clone()),
            recipient: recipient.clone(),
            body: "read message".into(),
            deleted: false,
            is_read: true,
        };
        store
            .update_message(&id_read, read_msg, &ActorRef::test())
            .await
            .unwrap();

        // Filter for read messages only
        let mut query = SearchMessagesQuery::default();
        query.is_read = Some(true);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id_read);
        assert_eq!(results[0].1.item.body, "read message");
        assert!(results[0].1.item.is_read);

        // Filter for unread messages only
        let mut query = SearchMessagesQuery::default();
        query.is_read = Some(false);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.body, "unread message");
        assert!(!results[0].1.item.is_read);

        // No filter returns all messages
        let query = SearchMessagesQuery::default();
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    // ---- Notification tests ----

    fn sample_notification(recipient: ActorId) -> Notification {
        Notification::new(
            recipient,
            Some(ActorId::Username(Username::from("alice").into())),
            "issue".to_string(),
            IssueId::new().into(),
            1,
            "updated".to_string(),
            "Issue status changed".to_string(),
            None,
            "walk_up".to_string(),
        )
    }

    fn make_notif_query(recipient: Option<String>) -> ListNotificationsQuery {
        let mut q = ListNotificationsQuery::default();
        q.recipient = recipient;
        q
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_insert_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient);

        let id = store.insert_notification(notif).await.unwrap();
        assert!(
            id.as_ref().starts_with("nf-"),
            "notification id should start with nf-, got: {id}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_get_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient.clone());

        let id = store.insert_notification(notif.clone()).await.unwrap();
        let fetched = store.get_notification(&id).await.unwrap();

        assert_eq!(fetched.recipient, recipient);
        assert_eq!(
            fetched.source_actor,
            Some(ActorId::Username(Username::from("alice").into()))
        );
        assert_eq!(fetched.object_kind, "issue");
        assert_eq!(fetched.object_id, notif.object_id);
        assert_eq!(fetched.object_version, 1);
        assert_eq!(fetched.event_type, "updated");
        assert_eq!(fetched.summary, "Issue status changed");
        assert_eq!(fetched.source_issue_id, None);
        assert_eq!(fetched.policy, "walk_up");
        assert!(!fetched.is_read);
        // created_at should be close to now (within a few seconds)
        let elapsed = Utc::now() - fetched.created_at;
        assert!(elapsed.num_seconds() < 10);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_list_filters_by_recipient_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alice = ActorId::Username(Username::from("alice").into());
        let bob = ActorId::Username(Username::from("bob").into());

        store
            .insert_notification(sample_notification(alice.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(bob.clone()))
            .await
            .unwrap();

        let query = make_notif_query(Some(alice.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.recipient, alice);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_list_filters_by_is_read_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        let notif1 = sample_notification(recipient.clone());
        let id1 = store.insert_notification(notif1).await.unwrap();
        let notif2 = sample_notification(recipient.clone());
        let _id2 = store.insert_notification(notif2).await.unwrap();

        // Mark one as read
        store.mark_notification_read(&id1).await.unwrap();

        // List unread only
        let mut query = make_notif_query(Some(recipient.to_string()));
        query.is_read = Some(false);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1.is_read);

        // List read only
        let mut query = make_notif_query(Some(recipient.to_string()));
        query.is_read = Some(true);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_read);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_list_sorted_by_created_at_desc_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        let mut notif1 = sample_notification(recipient.clone());
        notif1.created_at = Utc::now() - chrono::Duration::hours(2);
        notif1.summary = "oldest".to_string();
        store.insert_notification(notif1).await.unwrap();

        let mut notif2 = sample_notification(recipient.clone());
        notif2.created_at = Utc::now() - chrono::Duration::hours(1);
        notif2.summary = "middle".to_string();
        store.insert_notification(notif2).await.unwrap();

        let mut notif3 = sample_notification(recipient.clone());
        notif3.created_at = Utc::now();
        notif3.summary = "newest".to_string();
        store.insert_notification(notif3).await.unwrap();

        let query = make_notif_query(Some(recipient.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].1.summary, "newest");
        assert_eq!(results[1].1.summary, "middle");
        assert_eq!(results[2].1.summary, "oldest");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_list_respects_limit_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        for _ in 0..5 {
            store
                .insert_notification(sample_notification(recipient.clone()))
                .await
                .unwrap();
        }

        let mut query = make_notif_query(Some(recipient.to_string()));
        query.limit = Some(3);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_count_unread_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        // Initially 0
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );

        // Insert 3 notifications
        let id1 = store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            3
        );

        // Mark one as read
        store.mark_notification_read(&id1).await.unwrap();
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            2
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_mark_read_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient);
        let id = store.insert_notification(notif).await.unwrap();

        store.mark_notification_read(&id).await.unwrap();

        let fetched = store.get_notification(&id).await.unwrap();
        assert!(fetched.is_read);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_mark_read_not_found_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let unknown_id: NotificationId = "nf-abcdef".parse().unwrap();
        let result = store.mark_notification_read(&unknown_id).await;
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), StoreError::NotificationNotFound(_)),
            "expected NotificationNotFound error"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_mark_all_read_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();

        let marked = store
            .mark_all_notifications_read(&recipient, None)
            .await
            .unwrap();
        assert_eq!(marked, 3);

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );

        // Calling again returns 0 (all already read)
        let marked = store
            .mark_all_notifications_read(&recipient, None)
            .await
            .unwrap();
        assert_eq!(marked, 0);
    }

    fn sample_label() -> Label {
        Label::new("bug".to_string(), "#ff0000".parse().unwrap(), true, false)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn label_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let label = sample_label();

        // CREATE
        let label_id = store.add_label(label).await.unwrap();

        // READ
        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "bug");
        assert_eq!(fetched.color.as_ref(), "#ff0000");
        assert!(!fetched.deleted);

        // UPDATE
        let updated_label = Label::new(
            "critical-bug".to_string(),
            "#cc0000".parse().unwrap(),
            true,
            false,
        );
        store.update_label(&label_id, updated_label).await.unwrap();

        let fetched_updated = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched_updated.name, "critical-bug");
        assert_eq!(fetched_updated.color.as_ref(), "#cc0000");

        // LIST
        let list = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, label_id);
        assert_eq!(list[0].1.name, "critical-bug");
        assert_eq!(list[0].1.color.as_ref(), "#cc0000");

        // GET BY NAME
        let by_name = store.get_label_by_name("critical-bug").await.unwrap();
        assert!(by_name.is_some());
        let (found_id, found_label) = by_name.unwrap();
        assert_eq!(found_id, label_id);
        assert_eq!(found_label.name, "critical-bug");

        // DELETE
        store.delete_label(&label_id).await.unwrap();

        // get_label should return LabelNotFound for deleted label
        let get_result = store.get_label(&label_id).await;
        assert!(matches!(get_result, Err(StoreError::LabelNotFound(_))));

        // list_labels with include_deleted should still show the soft-deleted label
        let mut query_deleted = SearchLabelsQuery::default();
        query_deleted.include_deleted = Some(true);
        let list_with_deleted = store.list_labels(&query_deleted).await.unwrap();
        assert_eq!(list_with_deleted.len(), 1);
        assert_eq!(list_with_deleted[0].0, label_id);
        assert!(list_with_deleted[0].1.deleted);

        // list_labels without include_deleted should not show deleted label
        let list_without_deleted = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert!(list_without_deleted.is_empty());

        // UNIQUENESS — creating a label with a duplicate name should fail
        let label2 = Label::new(
            "feature".to_string(),
            "#00ff00".parse().unwrap(),
            true,
            false,
        );
        store.add_label(label2).await.unwrap();

        let duplicate = Label::new(
            "feature".to_string(),
            "#0000ff".parse().unwrap(),
            true,
            false,
        );
        let dup_result = store.add_label(duplicate).await;
        assert!(matches!(dup_result, Err(StoreError::LabelAlreadyExists(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn notification_mark_all_read_with_before_filter_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let recipient = ActorId::Username(Username::from("bob").into());

        // Insert notification with known created_at
        let mut notif1 = sample_notification(recipient.clone());
        notif1.created_at = Utc::now() - chrono::Duration::hours(2);
        store.insert_notification(notif1).await.unwrap();

        let mut notif2 = sample_notification(recipient.clone());
        notif2.created_at = Utc::now() - chrono::Duration::hours(1);
        store.insert_notification(notif2).await.unwrap();

        let notif3 = sample_notification(recipient.clone());
        store.insert_notification(notif3).await.unwrap();

        // Mark only notifications before 30 minutes ago
        let cutoff = Utc::now() - chrono::Duration::minutes(30);
        let marked = store
            .mark_all_notifications_read(&recipient, Some(cutoff))
            .await
            .unwrap();
        assert_eq!(marked, 2);

        // One still unread (the most recent)
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            1
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn label_association_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create two labels
        let label1 = Label::new("bug".to_string(), "#e74c3c".parse().unwrap(), true, false);
        let label1_id = store.add_label(label1).await.unwrap();

        let label2 = Label::new(
            "feature".to_string(),
            "#3498db".parse().unwrap(),
            true,
            false,
        );
        let label2_id = store.add_label(label2).await.unwrap();

        // Create an issue to associate labels with
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let object_id = MetisId::from(issue_id.clone());

        // Associate both labels with the issue
        store
            .add_label_association(&label1_id, &object_id)
            .await
            .unwrap();
        store
            .add_label_association(&label2_id, &object_id)
            .await
            .unwrap();

        // get_labels_for_object returns both labels
        let labels = store.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels.len(), 2);
        let label_names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        assert!(label_names.contains(&"bug"));
        assert!(label_names.contains(&"feature"));

        // get_labels_for_objects (batch) returns labels keyed by object
        let batch = store
            .get_labels_for_objects(&[object_id.clone()])
            .await
            .unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[&object_id].len(), 2);

        // get_objects_for_label returns the issue for each label
        let objects1 = store.get_objects_for_label(&label1_id).await.unwrap();
        assert_eq!(objects1.len(), 1);
        assert_eq!(objects1[0], object_id);

        let objects2 = store.get_objects_for_label(&label2_id).await.unwrap();
        assert_eq!(objects2.len(), 1);
        assert_eq!(objects2[0], object_id);

        // Idempotent add — adding the same association again is a no-op
        store
            .add_label_association(&label1_id, &object_id)
            .await
            .unwrap();
        let labels_after_dup = store.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels_after_dup.len(), 2);

        // Remove one association
        store
            .remove_label_association(&label1_id, &object_id)
            .await
            .unwrap();
        let labels_after_remove = store.get_labels_for_object(&object_id).await.unwrap();
        assert_eq!(labels_after_remove.len(), 1);
        assert_eq!(labels_after_remove[0].name, "feature");

        // Idempotent remove — removing a non-existent association is a no-op
        store
            .remove_label_association(&label1_id, &object_id)
            .await
            .unwrap();

        // Deleted labels are excluded from results
        store.delete_label(&label2_id).await.unwrap();
        let labels_after_delete = store.get_labels_for_object(&object_id).await.unwrap();
        assert!(labels_after_delete.is_empty());

        // Batch query also excludes deleted labels
        let batch_after_delete = store
            .get_labels_for_objects(&[object_id.clone()])
            .await
            .unwrap();
        let empty_or_missing = batch_after_delete
            .get(&object_id)
            .map(|v| v.is_empty())
            .unwrap_or(true);
        assert!(empty_or_missing);
    }

    mod map_sqlx_error_tests {
        use super::super::map_sqlx_error;
        use crate::store::StoreError;
        use std::borrow::Cow;

        #[derive(Debug)]
        struct MockDatabaseError {
            code: Option<String>,
            constraint: Option<String>,
            message: String,
        }

        impl std::fmt::Display for MockDatabaseError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.message)
            }
        }

        impl std::error::Error for MockDatabaseError {}

        impl sqlx::error::DatabaseError for MockDatabaseError {
            fn message(&self) -> &str {
                &self.message
            }

            fn code(&self) -> Option<Cow<'_, str>> {
                self.code.as_deref().map(Cow::Borrowed)
            }

            fn constraint(&self) -> Option<&str> {
                self.constraint.as_deref()
            }

            fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
                self
            }

            fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
                self
            }

            fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
                self
            }

            fn kind(&self) -> sqlx::error::ErrorKind {
                sqlx::error::ErrorKind::UniqueViolation
            }
        }

        #[test]
        fn maps_label_unique_violation_to_label_already_exists() {
            let db_err = MockDatabaseError {
                code: Some("23505".to_string()),
                constraint: Some("labels_name_idx".to_string()),
                message: "duplicate key value violates unique constraint \"labels_name_idx\" Detail: Key (name)=(bug) already exists.".to_string(),
            };
            let err = sqlx::Error::Database(Box::new(db_err));
            let result = map_sqlx_error(err);
            assert!(
                matches!(result, StoreError::LabelAlreadyExists(ref name) if name == "bug"),
                "expected LabelAlreadyExists(\"bug\"), got: {result:?}"
            );
        }

        #[test]
        fn maps_non_matching_constraint_to_internal() {
            let db_err = MockDatabaseError {
                code: Some("23505".to_string()),
                constraint: Some("other_constraint".to_string()),
                message: "duplicate key value violates unique constraint".to_string(),
            };
            let err = sqlx::Error::Database(Box::new(db_err));
            let result = map_sqlx_error(err);
            assert!(
                matches!(result, StoreError::Internal(_)),
                "expected Internal, got: {result:?}"
            );
        }

        #[test]
        fn maps_non_database_error_to_internal() {
            let err = sqlx::Error::RowNotFound;
            let result = map_sqlx_error(err);
            assert!(
                matches!(result, StoreError::Internal(_)),
                "expected Internal, got: {result:?}"
            );
        }
    }

    // ---- Agent helpers & round-trip tests ----

    fn sample_agent() -> Agent {
        Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt.md".to_string(),
            3,
            5,
            false,
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let agent = sample_agent();

        // ADD
        store.add_agent(agent).await.unwrap();

        // GET — verify all fields
        let fetched = store.get_agent("test-agent").await.unwrap();
        assert_eq!(fetched.name, "test-agent");
        assert_eq!(fetched.prompt_path, "/agents/test-agent/prompt.md");
        assert_eq!(fetched.max_tries, 3);
        assert_eq!(fetched.max_simultaneous, 5);
        assert!(!fetched.is_assignment_agent);
        assert!(!fetched.deleted);

        // UPDATE — change prompt_path, max_tries, max_simultaneous
        let updated = Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt_v2.md".to_string(),
            5,
            10,
            false,
        );
        store.update_agent(updated).await.unwrap();

        // GET — verify updated fields persisted
        let fetched2 = store.get_agent("test-agent").await.unwrap();
        assert_eq!(fetched2.prompt_path, "/agents/test-agent/prompt_v2.md");
        assert_eq!(fetched2.max_tries, 5);
        assert_eq!(fetched2.max_simultaneous, 10);
        assert!(fetched2.updated_at >= fetched.created_at);

        // LIST — verify agent appears with updated values
        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "test-agent");
        assert_eq!(list[0].prompt_path, "/agents/test-agent/prompt_v2.md");

        // DELETE
        store.delete_agent("test-agent").await.unwrap();

        // GET after delete — should return AgentNotFound
        let get_result = store.get_agent("test-agent").await;
        assert!(
            matches!(get_result, Err(StoreError::AgentNotFound(_))),
            "expected AgentNotFound, got: {get_result:?}"
        );

        // LIST after delete — should return empty vec
        let list_after = store.list_agents().await.unwrap();
        assert!(list_after.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_duplicate_name_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Add first agent
        store.add_agent(sample_agent()).await.unwrap();

        // Adding another agent with the same name should fail
        let dup_result = store.add_agent(sample_agent()).await;
        assert!(
            matches!(dup_result, Err(StoreError::AgentAlreadyExists(_))),
            "expected AgentAlreadyExists, got: {dup_result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_assignment_uniqueness_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Add agent A with is_assignment_agent = true
        let agent_a = Agent::new(
            "agent-a".to_string(),
            "/agents/a/prompt.md".to_string(),
            3,
            5,
            true,
        );
        store.add_agent(agent_a).await.unwrap();

        // Add agent B with is_assignment_agent = true — should fail
        let agent_b = Agent::new(
            "agent-b".to_string(),
            "/agents/b/prompt.md".to_string(),
            3,
            5,
            true,
        );
        let add_result = store.add_agent(agent_b).await;
        assert!(
            matches!(add_result, Err(StoreError::AssignmentAgentAlreadyExists)),
            "expected AssignmentAgentAlreadyExists, got: {add_result:?}"
        );

        // Add agent B with is_assignment_agent = false — should succeed
        let agent_b_no_assign = Agent::new(
            "agent-b".to_string(),
            "/agents/b/prompt.md".to_string(),
            3,
            5,
            false,
        );
        store.add_agent(agent_b_no_assign).await.unwrap();

        // Update agent B to set is_assignment_agent = true — should fail
        let agent_b_assign = Agent::new(
            "agent-b".to_string(),
            "/agents/b/prompt.md".to_string(),
            3,
            5,
            true,
        );
        let update_result = store.update_agent(agent_b_assign).await;
        assert!(
            matches!(update_result, Err(StoreError::AssignmentAgentAlreadyExists)),
            "expected AssignmentAgentAlreadyExists, got: {update_result:?}"
        );

        // Delete agent A, then add agent C with is_assignment_agent = true — should succeed
        store.delete_agent("agent-a").await.unwrap();
        let agent_c = Agent::new(
            "agent-c".to_string(),
            "/agents/c/prompt.md".to_string(),
            3,
            5,
            true,
        );
        store.add_agent(agent_c).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_reactivation_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Add and then soft-delete an agent
        store.add_agent(sample_agent()).await.unwrap();
        store.delete_agent("test-agent").await.unwrap();

        // Verify it's gone
        assert!(matches!(
            store.get_agent("test-agent").await,
            Err(StoreError::AgentNotFound(_))
        ));

        // Add a new agent with the same name but different fields — reactivation
        let reactivated = Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt_new.md".to_string(),
            7,
            12,
            false,
        );
        store.add_agent(reactivated).await.unwrap();

        // Get the agent — verify it has the new field values and deleted = false
        let fetched = store.get_agent("test-agent").await.unwrap();
        assert_eq!(fetched.name, "test-agent");
        assert_eq!(fetched.prompt_path, "/agents/test-agent/prompt_new.md");
        assert_eq!(fetched.max_tries, 7);
        assert_eq!(fetched.max_simultaneous, 12);
        assert!(!fetched.is_assignment_agent);
        assert!(!fetched.deleted);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn secret_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let alice = Username::from("alice");
        let bob = Username::from("bob");

        // SET — store secrets for alice
        store
            .set_user_secret(&alice, "api-key", b"encrypted-alice-key")
            .await
            .unwrap();
        store
            .set_user_secret(&alice, "db-password", b"encrypted-alice-db")
            .await
            .unwrap();

        // SET — store a secret for bob
        store
            .set_user_secret(&bob, "api-key", b"encrypted-bob-key")
            .await
            .unwrap();

        // GET — verify alice's secrets round-trip correctly
        let value = store
            .get_user_secret(&alice, "api-key")
            .await
            .unwrap()
            .expect("expected alice api-key to exist");
        assert_eq!(value, b"encrypted-alice-key");

        let value = store
            .get_user_secret(&alice, "db-password")
            .await
            .unwrap()
            .expect("expected alice db-password to exist");
        assert_eq!(value, b"encrypted-alice-db");

        // GET — verify bob's secret is isolated from alice's
        let value = store
            .get_user_secret(&bob, "api-key")
            .await
            .unwrap()
            .expect("expected bob api-key to exist");
        assert_eq!(value, b"encrypted-bob-key");

        // GET — non-existent secret returns None
        assert!(
            store
                .get_user_secret(&alice, "nonexistent")
                .await
                .unwrap()
                .is_none()
        );

        // LIST — verify alice's secret names
        let names = store.list_user_secret_names(&alice).await.unwrap();
        assert_eq!(names, vec!["api-key", "db-password"]);

        // LIST — verify bob's secret names
        let names = store.list_user_secret_names(&bob).await.unwrap();
        assert_eq!(names, vec!["api-key"]);

        // UPSERT — overwrite alice's api-key
        store
            .set_user_secret(&alice, "api-key", b"encrypted-alice-key-v2")
            .await
            .unwrap();
        let value = store
            .get_user_secret(&alice, "api-key")
            .await
            .unwrap()
            .expect("expected alice api-key to exist after upsert");
        assert_eq!(value, b"encrypted-alice-key-v2");

        // DELETE — remove alice's api-key
        store.delete_user_secret(&alice, "api-key").await.unwrap();

        // GET after delete — returns None
        assert!(
            store
                .get_user_secret(&alice, "api-key")
                .await
                .unwrap()
                .is_none()
        );

        // LIST after delete — alice should only have db-password
        let names = store.list_user_secret_names(&alice).await.unwrap();
        assert_eq!(names, vec!["db-password"]);

        // Bob's secret should be unaffected
        let value = store
            .get_user_secret(&bob, "api-key")
            .await
            .unwrap()
            .expect("bob api-key should still exist");
        assert_eq!(value, b"encrypted-bob-key");

        // DELETE — remove remaining secrets
        store
            .delete_user_secret(&alice, "db-password")
            .await
            .unwrap();
        store.delete_user_secret(&bob, "api-key").await.unwrap();

        // LIST after full delete — both users should be empty
        assert!(
            store
                .list_user_secret_names(&alice)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.list_user_secret_names(&bob).await.unwrap().is_empty());

        // DELETE non-existent — should not error
        store
            .delete_user_secret(&alice, "nonexistent")
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn object_relationship_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create two issues to use as relationship endpoints.
        let (issue_a, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_b, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_c, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let id_a = MetisId::from(issue_a.clone());
        let id_b = MetisId::from(issue_b.clone());
        let id_c = MetisId::from(issue_c.clone());

        // ADD — create relationships
        let created = store
            .add_relationship(&id_a, &id_b, "child-of")
            .await
            .unwrap();
        assert!(created, "first insert should return true");

        let created = store
            .add_relationship(&id_a, &id_c, "blocked-by")
            .await
            .unwrap();
        assert!(created);

        let created = store
            .add_relationship(&id_b, &id_c, "child-of")
            .await
            .unwrap();
        assert!(created);

        // ADD duplicate — should return false (ON CONFLICT DO NOTHING)
        let created = store
            .add_relationship(&id_a, &id_b, "child-of")
            .await
            .unwrap();
        assert!(!created, "duplicate insert should return false");

        // GET — filter by source_id
        let rels = store
            .get_relationships(Some(&id_a), None, None)
            .await
            .unwrap();
        assert_eq!(rels.len(), 2);
        assert!(
            rels.iter()
                .any(|r| r.target_id == id_b && r.rel_type == "child-of")
        );
        assert!(
            rels.iter()
                .any(|r| r.target_id == id_c && r.rel_type == "blocked-by")
        );

        // GET — filter by target_id
        let rels = store
            .get_relationships(None, Some(&id_c), None)
            .await
            .unwrap();
        assert_eq!(rels.len(), 2);
        assert!(rels.iter().any(|r| r.source_id == id_a));
        assert!(rels.iter().any(|r| r.source_id == id_b));

        // GET — filter by rel_type
        let rels = store
            .get_relationships(None, None, Some("child-of"))
            .await
            .unwrap();
        assert_eq!(rels.len(), 2);

        // GET — filter by source_id + rel_type
        let rels = store
            .get_relationships(Some(&id_a), None, Some("child-of"))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].target_id, id_b);

        // GET — filter by source_id + target_id + rel_type (exact match)
        let rels = store
            .get_relationships(Some(&id_a), Some(&id_b), Some("child-of"))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, id_a);
        assert_eq!(rels[0].target_id, id_b);
        assert_eq!(rels[0].rel_type, "child-of");
        assert_eq!(rels[0].source_kind, "issue");
        assert_eq!(rels[0].target_kind, "issue");

        // REMOVE — delete one relationship
        let removed = store
            .remove_relationship(&id_a, &id_b, "child-of")
            .await
            .unwrap();
        assert!(removed, "removing existing relationship should return true");

        // REMOVE again — should return false
        let removed = store
            .remove_relationship(&id_a, &id_b, "child-of")
            .await
            .unwrap();
        assert!(
            !removed,
            "removing non-existent relationship should return false"
        );

        // GET after remove — only two relationships remain
        let rels = store
            .get_relationships(Some(&id_a), None, None)
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].rel_type, "blocked-by");

        // REMOVE remaining relationships
        store
            .remove_relationship(&id_a, &id_c, "blocked-by")
            .await
            .unwrap();
        store
            .remove_relationship(&id_b, &id_c, "child-of")
            .await
            .unwrap();

        // GET — no relationships left
        let rels = store.get_relationships(None, None, None).await.unwrap();
        assert!(rels.is_empty());
    }
}
