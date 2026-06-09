//! PostgresStoreV2 implementation using column-based v2 tables.
//!
//! This store implementation uses the v2 tables with proper column definitions
//! instead of JSONB payloads, providing better query performance and type safety.

use crate::domain::conversations::{Conversation, ConversationStatus};
use crate::store::status_to_db_str;
use crate::{
    domain::{
        actors::ActorRef,
        agents::Agent,
        documents::Document,
        issues::{Issue, IssueDependency, IssueDependencyType, IssueType, SessionSettings},
        labels::Label,
        patches::{CommitRange, GithubPr, Patch, PatchStatus, Review},
        secrets::SecretRef,
        sessions::Session,
        task_status::{Status, TaskError},
        users::{User, Username},
    },
    store::{
        AuthTokenRow, ConversationEventSummary, ReadOnlyStore, SessionEvent, SessionEventSummary,
        Store, StoreError, TaskStatusLog,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::documents::SearchDocumentsQuery;
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::pagination::{DecodedCursor, MAX_LIMIT as PAGINATION_MAX_LIMIT};
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::api::v1::projects::{Project, ProjectKey, StatusDefinition, StatusKey};
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::api::v1::users::SearchUsersQuery;
use hydra_common::triggers::Trigger;
use hydra_common::{
    ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, ProjectId, RepoName, Rgb,
    SessionId, TriggerId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    ids::random_len_for_count,
    repositories::{Repository, SearchRepositoriesQuery},
};
use serde_json::Value;
use sqlx::{
    Pool, Postgres,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    time::Duration,
};

use crate::config::DatabaseSection;

pub type PgStorePool = Pool<Postgres>;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

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

/// Run the combined SQL+Rust migration sequence against `pool` up to (and
/// including) `up_to`, or to HEAD when `up_to == None`. Replaces the prior
/// `MIGRATOR.run(pool)` + background events-backfill spawn with a single
/// interleaved sequence. See `store/migrations/mod.rs` for the planning
/// helper. The numbered SQL migration list (under `migrations/`) plus
/// the Rust migration registry is the single source of truth for the
/// combined SQL+Rust ordering; new migrations append at the end and
/// must not edit prior entries — sqlx checksums each SQL migration body
/// and refuses to start if a previously applied checksum changes.
pub async fn run_migrations(pool: &PgStorePool, up_to: Option<u64>) -> Result<()> {
    use crate::store::migrations::{Backend, MigrationStep, plan_migrations, rust_migrations};
    use sqlx::migrate::Migrate;

    let steps = plan_migrations(&MIGRATOR, rust_migrations(), up_to);

    let mut conn = pool
        .acquire()
        .await
        .context("acquire postgres connection for migrations")?;
    let conn: &mut sqlx::PgConnection = &mut conn;

    conn.ensure_migrations_table()
        .await
        .context("ensure _sqlx_migrations table")?;
    if let Some(version) = conn.dirty_version().await? {
        anyhow::bail!("postgres database is in a dirty state at migration version {version}");
    }
    let mut applied: HashSet<i64> = conn
        .list_applied_migrations()
        .await?
        .into_iter()
        .map(|m| m.version)
        .collect();

    for step in steps {
        match step {
            MigrationStep::Sql(migration) => {
                if !applied.contains(&migration.version) {
                    conn.apply(migration).await.with_context(|| {
                        format!("apply postgres migration {}", migration.version)
                    })?;
                    applied.insert(migration.version);
                }
            }
            MigrationStep::Rust(rust) => {
                let name = rust.name();
                rust.run(&Backend::Postgres(pool.clone()))
                    .await
                    .with_context(|| format!("apply rust migration {name}"))?;
            }
        }
    }
    Ok(())
}

const TABLE_ISSUES_V2: &str = "metis.issues_v2";
const TABLE_PATCHES_V2: &str = "metis.patches_v2";
const TABLE_TASKS_V2: &str = "metis.tasks_v2";
const TABLE_USERS_V2: &str = "metis.users_v2";
const TABLE_REPOSITORIES_V2: &str = "metis.repositories_v2";
const TABLE_DOCUMENTS_V2: &str = "metis.documents_v2";
const TABLE_AGENTS: &str = "metis.agents";
const TABLE_LABELS: &str = "metis.labels";
const TABLE_LABEL_ASSOCIATIONS: &str = "metis.label_associations";
const TABLE_AUTH_TOKENS: &str = "metis.auth_tokens";
const TABLE_USER_SECRETS: &str = "metis.user_secrets";
const TABLE_OBJECT_RELATIONSHIPS: &str = "metis.object_relationships";
const TABLE_CONVERSATIONS_V2: &str = "metis.conversations_v2";
const TABLE_TRIGGERS: &str = "metis.triggers";
const TABLE_PROJECTS: &str = "metis.projects";
const TABLE_SESSION_EVENTS_V2: &str = "metis.session_events_v2";
const TABLE_SESSION_STATE_V2: &str = "metis.session_state_v2";

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
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {TABLE_ISSUES_V2} WHERE id = $1 LIMIT 1)"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if !exists {
            Err(StoreError::IssueNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_patch_exists(&self, id: &PatchId) -> Result<(), StoreError> {
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {TABLE_PATCHES_V2} WHERE id = $1 LIMIT 1)"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if !exists {
            Err(StoreError::PatchNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_session_exists(&self, id: &SessionId) -> Result<(), StoreError> {
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {TABLE_TASKS_V2} WHERE id = $1 LIMIT 1)"
        ))
        .bind(id.as_ref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if !exists {
            Err(StoreError::SessionNotFound(id.clone()))
        } else {
            Ok(())
        }
    }

    async fn ensure_repository_exists(&self, name: &RepoName) -> Result<(), StoreError> {
        let name_str = name.as_str();
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {TABLE_REPOSITORIES_V2} WHERE id = $1 LIMIT 1)"
        ))
        .bind(name_str.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if !exists {
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

    // Approximate row count from pg_class.reltuples. Only feeds
    // random_len_for_count, which takes ceil(log_26(n)) — a 26x error only
    // bumps the suffix by one char, so reltuples staleness is harmless.
    // Versioned tables (is_latest = true) are over-counted by the version
    // cardinality, also bounded to ~1 char of error. Soft-delete tables
    // (labels) are over-counted by the soft-deleted row count — same family
    // of small inaccuracies, same bound. reltuples is -1 on never-ANALYZEd
    // tables; GREATEST clamps it to 0 so fresh deployments fall back to the
    // default suffix until autovacuum runs ANALYZE. to_regclass resolves
    // schema-qualified names like "metis.tasks_v2" to a pg_class oid (or NULL
    // when missing), so the WHERE never matches a not-yet-created table and
    // fetch_optional falls through to unwrap_or(0).
    async fn estimated_row_count(&self, table: &str) -> Result<u64, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT GREATEST(reltuples, 0)::bigint
             FROM pg_class
             WHERE oid = to_regclass($1)
             LIMIT 1",
        )
        .bind(table)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?
        .unwrap_or(0);
        Ok(count.max(0) as u64)
    }

    async fn next_issue_id(&self) -> Result<IssueId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_ISSUES_V2).await?);
        Ok(IssueId::generate(len).expect("length within bounds"))
    }

    async fn next_patch_id(&self) -> Result<PatchId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_PATCHES_V2).await?);
        Ok(PatchId::generate(len).expect("length within bounds"))
    }

    async fn next_document_id(&self) -> Result<DocumentId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_DOCUMENTS_V2).await?);
        Ok(DocumentId::generate(len).expect("length within bounds"))
    }

    async fn next_session_id(&self) -> Result<SessionId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_TASKS_V2).await?);
        Ok(SessionId::generate(len).expect("length within bounds"))
    }

    async fn next_label_id(&self) -> Result<LabelId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_LABELS).await?);
        Ok(LabelId::generate(len).expect("length within bounds"))
    }

    async fn next_conversation_id(&self) -> Result<ConversationId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_CONVERSATIONS_V2).await?);
        Ok(ConversationId::generate(len).expect("length within bounds"))
    }

    async fn next_trigger_id(&self) -> Result<TriggerId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_TRIGGERS).await?);
        Ok(TriggerId::generate(len).expect("length within bounds"))
    }

    async fn next_project_id(&self) -> Result<ProjectId, StoreError> {
        let len = random_len_for_count(self.estimated_row_count(TABLE_PROJECTS).await?);
        Ok(ProjectId::generate(len).expect("length within bounds"))
    }

    async fn fetch_latest_version_number(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<VersionNumber>, StoreError> {
        let query = format!(
            "SELECT version_number FROM {table} WHERE id = $1 ORDER BY is_latest DESC, version_number DESC LIMIT 1"
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

    async fn insert_issue_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &IssueId,
        version_number: VersionNumber,
        issue: &Issue,
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for issue '{id}'"))
        })?;

        let job_settings_json = serde_json::to_value(&issue.session_settings).map_err(|e| {
            StoreError::Internal(format!("failed to serialize session_settings: {e}"))
        })?;
        let form_json = issue
            .form
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize form: {e}")))?;
        let form_response_json = issue
            .form_response
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize form_response: {e}")))?;
        let assignee_principal_json = issue
            .assignee
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize assignee_principal: {e}"))
            })?;
        // Keep the legacy `assignee TEXT` column populated from the
        // typed Principal's canonical path form so out-of-band readers
        // keep working.
        let assignee_path = issue.assignee.as_ref().map(|p| p.to_path());
        let status_sequence = Self::resolve_status_sequence(
            &mut **tx,
            issue.project_id.as_ref(),
            issue.status.as_str(),
        )
        .await?;
        let query = format!(
            "INSERT INTO {TABLE_ISSUES_V2} (id, version_number, issue_type, title, description, creator, progress, status_sequence, assignee, assignee_principal, job_settings, deleted, actor, form, form_response, feedback, project_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(issue.issue_type.as_str())
            .bind(&issue.title)
            .bind(&issue.description)
            .bind(issue.creator.as_str())
            .bind(&issue.progress)
            .bind(status_sequence)
            .bind(assignee_path.as_deref())
            .bind(&assignee_principal_json)
            .bind(&job_settings_json)
            .bind(issue.deleted)
            .bind(actor)
            .bind(&form_json)
            .bind(&form_response_json)
            .bind(issue.feedback.as_deref())
            .bind(issue.project_id.as_ref())
            .execute(&mut **tx)
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
        // Delete only the relationships managed by this function. Other
        // rel_types (e.g. has-document) are owned by other code paths and
        // must not be stomped by issue updates.
        let delete_sql = format!(
            "DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = $1 \
               AND rel_type IN ('child-of', 'blocked-on', 'has-patch')"
        );
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
            let rel_type = crate::store::RelationshipType::from(dep.dependency_type);
            sqlx::query(&insert_sql)
                .bind(issue_id.as_ref())
                .bind(crate::store::ObjectKind::Issue.as_str())
                .bind(dep.issue_id.as_ref())
                .bind(crate::store::ObjectKind::Issue.as_str())
                .bind(rel_type.as_str())
                .execute(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        }

        // Insert patch relationships
        for patch_id in &issue.patches {
            sqlx::query(&insert_sql)
                .bind(issue_id.as_ref())
                .bind(crate::store::ObjectKind::Issue.as_str())
                .bind(patch_id.as_ref())
                .bind(crate::store::ObjectKind::Patch.as_str())
                .bind(crate::store::RelationshipType::HasPatch.as_str())
                .execute(&mut **tx)
                .await
                .map_err(map_sqlx_error)?;
        }

        Ok(())
    }

    fn row_to_issue(&self, row: &IssueRow) -> Result<Issue, StoreError> {
        let issue_type = IssueType::from_str(&row.issue_type)
            .map_err(|e| StoreError::Internal(format!("invalid issue_type: {e}")))?;
        let status = StatusKey::try_new(row.status.clone())
            .map_err(|e| StoreError::InvalidIssueStatus(e.to_string()))?;
        let session_settings: SessionSettings = serde_json::from_value(row.job_settings.clone())
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize session_settings: {e}"))
            })?;
        let form = row
            .form
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to deserialize form: {e}")))?;
        let form_response = row
            .form_response
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize form_response: {e}"))
            })?;
        // `assignee_principal` is the source of truth for `Issue.assignee`.
        // The legacy `assignee TEXT` column is still dual-written but no
        // longer read here.
        let assignee = row
            .assignee_principal
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize assignee_principal: {e}"))
            })?;
        let project_id = ProjectId::try_from(row.project_id.clone())
            .map_err(|e| StoreError::Internal(format!("invalid project_id: {e}")))?;
        Ok(Issue {
            issue_type,
            title: row.title.clone(),
            description: row.description.clone(),
            creator: Username::from(row.creator.clone()),
            progress: row.progress.clone(),
            status,
            project_id,
            assignee,
            session_settings,
            dependencies: vec![],
            patches: vec![],
            deleted: row.deleted,
            form,
            form_response,
            feedback: row.feedback.clone(),
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
             WHERE source_id = $1 AND source_kind = 'issue'"
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
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
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
            "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&patch.title)
            .bind(&patch.description)
            .bind(&patch.diff)
            .bind(patch.status.as_str())
            .bind(patch.is_automatic_backup)
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

        let commit_range: Option<CommitRange> = row
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::from_value(cr.clone()).map_err(|e| {
                    StoreError::Internal(format!("failed to deserialize commit_range: {e}"))
                })
            })
            .transpose()?;

        let creator = Username::from(row.creator.as_str());

        Ok(Patch {
            title: row.title.clone(),
            description: row.description.clone(),
            diff: row.diff.clone(),
            status,
            is_automatic_backup: row.is_automatic_backup,
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
    // Session helpers
    // -------------------------------------------------------------------------

    async fn insert_session(
        &self,
        id: &SessionId,
        version_number: VersionNumber,
        session: &Session,
        actor: Option<&Value>,
    ) -> Result<(), StoreError> {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for task '{id}'"))
        })?;

        let legacy_conversation_id = session.conversation_id().cloned();

        let env_vars_json = serde_json::to_value(&session.env_vars)
            .map_err(|e| StoreError::Internal(format!("failed to serialize env_vars: {e}")))?;
        let error_json = session
            .error
            .as_ref()
            .map(|e| {
                serde_json::to_value(e).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize error: {err}"))
                })
            })
            .transpose()?;

        let secrets_json = session
            .secrets
            .as_ref()
            .map(|s| {
                serde_json::to_value(s).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize secrets: {err}"))
                })
            })
            .transpose()?;

        let status_str = match session.status {
            Status::Created => "created",
            Status::Pending => "pending",
            Status::Running => "running",
            Status::Complete => "complete",
            Status::Failed => "failed",
        };

        let usage_json = session
            .usage
            .as_ref()
            .map(|u| {
                serde_json::to_value(u).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize usage: {err}"))
                })
            })
            .transpose()?;

        let mount_spec_json = crate::store::dual_write_mount_spec_json(session)?;
        let agent_config_json = crate::store::dual_write_agent_config_json(session)?;
        let mode_json = crate::store::dual_write_mode_json(session)?;
        let resumed_from_str = session
            .resumed_from
            .as_ref()
            .map(|s| s.as_ref().to_string());

        let proxy_targets_json = if session.proxy_targets.is_empty() {
            None
        } else {
            Some(serde_json::to_value(&session.proxy_targets).map_err(|e| {
                StoreError::Internal(format!("failed to serialize proxy_targets: {e}"))
            })?)
        };

        let query = format!(
            "INSERT INTO {TABLE_TASKS_V2} (id, version_number, spawned_from, creator, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, proxy_targets)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(session.spawned_from.as_ref().map(|i| i.as_ref()))
            .bind(session.creator.as_str())
            .bind(session.image.as_deref())
            .bind(&env_vars_json)
            .bind(session.cpu_limit.as_deref())
            .bind(session.memory_limit.as_deref())
            .bind(status_str)
            .bind(session.last_message.as_deref())
            .bind(&error_json)
            .bind(session.deleted)
            .bind(actor)
            .bind(&secrets_json)
            .bind(session.creation_time)
            .bind(session.start_time)
            .bind(session.end_time)
            .bind(legacy_conversation_id.as_ref().map(|c| c.as_ref()))
            .bind(usage_json.as_ref())
            .bind(&mount_spec_json)
            .bind(&agent_config_json)
            .bind(&mode_json)
            .bind(resumed_from_str.as_deref())
            .bind(proxy_targets_json.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_session(&self, row: &TaskRow) -> Result<Session, StoreError> {
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

        let usage = row
            .usage
            .as_ref()
            .map(|u| {
                serde_json::from_value(u.clone()).map_err(|err| {
                    StoreError::Internal(format!("failed to deserialize usage: {err}"))
                })
            })
            .transpose()?;

        // `mount_spec`, `agent_config`, and `mode` are NOT NULL in every
        // row and are the canonical source for session shape on this
        // read path.
        let mount_spec = serde_json::from_value(row.mount_spec.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize mount_spec: {e}")))?;
        let agent_config = serde_json::from_value(row.agent_config.clone()).map_err(|e| {
            StoreError::Internal(format!("failed to deserialize agent_config: {e}"))
        })?;
        let mode = serde_json::from_value(row.mode.clone())
            .map_err(|e| StoreError::Internal(format!("failed to deserialize mode: {e}")))?;
        let resumed_from = row
            .resumed_from
            .as_deref()
            .map(|s| {
                s.parse::<SessionId>()
                    .map_err(|e| StoreError::Internal(format!("invalid resumed_from: {e}")))
            })
            .transpose()?;

        let proxy_targets = row
            .proxy_targets
            .as_ref()
            .map(|v| {
                serde_json::from_value(v.clone()).map_err(|e| {
                    StoreError::Internal(format!("failed to deserialize proxy_targets: {e}"))
                })
            })
            .transpose()?
            .unwrap_or_default();

        Ok(Session {
            creator: Username::from(row.creator.as_str()),
            spawned_from,
            resumed_from,
            agent_config,
            mount_spec,
            image: row.image.clone(),
            env_vars,
            cpu_limit: row.cpu_limit.clone(),
            memory_limit: row.memory_limit.clone(),
            secrets,
            mode,
            status,
            last_message: row.last_message.clone(),
            error,
            deleted: row.deleted,
            creation_time: row.creation_time,
            start_time: row.start_time,
            end_time: row.end_time,
            usage,
            proxy_targets,
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
            "INSERT INTO {TABLE_DOCUMENTS_V2} (id, version_number, title, body_markdown, path, deleted, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&document.title)
            .bind(&document.body_markdown)
            .bind(document.path.as_ref().map(|p| p.as_str()))
            .bind(document.deleted)
            .bind(actor)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_document(&self, row: &DocumentRow) -> Result<Document, StoreError> {
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

        let merge_policy_json = repo
            .merge_policy
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize merge_policy: {e}")))?;

        let query = format!(
            "INSERT INTO {TABLE_REPOSITORIES_V2} (id, version_number, remote_url, default_branch, default_image, deleted, merge_policy, actor)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        );
        sqlx::query(&query)
            .bind(id)
            .bind(version_number)
            .bind(&repo.remote_url)
            .bind(repo.default_branch.as_deref())
            .bind(repo.default_image.as_deref())
            .bind(repo.deleted)
            .bind(&merge_policy_json)
            .bind(actor)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_repository(&self, row: &RepositoryRow) -> Result<Repository, StoreError> {
        let merge_policy = row
            .merge_policy
            .as_ref()
            .map(|v| {
                serde_json::from_value(v.clone()).map_err(|e| {
                    StoreError::Internal(format!("failed to deserialize merge_policy: {e}"))
                })
            })
            .transpose()?;

        let mut repo = Repository::new(
            row.remote_url.clone(),
            row.default_branch.clone(),
            row.default_image.clone(),
        );
        repo.deleted = row.deleted;
        repo.merge_policy = merge_policy;
        Ok(repo)
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
        let mut sql = format!(
            "SELECT id, version_number, username, github_user_id, deleted, actor, created_at, updated_at
             FROM {TABLE_USERS_V2} WHERE is_latest = true"
        );
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
            sql.push_str(" AND ");
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
    // Conversation helpers
    // -------------------------------------------------------------------------

    async fn insert_trigger_in_tx<'e, E>(
        executor: E,
        id: &TriggerId,
        version_number: VersionNumber,
        trigger: &Trigger,
        actor: Option<&Value>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for trigger '{id}'"))
        })?;

        let schedule_json = serde_json::to_value(&trigger.schedule).map_err(|e| {
            StoreError::Internal(format!("failed to serialize trigger schedule: {e}"))
        })?;
        let actions_json = serde_json::to_value(&trigger.actions).map_err(|e| {
            StoreError::Internal(format!("failed to serialize trigger actions: {e}"))
        })?;

        let query = format!(
            "INSERT INTO {TABLE_TRIGGERS} \
             (id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(trigger.enabled)
            .bind(trigger.creator.as_str())
            .bind(&schedule_json)
            .bind(&actions_json)
            .bind(trigger.last_fired_at)
            .bind(trigger.deleted)
            .bind(actor)
            .execute(executor)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_trigger(row: &TriggerRow) -> Result<Trigger, StoreError> {
        let schedule: hydra_common::triggers::Schedule =
            serde_json::from_value(row.schedule.clone()).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize trigger schedule: {e}"))
            })?;
        let actions: Vec<hydra_common::triggers::Action> =
            serde_json::from_value(row.actions.clone()).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize trigger actions: {e}"))
            })?;
        Ok(Trigger::new(
            row.enabled,
            schedule,
            actions,
            hydra_common::api::v1::users::Username::from(row.creator.clone()),
            row.last_fired_at,
            row.deleted,
        ))
    }

    async fn insert_project_row_in_tx<'e, E>(
        executor: E,
        id: &ProjectId,
        version_number: i64,
        project: &Project,
        actor: Option<&Value>,
        next_status_sequence: i64,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let query = format!(
            "INSERT INTO {TABLE_PROJECTS} \
             (id, version_number, key, name, creator, deleted, actor, prompt_path, priority, next_status_sequence) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(project.key.as_str())
            .bind(&project.name)
            .bind(project.creator.as_str())
            .bind(project.deleted)
            .bind(actor)
            .bind(project.prompt_path.as_deref())
            .bind(project.priority)
            .bind(next_status_sequence)
            .execute(executor)
            .await
            .map_err(|err| {
                if is_project_key_unique_violation_pg(&err) {
                    StoreError::ProjectKeyExists(project.key.clone())
                } else {
                    map_sqlx_error(err)
                }
            })?;

        Ok(())
    }

    fn row_to_project(
        row: &ProjectRow,
        statuses: Vec<StatusDefinition>,
    ) -> Result<Project, StoreError> {
        let key = ProjectKey::try_new(row.key.clone()).map_err(|e| {
            StoreError::Internal(format!("invalid project key stored for project: {e}"))
        })?;
        let mut project = Project::new(
            key,
            row.name.clone(),
            statuses,
            hydra_common::api::v1::users::Username::from(row.creator.clone()),
            row.deleted,
            row.priority,
        );
        project.prompt_path = row.prompt_path.clone();
        Ok(project)
    }

    fn status_row_to_definition(row: &StatusRow) -> Result<StatusDefinition, StoreError> {
        let key = StatusKey::try_new(row.key.clone())
            .map_err(|e| StoreError::Internal(format!("invalid status key in database: {e}")))?;
        let color = row
            .color
            .parse()
            .map_err(|e| StoreError::Internal(format!("invalid status color in database: {e}")))?;
        let on_enter = row
            .on_enter
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize status on_enter: {e}"))
            })?;
        let mut def = StatusDefinition::new(
            key,
            row.label.clone(),
            color,
            row.unblocks_parents,
            row.unblocks_dependents,
            row.cascades_to_children,
            on_enter,
        );
        def.prompt_path = row.prompt_path.clone();
        def.interactive = row.interactive;
        def.position = row.position;
        Ok(def)
    }

    async fn fetch_statuses_for_project<'e, E>(
        executor: E,
        project_id: &str,
    ) -> Result<Vec<StatusDefinition>, StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let rows = sqlx::query_as::<_, StatusRow>(
            "SELECT project_id, sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive, position \
             FROM metis.statuses WHERE project_id = $1 ORDER BY position, sequence",
        )
        .bind(project_id)
        .fetch_all(executor)
        .await
        .map_err(map_sqlx_error)?;
        rows.iter().map(Self::status_row_to_definition).collect()
    }

    async fn fetch_statuses_for_projects(
        pool: &sqlx::PgPool,
        project_ids: &[String],
    ) -> Result<HashMap<String, Vec<StatusDefinition>>, StoreError> {
        let mut out: HashMap<String, Vec<StatusDefinition>> = HashMap::new();
        if project_ids.is_empty() {
            return Ok(out);
        }
        for id in project_ids {
            out.entry(id.clone()).or_default();
        }
        let rows = sqlx::query_as::<_, StatusRow>(
            "SELECT project_id, sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive, position \
             FROM metis.statuses WHERE project_id = ANY($1) ORDER BY project_id, position, sequence",
        )
        .bind(project_ids)
        .fetch_all(pool)
        .await
        .map_err(map_sqlx_error)?;
        for row in &rows {
            let def = Self::status_row_to_definition(row)?;
            out.entry(row.project_id.clone()).or_default().push(def);
        }
        Ok(out)
    }

    /// Insert a single `metis.statuses` row for `add_status`. Pulled
    /// out of the trait method so the caller can sequence it with the
    /// preflight existence check + the project version bump.
    async fn insert_status_row_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        project_id: &str,
        sequence: i64,
        status: &StatusDefinition,
    ) -> Result<(), StoreError> {
        let color_str = status.color.as_ref().to_string();
        let on_enter_json = status
            .on_enter
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize status on_enter: {e}"))
            })?;
        sqlx::query(
            "INSERT INTO metis.statuses (project_id, sequence, key, label, color, unblocks_parents, unblocks_dependents, cascades_to_children, on_enter, prompt_path, interactive, position) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(project_id)
        .bind(sequence)
        .bind(status.key.as_str())
        .bind(&status.label)
        .bind(&color_str)
        .bind(status.unblocks_parents)
        .bind(status.unblocks_dependents)
        .bind(status.cascades_to_children)
        .bind(&on_enter_json)
        .bind(status.prompt_path.as_deref())
        .bind(status.interactive)
        .bind(status.position)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Load the latest `metis.projects` row inside a status-mutation
    /// transaction, holding a row-level lock for the duration so a
    /// concurrent status mutation observes a consistent
    /// `(version_number, next_status_sequence)` snapshot.
    async fn load_project_for_status_mutation_pg(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &ProjectId,
    ) -> Result<ProjectRow, StoreError> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT id, version_number, key, name, creator, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = $1) AS creation_time, \
             prompt_path, priority, next_status_sequence \
             FROM {TABLE_PROJECTS} \
             WHERE id = $1 AND is_latest = true \
             FOR UPDATE"
        ))
        .bind(id.as_ref())
        .fetch_optional(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
        row.ok_or_else(|| StoreError::ProjectNotFound(id.clone()))
    }

    /// Flip the prior `is_latest` row off and insert a new versioned
    /// `metis.projects` row carrying the same project-level fields.
    /// Used by the per-status mutation paths to bump the project
    /// version after a status add / update / delete.
    async fn bump_project_version_for_status_mutation_pg(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: &ProjectId,
        row: &ProjectRow,
        latest_version: VersionNumber,
        actor: &ActorRef,
        next_status_sequence: i64,
    ) -> Result<VersionNumber, StoreError> {
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;
        let next_version_i64 = i64::try_from(next_version).map_err(|_| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        sqlx::query(&format!(
            "INSERT INTO {TABLE_PROJECTS} \
             (id, version_number, key, name, creator, deleted, actor, prompt_path, priority, next_status_sequence) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        ))
        .bind(id.as_ref())
        .bind(next_version_i64)
        .bind(&row.key)
        .bind(&row.name)
        .bind(&row.creator)
        .bind(row.deleted)
        .bind(&actor_json)
        .bind(row.prompt_path.as_deref())
        .bind(row.priority)
        .bind(next_status_sequence)
        .execute(&mut **tx)
        .await
        .map_err(map_sqlx_error)?;
        Ok(next_version)
    }

    /// Resolve a `(project_id, status_key)` pair to its
    /// `metis.statuses.sequence` integer. Errors with
    /// `InvalidIssueStatus` if no matching status row exists.
    async fn resolve_status_sequence<'e, E>(
        executor: E,
        project_id: &str,
        status_key: &str,
    ) -> Result<i64, StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let value: Option<i64> = sqlx::query_scalar(
            "SELECT sequence FROM metis.statuses WHERE project_id = $1 AND key = $2 LIMIT 1",
        )
        .bind(project_id)
        .bind(status_key)
        .fetch_optional(executor)
        .await
        .map_err(map_sqlx_error)?;
        value.ok_or_else(|| {
            StoreError::InvalidIssueStatus(format!(
                "status '{status_key}' does not exist on project '{project_id}'"
            ))
        })
    }

    async fn insert_conversation_in_tx<'e, E>(
        executor: E,
        id: &ConversationId,
        version_number: VersionNumber,
        conversation: &Conversation,
        actor: Option<&Value>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for conversation '{id}'"))
        })?;

        let status_str = match conversation.status {
            ConversationStatus::Active => "active",
            ConversationStatus::Idle => "idle",
            ConversationStatus::Closed => "closed",
        };

        let session_settings_json =
            serde_json::to_value(&conversation.session_settings).map_err(|e| {
                StoreError::Internal(format!(
                    "failed to serialize conversation session_settings: {e}"
                ))
            })?;

        let query = format!(
            "INSERT INTO {TABLE_CONVERSATIONS_V2} \
             (id, version_number, title, agent_name, session_settings, spawned_from, status, creator, deleted, actor) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(version_number)
            .bind(&conversation.title)
            .bind(conversation.agent_name.as_ref().map(|n| n.as_str()))
            .bind(&session_settings_json)
            .bind(
                conversation
                    .spawned_from
                    .as_ref()
                    .map(|i| i.as_ref().to_string()),
            )
            .bind(status_str)
            .bind(conversation.creator.as_str())
            .bind(conversation.deleted)
            .bind(actor)
            .execute(executor)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_conversation(row: &ConversationRow) -> Result<Conversation, StoreError> {
        use hydra_common::api::v1::agents::AgentName;
        let status = match row.status.as_str() {
            "active" => ConversationStatus::Active,
            "idle" => ConversationStatus::Idle,
            "closed" => ConversationStatus::Closed,
            other => {
                return Err(StoreError::Internal(format!(
                    "invalid conversation status in database: {other}"
                )));
            }
        };
        let session_settings: crate::domain::issues::SessionSettings =
            serde_json::from_value(row.session_settings.clone()).map_err(|e| {
                StoreError::Internal(format!(
                    "failed to deserialize conversation session_settings: {e}"
                ))
            })?;
        // Re-validate the persisted `agent_name` on read. SQLite +
        // Postgres columns stay `TEXT`; the type-tightening happens at
        // the Rust boundary so malformed legacy values surface as an
        // internal error rather than silently passing.
        let agent_name = row
            .agent_name
            .as_ref()
            .map(|s| AgentName::try_new(s.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("invalid agent_name in conversation row: {e}"))
            })?;

        let spawned_from = row
            .spawned_from
            .as_deref()
            .map(|s| s.parse::<hydra_common::IssueId>())
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("invalid spawned_from in conversation row: {e}"))
            })?;

        Ok(Conversation {
            title: row.title.clone(),
            agent_name,
            status,
            creator: Username::from(row.creator.as_str()),
            session_settings,
            spawned_from,
            deleted: row.deleted,
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
    created_at: DateTime<Utc>,
}

fn parse_relationship_row(
    r: ObjectRelationshipRow,
) -> Result<crate::store::ObjectRelationship, StoreError> {
    let source_id: HydraId = r.source_id.parse().map_err(|_| {
        StoreError::Internal("invalid source_id in object_relationships".to_string())
    })?;
    let target_id: HydraId = r.target_id.parse().map_err(|_| {
        StoreError::Internal("invalid target_id in object_relationships".to_string())
    })?;
    let source_kind = crate::store::ObjectKind::from_str(&r.source_kind).map_err(|e| {
        StoreError::Internal(format!("invalid source_kind in object_relationships: {e}"))
    })?;
    let target_kind = crate::store::ObjectKind::from_str(&r.target_kind).map_err(|e| {
        StoreError::Internal(format!("invalid target_kind in object_relationships: {e}"))
    })?;
    let rel_type = crate::store::RelationshipType::from_str(&r.rel_type).map_err(|e| {
        StoreError::Internal(format!("invalid rel_type in object_relationships: {e}"))
    })?;
    Ok(crate::store::ObjectRelationship {
        source_id,
        source_kind,
        target_id,
        target_kind,
        rel_type,
        created_at: r.created_at,
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
    /// Legacy `assignee TEXT` column. `assignee_principal` is the source
    /// of truth; this field is still selected so the dual-written column
    /// round-trips through `sqlx::FromRow`, but is no longer consumed
    /// at the Rust layer.
    #[allow(dead_code)]
    assignee: Option<String>,
    #[sqlx(default)]
    assignee_principal: Option<Value>,
    job_settings: Value,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
    #[sqlx(default)]
    form: Option<Value>,
    #[sqlx(default)]
    form_response: Option<Value>,
    #[sqlx(default)]
    feedback: Option<String>,
    project_id: String,
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
    reviews: Value,
    service_repo_name: String,
    github: Option<Value>,
    deleted: bool,
    branch_name: Option<String>,
    commit_range: Option<Value>,
    creator: String,
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
    spawned_from: Option<String>,
    image: Option<String>,
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
    creator: String,
    secrets: Option<Value>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
    #[sqlx(default)]
    start_time: Option<DateTime<Utc>>,
    #[sqlx(default)]
    end_time: Option<DateTime<Utc>>,
    // Denormalized from `mode.Interactive.conversation_id` at insert time
    // and never edited independently. Retained as a single-query lookup
    // index for `list_session_ids_by_conversation_id`; SELECTed to keep
    // the row shape consistent with the table even though the read path
    // reads `mode` JSON.
    #[allow(dead_code)]
    #[sqlx(default)]
    conversation_id: Option<String>,
    #[sqlx(default)]
    usage: Option<Value>,
    // `mount_spec` / `agent_config` / `mode` are the canonical source
    // for session shape, NOT NULL in every row and populated from the
    // domain object's typed fields on INSERT.
    mount_spec: Value,
    agent_config: Value,
    mode: Value,
    #[sqlx(default)]
    resumed_from: Option<String>,
    #[sqlx(default)]
    proxy_targets: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    version_number: i64,
    title: String,
    body_markdown: String,
    path: Option<String>,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    id: String,
    version_number: i64,
    title: Option<String>,
    agent_name: Option<String>,
    session_settings: Value,
    spawned_from: Option<String>,
    status: String,
    creator: String,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct TriggerRow {
    id: String,
    version_number: i64,
    enabled: bool,
    creator: String,
    schedule: Value,
    actions: Value,
    last_fired_at: Option<DateTime<Utc>>,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    version_number: i64,
    key: String,
    name: String,
    creator: String,
    deleted: bool,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    updated_at: DateTime<Utc>,
    #[sqlx(default)]
    creation_time: Option<DateTime<Utc>>,
    #[sqlx(default)]
    prompt_path: Option<String>,
    // No `#[sqlx(default)]`: forces every SELECT site that produces a
    // `ProjectRow` to project `p.priority`. A missing column should fail
    // loud at runtime instead of silently surfacing `0.0` in place of the
    // backfilled value.
    priority: f64,
    // Per-project high-water mark for `metis.statuses.sequence`
    // assignment. Monotonically non-decreasing across status add/remove
    // cycles to forbid sequence id reuse. Read here only for
    // `get_project`/`list_projects` sanity.
    #[allow(dead_code)]
    next_status_sequence: i64,
}

#[derive(sqlx::FromRow)]
struct StatusRow {
    project_id: String,
    #[allow(dead_code)]
    sequence: i64,
    key: String,
    label: String,
    color: String,
    unblocks_parents: bool,
    unblocks_dependents: bool,
    cascades_to_children: bool,
    on_enter: Option<Value>,
    prompt_path: Option<String>,
    interactive: bool,
    // No `#[sqlx(default)]`: forces every SELECT site on
    // `metis.statuses` to project `position`. A missing column should
    // fail loud at runtime instead of silently surfacing `0.0` in
    // place of the backfilled value.
    position: f64,
}

#[derive(sqlx::FromRow)]
struct ConversationEventCountRow {
    conversation_id: String,
    event_count: i64,
}

#[derive(sqlx::FromRow)]
struct ConversationPreviewRow {
    conversation_id: String,
    event_data: Value,
}

#[derive(sqlx::FromRow)]
struct SessionEventRow {
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    session_id: String,
    version_number: i64,
    event_data: Value,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SessionEventSummaryRow {
    session_id: String,
    event_count: i64,
    last_event_data: Option<Value>,
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: String,
    version_number: i64,
    remote_url: String,
    default_branch: Option<String>,
    default_image: Option<String>,
    deleted: bool,
    merge_policy: Option<Value>,
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
    mcp_config_path: Option<String>,
    max_tries: i32,
    max_simultaneous: i32,
    is_assignment_agent: bool,
    #[sqlx(default)]
    is_default_conversation_agent: bool,
    secrets: serde_json::Value,
    deleted: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Build WHERE predicates and bindings for issues queries (PostgreSQL `$N` placeholders).
/// Build WHERE predicates and bindings for issues queries. Issue
/// columns are qualified with `i.`; the joined `statuses` row is
/// qualified with `s.`. Callers must ensure the query has
/// `FROM metis.issues_v2 i INNER JOIN metis.statuses s ON …` in
/// scope.
fn build_issues_predicates_pg(query: &SearchIssuesQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if !query.ids.is_empty() {
        let placeholders: Vec<String> = query
            .ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("i.id IN ({})", placeholders.join(", ")));
        for id in &query.ids {
            bindings.push(id.as_ref().to_string());
        }
    }

    if let Some(issue_type) = query.issue_type.as_ref() {
        predicates.push(format!("i.issue_type = ${}", bindings.len() + 1));
        bindings.push(issue_type.as_str().to_string());
    }

    if !query.status.is_empty() {
        let placeholders: Vec<String> = query
            .status
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("s.key IN ({})", placeholders.join(", ")));
        for s in &query.status {
            bindings.push(s.as_str().to_string());
        }
    }

    if let Some(project_id) = query.project_id.as_ref() {
        predicates.push(format!("i.project_id = ${}", bindings.len() + 1));
        bindings.push(project_id.as_ref().to_string());
    }

    if let Some(assignee) = query.assignee.as_ref() {
        // Filter against the typed `assignee_principal` JSONB column
        // using canonical serialization. The TEXT-column LIKE search
        // continues to participate in the `q` free-text predicate below.
        let serialized = serde_json::to_string(assignee).unwrap_or_default();
        predicates.push(format!(
            "i.assignee_principal = ${}::jsonb",
            bindings.len() + 1
        ));
        bindings.push(serialized);
    }

    if let Some(creator) = query
        .creator
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        predicates.push(format!("LOWER(i.creator) = ${}", bindings.len() + 1));
        bindings.push(creator.to_lowercase());
    }

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
            "(LOWER(i.id) LIKE ${idx_id} \
             OR LOWER(i.title) LIKE ${idx_title} \
             OR LOWER(i.description) LIKE ${idx_desc} \
             OR LOWER(i.progress) LIKE ${idx_progress} \
             OR i.issue_type = ${idx_type} \
             OR s.key = ${idx_status} \
             OR LOWER(i.creator) LIKE ${idx_creator} \
             OR LOWER(COALESCE(i.assignee,'')) LIKE ${idx_assignee})"
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

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("i.deleted = false".to_string());
    }

    if !query.label_ids.is_empty() {
        let label_count = query.label_ids.len();
        let placeholders: Vec<String> = query
            .label_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!(
            "i.id IN (SELECT la.object_id FROM {TABLE_LABEL_ASSOCIATIONS} la \
             WHERE la.label_id IN ({}) \
             GROUP BY la.object_id \
             HAVING COUNT(DISTINCT la.label_id) = {label_count})",
            placeholders.join(", ")
        ));
        for label_id in &query.label_ids {
            bindings.push(label_id.to_string());
        }
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for patches queries (PostgreSQL `$N` placeholders).
fn build_patches_predicates_pg(query: &SearchPatchesQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings = Vec::new();

    if !query.ids.is_empty() {
        let placeholders: Vec<String> = query
            .ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("id IN ({})", placeholders.join(", ")));
        for id in &query.ids {
            bindings.push(id.as_ref().to_string());
        }
    }

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

    if let Some(branch) = query
        .branch_name
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        let idx = bindings.len() + 1;
        predicates.push(format!("branch_name = ${idx}"));
        bindings.push(branch.to_string());
    }

    if let Some(repo_name) = query
        .repo_name
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        let idx = bindings.len() + 1;
        predicates.push(format!("service_repo_name = ${idx}"));
        bindings.push(repo_name.to_string());
    }

    if let Some(creator) = query
        .creator
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        let idx = bindings.len() + 1;
        predicates.push(format!("LOWER(creator) = ${idx}"));
        bindings.push(creator.to_lowercase());
    }

    if let Some(term) = query
        .q
        .as_ref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
    {
        let idx_start = bindings.len() + 1;
        predicates.push(format!(
            "(LOWER(id) LIKE ${idx_id} \
             OR LOWER(title) LIKE ${idx_title} \
             OR LOWER(description) LIKE ${idx_desc} \
             OR LOWER(status) LIKE ${idx_status} \
             OR LOWER(service_repo_name) LIKE ${idx_repo} \
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
            idx_branch = idx_start + 5,
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

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for documents queries (PostgreSQL `$N` placeholders).
fn build_documents_predicates_pg(query: &SearchDocumentsQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings = Vec::new();

    if !query.ids.is_empty() {
        let placeholders: Vec<String> = query
            .ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("id IN ({})", placeholders.join(", ")));
        for id in &query.ids {
            bindings.push(id.as_ref().to_string());
        }
    }

    if let Some(path) = query.path_prefix.as_ref() {
        if query.path_is_exact.unwrap_or(false) {
            predicates.push(format!("COALESCE(path,'') = ${}", bindings.len() + 1));
            bindings.push(path.clone());
        } else {
            predicates.push(format!("COALESCE(path,'') LIKE ${}", bindings.len() + 1));
            bindings.push(format!("{path}%"));
        }
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

    if let Some(has_path) = query.has_path {
        if has_path {
            predicates.push("path IS NOT NULL".to_string());
        } else {
            predicates.push("path IS NULL".to_string());
        }
    }

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = false".to_string());
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for tasks queries (PostgreSQL `$N` placeholders).
fn build_tasks_predicates_pg(query: &SearchSessionsQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(spawned_from) = query.spawned_from.as_ref() {
        predicates.push(format!("spawned_from = ${}", bindings.len() + 1));
        bindings.push(spawned_from.as_ref().to_string());
    }

    if !query.spawned_from_ids.is_empty() {
        let placeholders: Vec<String> = query
            .spawned_from_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("${}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("spawned_from IN ({})", placeholders.join(", ")));
        for id in &query.spawned_from_ids {
            bindings.push(id.as_ref().to_string());
        }
    }

    if let Some(creator) = query.creator.as_deref() {
        predicates.push(format!("creator = ${}", bindings.len() + 1));
        bindings.push(creator.to_string());
    }

    if let Some(conversation_id) = query.conversation_id.as_ref() {
        predicates.push(format!("conversation_id = ${}", bindings.len() + 1));
        bindings.push(conversation_id.as_ref().to_string());
    }

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
             OR LOWER(COALESCE(agent_config->>'system_prompt', '')) LIKE ${idx_prompt} \
             OR LOWER(status) LIKE ${idx_status})"
        ));
        let pattern = format!("%{term}%");
        bindings.push(pattern.clone());
        bindings.push(pattern.clone());
        bindings.push(pattern);
    }

    if !query.status.is_empty() {
        let status_strings: Vec<String> = query
            .status
            .iter()
            .filter_map(|s| {
                let server_status: Status = (*s).try_into().ok()?;
                Some(status_to_db_str(server_status).to_string())
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

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = false".to_string());
    }

    (predicates, bindings)
}

/// Build WHERE predicates and bindings for labels queries (PostgreSQL `$N` placeholders).
fn build_labels_predicates_pg(query: &SearchLabelsQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if !query.include_deleted.unwrap_or(false) {
        predicates.push("deleted = false".to_string());
    }

    if let Some(ref q) = query.q {
        predicates.push(format!("LOWER(name) LIKE ${}", bindings.len() + 1));
        bindings.push(format!("%{}%", q.to_lowercase()));
    }

    (predicates, bindings)
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db_err) = err {
        if db_err.code().as_deref() == Some("23505") {
            if db_err.constraint() == Some("labels_name_idx") {
                let name = db_err
                    .message()
                    .split("=(")
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .unwrap_or("unknown")
                    .to_string();
                return StoreError::LabelAlreadyExists(name);
            }
            if db_err.constraint() == Some("documents_v2_path_unique_active_idx") {
                return StoreError::DocumentPathConflict;
            }
        }
    }
    StoreError::Internal(err.to_string())
}

/// True iff `err` is a Postgres unique-violation on the partial
/// `projects_key_unique_active_idx` index. Used by `add_project` /
/// `update_project` to translate the raw sqlx error into a
/// [`StoreError::ProjectKeyExists`] that carries the colliding key.
fn is_project_key_unique_violation_pg(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        if db_err.code().as_deref() == Some("23505") {
            return db_err.constraint() == Some("projects_key_unique_active_idx");
        }
    }
    false
}

/// True iff `err` is a Postgres FK-violation on
/// `issues_v2_status_sequence_fkey` — the RESTRICT that blocks deleting
/// a `metis.statuses` row while an `issues_v2` row still references it.
/// Used by `apply_statuses_diff_in_tx` to translate the raw sqlx error
/// into [`StoreError::InvalidIssueStatus`] so the route layer can
/// surface a 400 instead of an opaque 500.
fn is_status_sequence_fk_violation_pg(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        if db_err.code().as_deref() == Some("23503") {
            return db_err.constraint() == Some("issues_v2_status_sequence_fkey");
        }
    }
    false
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
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, merge_policy, actor, created_at, updated_at
             FROM {TABLE_REPOSITORIES_V2}
             WHERE id = $1
             ORDER BY is_latest DESC, version_number DESC
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
        let normalized_needle = query
            .remote_url
            .as_deref()
            .map(Repository::normalize_remote_url);
        let sql = format!(
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, merge_policy, actor, created_at, updated_at
             FROM {TABLE_REPOSITORIES_V2}
             WHERE is_latest = true
             ORDER BY id"
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
            if let Some(needle) = normalized_needle.as_deref()
                && Repository::normalize_remote_url(&row.remote_url) != needle
            {
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
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, i.progress, s.key AS status, i.assignee, i.assignee_principal, i.job_settings, i.deleted, i.actor, i.created_at, i.updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = $1) AS creation_time, \
             i.form, i.form_response, i.feedback, i.project_id
             FROM {TABLE_ISSUES_V2} i
             INNER JOIN metis.statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence
             WHERE i.id = $1
             ORDER BY i.is_latest DESC, i.version_number DESC
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
        let mut issue = self.row_to_issue(&row)?;

        if !include_deleted && issue.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        self.populate_issue_relationships(id, &mut issue).await?;

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
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, i.progress, s.key AS status, i.assignee, i.assignee_principal, i.job_settings, i.deleted, i.actor, i.created_at, i.updated_at, \
             i.form, i.form_response, i.feedback, i.project_id
             FROM {TABLE_ISSUES_V2} i
             INNER JOIN metis.statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence
             WHERE i.id = $1
             ORDER BY i.version_number"
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

        // Populate relationships from object_relationships table.
        // Project current relationships onto all historical versions.
        if let Some(first) = results.first_mut() {
            self.populate_issue_relationships(id, &mut first.item)
                .await?;
            let dependencies = first.item.dependencies.clone();
            let patches = first.item.patches.clone();
            for r in results.iter_mut().skip(1) {
                r.item.dependencies = dependencies.clone();
                r.item.patches = patches.clone();
            }
        }

        Ok(results)
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        // Filter to the latest version of each issue using the is_latest
        // column maintained by a BEFORE INSERT trigger, avoiding correlated
        // subqueries or DISTINCT ON.
        let mut sql = format!(
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, \
             i.progress, s.key AS status, i.assignee, i.assignee_principal, i.job_settings, i.deleted, i.actor, \
             i.created_at, i.updated_at, \
             (SELECT MIN(i2.created_at) FROM {TABLE_ISSUES_V2} i2 WHERE i2.id = i.id) AS creation_time, \
             i.form, i.form_response, i.feedback, i.project_id \
             FROM {TABLE_ISSUES_V2} i \
             INNER JOIN metis.statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence"
        );
        let (mut predicates, mut bindings) = build_issues_predicates_pg(query);
        predicates.push("i.is_latest = true".to_string());

        apply_pagination_sql_pg(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "i.updated_at",
            "i.id",
        )?;

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

        self.populate_issues_relationships(&mut issues).await?;

        Ok(issues)
    }

    async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        let mut sql = format!(
            "SELECT COUNT(*) FROM {TABLE_ISSUES_V2} i \
             INNER JOIN metis.statuses s ON s.project_id = i.project_id AND s.sequence = i.status_sequence"
        );
        let (mut predicates, bindings) = build_issues_predicates_pg(query);
        predicates.push("i.is_latest = true".to_string());

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let sql = format!(
            "SELECT source_id FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE target_id = $1 AND rel_type = $2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(issue_id.as_ref())
            .bind(crate::store::RelationshipType::ChildOf.as_str())
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
             WHERE target_id = $1 AND rel_type = $2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(issue_id.as_ref())
            .bind(crate::store::RelationshipType::BlockedOn.as_str())
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
        // Use spawned_from filter at the database level for efficiency
        let query = SearchSessionsQuery::new(None, Some(issue_id.clone()), None, vec![]);
        let tasks = self.list_sessions(&query).await?;
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
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PATCHES_V2} WHERE id = $1) AS creation_time
             FROM {TABLE_PATCHES_V2}
             WHERE id = $1
             ORDER BY is_latest DESC, version_number DESC
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
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, reviews, service_repo_name, github, deleted, branch_name, commit_range, creator, base_branch, actor, created_at, updated_at
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
        let mut sql = format!(
            "SELECT p.id, p.version_number, p.title, p.description, '' AS diff, p.status, p.is_automatic_backup, p.reviews, p.service_repo_name, p.github, p.deleted, p.branch_name, p.commit_range, p.creator, p.base_branch, p.actor, p.created_at, p.updated_at, \
             (SELECT MIN(p2.created_at) FROM {TABLE_PATCHES_V2} p2 WHERE p2.id = p.id) AS creation_time \
             FROM {TABLE_PATCHES_V2} p"
        );
        let (mut predicates, mut bindings) = build_patches_predicates_pg(query);
        predicates.push("p.is_latest = true".to_string());

        apply_pagination_sql_pg(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

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

    async fn count_patches(&self, query: &SearchPatchesQuery) -> Result<u64, StoreError> {
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_PATCHES_V2} p");
        let (mut predicates, bindings) = build_patches_predicates_pg(query);
        predicates.push("p.is_latest = true".to_string());

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in bindings {
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
             WHERE target_id = $1 AND rel_type = $2"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(patch_id.as_ref())
            .bind(crate::store::RelationshipType::HasPatch.as_str())
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

    // -------------------------------------------------------------------------
    // Document methods
    // -------------------------------------------------------------------------

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, body_markdown, path, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = $1) AS creation_time
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = $1
             ORDER BY is_latest DESC, version_number DESC
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
            "SELECT id, version_number, title, body_markdown, path, deleted, actor, created_at, updated_at
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
        let mut sql = format!(
            "SELECT d.id, d.version_number, d.title, d.body_markdown, d.path, d.deleted, d.actor, d.created_at, d.updated_at, \
             (SELECT MIN(d2.created_at) FROM {TABLE_DOCUMENTS_V2} d2 WHERE d2.id = d.id) AS creation_time \
             FROM {TABLE_DOCUMENTS_V2} d"
        );
        let (mut predicates, mut bindings) = build_documents_predicates_pg(query);
        predicates.push("d.is_latest = true".to_string());

        apply_pagination_sql_pg(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

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

    async fn count_documents(&self, query: &SearchDocumentsQuery) -> Result<u64, StoreError> {
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_DOCUMENTS_V2} d");
        let (mut predicates, bindings) = build_documents_predicates_pg(query);
        predicates.push("d.is_latest = true".to_string());

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let count = query_builder
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(count as u64)
    }

    async fn find_non_deleted_document_by_exact_path(
        &self,
        path: &str,
    ) -> Result<Option<DocumentId>, StoreError> {
        let row = sqlx::query_as::<_, (String,)>(&format!(
            "SELECT d.id FROM {TABLE_DOCUMENTS_V2} d
                 WHERE d.path = $1 AND d.is_latest = true AND COALESCE(d.deleted, false) = false
                 LIMIT 1"
        ))
        .bind(path)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(row
            .map(|(id,)| id.parse::<DocumentId>())
            .transpose()
            .map_err(|e| StoreError::Internal(format!("invalid document id: {e}")))?)
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
        ))
        .await
    }

    async fn get_documents_by_paths(
        &self,
        paths: &[String],
    ) -> Result<Vec<(String, DocumentId, String)>, StoreError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        let mut deduped: Vec<&str> = Vec::with_capacity(paths.len());
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for path in paths {
            if seen.insert(path.as_str()) {
                deduped.push(path.as_str());
            }
        }

        let placeholders: Vec<String> = (1..=deduped.len()).map(|i| format!("${i}")).collect();
        let sql = format!(
            "SELECT d.path, d.id, d.title FROM {TABLE_DOCUMENTS_V2} d \
             WHERE d.is_latest = true \
               AND COALESCE(d.deleted, false) = false \
               AND d.path IN ({})",
            placeholders.join(", ")
        );
        let mut query_builder = sqlx::query_as::<_, (Option<String>, String, String)>(&sql);
        for path in &deduped {
            query_builder = query_builder.bind(*path);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for (path, id, title) in rows {
            let Some(path) = path else { continue };
            let document_id = id
                .parse::<DocumentId>()
                .map_err(|e| StoreError::Internal(format!("invalid document id: {e}")))?;
            results.push((path, document_id, title));
        }
        Ok(results)
    }

    async fn list_document_path_children(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String, u64, bool)>, StoreError> {
        // Normalize prefix: ensure it ends with '/'
        let prefix = if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{prefix}/")
        };
        let prefix_len = prefix.len() as i32;

        let sql = format!(
            "SELECT
                CASE
                    WHEN POSITION('/' IN SUBSTRING(d.path FROM $1 + 1)) > 0
                    THEN SUBSTRING(d.path FROM $1 + 1 FOR POSITION('/' IN SUBSTRING(d.path FROM $1 + 1)) - 1)
                    ELSE SUBSTRING(d.path FROM $1 + 1)
                END AS segment,
                COUNT(*) AS child_count,
                MAX(CASE WHEN d.path = $3 || CASE
                    WHEN POSITION('/' IN SUBSTRING(d.path FROM $1 + 1)) > 0
                    THEN SUBSTRING(d.path FROM $1 + 1 FOR POSITION('/' IN SUBSTRING(d.path FROM $1 + 1)) - 1)
                    ELSE SUBSTRING(d.path FROM $1 + 1)
                END THEN 1 ELSE 0 END)::int AS is_doc
             FROM {TABLE_DOCUMENTS_V2} d
             WHERE d.is_latest = true
               AND COALESCE(d.deleted, false) = false
               AND d.path IS NOT NULL
               AND d.path LIKE $2
               AND LENGTH(d.path) > $1
             GROUP BY segment
             ORDER BY segment"
        );

        let rows = sqlx::query_as::<_, (String, i64, i32)>(&sql)
            .bind(prefix_len)
            .bind(format!("{prefix}%"))
            .bind(&prefix)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(rows
            .into_iter()
            .map(|(segment, count, is_doc)| {
                let full_path = format!("{prefix}{segment}");
                (segment, full_path, count as u64, is_doc != 0)
            })
            .collect())
    }

    // -------------------------------------------------------------------------
    // Session methods
    // -------------------------------------------------------------------------

    async fn get_session(
        &self,
        id: &SessionId,
        include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError> {
        let query = format!(
            "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, proxy_targets
             FROM {TABLE_TASKS_V2}
             WHERE id = $1
             ORDER BY is_latest DESC, version_number DESC
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, TaskRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::SessionNotFound(id.clone()))?;
        if !include_deleted && row.deleted {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for task '{}'",
                row.id
            ))
        })?;
        let task = self.row_to_session(&row)?;
        Ok(Versioned::with_optional_actor(
            task,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.created_at,
        ))
    }

    async fn get_session_versions(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, proxy_targets
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
            return Err(StoreError::SessionNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for task '{}'",
                    row.id
                ))
            })?;
            let task = self.row_to_session(&row)?;
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

    async fn list_sessions(
        &self,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError> {
        let mut sql = format!(
            "SELECT t.id, t.version_number, t.spawned_from, t.image, t.env_vars, t.cpu_limit, t.memory_limit, t.status, t.last_message, t.error, t.deleted, t.actor, t.created_at, t.updated_at, t.creator, t.secrets, t.creation_time, t.start_time, t.end_time, t.conversation_id, t.usage, t.mount_spec, t.agent_config, t.mode, t.resumed_from, t.proxy_targets \
             FROM {TABLE_TASKS_V2} t"
        );
        let (mut predicates, mut bindings) = build_tasks_predicates_pg(query);
        predicates.push("t.is_latest = true".to_string());

        apply_pagination_sql_pg(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

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
            let task = self.row_to_session(&row)?;
            let task_id = row.id.parse::<SessionId>().map_err(|err| {
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

    async fn count_sessions(&self, query: &SearchSessionsQuery) -> Result<u64, StoreError> {
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_TASKS_V2} t");
        let (mut predicates, bindings) = build_tasks_predicates_pg(query);
        predicates.push("t.is_latest = true".to_string());

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        let mut query_builder = sqlx::query_scalar::<_, i64>(&sql);
        for value in bindings {
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
        crate::store::session_status_log_from_versions(&versions)
            .ok_or_else(|| StoreError::SessionNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let id_strings: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();
        let query = format!(
            "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, created_at, updated_at, creator, secrets, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, proxy_targets
             FROM {TABLE_TASKS_V2}
             WHERE id = ANY($1)
             ORDER BY id, version_number"
        );
        let rows = sqlx::query_as::<_, TaskRow>(&query)
            .bind(&id_strings)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut grouped: HashMap<SessionId, Vec<Versioned<Session>>> = HashMap::new();
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for task '{}'",
                    row.id
                ))
            })?;
            let task = self.row_to_session(&row)?;
            let task_id = row.id.parse::<SessionId>().map_err(|err| {
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
            if let Some(log) = crate::store::session_status_log_from_versions(&versions) {
                result.insert(task_id, log);
            }
        }

        Ok(result)
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
             ORDER BY is_latest DESC, version_number DESC
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

    // ---- Agent (read-only) ----

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, mcp_config_path, max_tries, max_simultaneous, \
                    is_assignment_agent, is_default_conversation_agent, secrets, deleted, \
                    created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE name = $1"
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
            "SELECT name, prompt_path, mcp_config_path, max_tries, max_simultaneous, \
                    is_assignment_agent, is_default_conversation_agent, secrets, deleted, \
                    created_at, updated_at \
             FROM {TABLE_AGENTS} WHERE deleted = false ORDER BY name"
        );
        let rows = sqlx::query_as::<_, AgentRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(row_to_agent).collect()
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
        let label = row.to_label()?;
        if label.deleted {
            return Err(StoreError::LabelNotFound(id.clone()));
        }
        Ok(label)
    }

    async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        let mut sql = format!(
            "SELECT id, name, color, deleted, recurse, hidden, created_at, updated_at \
             FROM {TABLE_LABELS}"
        );
        let (mut predicates, mut bindings) = build_labels_predicates_pg(query);

        if query.limit.is_some() || query.cursor.is_some() {
            apply_pagination_sql_pg(
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
            let label = row.to_label()?;
            labels.push((label_id, label));
        }

        Ok(labels)
    }

    async fn count_labels(&self, query: &SearchLabelsQuery) -> Result<u64, StoreError> {
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_LABELS}");
        let (predicates, bindings) = build_labels_predicates_pg(query);

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
                Ok(Some((label_id, row.to_label()?)))
            }
            None => Ok(None),
        }
    }

    async fn get_labels_for_object(
        &self,
        object_id: &HydraId,
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
        object_ids: &[HydraId],
    ) -> Result<HashMap<HydraId, Vec<LabelSummary>>, StoreError> {
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

        let mut result: HashMap<HydraId, Vec<LabelSummary>> = HashMap::new();
        for (obj_id_str, label_id_str, name, color, recurse, hidden) in rows {
            let obj_id = obj_id_str.parse::<HydraId>().map_err(|err| {
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

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<HydraId>, StoreError> {
        let sql = format!("SELECT object_id FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = $1");
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(label_id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|id| {
                id.parse::<HydraId>().map_err(|err| {
                    StoreError::Internal(format!("invalid object id stored in database: {err}"))
                })
            })
            .collect()
    }

    // ---- Trigger (read-only) ----

    async fn get_trigger(
        &self,
        id: &TriggerId,
        include_deleted: bool,
    ) -> Result<Versioned<Trigger>, StoreError> {
        let row = sqlx::query_as::<_, TriggerRow>(&format!(
            "SELECT id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_TRIGGERS} WHERE id = $1) AS creation_time \
             FROM {TABLE_TRIGGERS} \
             WHERE id = $1 \
             ORDER BY version_number DESC \
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::TriggerNotFound(id.clone()))?;
        let trigger = Self::row_to_trigger(&row)?;

        if trigger.deleted && !include_deleted {
            return Err(StoreError::TriggerNotFound(id.clone()));
        }

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for trigger '{}'",
                row.id
            ))
        })?;
        let creation_time = row.creation_time.unwrap_or(row.created_at);
        let actor_ref = row
            .actor
            .map(serde_json::from_value::<ActorRef>)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;

        Ok(Versioned::with_optional_actor(
            trigger,
            version,
            row.created_at,
            actor_ref,
            creation_time,
        ))
    }

    async fn get_trigger_versions(
        &self,
        id: &TriggerId,
    ) -> Result<Vec<Versioned<Trigger>>, StoreError> {
        let rows = sqlx::query_as::<_, TriggerRow>(&format!(
            "SELECT id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, created_at, updated_at, \
             NULL::timestamptz AS creation_time \
             FROM {TABLE_TRIGGERS} \
             WHERE id = $1 \
             ORDER BY version_number"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::TriggerNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for trigger '{}'",
                    row.id
                ))
            })?;
            let actor_ref = row
                .actor
                .clone()
                .map(serde_json::from_value::<ActorRef>)
                .transpose()
                .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;
            let trigger = Self::row_to_trigger(&row)?;
            results.push(Versioned::with_optional_actor(
                trigger,
                version,
                row.created_at,
                actor_ref,
                row.created_at,
            ));
        }

        let creation_time = results.first().map(|r| r.timestamp);
        for r in &mut results {
            r.creation_time = creation_time.unwrap_or(r.timestamp);
        }

        Ok(results)
    }

    async fn list_triggers(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(TriggerId, Versioned<Trigger>)>, StoreError> {
        let mut sql = format!(
            "SELECT t.id, t.version_number, t.enabled, t.creator, t.schedule, t.actions, t.last_fired_at, t.deleted, t.actor, t.created_at, t.updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_TRIGGERS} WHERE id = t.id) AS creation_time \
             FROM {TABLE_TRIGGERS} t \
             WHERE t.is_latest = true"
        );
        if !include_deleted {
            sql.push_str(" AND t.deleted = false");
        }
        sql.push_str(" ORDER BY t.created_at DESC, t.id DESC");

        let rows = sqlx::query_as::<_, TriggerRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut triggers = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for trigger '{}'",
                    row.id
                ))
            })?;
            let creation_time = row.creation_time.unwrap_or(row.created_at);
            let actor_ref = row
                .actor
                .clone()
                .map(serde_json::from_value::<ActorRef>)
                .transpose()
                .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;
            let trigger_id = TriggerId::from_str(&row.id).map_err(|err| {
                StoreError::Internal(format!("invalid trigger id stored '{}': {err}", row.id))
            })?;
            let trigger = Self::row_to_trigger(&row)?;
            triggers.push((
                trigger_id,
                Versioned::with_optional_actor(
                    trigger,
                    version,
                    row.created_at,
                    actor_ref,
                    creation_time,
                ),
            ));
        }
        Ok(triggers)
    }

    // ---- Project (read-only) ----

    async fn get_project(
        &self,
        id: &ProjectId,
        include_deleted: bool,
    ) -> Result<Versioned<Project>, StoreError> {
        let row = sqlx::query_as::<_, ProjectRow>(&format!(
            "SELECT id, version_number, key, name, creator, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = $1) AS creation_time, \
             prompt_path, priority, next_status_sequence \
             FROM {TABLE_PROJECTS} \
             WHERE id = $1 \
             ORDER BY version_number DESC \
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::ProjectNotFound(id.clone()))?;
        let statuses = Self::fetch_statuses_for_project(&self.pool, &row.id).await?;
        let project = Self::row_to_project(&row, statuses)?;

        if project.deleted && !include_deleted {
            return Err(StoreError::ProjectNotFound(id.clone()));
        }

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for project '{}'",
                row.id
            ))
        })?;
        let creation_time = row.creation_time.unwrap_or(row.created_at);
        let actor_ref = row
            .actor
            .map(serde_json::from_value::<ActorRef>)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;

        Ok(Versioned::with_optional_actor(
            project,
            version,
            row.created_at,
            actor_ref,
            creation_time,
        ))
    }

    async fn get_project_by_key(
        &self,
        key: &ProjectKey,
        include_deleted: bool,
    ) -> Result<Option<(ProjectId, Versioned<Project>)>, StoreError> {
        // The partial unique index `projects_key_unique_active_idx`
        // covers `(is_latest, key) WHERE is_latest AND NOT deleted`.
        // The happy path hits it directly; the `include_deleted` branch
        // widens the filter to scan tombstoned rows too.
        let mut sql = format!(
            "SELECT p.id, p.version_number, p.key, p.name, p.creator, p.deleted, p.actor, p.created_at, p.updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = p.id) AS creation_time, \
             p.prompt_path, p.priority, p.next_status_sequence \
             FROM {TABLE_PROJECTS} p \
             WHERE p.is_latest = true AND p.key = $1"
        );
        if !include_deleted {
            sql.push_str(" AND p.deleted = false");
        }
        sql.push_str(" ORDER BY p.deleted ASC, p.created_at DESC LIMIT 1");

        let row = sqlx::query_as::<_, ProjectRow>(&sql)
            .bind(key.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let Some(row) = row else {
            return Ok(None);
        };
        let statuses = Self::fetch_statuses_for_project(&self.pool, &row.id).await?;
        let project = Self::row_to_project(&row, statuses)?;

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for project '{}'",
                row.id
            ))
        })?;
        let project_id: ProjectId = row
            .id
            .parse()
            .map_err(|e| StoreError::Internal(format!("invalid project id stored: {e}")))?;
        let creation_time = row.creation_time.unwrap_or(row.created_at);
        let actor_ref = row
            .actor
            .map(serde_json::from_value::<ActorRef>)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;

        Ok(Some((
            project_id,
            Versioned::with_optional_actor(
                project,
                version,
                row.created_at,
                actor_ref,
                creation_time,
            ),
        )))
    }

    async fn list_projects(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(ProjectId, Versioned<Project>)>, StoreError> {
        let mut sql = format!(
            "SELECT p.id, p.version_number, p.key, p.name, p.creator, p.deleted, p.actor, p.created_at, p.updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = p.id) AS creation_time, \
             p.prompt_path, p.priority, p.next_status_sequence \
             FROM {TABLE_PROJECTS} p \
             WHERE p.is_latest = true"
        );
        if !include_deleted {
            sql.push_str(" AND p.deleted = false");
        }
        sql.push_str(" ORDER BY p.priority ASC, p.created_at DESC, p.id DESC");

        let rows = sqlx::query_as::<_, ProjectRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let project_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let mut statuses_by_project =
            Self::fetch_statuses_for_projects(&self.pool, &project_ids).await?;

        let mut projects = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for project '{}'",
                    row.id
                ))
            })?;
            let creation_time = row.creation_time.unwrap_or(row.created_at);
            let actor_ref = row
                .actor
                .clone()
                .map(serde_json::from_value::<ActorRef>)
                .transpose()
                .map_err(|e| StoreError::Internal(format!("failed to parse actor JSON: {e}")))?;
            let project_id: ProjectId = row
                .id
                .parse()
                .map_err(|e| StoreError::Internal(format!("invalid project id stored: {e}")))?;
            let statuses = statuses_by_project.remove(&row.id).unwrap_or_default();
            let project = Self::row_to_project(&row, statuses)?;
            projects.push((
                project_id,
                Versioned::with_optional_actor(
                    project,
                    version,
                    row.created_at,
                    actor_ref,
                    creation_time,
                ),
            ));
        }
        Ok(projects)
    }

    // ---- Object relationships (read-only) ----

    async fn get_relationships(
        &self,
        source_id: Option<&HydraId>,
        target_id: Option<&HydraId>,
        rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
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
            "SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
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
        source_ids: Option<&[HydraId]>,
        target_ids: Option<&[HydraId]>,
        rel_type: Option<crate::store::RelationshipType>,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        let mut conditions = Vec::new();
        let mut bind_index = 1u32;
        let mut source_id_strings: Vec<String> = Vec::new();
        let mut target_id_strings: Vec<String> = Vec::new();

        if let Some(sids) = source_ids {
            if sids.is_empty() {
                return Ok(Vec::new());
            }
            conditions.push(format!("source_id = ANY(${bind_index})"));
            bind_index += 1;
            source_id_strings = sids.iter().map(|id| id.as_ref().to_string()).collect();
        }
        if let Some(tids) = target_ids {
            if tids.is_empty() {
                return Ok(Vec::new());
            }
            conditions.push(format!("target_id = ANY(${bind_index})"));
            bind_index += 1;
            target_id_strings = tids.iter().map(|id| id.as_ref().to_string()).collect();
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
            "SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
             FROM {TABLE_OBJECT_RELATIONSHIPS}{where_clause} \
             ORDER BY created_at"
        );

        let mut query = sqlx::query_as::<_, ObjectRelationshipRow>(&sql);
        if source_ids.is_some() {
            query = query.bind(&source_id_strings);
        }
        if target_ids.is_some() {
            query = query.bind(&target_id_strings);
        }
        if let Some(rt) = rel_type {
            query = query.bind(rt.as_str());
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    async fn get_relationships_transitive(
        &self,
        ids: &[HydraId],
        direction: crate::store::TransitiveDirection,
        rel_type: crate::store::RelationshipType,
    ) -> Result<Vec<crate::store::ObjectRelationship>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let id_refs: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();

        let sql = match direction {
            crate::store::TransitiveDirection::Forward => format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE source_id = ANY($1) AND rel_type = $2 \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type, r.created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.source_id = tr.target_id \
                     WHERE r.rel_type = $2 \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                 FROM transitive_rels"
            ),
            crate::store::TransitiveDirection::Backward => format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE target_id = ANY($1) AND rel_type = $2 \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type, r.created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.target_id = tr.source_id \
                     WHERE r.rel_type = $2 \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                 FROM transitive_rels"
            ),
        };

        let rows = sqlx::query_as::<_, ObjectRelationshipRow>(&sql)
            .bind(&id_refs)
            .bind(rel_type.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    // ---- Auth tokens (read-only) ----

    async fn get_auth_token_hashes(&self, actor_name: &str) -> Result<Vec<String>, StoreError> {
        let sql = format!(
            "SELECT token_hash FROM {TABLE_AUTH_TOKENS} WHERE actor_name = $1 ORDER BY created_at"
        );
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .bind(actor_name)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows)
    }

    async fn get_auth_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<AuthTokenRow>, StoreError> {
        let sql = format!(
            "SELECT actor_name, session_id, is_revoked, creator FROM {TABLE_AUTH_TOKENS} \
             WHERE token_hash = $1 LIMIT 1"
        );
        let row = sqlx::query_as::<_, (String, Option<String>, bool, String)>(&sql)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        let Some((actor_name, session_id, is_revoked, creator)) = row else {
            return Ok(None);
        };
        let session_id = match session_id {
            Some(s) => Some(SessionId::from_str(&s).map_err(|e| {
                StoreError::Internal(format!("invalid session_id in auth_tokens: {e}"))
            })?),
            None => None,
        };
        Ok(Some(AuthTokenRow {
            actor_name,
            session_id,
            is_revoked,
            creator: Username::from(creator),
        }))
    }

    // ---- User secrets (read-only) ----

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let sql = format!(
            "SELECT encrypted_value FROM {TABLE_USER_SECRETS} WHERE username = $1 AND secret_name = $2 ORDER BY internal ASC LIMIT 1"
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
            "SELECT secret_name, MIN(internal::int)::bool as internal FROM {TABLE_USER_SECRETS} WHERE username = $1 GROUP BY secret_name ORDER BY secret_name"
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

    async fn get_conversation(
        &self,
        id: &ConversationId,
        include_deleted: bool,
    ) -> Result<Versioned<Conversation>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, agent_name, session_settings, spawned_from, status, creator, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_CONVERSATIONS_V2} WHERE id = $1) AS creation_time \
             FROM {TABLE_CONVERSATIONS_V2} \
             WHERE id = $1 \
             ORDER BY is_latest DESC, version_number DESC \
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, ConversationRow>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        if row.deleted && !include_deleted {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }
        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for conversation '{}'",
                row.id
            ))
        })?;
        let conversation = Self::row_to_conversation(&row)?;
        Ok(Versioned::with_optional_actor(
            conversation,
            version,
            row.created_at,
            parse_actor_json(row.actor)?,
            row.creation_time.unwrap_or(row.created_at),
        ))
    }

    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<(ConversationId, Versioned<Conversation>)>, StoreError> {
        let subquery = format!(
            "SELECT c.id, c.version_number, c.title, c.agent_name, c.session_settings, c.spawned_from, \
             c.status, c.creator, c.deleted, c.actor, c.created_at, c.updated_at, \
             (SELECT MIN(c2.created_at) FROM {TABLE_CONVERSATIONS_V2} c2 WHERE c2.id = c.id) AS creation_time \
             FROM {TABLE_CONVERSATIONS_V2} c \
             WHERE c.is_latest = true"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");

        let mut predicates = Vec::new();
        let mut bindings = Vec::new();

        if !query.include_deleted.unwrap_or(false) {
            predicates.push("deleted = false".to_string());
        }

        if let Some(ref status) = query.status {
            let status_str = match ConversationStatus::from(*status) {
                ConversationStatus::Active => "active",
                ConversationStatus::Idle => "idle",
                ConversationStatus::Closed => "closed",
            };
            predicates.push(format!("status = ${}", bindings.len() + 1));
            bindings.push(status_str.to_string());
        }

        if let Some(ref creator) = query.creator {
            let trimmed = creator.trim();
            if !trimmed.is_empty() {
                predicates.push(format!("LOWER(creator) = ${}", bindings.len() + 1));
                bindings.push(trimmed.to_lowercase());
            }
        }

        if let Some(ref q) = query.q {
            let term = q.trim().to_lowercase();
            if !term.is_empty() {
                let idx = bindings.len() + 1;
                predicates.push(format!(
                    "(LOWER(id) LIKE ${idx} OR LOWER(COALESCE(title, '')) LIKE ${idx} OR LOWER(COALESCE(agent_name, '')) LIKE ${idx})"
                ));
                bindings.push(format!("%{term}%"));
            }
        }

        if let Some(ref spawned_from) = query.spawned_from {
            predicates.push(format!("spawned_from = ${}", bindings.len() + 1));
            bindings.push(spawned_from.as_ref().to_string());
        }

        apply_pagination_sql_pg(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "created_at",
            "id",
        )?;

        let mut query_builder = sqlx::query_as::<_, ConversationRow>(&sql);
        for value in bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for conversation '{}'",
                    row.id
                ))
            })?;
            let conversation = Self::row_to_conversation(&row)?;
            let conversation_id = row.id.parse::<ConversationId>().map_err(|err| {
                StoreError::Internal(format!("invalid conversation id stored in database: {err}"))
            })?;
            let versioned = Versioned::with_optional_actor(
                conversation,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
            );
            results.push((conversation_id, versioned));
        }

        Ok(results)
    }

    async fn get_conversation_versions(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<Versioned<Conversation>>, StoreError> {
        let query = format!(
            "SELECT id, version_number, title, agent_name, session_settings, spawned_from, status, creator, deleted, actor, created_at, updated_at, \
             (SELECT MIN(created_at) FROM {TABLE_CONVERSATIONS_V2} WHERE id = $1) AS creation_time \
             FROM {TABLE_CONVERSATIONS_V2} \
             WHERE id = $1 \
             ORDER BY version_number"
        );
        let rows = sqlx::query_as::<_, ConversationRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for conversation '{}'",
                    row.id
                ))
            })?;
            let conversation = Self::row_to_conversation(&row)?;
            results.push(Versioned::with_optional_actor(
                conversation,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.creation_time.unwrap_or(row.created_at),
            ));
        }

        Ok(results)
    }

    async fn get_conversation_event_summaries(
        &self,
        ids: &[ConversationId],
    ) -> Result<HashMap<ConversationId, ConversationEventSummary>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let id_strings: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();

        // Query 1: Chat-text SessionEvent count per conversation_id —
        // summed across every live session linked to the conversation.
        let count_query = format!(
            "SELECT t.conversation_id AS conversation_id, COUNT(*) AS event_count \
             FROM {TABLE_SESSION_EVENTS_V2} e \
             JOIN {TABLE_TASKS_V2} t ON t.id = e.session_id \
                 AND t.is_latest = TRUE \
                 AND t.deleted = FALSE \
             WHERE t.conversation_id = ANY($1) \
               AND e.event_type IN ('user_message', 'assistant_message') \
             GROUP BY t.conversation_id"
        );
        let count_rows = sqlx::query_as::<_, ConversationEventCountRow>(&count_query)
            .bind(&id_strings)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        // Query 2: latest chat-text SessionEvent (UserMessage / AssistantMessage)
        // per conversation_id. Ordered by latest linked session first
        // (creation_time DESC), then latest event within that session.
        let preview_query = format!(
            "SELECT DISTINCT ON (t.conversation_id) \
                 t.conversation_id AS conversation_id, e.event_data AS event_data \
             FROM {TABLE_SESSION_EVENTS_V2} e \
             JOIN {TABLE_TASKS_V2} t ON t.id = e.session_id \
                 AND t.is_latest = TRUE \
                 AND t.deleted = FALSE \
             WHERE t.conversation_id = ANY($1) \
               AND e.event_type IN ('user_message', 'assistant_message') \
             ORDER BY t.conversation_id, t.creation_time DESC, t.id DESC, e.id DESC"
        );
        let preview_rows = sqlx::query_as::<_, ConversationPreviewRow>(&preview_query)
            .bind(&id_strings)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut counts: HashMap<ConversationId, usize> = HashMap::new();
        for row in count_rows {
            let conv_id = row
                .conversation_id
                .parse::<ConversationId>()
                .map_err(|e| StoreError::Internal(format!("invalid conversation id: {e}")))?;
            counts.insert(conv_id, row.event_count as usize);
        }

        let mut previews: HashMap<ConversationId, String> = HashMap::new();
        for row in preview_rows {
            let conv_id = row
                .conversation_id
                .parse::<ConversationId>()
                .map_err(|e| StoreError::Internal(format!("invalid conversation id: {e}")))?;
            let event: SessionEvent = serde_json::from_value(row.event_data).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize session event: {e}"))
            })?;
            previews.insert(conv_id, event.preview());
        }

        let mut result = HashMap::new();
        let mut all_ids: HashSet<ConversationId> = HashSet::new();
        all_ids.extend(counts.keys().cloned());
        all_ids.extend(previews.keys().cloned());
        for conv_id in all_ids {
            let event_count = counts.get(&conv_id).copied().unwrap_or(0);
            let last_event_preview = previews.get(&conv_id).cloned();
            result.insert(
                conv_id,
                ConversationEventSummary {
                    event_count,
                    last_event_preview,
                },
            );
        }

        Ok(result)
    }

    async fn get_session_events(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<SessionEvent>>, StoreError> {
        self.ensure_session_exists(id).await?;

        let query = format!(
            "SELECT id, session_id, version_number, event_data, actor, created_at \
             FROM {TABLE_SESSION_EVENTS_V2} \
             WHERE session_id = $1 \
             ORDER BY id ASC"
        );
        let rows = sqlx::query_as::<_, SessionEventRow>(&query)
            .bind(id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal("invalid version number stored for session event".to_string())
            })?;
            let event: SessionEvent = serde_json::from_value(row.event_data).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize session event: {e}"))
            })?;
            results.push(Versioned::with_optional_actor(
                event,
                version,
                row.created_at,
                parse_actor_json(row.actor)?,
                row.created_at,
            ));
        }

        Ok(results)
    }

    async fn list_session_ids_by_conversation_id(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<SessionId>, StoreError> {
        let query = format!(
            "SELECT id FROM {TABLE_TASKS_V2} \
             WHERE conversation_id = $1 \
               AND is_latest = TRUE \
               AND deleted = FALSE \
             ORDER BY creation_time ASC, id ASC"
        );
        let rows = sqlx::query_scalar::<_, String>(&query)
            .bind(conversation_id.as_ref())
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        rows.into_iter()
            .map(|id| {
                id.parse::<SessionId>()
                    .map_err(|e| StoreError::Internal(format!("invalid session id: {e}")))
            })
            .collect()
    }

    async fn get_session_event_summaries(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, SessionEventSummary>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let id_strings: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();
        let query = format!(
            "SELECT e.session_id, COUNT(*) AS event_count, \
             (SELECT e2.event_data FROM {TABLE_SESSION_EVENTS_V2} e2 \
              WHERE e2.session_id = e.session_id ORDER BY e2.id DESC LIMIT 1) AS last_event_data \
             FROM {TABLE_SESSION_EVENTS_V2} e \
             WHERE e.session_id = ANY($1) \
             GROUP BY e.session_id"
        );

        let rows = sqlx::query_as::<_, SessionEventSummaryRow>(&query)
            .bind(&id_strings)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut result = HashMap::new();
        for row in rows {
            let session_id = row
                .session_id
                .parse::<SessionId>()
                .map_err(|e| StoreError::Internal(format!("invalid session id: {e}")))?;
            let last_event_preview = row
                .last_event_data
                .map(|data| {
                    serde_json::from_value::<SessionEvent>(data)
                        .map(|event| event.preview())
                        .map_err(|e| {
                            StoreError::Internal(format!(
                                "failed to deserialize session event: {e}"
                            ))
                        })
                })
                .transpose()?;
            result.insert(
                session_id,
                SessionEventSummary {
                    event_count: row.event_count as usize,
                    last_event_preview,
                },
            );
        }

        Ok(result)
    }

    async fn get_session_state(&self, id: &SessionId) -> Result<Option<Vec<u8>>, StoreError> {
        self.ensure_session_exists(id).await?;

        let query = format!("SELECT data FROM {TABLE_SESSION_STATE_V2} WHERE session_id = $1");
        let row = sqlx::query_scalar::<_, Vec<u8>>(&query)
            .bind(id.as_ref())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(row)
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
        let id = self.next_issue_id().await?;
        let actor_json = actor_to_json(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        Self::insert_issue_in_tx(&mut tx, &id, 1, &issue, Some(&actor_json)).await?;
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
        Self::insert_issue_in_tx(&mut tx, id, next_version, &issue, Some(&actor_json)).await?;
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
        let id = self.next_patch_id().await?;
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
        let id = self.next_document_id().await?;
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
    // Session methods
    // -------------------------------------------------------------------------

    async fn add_session(
        &self,
        mut session: Session,
        creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(SessionId, VersionNumber), StoreError> {
        let id = self.next_session_id().await?;

        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        session.creation_time = Some(creation_time);
        let actor_json = actor_to_json(actor);
        self.insert_session(&id, 1, &session, Some(&actor_json))
            .await?;
        Ok((id, 1))
    }

    async fn update_session(
        &self,
        hydra_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        self.ensure_session_exists(hydra_id).await?;
        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        let latest_version = self
            .fetch_latest_version_number(TABLE_TASKS_V2, hydra_id.as_ref())
            .await?
            .ok_or_else(|| {
                StoreError::Internal(format!("session '{hydra_id}' was missing during update"))
            })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for session '{hydra_id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        self.insert_session(hydra_id, next_version, &session, Some(&actor_json))
            .await?;
        self.get_session(hydra_id, true).await
    }

    async fn delete_session(
        &self,
        id: &SessionId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_session(id, true).await?;
        let mut session = current.item;
        session.deleted = true;
        let versioned = self.update_session(id, session, actor).await?;
        Ok(versioned.version)
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
             ORDER BY is_latest DESC, version_number DESC
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
             ORDER BY is_latest DESC, version_number DESC
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
                // Soft-deleted agent exists — reactivate.
                let now = Utc::now();
                let secrets_json = serde_json::to_value(&agent.secrets).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize secrets: {e}"))
                })?;
                let sql = format!(
                    "UPDATE {TABLE_AGENTS} \
                     SET prompt_path = $1, mcp_config_path = $2, max_tries = $3, max_simultaneous = $4, \
                         is_assignment_agent = $5, is_default_conversation_agent = $6, secrets = $7, \
                         deleted = false, created_at = $8, updated_at = $9 \
                     WHERE name = $10"
                );
                sqlx::query(&sql)
                    .bind(&agent.prompt_path)
                    .bind(agent.mcp_config_path.as_deref())
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(agent.is_default_conversation_agent)
                    .bind(&secrets_json)
                    .bind(now)
                    .bind(now)
                    .bind(&agent.name)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;

                Ok(())
            }
            None => {
                // No existing row — insert.
                let secrets_json = serde_json::to_value(&agent.secrets).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize secrets: {e}"))
                })?;
                let sql = format!(
                    "INSERT INTO {TABLE_AGENTS} \
                     (name, prompt_path, mcp_config_path, max_tries, max_simultaneous, is_assignment_agent, \
                      is_default_conversation_agent, secrets, deleted, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
                );
                sqlx::query(&sql)
                    .bind(&agent.name)
                    .bind(&agent.prompt_path)
                    .bind(agent.mcp_config_path.as_deref())
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(agent.is_default_conversation_agent)
                    .bind(&secrets_json)
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

        let secrets_json = serde_json::to_value(&agent.secrets)
            .map_err(|e| StoreError::Internal(format!("failed to serialize secrets: {e}")))?;
        let sql = format!(
            "UPDATE {TABLE_AGENTS} \
             SET prompt_path = $1, mcp_config_path = $2, max_tries = $3, max_simultaneous = $4, \
                 is_assignment_agent = $5, is_default_conversation_agent = $6, secrets = $7, \
                 updated_at = $8 \
             WHERE name = $9"
        );
        sqlx::query(&sql)
            .bind(&agent.prompt_path)
            .bind(agent.mcp_config_path.as_deref())
            .bind(agent.max_tries)
            .bind(agent.max_simultaneous)
            .bind(agent.is_assignment_agent)
            .bind(agent.is_default_conversation_agent)
            .bind(&secrets_json)
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

        let id = self.next_label_id().await?;

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
        object_id: &HydraId,
    ) -> Result<bool, StoreError> {
        let object_kind = crate::store::object_kind_from_id(object_id)?;
        let sql = format!(
            "INSERT INTO {TABLE_LABEL_ASSOCIATIONS} (label_id, object_id, object_kind) \
             VALUES ($1, $2, $3) \
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
        object_id: &HydraId,
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
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
        let source_kind = crate::store::object_kind_from_id(source_id)?;
        let target_kind = crate::store::object_kind_from_id(target_id)?;
        let sql = format!(
            "INSERT INTO {TABLE_OBJECT_RELATIONSHIPS} \
             (source_id, source_kind, target_id, target_kind, rel_type) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (source_id, rel_type, target_id) DO NOTHING"
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
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: crate::store::RelationshipType,
    ) -> Result<bool, StoreError> {
        let sql = format!(
            "DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = $1 AND target_id = $2 AND rel_type = $3"
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

    // ---- Auth token mutations ----

    async fn add_auth_token(
        &self,
        actor_name: &str,
        token_hash: &str,
        session_id: Option<&SessionId>,
        creator: &Username,
    ) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO {TABLE_AUTH_TOKENS} (actor_name, token_hash, session_id, creator) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING"
        );
        sqlx::query(&sql)
            .bind(actor_name)
            .bind(token_hash)
            .bind(session_id.map(|s| s.to_string()))
            .bind(creator.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn delete_auth_tokens_for_actor(&self, actor_name: &str) -> Result<(), StoreError> {
        let sql = format!("DELETE FROM {TABLE_AUTH_TOKENS} WHERE actor_name = $1");
        sqlx::query(&sql)
            .bind(actor_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn revoke_auth_tokens_for_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), StoreError> {
        let sql = format!(
            "UPDATE {TABLE_AUTH_TOKENS} SET is_revoked = TRUE \
             WHERE session_id = $1 AND is_revoked = FALSE"
        );
        sqlx::query(&sql)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    // ---- User secret mutations ----

    async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
        internal: bool,
    ) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO {TABLE_USER_SECRETS} (username, secret_name, encrypted_value, internal, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) \
             ON CONFLICT (username, secret_name, internal) \
             DO UPDATE SET encrypted_value = $3, updated_at = $5"
        );
        let now = chrono::Utc::now();
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .bind(encrypted_value)
            .bind(internal)
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
        let sql = format!(
            "DELETE FROM {TABLE_USER_SECRETS} WHERE username = $1 AND secret_name = $2 AND internal = FALSE"
        );
        sqlx::query(&sql)
            .bind(username.as_str())
            .bind(secret_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn add_conversation(
        &self,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<(ConversationId, VersionNumber), StoreError> {
        let id = self.next_conversation_id().await?;
        let actor_json = actor_to_json(actor);
        Self::insert_conversation_in_tx(&self.pool, &id, 1, &conversation, Some(&actor_json))
            .await?;
        Ok((id, 1))
    }

    async fn update_conversation(
        &self,
        id: &ConversationId,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let latest_version = self
            .fetch_latest_version_number(TABLE_CONVERSATIONS_V2, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for conversation '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        Self::insert_conversation_in_tx(
            &self.pool,
            id,
            next_version,
            &conversation,
            Some(&actor_json),
        )
        .await?;

        Ok(next_version)
    }

    async fn append_session_event(
        &self,
        id: &SessionId,
        event: SessionEvent,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Ensure session exists.
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {TABLE_TASKS_V2} WHERE id = $1 LIMIT 1)"
        ))
        .bind(id.as_ref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        if !exists {
            return Err(StoreError::SessionNotFound(id.clone()));
        }

        // Lock existing rows for this session to serialize concurrent appends.
        // The UNIQUE (session_id, version_number) constraint is the safety net.
        let _lock_rows = {
            let query = format!(
                "SELECT id FROM {TABLE_SESSION_EVENTS_V2} \
                 WHERE session_id = $1 FOR UPDATE"
            );
            sqlx::query_scalar::<_, i64>(&query)
                .bind(id.as_ref())
                .fetch_all(&mut *tx)
                .await
                .map_err(map_sqlx_error)?
        };
        let latest_event_version = {
            let query = format!(
                "SELECT COALESCE(MAX(version_number), 0) FROM {TABLE_SESSION_EVENTS_V2} \
                 WHERE session_id = $1"
            );
            sqlx::query_scalar::<_, i64>(&query)
                .bind(id.as_ref())
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx_error)?
        };
        let next_version = VersionNumber::try_from(latest_event_version + 1).map_err(|_| {
            StoreError::Internal(format!("version number overflow for session event '{id}'"))
        })?;

        let event_data = serde_json::to_value(&event)
            .map_err(|e| StoreError::Internal(format!("failed to serialize session event: {e}")))?;
        let event_type = match &event {
            SessionEvent::UserMessage { .. } => "user_message",
            SessionEvent::AssistantMessage { .. } => "assistant_message",
            SessionEvent::ToolUse { .. } => "tool_use",
            SessionEvent::Suspending { .. } => "suspending",
            SessionEvent::Resumed { .. } => "resumed",
            SessionEvent::Closed { .. } => "closed",
        };
        let actor_json = actor_to_json(actor);

        let query = format!(
            "INSERT INTO {TABLE_SESSION_EVENTS_V2} \
             (session_id, version_number, event_type, event_data, actor) \
             VALUES ($1, $2, $3, $4, $5)"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(
                i64::try_from(next_version)
                    .map_err(|_| StoreError::Internal("version number overflow".to_string()))?,
            )
            .bind(event_type)
            .bind(&event_data)
            .bind(&actor_json)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(next_version)
    }

    async fn store_session_state(
        &self,
        id: &SessionId,
        data: Vec<u8>,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        self.ensure_session_exists(id).await?;

        let query = format!(
            "INSERT INTO {TABLE_SESSION_STATE_V2} (session_id, data) \
             VALUES ($1, $2) \
             ON CONFLICT (session_id) DO UPDATE SET data = $2, updated_at = NOW()"
        );
        sqlx::query(&query)
            .bind(id.as_ref())
            .bind(&data)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        Ok(())
    }

    // ---- Trigger mutations ----

    async fn add_trigger(
        &self,
        trigger: Trigger,
        actor: &ActorRef,
    ) -> Result<(TriggerId, VersionNumber), StoreError> {
        let id = self.next_trigger_id().await?;
        let actor_json = actor_to_json(actor);
        Self::insert_trigger_in_tx(&self.pool, &id, 1, &trigger, Some(&actor_json)).await?;
        Ok((id, 1))
    }

    async fn update_trigger(
        &self,
        id: &TriggerId,
        mut trigger: Trigger,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Read the current latest row inside the transaction with FOR UPDATE
        // so a concurrent record_trigger_fire's last_fired_at value is carried
        // forward atomically.
        let latest_row = sqlx::query_as::<_, TriggerRow>(&format!(
            "SELECT id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, created_at, updated_at \
             FROM {TABLE_TRIGGERS} \
             WHERE id = $1 AND is_latest = true \
             FOR UPDATE"
        ))
        .bind(id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let latest_row = latest_row.ok_or_else(|| StoreError::TriggerNotFound(id.clone()))?;

        // Always overwrite the supplied `last_fired_at` with the latest
        // row's value (Some or None). `record_trigger_fire` mutates the
        // latest row in place; a stale snapshot round-tripped by the
        // caller must not regress it.
        trigger.last_fired_at = latest_row.last_fired_at;

        let latest_version = VersionNumber::try_from(latest_row.version_number).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for trigger '{id}'"))
        })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for trigger '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        Self::insert_trigger_in_tx(&mut *tx, id, next_version, &trigger, Some(&actor_json)).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(next_version)
    }

    async fn delete_trigger(
        &self,
        id: &TriggerId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_trigger(id, true).await?;
        let mut trigger = current.item;
        trigger.deleted = true;
        self.update_trigger(id, trigger, actor).await
    }

    async fn record_trigger_fire(
        &self,
        id: &TriggerId,
        fired_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(&format!(
            "UPDATE {TABLE_TRIGGERS} SET last_fired_at = $1 WHERE id = $2 AND is_latest = true"
        ))
        .bind(fired_at)
        .bind(id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(StoreError::TriggerNotFound(id.clone()));
        }
        Ok(())
    }

    // ---- Project mutations ----

    async fn add_project(
        &self,
        project: Project,
        actor: &ActorRef,
    ) -> Result<(ProjectId, VersionNumber), StoreError> {
        let id = self.next_project_id().await?;
        let actor_json = actor_to_json(actor);
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Post-cutover, `add_project` is project-level only. The new
        // row starts with `next_status_sequence = 1`; statuses are
        // created independently via `add_status`.
        Self::insert_project_row_in_tx(&mut *tx, &id, 1, &project, Some(&actor_json), 1).await?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok((id, 1))
    }

    async fn update_project(
        &self,
        id: &ProjectId,
        project: Project,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let row = sqlx::query_as::<_, (i64, i64)>(&format!(
            "SELECT version_number, next_status_sequence FROM {TABLE_PROJECTS} \
             WHERE id = $1 AND is_latest = true \
             FOR UPDATE"
        ))
        .bind(id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let (latest_version, current_next_seq) =
            row.ok_or_else(|| StoreError::ProjectNotFound(id.clone()))?;
        let latest_version = VersionNumber::try_from(latest_version).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for project '{id}'"))
        })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;
        let next_version_i64 = i64::try_from(next_version).map_err(|_| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;

        let actor_json = actor_to_json(actor);
        // Post-cutover, `update_project` is project-level only and
        // carries the existing `next_status_sequence` forward unchanged
        // — only `add_status` mutates it.
        Self::insert_project_row_in_tx(
            &mut *tx,
            id,
            next_version_i64,
            &project,
            Some(&actor_json),
            current_next_seq,
        )
        .await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(next_version)
    }

    async fn delete_project(
        &self,
        id: &ProjectId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_project(id, true).await?;
        let mut project = current.item;
        project.deleted = true;
        self.update_project(id, project, actor).await
    }

    async fn add_status(
        &self,
        id: &ProjectId,
        status: StatusDefinition,
        actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let project_row = Self::load_project_for_status_mutation_pg(&mut tx, id).await?;
        let latest_version = VersionNumber::try_from(project_row.version_number).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for project '{id}'"))
        })?;

        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM metis.statuses WHERE project_id = $1 AND key = $2 LIMIT 1",
        )
        .bind(id.as_ref())
        .bind(status.key.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        if existing.is_some() {
            return Err(StoreError::InvalidIssueStatus(format!(
                "status '{}' already exists on project '{id}'",
                status.key.as_str()
            )));
        }

        let sequence = project_row.next_status_sequence;
        let new_next_seq = sequence.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("next_status_sequence overflow for project '{id}'"))
        })?;
        Self::insert_status_row_in_tx(&mut tx, id.as_ref(), sequence, &status).await?;

        let next_version = Self::bump_project_version_for_status_mutation_pg(
            &mut tx,
            id,
            &project_row,
            latest_version,
            actor,
            new_next_seq,
        )
        .await?;

        tx.commit().await.map_err(map_sqlx_error)?;
        Ok((status, next_version))
    }

    async fn update_status(
        &self,
        id: &ProjectId,
        status_key: &StatusKey,
        status: StatusDefinition,
        actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let project_row = Self::load_project_for_status_mutation_pg(&mut tx, id).await?;
        let latest_version = VersionNumber::try_from(project_row.version_number).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for project '{id}'"))
        })?;

        let sequence: Option<i64> = sqlx::query_scalar(
            "SELECT sequence FROM metis.statuses WHERE project_id = $1 AND key = $2 LIMIT 1",
        )
        .bind(id.as_ref())
        .bind(status_key.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        let sequence = sequence.ok_or_else(|| {
            StoreError::InvalidIssueStatus(format!(
                "status '{}' does not exist on project '{id}'",
                status_key.as_str()
            ))
        })?;

        if &status.key != status_key {
            let collides: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM metis.statuses WHERE project_id = $1 AND key = $2 LIMIT 1",
            )
            .bind(id.as_ref())
            .bind(status.key.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
            if collides.is_some() {
                return Err(StoreError::InvalidIssueStatus(format!(
                    "status '{}' already exists on project '{id}'",
                    status.key.as_str()
                )));
            }
        }

        let color_str = status.color.as_ref().to_string();
        let on_enter_json = status
            .on_enter
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize status on_enter: {e}"))
            })?;
        sqlx::query(
            "UPDATE metis.statuses SET key = $1, label = $2, color = $3, unblocks_parents = $4, unblocks_dependents = $5, cascades_to_children = $6, on_enter = $7, prompt_path = $8, interactive = $9, position = $10 \
             WHERE project_id = $11 AND sequence = $12",
        )
        .bind(status.key.as_str())
        .bind(&status.label)
        .bind(&color_str)
        .bind(status.unblocks_parents)
        .bind(status.unblocks_dependents)
        .bind(status.cascades_to_children)
        .bind(&on_enter_json)
        .bind(status.prompt_path.as_deref())
        .bind(status.interactive)
        .bind(status.position)
        .bind(id.as_ref())
        .bind(sequence)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let next_version = Self::bump_project_version_for_status_mutation_pg(
            &mut tx,
            id,
            &project_row,
            latest_version,
            actor,
            project_row.next_status_sequence,
        )
        .await?;

        tx.commit().await.map_err(map_sqlx_error)?;
        Ok((status, next_version))
    }

    async fn delete_status(
        &self,
        id: &ProjectId,
        status_key: &StatusKey,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let project_row = Self::load_project_for_status_mutation_pg(&mut tx, id).await?;
        let latest_version = VersionNumber::try_from(project_row.version_number).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for project '{id}'"))
        })?;

        let result = sqlx::query("DELETE FROM metis.statuses WHERE project_id = $1 AND key = $2")
            .bind(id.as_ref())
            .bind(status_key.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|err| {
                if is_status_sequence_fk_violation_pg(&err) {
                    StoreError::InvalidIssueStatus(format!(
                        "cannot remove status '{}' from project '{id}': an issue still references it",
                        status_key.as_str()
                    ))
                } else {
                    map_sqlx_error(err)
                }
            })?;
        if result.rows_affected() == 0 {
            return Err(StoreError::InvalidIssueStatus(format!(
                "status '{}' does not exist on project '{id}'",
                status_key.as_str()
            )));
        }

        let next_version = Self::bump_project_version_for_status_mutation_pg(
            &mut tx,
            id,
            &project_row,
            latest_version,
            actor,
            project_row.next_status_sequence,
        )
        .await?;

        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(next_version)
    }
}

impl LabelRow {
    fn to_label(&self) -> Result<Label, StoreError> {
        let color = self.color.parse().map_err(|err| {
            StoreError::Internal(format!("invalid label color in database: {err}"))
        })?;
        Ok(Label {
            name: self.name.clone(),
            color,
            deleted: self.deleted,
            recurse: self.recurse,
            hidden: self.hidden,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn row_to_agent(row: AgentRow) -> Result<Agent, StoreError> {
    let secrets: Vec<String> = serde_json::from_value(row.secrets)
        .map_err(|e| StoreError::Internal(format!("invalid secrets JSON in database: {e}")))?;
    Ok(Agent {
        name: row.name,
        prompt_path: row.prompt_path,
        mcp_config_path: row.mcp_config_path,
        max_tries: row.max_tries,
        max_simultaneous: row.max_simultaneous,
        is_assignment_agent: row.is_assignment_agent,
        is_default_conversation_agent: row.is_default_conversation_agent,
        secrets,
        deleted: row.deleted,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Appends cursor-based keyset pagination to a SQL query (PostgreSQL dialect).
///
/// Adds the cursor WHERE predicate into `predicates`, and appends
/// ORDER BY / LIMIT clauses to `sql`. Returns the effective limit if set.
///
/// The `timestamp_col` is the SQL column name used for the timestamp
/// component of the cursor (e.g. `"created_at"` or `"updated_at"`).
fn apply_pagination_sql_pg(
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
        let ts_idx = bindings.len() + 1;
        let id_idx = bindings.len() + 2;
        predicates.push(format!(
            "({timestamp_col}, {id_col}) < (${ts_idx}::timestamptz, ${id_idx})"
        ));
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
    use crate::{
        domain::{
            documents::Document,
            issues::{Issue, IssueDependency, IssueDependencyType, IssueType, SessionSettings},
            patches::{CommitRange, GitOid, GithubPr, Patch, PatchStatus, Review},
            users::{User, Username},
        },
        test_utils::test_state_with_store,
    };
    use chrono::Timelike;
    use hydra_common::{
        PatchId, RepoName, SessionId, VersionNumber, Versioned,
        actor_ref::ActorId,
        api::v1::form::{
            Action, ActionStyle, Effect, Field, Form, FormResponse, Input, SelectOption,
        },
        repositories::{Repository, SearchRepositoriesQuery},
        test_utils::status::status,
    };
    use std::{collections::HashMap, collections::HashSet, str::FromStr, sync::Arc};

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
            status("open"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            dependencies,
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn sample_patch() -> Patch {
        Patch::new(
            "patch title".to_string(),
            "desc".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn sample_document(path: &str) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: Some(path.parse().unwrap()),
            deleted: false,
        }
    }

    fn sample_session() -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("hydra-worker:latest".to_string()),
            Default::default(),
            None,
            None,
            None,
            SessionMode::Headless,
            Status::Created,
            None,
            None,
        )
    }

    /// Session with creator and other fields set for round-trip tests.
    fn session_with_creator_for_round_trip() -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session::new(
            Username::from("alice"),
            None,
            None,
            AgentConfig::new(None, Some("model-v1".to_string()), None, None),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("hydra-worker:latest".to_string()),
            Default::default(),
            None,
            None,
            None,
            SessionMode::Headless,
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
        )
    }

    /// Truncate a DateTime to microsecond precision to match Postgres TIMESTAMPTZ storage.
    fn truncate_to_micros(dt: DateTime<Utc>) -> DateTime<Utc> {
        let nanos = dt.timestamp_subsec_nanos();
        let micros_only = (nanos / 1_000) * 1_000;
        dt.with_nanosecond(micros_only).unwrap()
    }

    /// Session with every optional field set so serialization round-trip can assert full equality.
    fn sample_session_all_fields() -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        let mcp_config = serde_json::json!({"mcpServers": {"playwright": {"command": "npx", "args": ["@anthropic/mcp-playwright"]}}});
        let mut session = Session::new(
            Username::from("bob"),
            None,
            None,
            AgentConfig::new(None, Some("model-x".to_string()), None, Some(mcp_config)),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("img:tag".to_string()),
            [("K".to_string(), "V".to_string())].into_iter().collect(),
            Some("1000m".to_string()),
            Some("512Mi".to_string()),
            Some(vec!["secret-a".to_string(), "secret-b".to_string()]),
            SessionMode::Headless,
            Status::Created,
            Some("last message".to_string()),
            None,
        );
        session.start_time = Some(truncate_to_micros(
            Utc::now() - chrono::Duration::minutes(10),
        ));
        session.end_time = Some(truncate_to_micros(
            Utc::now() - chrono::Duration::minutes(5),
        ));
        session.usage = Some(hydra_common::sessions::TokenUsage {
            input_tokens: 1234,
            output_tokens: 567,
            cache_read_input_tokens: 89,
            cache_creation_input_tokens: 10,
        });
        session.proxy_targets = vec![
            hydra_common::api::v1::sessions::ProxyTarget {
                port: 3000,
                ready_path: Some("/ready".to_string()),
            },
            hydra_common::api::v1::sessions::ProxyTarget {
                port: 5173,
                ready_path: None,
            },
        ];
        session
    }

    /// Patch with every optional field set so serialization round-trip can assert full equality.
    fn sample_patch_all_fields() -> Patch {
        let base_oid = GitOid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let head_oid = GitOid::from_str("fedcba9876543210fedcba9876543210fedcba98").unwrap();
        let mut patch = Patch::new(
            "full title".to_string(),
            "full desc".to_string(),
            "full diff".to_string(),
            PatchStatus::Open,
            true,
            Username::from("test-creator"),
            vec![Review::new(
                "looks good".to_string(),
                true,
                hydra_common::Principal::User {
                    name: hydra_common::api::v1::users::Username::try_new("alice").unwrap(),
                },
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
            status("open"),
            crate::domain::projects::default_project_id(),
            Some(hydra_common::principal::Principal::User {
                name: hydra_common::api::v1::users::Username::try_new("assignee").unwrap(),
            }),
            Some(SessionSettings {
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
            dependencies,
            patches,
            Some(Form {
                prompt: "Please review and respond".to_string(),
                fields: vec![
                    Field {
                        key: "name".to_string(),
                        label: "Name".to_string(),
                        description: Some("Your full name".to_string()),
                        input: Input::Text {
                            placeholder: Some("John Doe".to_string()),
                            min_length: Some(1),
                            max_length: Some(100),
                            pattern: Some(r"^[a-zA-Z ]+$".to_string()),
                        },
                        default: Some(serde_json::json!("Default Name")),
                    },
                    Field {
                        key: "notes".to_string(),
                        label: "Notes".to_string(),
                        description: None,
                        input: Input::Textarea {
                            placeholder: Some("Enter notes...".to_string()),
                            min_length: None,
                            max_length: Some(5000),
                            rows: 6,
                        },
                        default: None,
                    },
                    Field {
                        key: "priority".to_string(),
                        label: "Priority".to_string(),
                        description: Some("Select priority level".to_string()),
                        input: Input::Select {
                            options: vec![
                                SelectOption {
                                    value: "low".to_string(),
                                    label: "Low".to_string(),
                                },
                                SelectOption {
                                    value: "high".to_string(),
                                    label: "High".to_string(),
                                },
                            ],
                            radio: true,
                        },
                        default: Some(serde_json::json!("low")),
                    },
                    Field {
                        key: "agree".to_string(),
                        label: "I agree".to_string(),
                        description: None,
                        input: Input::Checkbox,
                        default: Some(serde_json::json!(false)),
                    },
                    Field {
                        key: "count".to_string(),
                        label: "Count".to_string(),
                        description: None,
                        input: Input::Number {
                            min: Some(0.0),
                            max: Some(100.0),
                            step: Some(1.0),
                        },
                        default: Some(serde_json::json!(42)),
                    },
                ],
                actions: vec![
                    Action {
                        id: "approve".to_string(),
                        label: "Approve".to_string(),
                        style: ActionStyle::Primary,
                        requires: vec!["name".to_string(), "agree".to_string()],
                        effect: Effect::UpdateIssue {
                            status: status("closed"),
                            set_feedback_from: None,
                        },
                    },
                    Action {
                        id: "reject".to_string(),
                        label: "Reject".to_string(),
                        style: ActionStyle::Danger,
                        requires: vec![],
                        effect: Effect::RecordOnly,
                    },
                ],
            }),
            Some(FormResponse {
                action_id: "approve".to_string(),
                actor: ActorId::User(Username::from("responder").into()),
                values: HashMap::from([
                    ("name".to_string(), serde_json::json!("Jane Doe")),
                    ("notes".to_string(), serde_json::json!("Looks good")),
                    ("priority".to_string(), serde_json::json!("high")),
                    ("agree".to_string(), serde_json::json!(true)),
                    ("count".to_string(), serde_json::json!(7)),
                ]),
                submitted_at: truncate_to_micros(Utc::now()),
            }),
            Some("some feedback text".to_string()),
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_repositories_filters_by_remote_url_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alpha = RepoName::from_str("dourolabs/alpha").unwrap();
        let beta = RepoName::from_str("dourolabs/beta").unwrap();
        let gamma = RepoName::from_str("dourolabs/gamma").unwrap();

        store
            .add_repository(
                alpha.clone(),
                Repository::new(
                    "https://github.com/dourolabs/alpha.git".to_string(),
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_repository(
                beta.clone(),
                Repository::new(
                    "https://github.com/dourolabs/beta.git".to_string(),
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_repository(
                gamma.clone(),
                Repository::new("git@github.com:dourolabs/alpha".to_string(), None, None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://GitHub.com/dourolabs/alpha/".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert_eq!(list.len(), 2);
        let names: Vec<_> = list.iter().map(|(n, _)| n.clone()).collect();
        assert!(names.contains(&alpha));
        assert!(names.contains(&gamma));

        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://github.com/dourolabs/beta".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, beta);

        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://github.com/dourolabs/missing".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert!(list.is_empty());

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
        let task = session_with_creator_for_round_trip();

        let (task_id, version) = store
            .add_session(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_session(&task_id, false).await.unwrap();
        assert_eq!(
            fetched.item.creator, task.creator,
            "creator must round-trip"
        );
        assert_eq!(fetched.item.mode, task.mode);
        assert_eq!(fetched.item.image, task.image);
        assert_eq!(fetched.item.agent_config.model, task.agent_config.model);
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.mode = crate::domain::sessions::SessionMode::Headless;
        updated.agent_config.system_prompt = Some("updated prompt".to_string());
        store
            .update_session(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched2 = store.get_session(&task_id, false).await.unwrap();
        assert_eq!(
            fetched2.item.creator, task.creator,
            "creator must persist across updates"
        );
        assert!(matches!(
            &fetched2.item.mode,
            crate::domain::sessions::SessionMode::Headless
        ));
        assert_eq!(
            fetched2.item.agent_config.system_prompt.as_deref(),
            Some("updated prompt")
        );
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
    async fn issue_round_trips_assignee_principal_user_v2(pool: PgStorePool) {
        use hydra_common::principal::Principal as ActorPrincipal;
        let store = PostgresStoreV2::new(pool);
        let mut issue = sample_issue(vec![]);
        let alice = ActorPrincipal::User {
            name: hydra_common::api::v1::users::Username::try_new("alice").unwrap(),
        };
        issue.assignee = Some(alice.clone());
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.assignee, Some(alice));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_round_trips_assignee_none_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let mut issue = sample_issue(vec![]);
        issue.assignee = None;
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.assignee, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_issue_versions_populates_relationships_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create a parent issue.
        let (parent_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Create a child issue with a dependency on the parent.
        let (issue_id, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent_id.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // Update the issue to create a second version.
        let mut updated = sample_issue(vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )]);
        updated.description = "updated details".to_string();
        store
            .update_issue(&issue_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Fetch all versions and verify relationships are populated on every version.
        let versions = store.get_issue_versions(&issue_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        for v in &versions {
            assert_eq!(
                v.item.dependencies.len(),
                1,
                "version {} should have 1 dependency",
                v.version
            );
            assert_eq!(v.item.dependencies[0].issue_id, parent_id);
        }
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

        let mut task = sample_session();
        task.spawned_from = Some(issue_id.clone());
        let (task_id, _) = handles
            .store
            .add_session(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            handles
                .store
                .get_session(&task_id, false)
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
                .get_session(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Running
        );

        handles
            .state
            .transition_task_to_completion(
                &task_id,
                Ok(()),
                Some("done".into()),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(
            handles
                .store
                .get_session(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Complete
        );

        let tasks = handles
            .store
            .get_sessions_for_issue(&issue_id)
            .await
            .unwrap();
        assert_eq!(tasks, vec![task_id.clone()]);

        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Complete.into()]);
        let complete: Vec<_> = handles
            .store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(complete, vec![task_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_list_filters_by_multiple_statuses_v2(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
        let handles = test_state_with_store(store.clone());
        let (issue_id, _) = handles
            .store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Create four tasks under the same issue
        let mut task1 = sample_session();
        task1.spawned_from = Some(issue_id.clone());
        let (task1_id, _) = handles
            .store
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task2 = sample_session();
        task2.spawned_from = Some(issue_id.clone());
        let (task2_id, _) = handles
            .store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task3 = sample_session();
        task3.spawned_from = Some(issue_id.clone());
        let (task3_id, _) = handles
            .store
            .add_session(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task4 = sample_session();
        task4.spawned_from = Some(issue_id.clone());
        let (task4_id, _) = handles
            .store
            .add_session(task4, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Transition task2 to Running
        handles
            .state
            .transition_task_to_pending(&task2_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_running(&task2_id, ActorRef::test())
            .await
            .unwrap();

        // Transition task3 to Complete
        handles
            .state
            .transition_task_to_pending(&task3_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_running(&task3_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_completion(
                &task3_id,
                Ok(()),
                Some("done".into()),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Transition task4 to Failed
        handles
            .state
            .transition_task_to_pending(&task4_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_running(&task4_id, ActorRef::test())
            .await
            .unwrap();
        handles
            .state
            .transition_task_to_completion(
                &task4_id,
                Err(TaskError::JobEngineError {
                    reason: "error".to_string(),
                }),
                None,
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Filter by multiple statuses: Created and Running
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![Status::Created.into(), Status::Running.into()],
        );
        let tasks = handles.store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 2);
        let ids: Vec<_> = tasks.into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&task1_id));
        assert!(ids.contains(&task2_id));

        // Filter by single status: Complete
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Complete.into()]);
        let tasks = handles.store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, task3_id);

        // Empty status vec returns all tasks (no filter)
        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        let tasks = handles.store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 4);

        // Filter by three statuses: Running, Complete, Failed
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![
                Status::Running.into(),
                Status::Complete.into(),
                Status::Failed.into(),
            ],
        );
        let tasks = handles.store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 3);
        let ids: Vec<_> = tasks.into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&task2_id));
        assert!(ids.contains(&task3_id));
        assert!(ids.contains(&task4_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn documents_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (doc_id, _) = store
            .add_document(sample_document("docs/guide.md"), &ActorRef::test())
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
    async fn list_documents_filters_by_ids_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let (a, _) = store
            .add_document(sample_document("docs/a.md"), &ActorRef::test())
            .await
            .unwrap();
        let (b, _) = store
            .add_document(sample_document("docs/b.md"), &ActorRef::test())
            .await
            .unwrap();
        let (_c, _) = store
            .add_document(sample_document("notes/c.md"), &ActorRef::test())
            .await
            .unwrap();

        // (a) exact id match.
        let mut query = SearchDocumentsQuery::default();
        query.ids = vec![a.clone(), b.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        let mut found_ids: Vec<DocumentId> = filtered.iter().map(|d| d.0.clone()).collect();
        found_ids.sort();
        let mut expected = vec![a.clone(), b.clone()];
        expected.sort();
        assert_eq!(found_ids, expected);

        // (b) ids intersected with path_prefix.
        let mut query = SearchDocumentsQuery::new(None, Some("/docs/".to_string()), None, None);
        query.ids = vec![a.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, a);

        // ids that don't intersect with the path filter return no rows.
        let mut query = SearchDocumentsQuery::new(None, Some("/notes/".to_string()), None, None);
        query.ids = vec![a.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        assert!(filtered.is_empty());

        // (c) empty ids vec behaves like the field is absent.
        let all = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
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

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_round_trip_merge_policy_some_v2(pool: PgStorePool) {
        use hydra_common::Principal;
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::repositories::{AssigneeRef, MergePolicy, MergerRule, ReviewerGroup};

        let store = PostgresStoreV2::new(pool);
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
        let mut config = sample_repository_config();
        let static_user = |name: &str| {
            AssigneeRef::Static(Principal::User {
                name: ApiUsername::try_new(name).unwrap(),
            })
        };
        config.merge_policy = Some(MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("core".to_string()),
                any_of: vec![static_user("ada"), static_user("grace")],
                count: 1,
                exclude_author: true,
            }],
            mergers: Some(MergerRule {
                any_of: vec![static_user("ada")],
            }),
        });

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.item.merge_policy, config.merge_policy);

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1.item.merge_policy, config.merge_policy);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn repository_round_trip_merge_policy_none_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
        let config = sample_repository_config();
        assert!(config.merge_policy.is_none());

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert!(fetched.item.merge_policy.is_none());
        assert_eq!(fetched.item, config);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn migration_adds_merge_policy_column_to_repositories_v2(pool: PgStorePool) {
        let row: (String, String, String) = sqlx::query_as(
            "SELECT column_name, data_type, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = 'metis' \
               AND table_name = 'repositories_v2' \
               AND column_name = 'merge_policy'",
        )
        .fetch_one(&pool)
        .await
        .expect("merge_policy column should exist on metis.repositories_v2 after migrations");

        assert_eq!(row.1, "jsonb", "merge_policy should be JSONB");
        assert_eq!(row.2, "YES", "merge_policy should be nullable");
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn task_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let task = sample_session_all_fields();

        let now = truncate_to_micros(Utc::now());
        let (task_id, _) = store
            .add_session(task.clone(), now, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // add_task sets creation_time on the stored task
        let mut expected = task.clone();
        expected.creation_time = Some(now);
        assert_eq!(
            fetched.item, expected,
            "Session must round-trip all fields (creator, secrets, image, model, etc.)"
        );
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn patch_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let patch = sample_patch_all_fields();

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
            "Issue must round-trip all fields (assignee, job_settings, dependencies, patches)"
        );
    }

    /// Round-trip serialization: add then get; fetched entity must equal original.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_serialization_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let doc = sample_document("docs/roundtrip.md");

        let (doc_id, _) = store
            .add_document(doc.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(
            fetched.item, doc,
            "Document must round-trip all fields (path)"
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

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn document_search_only_matches_latest_version_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create a document with title "original_title"
        let doc = Document {
            title: "original_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".parse().unwrap()),
            deleted: false,
        };
        let (doc_id, _) = store.add_document(doc, &ActorRef::test()).await.unwrap();

        // Update the document to change the title to "changed_title"
        let updated_doc = Document {
            title: "changed_title".to_string(),
            body_markdown: "Body content".to_string(),
            path: Some("docs/test.md".parse().unwrap()),
            deleted: false,
        };
        store
            .update_document(&doc_id, updated_doc, &ActorRef::test())
            .await
            .unwrap();

        // Search for the old title - should return NO results
        let old_query =
            SearchDocumentsQuery::new(Some("original_title".to_string()), None, None, None);
        let old_results = store.list_documents(&old_query).await.unwrap();
        assert!(
            old_results.is_empty(),
            "Search for old title should return no results, but got {:?}",
            old_results.iter().map(|(id, _)| id).collect::<Vec<_>>()
        );

        // Search for the new title - should return the document
        let new_query =
            SearchDocumentsQuery::new(Some("changed_title".to_string()), None, None, None);
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
            status("open"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            None,
        );
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        // Update the issue to change the description
        let updated_issue = Issue::new(
            IssueType::Task,
            "Updated Title".to_string(),
            "changed_unique_description_xyz789".to_string(),
            Username::from("creator"),
            String::new(),
            status("open"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            None,
        );
        store
            .update_issue(&issue_id, updated_issue, &ActorRef::test())
            .await
            .unwrap();

        // Search for the old description - should return NO results
        let old_query = SearchIssuesQuery::new(
            None,
            vec![],
            None,
            Some("original_unique_description_abc123".to_string()),
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
            vec![],
            None,
            Some("changed_unique_description_xyz789".to_string()),
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
    async fn list_patches_filters_by_ids_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let mut ids = Vec::new();
        for title in ["alpha", "beta", "gamma"] {
            let mut patch = sample_patch();
            patch.title = title.to_string();
            let (id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();
            ids.push(id);
        }

        let mut single_query = SearchPatchesQuery::new(None, None, vec![], None);
        single_query.ids = vec![ids[0].clone()];
        let single = store.list_patches(&single_query).await.unwrap();
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].0, ids[0]);

        let mut pair_query = SearchPatchesQuery::new(None, None, vec![], None);
        pair_query.ids = vec![ids[2].clone(), ids[0].clone()];
        let pair = store.list_patches(&pair_query).await.unwrap();
        let returned: HashSet<PatchId> = pair.into_iter().map(|(id, _)| id).collect();
        let expected: HashSet<PatchId> = [ids[0].clone(), ids[2].clone()].into_iter().collect();
        assert_eq!(returned, expected);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_patches_filters_by_repo_name_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let mut patch_a = sample_patch();
        patch_a.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_a.status = PatchStatus::Open;
        let (patch_a_id, _) = store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_b.status = PatchStatus::Closed;
        let (patch_b_id, _) = store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let mut patch_c = sample_patch();
        patch_c.service_repo_name = RepoName::from_str("dourolabs/other").unwrap();
        store.add_patch(patch_c, &ActorRef::test()).await.unwrap();

        // (a) exact repo_name match returns only matching patches.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/hydra".to_string());
        let returned: HashSet<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let expected: HashSet<PatchId> = [patch_a_id.clone(), patch_b_id.clone()]
            .into_iter()
            .collect();
        assert_eq!(returned, expected);

        // (b) repo_name AND-intersects with status.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/hydra".to_string());
        query.status = vec![hydra_common::api::v1::patches::PatchStatus::Open];
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, patch_a_id);

        // (c) non-matching repo_name returns nothing.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/missing".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert!(filtered.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_patches_filters_by_creator_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let mut patch_a = sample_patch();
        patch_a.creator = Username::from("Alice");
        patch_a.status = PatchStatus::Open;
        let (patch_a_id, _) = store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.creator = Username::from("alice");
        patch_b.status = PatchStatus::Closed;
        let (patch_b_id, _) = store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let mut patch_c = sample_patch();
        patch_c.creator = Username::from("bob");
        store.add_patch(patch_c, &ActorRef::test()).await.unwrap();

        // (a) case-insensitive creator match returns both alice patches.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("ALICE".to_string());
        let returned: HashSet<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let expected: HashSet<PatchId> = [patch_a_id.clone(), patch_b_id.clone()]
            .into_iter()
            .collect();
        assert_eq!(returned, expected);

        // (b) creator AND-intersects with status.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("alice".to_string());
        query.status = vec![hydra_common::api::v1::patches::PatchStatus::Open];
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, patch_a_id);

        // (c) non-matching creator returns nothing.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("carol".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert!(filtered.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_patches_empty_string_filter_is_noop_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let mut patch_a = sample_patch();
        patch_a.creator = Username::from("alice");
        patch_a.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_a.branch_name = Some("feature/foo".to_string());
        store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.creator = Username::from("bob");
        patch_b.service_repo_name = RepoName::from_str("dourolabs/other").unwrap();
        patch_b.branch_name = Some("feature/bar".to_string());
        store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let baseline = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap()
            .len();
        assert_eq!(baseline, 2);

        // Each empty-string filter individually is a no-op.
        for field in ["creator", "repo_name", "branch_name"] {
            let mut query = SearchPatchesQuery::default();
            match field {
                "creator" => query.creator = Some(String::new()),
                "repo_name" => query.repo_name = Some(String::new()),
                "branch_name" => query.branch_name = Some(String::new()),
                _ => unreachable!(),
            }
            let filtered = store.list_patches(&query).await.unwrap();
            assert_eq!(filtered.len(), baseline, "empty {field} should be a no-op");
        }

        // All three empty filters combined is also a no-op.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some(String::new());
        query.repo_name = Some(String::new());
        query.branch_name = Some(String::new());
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), baseline);

        // Whitespace-only values are likewise a no-op after trim.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("   ".to_string());
        query.repo_name = Some("   ".to_string());
        query.branch_name = Some("   ".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), baseline);
    }

    // ---- Agent helpers & round-trip tests ----

    fn sample_agent() -> Agent {
        Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt.md".to_string(),
            Some("/agents/test-agent/mcp-config.json".to_string()),
            3,
            5,
            false,
            false,
            Vec::new(),
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
        assert!(!fetched.is_default_conversation_agent);
        assert!(!fetched.deleted);

        // UPDATE — change prompt_path, max_tries, max_simultaneous
        let updated = Agent::new(
            "test-agent".to_string(),
            "/agents/test-agent/prompt_v2.md".to_string(),
            None,
            5,
            10,
            false,
            false,
            Vec::new(),
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

    // Role-flag uniqueness (`is_assignment_agent`,
    // `is_default_conversation_agent`) is workflow state and is enforced by
    // the `agent_role_uniqueness` `Restriction` in `AppState`, not at the
    // store layer. This test exists to keep that boundary explicit: a direct
    // store insert of a second role-flagged agent must succeed.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn store_does_not_enforce_role_uniqueness_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let agent_a = Agent::new(
            "agent-a".to_string(),
            "/agents/a/prompt.md".to_string(),
            None,
            3,
            5,
            true,
            true,
            Vec::new(),
        );
        store.add_agent(agent_a).await.unwrap();

        let agent_b = Agent::new(
            "agent-b".to_string(),
            "/agents/b/prompt.md".to_string(),
            None,
            3,
            5,
            true,
            true,
            Vec::new(),
        );
        store
            .add_agent(agent_b)
            .await
            .expect("store layer should not enforce role-flag uniqueness");
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
            None,
            7,
            12,
            false,
            false,
            Vec::new(),
        );
        store.add_agent(reactivated).await.unwrap();

        // Get the agent — verify it has the new field values and deleted = false
        let fetched = store.get_agent("test-agent").await.unwrap();
        assert_eq!(fetched.name, "test-agent");
        assert_eq!(fetched.prompt_path, "/agents/test-agent/prompt_new.md");
        assert_eq!(fetched.max_tries, 7);
        assert_eq!(fetched.max_simultaneous, 12);
        assert!(!fetched.is_assignment_agent);
        assert!(!fetched.is_default_conversation_agent);
        assert!(!fetched.deleted);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_secrets_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Create agent with secrets
        let agent = Agent::new(
            "swe".to_string(),
            "/agents/swe/prompt.md".to_string(),
            None,
            3,
            i32::MAX,
            false,
            false,
            vec!["OPENAI_API_KEY".to_string(), "GITHUB_TOKEN".to_string()],
        );
        store.add_agent(agent).await.unwrap();

        // GET — verify secrets persisted
        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(
            fetched.secrets,
            vec!["OPENAI_API_KEY".to_string(), "GITHUB_TOKEN".to_string()]
        );

        // UPDATE — change secrets
        let mut updated = fetched;
        updated.secrets = vec!["NEW_SECRET".to_string()];
        store.update_agent(updated).await.unwrap();

        let fetched2 = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched2.secrets, vec!["NEW_SECRET".to_string()]);

        // LIST — verify secrets appear in list results
        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].secrets, vec!["NEW_SECRET".to_string()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn agent_default_secrets_is_empty_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        store.add_agent(sample_agent()).await.unwrap();

        let fetched = store.get_agent("test-agent").await.unwrap();
        assert!(fetched.secrets.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn auth_token_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // GET — empty for unknown actor
        let hashes = store.get_auth_token_hashes("users/nobody").await.unwrap();
        assert!(hashes.is_empty());

        // ADD — two tokens for alice
        let alice = Username::from("alice");
        let bob = Username::from("bob");
        store
            .add_auth_token("users/alice", "hash1", None, &alice)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash2", None, &alice)
            .await
            .unwrap();

        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string(), "hash2".to_string()]);

        // ADD — duplicate insert is idempotent
        store
            .add_auth_token("users/alice", "hash1", None, &alice)
            .await
            .unwrap();
        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string(), "hash2".to_string()]);

        // ADD — token for a different actor
        store
            .add_auth_token("users/bob", "hash3", None, &bob)
            .await
            .unwrap();
        let bob_hashes = store.get_auth_token_hashes("users/bob").await.unwrap();
        assert_eq!(bob_hashes, vec!["hash3".to_string()]);

        // DELETE — remove all tokens for alice
        store
            .delete_auth_tokens_for_actor("users/alice")
            .await
            .unwrap();
        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert!(hashes.is_empty());

        // Bob's tokens are unaffected
        let bob_hashes = store.get_auth_token_hashes("users/bob").await.unwrap();
        assert_eq!(bob_hashes, vec!["hash3".to_string()]);

        // DELETE — non-existent actor should not error
        store
            .delete_auth_tokens_for_actor("users/nobody")
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn auth_token_session_id_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let sid = SessionId::new();
        let alice = Username::from("alice");

        // Insert one token with a session_id and one without.
        store
            .add_auth_token("agents/swe", "hash-sess", Some(&sid), &alice)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash-user", None, &alice)
            .await
            .unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("session-spawned token should be found");
        assert_eq!(row.actor_name, "agents/swe");
        assert_eq!(row.session_id, Some(sid));
        assert_eq!(row.creator, alice);

        let row = store
            .get_auth_token_by_hash("hash-user")
            .await
            .unwrap()
            .expect("user-login token should be found");
        assert_eq!(row.actor_name, "users/alice");
        assert_eq!(row.session_id, None);
        assert_eq!(row.creator, alice);

        let missing = store.get_auth_token_by_hash("nope").await.unwrap();
        assert!(missing.is_none());
    }

    /// The postgres implementation must default new rows to
    /// `is_revoked = false`, flip exactly the rows for the given
    /// session id on `revoke_auth_tokens_for_session`, and be
    /// idempotent on repeated calls / no-match calls.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn revoke_auth_tokens_for_session_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let sid = SessionId::new();
        let other_sid = SessionId::new();
        let alice = Username::from("alice");

        store
            .add_auth_token("agents/swe", "hash-sess", Some(&sid), &alice)
            .await
            .unwrap();
        store
            .add_auth_token("agents/swe", "hash-other", Some(&other_sid), &alice)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash-user", None, &alice)
            .await
            .unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("token row should exist before revoke");
        assert!(!row.is_revoked);

        store.revoke_auth_tokens_for_session(&sid).await.unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("row should survive revoke");
        assert!(row.is_revoked);

        let other = store
            .get_auth_token_by_hash("hash-other")
            .await
            .unwrap()
            .expect("sibling session token should still exist");
        assert!(!other.is_revoked);

        let user = store
            .get_auth_token_by_hash("hash-user")
            .await
            .unwrap()
            .expect("user-login token should still exist");
        assert!(!user.is_revoked);

        // Idempotent: a second revoke / revoke of a session with no
        // rows must be harmless and leave existing state untouched.
        store.revoke_auth_tokens_for_session(&sid).await.unwrap();
        store
            .revoke_auth_tokens_for_session(&SessionId::new())
            .await
            .unwrap();
        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("row should survive double revoke");
        assert!(row.is_revoked);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn secret_round_trip_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        let alice = Username::from("alice");
        let bob = Username::from("bob");

        // SET — store secrets for alice
        store
            .set_user_secret(&alice, "api-key", b"encrypted-alice-key", false)
            .await
            .unwrap();
        store
            .set_user_secret(&alice, "db-password", b"encrypted-alice-db", false)
            .await
            .unwrap();

        // SET — store a secret for bob
        store
            .set_user_secret(&bob, "api-key", b"encrypted-bob-key", false)
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
        let refs = store.list_user_secret_names(&alice).await.unwrap();
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["api-key", "db-password"]);
        assert!(refs.iter().all(|r| !r.internal));

        // LIST — verify bob's secret names
        let refs = store.list_user_secret_names(&bob).await.unwrap();
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["api-key"]);

        // UPSERT — overwrite alice's api-key
        store
            .set_user_secret(&alice, "api-key", b"encrypted-alice-key-v2", false)
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
        let refs = store.list_user_secret_names(&alice).await.unwrap();
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
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
    async fn internal_and_external_secret_coexist_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alice = Username::from("alice");

        // Set internal then external version of the same secret
        store
            .set_user_secret(&alice, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&alice, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        // get_user_secret should return the external version
        let fetched = store.get_user_secret(&alice, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"external_val".to_vec()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_user_secret_returns_internal_when_only_internal_exists_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alice = Username::from("alice");

        store
            .set_user_secret(&alice, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();

        let fetched = store.get_user_secret(&alice, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"internal_val".to_vec()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn delete_user_secret_only_removes_external_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alice = Username::from("alice");

        // Set both internal and external
        store
            .set_user_secret(&alice, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&alice, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        // Delete should only remove external
        store.delete_user_secret(&alice, "MY_SECRET").await.unwrap();

        // Should fall back to internal
        let fetched = store.get_user_secret(&alice, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"internal_val".to_vec()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_user_secret_names_deduplicates_coexisting_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let alice = Username::from("alice");

        // Set both internal and external for the same secret
        store
            .set_user_secret(&alice, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&alice, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        let refs = store.list_user_secret_names(&alice).await.unwrap();
        // Should only appear once, reported as non-internal
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "MY_SECRET");
        assert!(!refs[0].internal);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn object_relationship_round_trip_v2(pool: PgStorePool) {
        use crate::store::{ObjectKind, RelationshipType};

        let store = PostgresStoreV2::new(pool);

        // Create three issues to use as relationship endpoints.
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

        let id_a = HydraId::from(issue_a.clone());
        let id_b = HydraId::from(issue_b.clone());
        let id_c = HydraId::from(issue_c.clone());

        // ADD — create relationships
        let created = store
            .add_relationship(&id_a, &id_b, RelationshipType::ChildOf)
            .await
            .unwrap();
        assert!(created, "first insert should return true");

        let created = store
            .add_relationship(&id_a, &id_c, RelationshipType::BlockedOn)
            .await
            .unwrap();
        assert!(created);

        let created = store
            .add_relationship(&id_b, &id_c, RelationshipType::ChildOf)
            .await
            .unwrap();
        assert!(created);

        // ADD duplicate — should return false (ON CONFLICT DO NOTHING)
        let created = store
            .add_relationship(&id_a, &id_b, RelationshipType::ChildOf)
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
                .any(|r| r.target_id == id_b && r.rel_type == RelationshipType::ChildOf)
        );
        assert!(
            rels.iter()
                .any(|r| r.target_id == id_c && r.rel_type == RelationshipType::BlockedOn)
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
            .get_relationships(None, None, Some(RelationshipType::ChildOf))
            .await
            .unwrap();
        assert_eq!(rels.len(), 2);

        // GET — filter by source_id + rel_type
        let rels = store
            .get_relationships(Some(&id_a), None, Some(RelationshipType::ChildOf))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].target_id, id_b);

        // GET — filter by source_id + target_id + rel_type (exact match)
        let rels = store
            .get_relationships(Some(&id_a), Some(&id_b), Some(RelationshipType::ChildOf))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, id_a);
        assert_eq!(rels[0].target_id, id_b);
        assert_eq!(rels[0].rel_type, RelationshipType::ChildOf);
        assert_eq!(rels[0].source_kind, ObjectKind::Issue);
        assert_eq!(rels[0].target_kind, ObjectKind::Issue);

        // REMOVE — delete one relationship
        let removed = store
            .remove_relationship(&id_a, &id_b, RelationshipType::ChildOf)
            .await
            .unwrap();
        assert!(removed, "removing existing relationship should return true");

        // REMOVE again — should return false
        let removed = store
            .remove_relationship(&id_a, &id_b, RelationshipType::ChildOf)
            .await
            .unwrap();
        assert!(
            !removed,
            "removing non-existent relationship should return false"
        );

        // GET after remove — only one relationship remains for id_a
        let rels = store
            .get_relationships(Some(&id_a), None, None)
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].rel_type, RelationshipType::BlockedOn);

        // REMOVE remaining relationships
        store
            .remove_relationship(&id_a, &id_c, RelationshipType::BlockedOn)
            .await
            .unwrap();
        store
            .remove_relationship(&id_b, &id_c, RelationshipType::ChildOf)
            .await
            .unwrap();

        // GET — no relationships left
        let rels = store.get_relationships(None, None, None).await.unwrap();
        assert!(rels.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_issues_returns_total_matching_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
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
            status("open"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        store.add_issue(bug, &actor).await.unwrap();

        let closed = Issue::new(
            IssueType::Task,
            "Closed".to_string(),
            "closed task".to_string(),
            Username::from("creator"),
            String::new(),
            status("closed"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        store.add_issue(closed, &actor).await.unwrap();

        // Count all issues
        let query = SearchIssuesQuery::new(None, vec![], None, None, None);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);

        // Count only bugs
        let query = SearchIssuesQuery::new(
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            None,
            None,
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);

        // Count only closed
        let query = SearchIssuesQuery::new(None, vec![status("closed")], None, None, None);
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_patches_returns_total_matching_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        for _ in 0..3 {
            store.add_patch(sample_patch(), &actor).await.unwrap();
        }

        let query = SearchPatchesQuery::new(None, None, Vec::new(), None);
        assert_eq!(store.count_patches(&query).await.unwrap(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_issues_filters_by_assignee_principal_v2(pool: PgStorePool) {
        use hydra_common::api::v1::agents::AgentName;
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::principal::Principal;

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let mut agent_issue = sample_issue(vec![]);
        agent_issue.assignee = Some(Principal::Agent {
            name: AgentName::try_new("swe").unwrap(),
        });
        store.add_issue(agent_issue, &actor).await.unwrap();

        let mut user_issue = sample_issue(vec![]);
        user_issue.assignee = Some(Principal::User {
            name: ApiUsername::try_new("alice").unwrap(),
        });
        store.add_issue(user_issue, &actor).await.unwrap();

        let query = SearchIssuesQuery::new(
            None,
            vec![],
            Some(Principal::Agent {
                name: AgentName::try_new("swe").unwrap(),
            }),
            None,
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_patches_filters_by_creator_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let patch_a = Patch::new(
            "patch a".to_string(),
            "patch a".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("alice"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        );
        store.add_patch(patch_a, &actor).await.unwrap();

        let patch_b = Patch::new(
            "patch b".to_string(),
            "patch b".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Username::from("bob"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        );
        store.add_patch(patch_b, &actor).await.unwrap();

        let mut query = SearchPatchesQuery::new(None, None, Vec::new(), None);
        query.creator = Some("alice".to_string());
        assert_eq!(store.count_patches(&query).await.unwrap(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_documents_returns_total_matching_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        store
            .add_document(sample_document("docs/a.md"), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document("docs/b.md"), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document("other/c.md"), &actor)
            .await
            .unwrap();

        // Count all
        let query = SearchDocumentsQuery::new(None, None, None, None);
        assert_eq!(store.count_documents(&query).await.unwrap(), 3);

        // Count with path prefix filter
        let query = SearchDocumentsQuery::new(Some("docs/".to_string()), None, None, None);
        assert_eq!(store.count_documents(&query).await.unwrap(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_tasks_returns_total_matching_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        for _ in 0..4 {
            store
                .add_session(sample_session(), Utc::now(), &actor)
                .await
                .unwrap();
        }

        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        assert_eq!(store.count_sessions(&query).await.unwrap(), 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_labels_returns_total_matching_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let default_color: hydra_common::Rgb = "#000000".parse().unwrap();

        store
            .add_label(Label::new(
                "bug".to_string(),
                default_color.clone(),
                true,
                false,
            ))
            .await
            .unwrap();
        store
            .add_label(Label::new(
                "feature".to_string(),
                default_color.clone(),
                true,
                false,
            ))
            .await
            .unwrap();
        store
            .add_label(Label::new("bugfix".to_string(), default_color, true, false))
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

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn count_issues_ignores_pagination_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        for _ in 0..5 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Count should return 5 even when limit is set
        let mut query = SearchIssuesQuery::new(None, vec![], None, None, None);
        query.limit = Some(2);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_issues_returns_latest_version_with_pagination_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        // Create 3 issues with small time gaps so created_at ordering is deterministic.
        let mut ids = Vec::new();
        for _ in 0..3 {
            let (id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
            ids.push(id);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Update the first issue twice so it has 3 versions.
        let mut updated = sample_issue(vec![]);
        updated.progress = "v2 progress".to_string();
        store.update_issue(&ids[0], updated, &actor).await.unwrap();

        let mut updated = sample_issue(vec![]);
        updated.progress = "v3 progress".to_string();
        updated.status = status("in-progress");
        store.update_issue(&ids[0], updated, &actor).await.unwrap();

        // list_issues should return 3 issues, each at their latest version.
        let query = SearchIssuesQuery::new(None, vec![], None, None, None);
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 3);

        // The first issue (ids[0]) should reflect the latest update.
        let first = results.iter().find(|(id, _)| *id == ids[0]).unwrap();
        assert_eq!(first.1.item.progress, "v3 progress");
        assert_eq!(first.1.item.status, status("in-progress"));
        assert_eq!(first.1.version, 3);

        // Paginate with limit=2 and verify we get 2 results, then use cursor
        // to get the remaining 1.
        let mut query = SearchIssuesQuery::new(None, vec![], None, None, None);
        query.limit = Some(2);
        let page1 = store.list_issues(&query).await.unwrap();
        // limit+1 is fetched internally but caller sees at most limit+1 rows;
        // the handler trims, so we just check we got results.
        assert!(page1.len() >= 2);

        // count_issues should still return the total (3), not affected by versions.
        let count_query = SearchIssuesQuery::new(None, vec![], None, None, None);
        assert_eq!(store.count_issues(&count_query).await.unwrap(), 3);

        // Filter by status=InProgress should return only the updated issue.
        let query = SearchIssuesQuery::new(None, vec![status("in-progress")], None, None, None);
        let filtered = store.list_issues(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, ids[0]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_issues_filters_by_per_project_status_key_v2(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let mut inbox_issue = sample_issue(vec![]);
        inbox_issue.status = StatusKey::try_new("inbox").unwrap();
        let (inbox_id, _) = store.add_issue(inbox_issue, &actor).await.unwrap();

        store.add_issue(sample_issue(vec![]), &actor).await.unwrap();

        let mut query = SearchIssuesQuery::default();
        query.status = vec![StatusKey::try_new("inbox").unwrap()];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, inbox_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_issues_filters_by_project_id_v2(pool: PgStorePool) {
        use hydra_common::ProjectId;
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let project_a = ProjectId::new();
        let project_b = ProjectId::new();

        let mut issue_a = sample_issue(vec![]);
        issue_a.project_id = project_a.clone();
        let (id_a, _) = store.add_issue(issue_a, &actor).await.unwrap();

        let mut issue_b = sample_issue(vec![]);
        issue_b.project_id = project_b;
        store.add_issue(issue_b, &actor).await.unwrap();

        // Issue with no project must not match a project_id filter.
        store.add_issue(sample_issue(vec![]), &actor).await.unwrap();

        let mut query = SearchIssuesQuery::default();
        query.project_id = Some(project_a);
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id_a);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_issues_status_key_and_project_id_intersect_v2(pool: PgStorePool) {
        use hydra_common::ProjectId;
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let project = ProjectId::new();
        let other_project = ProjectId::new();

        let mut target = sample_issue(vec![]);
        target.project_id = project.clone();
        target.status = StatusKey::try_new("inbox").unwrap();
        let (target_id, _) = store.add_issue(target, &actor).await.unwrap();

        let mut other_status = sample_issue(vec![]);
        other_status.project_id = project.clone();
        other_status.status = StatusKey::try_new("triage").unwrap();
        store.add_issue(other_status, &actor).await.unwrap();

        let mut other_proj = sample_issue(vec![]);
        other_proj.project_id = other_project;
        other_proj.status = StatusKey::try_new("inbox").unwrap();
        store.add_issue(other_proj, &actor).await.unwrap();

        let mut query = SearchIssuesQuery::default();
        query.project_id = Some(project);
        query.status = vec![StatusKey::try_new("inbox").unwrap()];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, target_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn has_document_relationship_round_trip(pool: PgStorePool) {
        use crate::store::RelationshipType;

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (doc_id, _) = store
            .add_document(sample_document("test/doc.md"), &actor)
            .await
            .unwrap();

        let sid = HydraId::from(issue_id.clone());
        let did = HydraId::from(doc_id.clone());

        let created = store
            .add_relationship(&sid, &did, RelationshipType::HasDocument)
            .await
            .unwrap();
        assert!(created);

        // Retrieve filtered by HasDocument type
        let rels = store
            .get_relationships(Some(&sid), None, Some(RelationshipType::HasDocument))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, sid);
        assert_eq!(rels[0].target_id, did);
        assert_eq!(rels[0].rel_type, RelationshipType::HasDocument);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn refers_to_relationship_round_trip_conversation_to_issue(pool: PgStorePool) {
        use crate::store::{ObjectKind, RelationshipType};

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let conversation_id = hydra_common::ConversationId::new();

        let source = HydraId::from(conversation_id.clone());
        let target = HydraId::from(issue_id.clone());

        let created = store
            .add_relationship(&source, &target, RelationshipType::RefersTo)
            .await
            .unwrap();
        assert!(created);

        let rels = store
            .get_relationships(Some(&source), None, Some(RelationshipType::RefersTo))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, source);
        assert_eq!(rels[0].source_kind, ObjectKind::Conversation);
        assert_eq!(rels[0].target_id, target);
        assert_eq!(rels[0].target_kind, ObjectKind::Issue);
        assert_eq!(rels[0].rel_type, RelationshipType::RefersTo);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_issue_preserves_has_document_relationships(pool: PgStorePool) {
        use crate::store::RelationshipType;

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (doc_id, _) = store
            .add_document(sample_document("test/doc.md"), &actor)
            .await
            .unwrap();

        let source = HydraId::from(issue_id.clone());
        let target = HydraId::from(doc_id.clone());

        // Seed an externally-owned (issue, document, has-document) row.
        store
            .add_relationship(&source, &target, RelationshipType::HasDocument)
            .await
            .unwrap();

        // Update the issue with no document changes; the unmanaged row must survive.
        let mut updated = sample_issue(vec![]);
        updated.progress = "halfway".to_string();
        store
            .update_issue(&issue_id, updated, &actor)
            .await
            .unwrap();

        let rels = store
            .get_relationships(Some(&source), None, Some(RelationshipType::HasDocument))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1, "has-document row must survive issue update");
        assert_eq!(rels[0].target_id, target);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn drop_deps_patches_columns_preserves_relationships(pool: PgStorePool) {
        // Regression: after dropping issues_v2.dependencies / issues_v2.patches, the
        // store must still round-trip Issue.dependencies / Issue.patches via
        // object_relationships.
        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let (parent_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (patch_id, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let mut issue = sample_issue(vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )]);
        issue.patches = vec![patch_id.clone()];
        let (issue_id, _) = store.add_issue(issue, &actor).await.unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.dependencies.len(), 1);
        assert_eq!(fetched.item.dependencies[0].issue_id, parent_id);
        assert_eq!(
            fetched.item.dependencies[0].dependency_type,
            IssueDependencyType::ChildOf
        );
        assert_eq!(fetched.item.patches, vec![patch_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_relationships_batch_filters_by_multiple_sources(pool: PgStorePool) {
        use crate::store::RelationshipType;

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        let (id1, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id2, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id3, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let sid1 = HydraId::from(id1.clone());
        let sid2 = HydraId::from(id2.clone());
        let sid3 = HydraId::from(id3.clone());
        let tpid = HydraId::from(pid.clone());

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

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_relationships_transitive_follows_same_type_only(pool: PgStorePool) {
        use crate::store::{RelationshipType, TransitiveDirection};

        let store = PostgresStoreV2::new(pool);
        let actor = ActorRef::test();

        // Create 3 issues: A -> B -> C (child-of chain)
        // Also B -> patch (has-patch, should NOT be followed)
        let (id_a, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id_b, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (id_c, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let a = HydraId::from(id_a.clone());
        let b = HydraId::from(id_b.clone());
        let c = HydraId::from(id_c.clone());
        let p = HydraId::from(pid.clone());

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
            .get_relationships_transitive(
                &[a.clone()],
                TransitiveDirection::Forward,
                RelationshipType::ChildOf,
            )
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
            .get_relationships_transitive(
                &[c.clone()],
                TransitiveDirection::Backward,
                RelationshipType::ChildOf,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Transitive has-patch from B should only find B->P
        let results = store
            .get_relationships_transitive(
                &[b.clone()],
                TransitiveDirection::Forward,
                RelationshipType::HasPatch,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, p);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_sessions_filters_by_spawned_from_ids(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
        let handles = test_state_with_store(store.clone());

        let (issue_a, _) = handles
            .store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_b, _) = handles
            .store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_c, _) = handles
            .store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut task_a = sample_session();
        task_a.spawned_from = Some(issue_a.clone());
        let (task_a_id, _) = handles
            .store
            .add_session(task_a, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b = sample_session();
        task_b.spawned_from = Some(issue_b.clone());
        let (task_b_id, _) = handles
            .store
            .add_session(task_b, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_c = sample_session();
        task_c.spawned_from = Some(issue_c.clone());
        handles
            .store
            .add_session(task_c, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Filter by spawned_from_ids should return only matching sessions
        let mut query = SearchSessionsQuery::default();
        query.spawned_from_ids = vec![issue_a.clone(), issue_b.clone()];
        let sessions: HashSet<_> = handles
            .store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(sessions, HashSet::from([task_a_id, task_b_id]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_sessions_filters_by_creator(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
        let handles = test_state_with_store(store.clone());

        let mut task_alice = sample_session();
        task_alice.creator = Username::from("alice");
        let (alice_id, _) = handles
            .store
            .add_session(task_alice, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_bob = sample_session();
        task_bob.creator = Username::from("bob");
        handles
            .store
            .add_session(task_bob, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.creator = Some("alice".to_string());
        let sessions: HashSet<_> = handles
            .store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(sessions, HashSet::from([alice_id]));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_sessions_filters_by_conversation_id(pool: PgStorePool) {
        let store = Arc::new(PostgresStoreV2::new(pool));
        let handles = test_state_with_store(store.clone());

        let conv_a = ConversationId::new();
        let conv_b = ConversationId::new();

        let mut task_a = sample_session();
        task_a.mode = crate::domain::sessions::SessionMode::Interactive {
            conversation_id: conv_a.clone(),
            idle_timeout_secs: None,
            greet_user: false,
        };
        let (task_a_id, _) = handles
            .store
            .add_session(task_a, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b = sample_session();
        task_b.mode = crate::domain::sessions::SessionMode::Interactive {
            conversation_id: conv_b.clone(),
            idle_timeout_secs: None,
            greet_user: false,
        };
        handles
            .store
            .add_session(task_b, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Non-interactive session (no `interactive`, so no conversation link).
        handles
            .store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(conv_a.clone());
        let sessions: HashSet<_> = handles
            .store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(sessions, HashSet::from([task_a_id]));

        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(ConversationId::new());
        let sessions = handles.store.list_sessions(&query).await.unwrap();
        assert!(sessions.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn conversation_round_trip_v2(pool: PgStorePool) {
        use crate::domain::conversations::{Conversation, ConversationStatus};
        use hydra_common::api::v1::conversations::SearchConversationsQuery;

        let store = PostgresStoreV2::new(pool);

        // -- Add a conversation --
        let conv = Conversation {
            title: Some("Test conversation".to_string()),
            agent_name: Some(
                hydra_common::api::v1::agents::AgentName::try_new("test-agent").unwrap(),
            ),
            status: ConversationStatus::Active,
            creator: Username::from("alice"),
            session_settings: SessionSettings {
                repo_name: Some(RepoName::from_str("org/repo").unwrap()),
                remote_url: Some("https://git.example.com/org/repo.git".to_string()),
                image: Some("img:v1".to_string()),
                model: Some("claude-3".to_string()),
                branch: Some("main".to_string()),
                max_retries: Some(3),
                cpu_limit: Some("2".to_string()),
                memory_limit: Some("4Gi".to_string()),
                secrets: Some(vec!["conv-secret".to_string()]),
            },
            spawned_from: None,
            deleted: false,
        };
        let (conv_id, version) = store
            .add_conversation(conv.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        // -- Fetch and verify fields --
        let fetched = store.get_conversation(&conv_id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.title, conv.title);
        assert_eq!(fetched.item.agent_name, conv.agent_name);
        assert_eq!(fetched.item.status, ConversationStatus::Active);
        assert_eq!(fetched.item.creator, conv.creator);
        assert_eq!(fetched.item.session_settings, conv.session_settings);
        assert!(!fetched.item.deleted);

        // -- List conversations --
        let list = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        let ids: Vec<_> = list.iter().map(|(id, _)| id.clone()).collect();
        assert!(ids.contains(&conv_id));

        // -- Update conversation --
        let updated_conv = Conversation {
            title: Some("Updated title".to_string()),
            agent_name: conv.agent_name.clone(),
            status: ConversationStatus::Idle,
            creator: conv.creator.clone(),
            session_settings: Default::default(),
            spawned_from: None,
            deleted: false,
        };
        let v2 = store
            .update_conversation(&conv_id, updated_conv.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let fetched2 = store.get_conversation(&conv_id, false).await.unwrap();
        assert_eq!(fetched2.version, 2);
        assert_eq!(fetched2.item.title.as_deref(), Some("Updated title"));
        assert_eq!(fetched2.item.status, ConversationStatus::Idle);

        // -- get_conversation_versions returns one row per update --
        let versions = store.get_conversation_versions(&conv_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        let statuses: Vec<_> = versions.iter().map(|v| v.item.status).collect();
        assert_eq!(
            statuses,
            vec![ConversationStatus::Active, ConversationStatus::Idle]
        );

        // -- Deletion filtering --
        let deleted_conv = Conversation {
            title: updated_conv.title.clone(),
            agent_name: updated_conv.agent_name.clone(),
            status: updated_conv.status,
            creator: updated_conv.creator.clone(),
            session_settings: Default::default(),
            spawned_from: None,
            deleted: true,
        };
        store
            .update_conversation(&conv_id, deleted_conv, &ActorRef::test())
            .await
            .unwrap();

        // get_conversation should return NotFound for deleted conversations (include_deleted=false)
        let result = store.get_conversation(&conv_id, false).await;
        assert!(result.is_err());

        // get_conversation with include_deleted=true should still return it
        let result = store.get_conversation(&conv_id, true).await;
        assert!(result.is_ok());

        // list_conversations should exclude deleted conversations
        let list_after = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        let ids_after: Vec<_> = list_after.iter().map(|(id, _)| id.clone()).collect();
        assert!(!ids_after.contains(&conv_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn conversation_round_trips_spawned_from_v2(pool: PgStorePool) {
        use hydra_common::IssueId;
        use std::str::FromStr;
        let store = PostgresStoreV2::new(pool);
        let issue_id = IssueId::from_str("i-spawnz").unwrap();
        let mut conv = sample_conversation("alice");
        conv.spawned_from = Some(issue_id.clone());
        let (id, _) = store
            .add_conversation(conv, &ActorRef::test())
            .await
            .unwrap();
        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_eq!(fetched.item.spawned_from, Some(issue_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_conversations_filters_by_spawned_from_v2(pool: PgStorePool) {
        use hydra_common::IssueId;
        use hydra_common::api::v1::conversations::SearchConversationsQuery;
        use std::str::FromStr;
        let store = PostgresStoreV2::new(pool);
        let issue_a = IssueId::from_str("i-aaaaaa").unwrap();
        let issue_b = IssueId::from_str("i-bbbbbb").unwrap();

        let mut conv_a = sample_conversation("alice");
        conv_a.spawned_from = Some(issue_a.clone());
        store
            .add_conversation(conv_a, &ActorRef::test())
            .await
            .unwrap();

        let mut conv_b = sample_conversation("alice");
        conv_b.spawned_from = Some(issue_b.clone());
        store
            .add_conversation(conv_b, &ActorRef::test())
            .await
            .unwrap();

        store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();

        let all = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);

        let results = store
            .list_conversations(&SearchConversationsQuery {
                spawned_from: Some(issue_a.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.spawned_from, Some(issue_a));
    }

    async fn insert_dummy_latest_sessions(store: &PostgresStoreV2, start: usize, count: usize) {
        for i in start..(start + count) {
            let id = format!("s-dummyaa{i:08}");
            sqlx::query(&format!(
                "INSERT INTO {TABLE_TASKS_V2} (id, version_number, env_vars, status, deleted, creator, mount_spec, agent_config, mode, is_latest)
                 VALUES ($1, 1, '{{}}'::jsonb, 'complete', false, '', '{{\"working_dir\":\"repo\",\"mounts\":[]}}'::jsonb, '{{}}'::jsonb, '{{\"type\":\"headless\",\"prompt\":\"\"}}'::jsonb, true)"
            ))
            .bind(&id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
    }

    async fn insert_dummy_latest_patches(store: &PostgresStoreV2, start: usize, count: usize) {
        for i in start..(start + count) {
            let id = format!("p-dumyaa{i:08}");
            sqlx::query(&format!(
                "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, creator, service_repo_name, deleted, is_latest)
                 VALUES ($1, 1, '', '', '', 'Open', false, '', 'dourolabs/sample', false, true)"
            ))
            .bind(&id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_patch_grows_id_suffix_with_table_size(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // pg_class.reltuples is only refreshed by ANALYZE — see the
        // sibling session test for the rationale.
        sqlx::query(&format!("ANALYZE {TABLE_PATCHES_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - PatchId::prefix().len(),
            6,
            "fresh table should use default suffix length"
        );

        insert_dummy_latest_patches(&store, 0, 26).await; // total = 27
        sqlx::query(&format!("ANALYZE {TABLE_PATCHES_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - PatchId::prefix().len(),
            6,
            "27 rows should still use default 6-char suffix"
        );

        insert_dummy_latest_patches(&store, 26, 649).await; // total = 677
        sqlx::query(&format!("ANALYZE {TABLE_PATCHES_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - PatchId::prefix().len(),
            7,
            "677 rows should bump suffix length to 7"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_session_grows_id_suffix_with_table_size(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // pg_class.reltuples is only refreshed by ANALYZE (and autovacuum),
        // not by INSERT — so the test runs ANALYZE after every bulk insert.
        sqlx::query(&format!("ANALYZE {TABLE_TASKS_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "fresh table should use default suffix length"
        );

        insert_dummy_latest_sessions(&store, 0, 26).await; // total = 27
        sqlx::query(&format!("ANALYZE {TABLE_TASKS_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "27 rows should still use default 6-char suffix"
        );

        insert_dummy_latest_sessions(&store, 26, 649).await; // total = 677
        sqlx::query(&format!("ANALYZE {TABLE_TASKS_V2}"))
            .execute(&store.pool)
            .await
            .unwrap();
        let (id, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            7,
            "677 rows should bump suffix length to 7"
        );
    }

    // ---- Session event log + state ----

    fn interactive_session(conversation_id: Option<ConversationId>) -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        let mode = match conversation_id {
            Some(cid) => SessionMode::Interactive {
                conversation_id: cid,
                idle_timeout_secs: None,
                greet_user: false,
            },
            None => SessionMode::Headless,
        };
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("hydra-worker:latest".to_string()),
            Default::default(),
            None,
            None,
            None,
            mode,
            Status::Created,
            None,
            None,
        )
    }

    fn sample_conversation(creator: &str) -> crate::domain::conversations::Conversation {
        crate::domain::conversations::Conversation {
            title: Some("Test conversation".to_string()),
            agent_name: None,
            status: crate::domain::conversations::ConversationStatus::Active,
            creator: Username::from(creator),
            session_settings: Default::default(),
            spawned_from: None,
            deleted: false,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_events_append_then_reload_in_insertion_order_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (sid, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // No events yet.
        let events = store.get_session_events(&sid).await.unwrap();
        assert!(events.is_empty());

        let e1 = SessionEvent::UserMessage {
            content: "hello".to_string(),
            timestamp: Utc::now(),
        };
        let v1 = store
            .append_session_event(&sid, e1.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v1, VersionNumber::from(1u64));

        let e2 = SessionEvent::AssistantMessage {
            content: "world".to_string(),
            timestamp: Utc::now(),
        };
        let v2 = store
            .append_session_event(&sid, e2.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, VersionNumber::from(2u64));

        let e3 = SessionEvent::Closed {
            timestamp: Utc::now(),
        };
        let v3 = store
            .append_session_event(&sid, e3.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v3, VersionNumber::from(3u64));

        let events = store.get_session_events(&sid).await.unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].item, e1);
        assert_eq!(events[1].item, e2);
        assert_eq!(events[2].item, e3);
        assert_eq!(events[0].version, VersionNumber::from(1u64));
        assert_eq!(events[1].version, VersionNumber::from(2u64));
        assert_eq!(events[2].version, VersionNumber::from(3u64));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_events_get_returns_not_found_for_missing_session_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let missing = SessionId::generate(6).unwrap();
        let err = store.get_session_events(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_events_append_to_missing_session_errors_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let missing = SessionId::generate(6).unwrap();
        let err = store
            .append_session_event(
                &missing,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_event_id_is_monotonic_across_sessions_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (sid_a, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (sid_b, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Interleave appends across two sessions.
        for i in 0..3 {
            store
                .append_session_event(
                    &sid_a,
                    SessionEvent::UserMessage {
                        content: format!("a-{i}"),
                        timestamp: Utc::now(),
                    },
                    &ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .append_session_event(
                    &sid_b,
                    SessionEvent::UserMessage {
                        content: format!("b-{i}"),
                        timestamp: Utc::now(),
                    },
                    &ActorRef::test(),
                )
                .await
                .unwrap();
        }

        // The `id` BIGSERIAL column must be strictly increasing across all
        // rows (cross-session insertion order). This is what §3.4.1's merge
        // relies on.
        let ids: Vec<i64> = sqlx::query_scalar(&format!(
            "SELECT id FROM {TABLE_SESSION_EVENTS_V2} ORDER BY id ASC"
        ))
        .fetch_all(&store.pool)
        .await
        .unwrap();
        assert_eq!(ids.len(), 6);
        for window in ids.windows(2) {
            assert!(window[0] < window[1], "id must strictly increase: {ids:?}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_state_empty_present_overwrite_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (sid, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Empty.
        let state = store.get_session_state(&sid).await.unwrap();
        assert!(state.is_none());

        // Store + read.
        let data1 = vec![0xDE, 0xAD, 0xBE, 0xEF];
        store
            .store_session_state(&sid, data1.clone(), &ActorRef::test())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data1));

        // Overwrite.
        let data2 = vec![0xCA, 0xFE];
        store
            .store_session_state(&sid, data2.clone(), &ActorRef::test())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data2));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_state_missing_session_errors_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let missing = SessionId::generate(6).unwrap();

        let err = store.get_session_state(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));

        let err = store
            .store_session_state(&missing, vec![1, 2, 3], &ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn session_event_summaries_returns_counts_and_previews_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (sid_a, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (sid_b, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (sid_empty, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        store
            .append_session_event(
                &sid_a,
                SessionEvent::UserMessage {
                    content: "hi a".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_a,
                SessionEvent::AssistantMessage {
                    content: "bye a".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_b,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_session_event_summaries(&[sid_a.clone(), sid_b.clone(), sid_empty.clone()])
            .await
            .unwrap();
        assert_eq!(summaries.len(), 2);
        let a = summaries.get(&sid_a).unwrap();
        assert_eq!(a.event_count, 2);
        assert_eq!(a.last_event_preview.as_deref(), Some("Assistant: bye a"));
        let b = summaries.get(&sid_b).unwrap();
        assert_eq!(b.event_count, 1);
        assert_eq!(b.last_event_preview.as_deref(), Some("Closed"));
        assert!(!summaries.contains_key(&sid_empty));

        // Empty input returns empty map.
        let empty = store.get_session_event_summaries(&[]).await.unwrap();
        assert!(empty.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn conversation_event_summaries_sources_preview_from_chat_text_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (conv_user_only, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let (conv_user_then_assistant, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let (conv_cross_session, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let (conv_empty, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();

        // Single UserMessage in a single session.
        let (sid_user_only, _) = store
            .add_session(
                interactive_session(Some(conv_user_only.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_user_only,
                SessionEvent::UserMessage {
                    content: "hello".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // UserMessage then AssistantMessage in one session — Assistant wins.
        let (sid_chat, _) = store
            .add_session(
                interactive_session(Some(conv_user_then_assistant.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_chat,
                SessionEvent::UserMessage {
                    content: "hi".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_chat,
                SessionEvent::AssistantMessage {
                    content: "hey".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Chat-text in older session, only tool/lifecycle in newer session.
        let t_old = Utc::now() - chrono::Duration::seconds(60);
        let (sid_old, _) = store
            .add_session(
                interactive_session(Some(conv_cross_session.clone())),
                t_old,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_old,
                SessionEvent::UserMessage {
                    content: "from old".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (sid_new, _) = store
            .add_session(
                interactive_session(Some(conv_cross_session.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::ToolUse {
                    tool_name: "bash".to_string(),
                    payload: serde_json::json!({}),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[
                conv_user_only.clone(),
                conv_user_then_assistant.clone(),
                conv_cross_session.clone(),
                conv_empty.clone(),
            ])
            .await
            .unwrap();

        let s = summaries.get(&conv_user_only).expect("user-only conv");
        assert_eq!(s.event_count, 1);
        assert_eq!(s.last_event_preview.as_deref(), Some("User: hello"));

        let s = summaries
            .get(&conv_user_then_assistant)
            .expect("user+assistant conv");
        // 2 chat-text events across the single linked session.
        assert_eq!(s.event_count, 2);
        assert_eq!(s.last_event_preview.as_deref(), Some("Assistant: hey"));

        let s = summaries.get(&conv_cross_session).expect("cross-session");
        // Only the older session has a chat-text event; the newer session's
        // ToolUse / Closed lifecycle events don't count.
        assert_eq!(s.event_count, 1);
        assert_eq!(s.last_event_preview.as_deref(), Some("User: from old"));

        assert!(!summaries.contains_key(&conv_empty));

        // Empty input → empty output.
        let empty = store.get_conversation_event_summaries(&[]).await.unwrap();
        assert!(empty.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn conversation_event_summaries_latest_session_wins_over_older_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (conv, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let t_old = Utc::now() - chrono::Duration::seconds(60);
        let (sid_old, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                t_old,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Older session, but message wall-clock timestamp is *later* — to verify
        // ordering is by session-creation, not by per-event time.
        store
            .append_session_event(
                &sid_old,
                SessionEvent::UserMessage {
                    content: "from older session, written later".to_string(),
                    timestamp: Utc::now() + chrono::Duration::seconds(60),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (sid_new, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::AssistantMessage {
                    content: "from newer".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // Both sessions contribute one chat-text event each.
        assert_eq!(s.event_count, 2);
        assert_eq!(
            s.last_event_preview.as_deref(),
            Some("Assistant: from newer")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn conversation_event_summaries_sums_chat_text_across_sessions_v2(pool: PgStorePool) {
        // Regression test for the chat-list "Messages" column: when a
        // conversation has multiple sessions (close → resume), the count
        // must sum chat-text events across every session, not just the
        // latest. ToolUse / lifecycle events are excluded.
        let store = PostgresStoreV2::new(pool);
        let (conv, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();

        let t_old = Utc::now() - chrono::Duration::seconds(60);
        let (sid_old, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                t_old,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        for content in ["one", "two"] {
            store
                .append_session_event(
                    &sid_old,
                    SessionEvent::UserMessage {
                        content: content.to_string(),
                        timestamp: Utc::now(),
                    },
                    &ActorRef::test(),
                )
                .await
                .unwrap();
        }
        store
            .append_session_event(
                &sid_old,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let (sid_new, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::UserMessage {
                    content: "three".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::ToolUse {
                    tool_name: "bash".to_string(),
                    payload: serde_json::json!({}),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid_new,
                SessionEvent::AssistantMessage {
                    content: "four".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // 2 chat-text events on the old session + 2 on the new session = 4.
        assert_eq!(s.event_count, 4);
        assert_eq!(s.last_event_preview.as_deref(), Some("Assistant: four"));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_session_ids_by_conversation_id_orders_and_filters_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);

        // Two conversations.
        let (conv_a, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let (conv_b, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();

        let t0 = Utc::now() - chrono::Duration::minutes(10);
        let t1 = t0 + chrono::Duration::minutes(1);
        let t2 = t0 + chrono::Duration::minutes(2);
        let t3 = t0 + chrono::Duration::minutes(3);

        // Two sessions linked to conv_a, in non-creation order.
        let (s1, _) = store
            .add_session(
                interactive_session(Some(conv_a.clone())),
                t2,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (s2, _) = store
            .add_session(
                interactive_session(Some(conv_a.clone())),
                t1,
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // A session linked to conv_b — must be excluded.
        store
            .add_session(
                interactive_session(Some(conv_b.clone())),
                t0,
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // A non-interactive session (no conversation_id) — must be excluded.
        store
            .add_session(sample_session(), t0, &ActorRef::test())
            .await
            .unwrap();

        // A deleted session linked to conv_a — must be excluded by `deleted = FALSE`.
        let (s_deleted, _) = store
            .add_session(
                interactive_session(Some(conv_a.clone())),
                t3,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .delete_session(&s_deleted, &ActorRef::test())
            .await
            .unwrap();

        // Order: by creation_time ASC -> s2 before s1.
        let ids = store
            .list_session_ids_by_conversation_id(&conv_a)
            .await
            .unwrap();
        assert_eq!(ids, vec![s2.clone(), s1.clone()]);

        // Unknown conversation: empty.
        let empty = store
            .list_session_ids_by_conversation_id(&ConversationId::new())
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    // ---- Session-shape column dual-write round-trip ----
    //
    // These tests cover the postgres `INSERT` path added in the same migration
    // as `mount_spec`, `agent_config`, `mode` (see
    // `20260523020000_add_session_shape_columns.sql`). The sqlite analogues
    // live in `store/sqlite_store.rs::tests::dual_write_*`; this trio
    // exercises the same shape across the postgres backend.

    /// Fetches the three dual-written JSONB columns for a session id.
    async fn fetch_postgres_session_shape(
        store: &PostgresStoreV2,
        id: &SessionId,
    ) -> (Option<Value>, Option<Value>, Option<Value>) {
        sqlx::query_as::<_, (Option<Value>, Option<Value>, Option<Value>)>(&format!(
            "SELECT mount_spec, agent_config, mode \
             FROM {TABLE_TASKS_V2} WHERE id = $1"
        ))
        .bind(id.as_ref())
        .fetch_one(&store.pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn dual_write_headless_session_populates_mode_and_mount_spec_v2(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (sid, _) = store
            .add_session(sample_session(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let (mount_spec, agent_config, mode) = fetch_postgres_session_shape(&store, &sid).await;

        let mode = mode.expect("mode is non-null");
        assert_eq!(mode["type"], "headless");
        // Headless is unit-like — no prompt field on `mode`.
        assert!(mode.get("prompt").is_none_or(|v| v.is_null()));

        let mount_spec = mount_spec.expect("mount_spec is non-null");
        assert_eq!(mount_spec["working_dir"], "repo");
        let mounts = mount_spec["mounts"].as_array().expect("mounts is an array");
        assert_eq!(
            mounts.len(),
            2,
            "headless dual-write emits Bundle + Documents"
        );
        assert_eq!(mounts[0]["type"], "bundle");
        assert_eq!(mounts[0]["target"], "repo");
        // PR-D: `session_id` no longer rides on `MountItem::Bundle`.
        assert!(mounts[0].get("session_id").is_none_or(|v| v.is_null()));
        assert_eq!(mounts[0]["bundle"]["type"], "none");
        assert_eq!(mounts[1]["type"], "documents");
        assert_eq!(mounts[1]["target"], "documents");

        let agent_config = agent_config.expect("agent_config is non-null");
        assert!(agent_config["agent_name"].is_null());
        assert!(agent_config["system_prompt"].is_null());
        // sample_session() leaves model and mcp_config as None.
        assert!(agent_config["model"].is_null());
        assert!(agent_config["mcp_config"].is_null());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn dual_write_interactive_session_populates_mode_with_conversation_id_v2(
        pool: PgStorePool,
    ) {
        let store = PostgresStoreV2::new(pool);
        let (conv_id, _) = store
            .add_conversation(sample_conversation("alice"), &ActorRef::test())
            .await
            .unwrap();
        let (sid, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let (_, _, mode) = fetch_postgres_session_shape(&store, &sid).await;
        let mode = mode.expect("mode is non-null");
        assert_eq!(mode["type"], "interactive");
        assert_eq!(mode["conversation_id"], conv_id.as_ref());
        // `idle_timeout_secs` is omitted when None (server applies default).
        assert!(mode.get("idle_timeout_secs").is_none_or(|v| v.is_null()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn dual_write_session_with_git_bundle_carries_url_into_mount_spec_v2(pool: PgStorePool) {
        use hydra_common::api::v1::sessions::Bundle;
        let store = PostgresStoreV2::new(pool);
        let mut session = sample_session();
        let bundle = Bundle::GitRepository {
            url: "https://github.com/example/repo".to_string(),
            rev: "main".to_string(),
        };
        session.mount_spec = crate::routes::sessions::mount_spec_from_create_request(bundle, None);
        session.agent_config.model = Some("gpt-4o".to_string());

        let (sid, _) = store
            .add_session(session, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let (mount_spec, agent_config, _) = fetch_postgres_session_shape(&store, &sid).await;
        let mount_spec = mount_spec.expect("mount_spec is non-null");
        let bundle = &mount_spec["mounts"][0]["bundle"];
        assert_eq!(bundle["type"], "git_repository");
        assert_eq!(bundle["url"], "https://github.com/example/repo");
        assert_eq!(bundle["rev"], "main");

        let agent_config = agent_config.expect("agent_config is non-null");
        assert_eq!(agent_config["model"], "gpt-4o");
    }

    // ---- Trigger tests --------------------------------------------------

    fn sample_trigger_pg() -> Trigger {
        use hydra_common::api::v1::issues::{IssueType, SessionSettings};
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::test_utils::status::status;
        use hydra_common::triggers::{Action, CreateIssueAction, Schedule, Trigger as ApiTrigger};
        ApiTrigger::new(
            true,
            Schedule::Cron {
                expression: "0 9 * * MON".to_string(),
                timezone: Some("UTC".to_string()),
            },
            vec![Action::CreateIssue(CreateIssueAction::new(
                IssueType::Task,
                "Daily triage".to_string(),
                "Run triage for {{ now.date }}".to_string(),
                Some("users/alice".to_string()),
                crate::domain::projects::default_project_id(),
                status("open"),
                SessionSettings::default(),
            ))],
            ApiUsername::from("alice"),
            None,
            false,
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn trigger_round_trip_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, version) = store
            .add_trigger(sample_trigger_pg(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert!(fetched.item.enabled);
        assert_eq!(fetched.item.actions, sample_trigger_pg().actions);

        assert_eq!(store.list_triggers(false).await.unwrap().len(), 1);

        let mut updated = sample_trigger_pg();
        updated.enabled = false;
        let v2 = store
            .update_trigger(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let v3 = store.delete_trigger(&id, &ActorRef::test()).await.unwrap();
        assert_eq!(v3, 3);
        assert!(store.list_triggers(false).await.unwrap().is_empty());
        assert_eq!(store.list_triggers(true).await.unwrap().len(), 1);
        assert!(matches!(
            store.get_trigger(&id, false).await,
            Err(StoreError::TriggerNotFound(_))
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn record_trigger_fire_does_not_bump_version_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_trigger(sample_trigger_pg(), &ActorRef::test())
            .await
            .unwrap();

        let fired_at: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, fired_at).await.unwrap();

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.last_fired_at, Some(fired_at));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_after_record_trigger_fire_carries_forward_last_fired_at_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_trigger(sample_trigger_pg(), &ActorRef::test())
            .await
            .unwrap();

        let fired_at: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, fired_at).await.unwrap();

        let mut next = sample_trigger_pg();
        next.enabled = false;
        assert!(next.last_fired_at.is_none());
        store
            .update_trigger(&id, next, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 2);
        assert!(!fetched.item.enabled);
        assert_eq!(fetched.item.last_fired_at, Some(fired_at));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_with_stale_last_fired_at_does_not_regress_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_trigger(sample_trigger_pg(), &ActorRef::test())
            .await
            .unwrap();

        let t_new: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, t_new).await.unwrap();

        // Caller supplies a stale `Some(t_old)` on the update payload.
        // `update_trigger` must ignore it and overwrite with the latest
        // row's `Some(t_new)`.
        let t_old: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let mut next = sample_trigger_pg();
        next.enabled = false;
        next.last_fired_at = Some(t_old);
        store
            .update_trigger(&id, next, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 2);
        assert!(!fetched.item.enabled);
        assert_eq!(fetched.item.last_fired_at, Some(t_new));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn record_trigger_fire_not_found_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let result = store
            .record_trigger_fire(&TriggerId::new(), Utc::now())
            .await;
        assert!(matches!(result, Err(StoreError::TriggerNotFound(_))));
    }

    // ---- Project tests --------------------------------------------------

    /// Fully-populated sample, including `on_enter` so the JSONB serde
    /// path for `StatusOnEnter` is exercised end-to-end in the round-trip
    /// test.
    fn sample_project_pg() -> Project {
        use hydra_common::api::v1::projects::{
            ProjectKey, StatusDefinition, StatusKey, StatusOnEnter,
        };
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::principal::Principal;

        let statuses = vec![
            StatusDefinition::new(
                StatusKey::try_new("backlog").unwrap(),
                "Backlog".to_string(),
                "#abcdef".parse().unwrap(),
                false,
                false,
                false,
                Some(StatusOnEnter::new(
                    Some(Principal::Agent {
                        name: "reviewer".parse().unwrap(),
                    }),
                    Some("forms/review.yaml".parse().unwrap()),
                )),
            ),
            StatusDefinition::new(
                StatusKey::try_new("done").unwrap(),
                "Done".to_string(),
                "#00ff00".parse().unwrap(),
                true,
                true,
                false,
                Some(StatusOnEnter::new(
                    Some(Principal::Agent {
                        name: "swe".parse().unwrap(),
                    }),
                    None,
                )),
            ),
        ];
        Project::new(
            ProjectKey::try_new("engineering").unwrap(),
            "Engineering".to_string(),
            statuses,
            ApiUsername::from("alice"),
            false,
            0.0,
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn project_round_trip_pg(pool: PgStorePool) {
        use crate::domain::projects::default_project_id;
        let store = PostgresStoreV2::new(pool);
        let (id, version) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_project(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.name, "Engineering");
        assert_eq!(fetched.item.statuses.len(), 2);
        // `on_enter` must round-trip through the JSONB column unchanged.
        assert_eq!(fetched.item.statuses, sample_project_pg().statuses);

        // The seed migration inserts the default project, so listing
        // should yield both it and the newly-added engineering project.
        let default_id = default_project_id();
        let listed = store.list_projects(false).await.unwrap();
        assert_eq!(listed.len(), 2);
        let ids: Vec<&ProjectId> = listed.iter().map(|(i, _)| i).collect();
        assert!(ids.contains(&&id));
        assert!(ids.contains(&&default_id));

        let mut updated = sample_project_pg();
        updated.name = "Engineering Renamed".to_string();
        let v2 = store
            .update_project(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);
        assert_eq!(
            store.get_project(&id, false).await.unwrap().item.name,
            "Engineering Renamed"
        );

        let v3 = store.delete_project(&id, &ActorRef::test()).await.unwrap();
        assert_eq!(v3, 3);
        let after_delete = store.list_projects(false).await.unwrap();
        assert_eq!(after_delete.len(), 1);
        assert_eq!(after_delete[0].0, default_id);
        assert_eq!(store.list_projects(true).await.unwrap().len(), 2);
        assert!(matches!(
            store.get_project(&id, false).await,
            Err(StoreError::ProjectNotFound(_))
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_project_not_found_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let result = store.get_project(&ProjectId::new(), false).await;
        assert!(matches!(result, Err(StoreError::ProjectNotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_project_by_key_round_trip_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::ProjectKey;
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();

        let key = ProjectKey::try_new("engineering").unwrap();
        let (resolved_id, versioned) = store
            .get_project_by_key(&key, false)
            .await
            .unwrap()
            .expect("active key lookup should hit");
        assert_eq!(resolved_id, id);
        assert_eq!(versioned.item.name, "Engineering");
        assert_eq!(versioned.item.statuses.len(), 2);

        let missing = ProjectKey::try_new("does-not-exist").unwrap();
        assert!(
            store
                .get_project_by_key(&missing, false)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn get_project_by_key_respects_include_deleted_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::ProjectKey;
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();
        store.delete_project(&id, &ActorRef::test()).await.unwrap();

        let key = ProjectKey::try_new("engineering").unwrap();

        assert!(
            store
                .get_project_by_key(&key, false)
                .await
                .unwrap()
                .is_none(),
            "soft-deleted key must not surface when include_deleted: false"
        );

        let (resolved_id, versioned) = store
            .get_project_by_key(&key, true)
            .await
            .unwrap()
            .expect("soft-deleted key must surface when include_deleted: true");
        assert_eq!(resolved_id, id);
        assert!(versioned.item.deleted);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_project_not_found_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let result = store
            .update_project(&ProjectId::new(), sample_project_pg(), &ActorRef::test())
            .await;
        assert!(matches!(result, Err(StoreError::ProjectNotFound(_))));
    }

    /// `update_project` must flip the prior `is_latest` row to false and
    /// insert the new latest in one transaction. Verify there is exactly
    /// one `is_latest = true` row after the second write — the BEFORE
    /// INSERT trigger `trg_maintain_latest_projects` is responsible for
    /// the flip.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_project_maintains_single_is_latest_row_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool.clone());
        let (id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_project_pg();
        updated.name = "v2".to_string();
        store
            .update_project(&id, updated, &ActorRef::test())
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM metis.projects WHERE id = $1 AND is_latest = true",
        )
        .bind(id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "exactly one is_latest row per project id");
    }

    /// The four-level prompt resolver depends on `Project.prompt_path`
    /// surviving a round trip through the store. Prior to the
    /// `add_projects_prompt_path` migration the column was missing, so
    /// the CLI's `projects update --prompt-path ...` set the field on
    /// the wire payload but `row_to_project` rebuilt the `Project` via
    /// `Project::new()` (which hard-codes `None`), and spawned sessions
    /// saw only the agent slice.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn project_prompt_path_round_trips_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let mut project = sample_project_pg();
        project.prompt_path = Some("/projects/engineering/prompt.md".to_string());
        let (id, _) = store
            .add_project(project.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_project(&id, false).await.unwrap();
        assert_eq!(
            fetched.item.prompt_path.as_deref(),
            Some("/projects/engineering/prompt.md"),
            "create-time prompt_path must survive the round trip"
        );

        let mut next = fetched.item.clone();
        next.prompt_path = Some("/projects/engineering/prompt-v2.md".to_string());
        store
            .update_project(&id, next, &ActorRef::test())
            .await
            .unwrap();

        let after_update = store.get_project(&id, false).await.unwrap();
        assert_eq!(
            after_update.item.prompt_path.as_deref(),
            Some("/projects/engineering/prompt-v2.md"),
        );

        let listed = store.list_projects(false).await.unwrap();
        let entry = listed
            .into_iter()
            .find(|(pid, _)| pid == &id)
            .expect("project missing from list");
        assert_eq!(
            entry.1.item.prompt_path.as_deref(),
            Some("/projects/engineering/prompt-v2.md"),
        );
    }

    /// `list_projects` must return projects in `priority ASC, created_at
    /// DESC, id DESC` order — the discriminator the priority-column
    /// migration adds. The default-project seed migration writes
    /// `priority = 1000.0` for `j-defaul`; this test inserts two custom
    /// projects with priorities straddling the default (1500.0 and
    /// 5000.0) and asserts the resulting order is `[default, custom-1500,
    /// custom-5000]`. Updating one project's priority must reflect in the
    /// next listing.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn list_projects_orders_by_priority_pg(pool: PgStorePool) {
        use crate::domain::projects::default_project_id;
        let store = PostgresStoreV2::new(pool);

        let mut high_priority = sample_project_pg();
        high_priority.key = ProjectKey::try_new("eng-high").unwrap();
        high_priority.priority = 5000.0;
        let (high_id, _) = store
            .add_project(high_priority, &ActorRef::test())
            .await
            .unwrap();

        let mut mid_priority = sample_project_pg();
        mid_priority.key = ProjectKey::try_new("eng-mid").unwrap();
        mid_priority.priority = 1500.0;
        let (mid_id, _) = store
            .add_project(mid_priority, &ActorRef::test())
            .await
            .unwrap();

        let listed = store.list_projects(false).await.unwrap();
        let ids: Vec<&ProjectId> = listed.iter().map(|(i, _)| i).collect();
        let priorities: Vec<f64> = listed.iter().map(|(_, v)| v.item.priority).collect();
        let default_id = default_project_id();
        assert_eq!(
            ids,
            vec![&default_id, &mid_id, &high_id],
            "list_projects must order by priority ASC: default(1000) → mid(1500) → high(5000)"
        );
        assert_eq!(priorities, vec![1000.0, 1500.0, 5000.0]);

        let mut bumped = store.get_project(&mid_id, false).await.unwrap().item;
        bumped.priority = 9000.0;
        store
            .update_project(&mid_id, bumped, &ActorRef::test())
            .await
            .unwrap();

        let listed = store.list_projects(false).await.unwrap();
        let ids: Vec<&ProjectId> = listed.iter().map(|(i, _)| i).collect();
        assert_eq!(
            ids,
            vec![&default_id, &high_id, &mid_id],
            "after bumping mid → 9000, order must be default(1000) → high(5000) → mid(9000)"
        );
    }

    /// `20260612000000_issues_v2_project_id_not_null.sql` tightens
    /// `metis.issues_v2.project_id` to NOT NULL after the seed migration
    /// backfills legacy NULL rows to `j-defaul`. Verify the column exists,
    /// remains `text`, and now reports `is_nullable = 'NO'`.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issues_v2_project_id_column_is_not_null_pg(pool: PgStorePool) {
        let row: (String, String) = sqlx::query_as(
            "SELECT data_type, is_nullable FROM information_schema.columns \
             WHERE table_schema = 'metis' AND table_name = 'issues_v2' AND column_name = 'project_id'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "text");
        assert_eq!(row.1, "NO");
    }

    /// Regression: every `issues_v2` SELECT must include `project_id` so a
    /// project-bound issue's `project_id` round-trips through `get_issue`,
    /// `list_issues`, and `get_issue_versions`. Before [[i-xnkrrggk]] the
    /// three Postgres SELECTs omitted the column and sqlx's `#[sqlx(default)]`
    /// silently coerced it to `None`, so `resolve_status` fell back to the
    /// synthesized default project and any custom status key blew up as
    /// `UnknownStatus` → HTTP 500.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn project_bound_issue_project_id_round_trips_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (project_id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();

        let mut issue = sample_issue(Vec::new());
        issue.project_id = project_id.clone();
        issue.status = hydra_common::api::v1::projects::StatusKey::try_new("backlog").unwrap();
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item.project_id, project_id,
            "get_issue must preserve project_id"
        );

        let listed = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        let found = listed
            .iter()
            .find(|(id, _)| id == &issue_id)
            .expect("list_issues must return the project-bound issue");
        assert_eq!(
            found.1.item.project_id, project_id,
            "list_issues must preserve project_id"
        );

        let versions = store.get_issue_versions(&issue_id).await.unwrap();
        assert!(
            !versions.is_empty(),
            "get_issue_versions must return at least one row"
        );
        for v in &versions {
            assert_eq!(
                v.item.project_id, project_id,
                "get_issue_versions must preserve project_id on every version"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_project_with_duplicate_key_returns_error_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();
        let result = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await;
        assert!(
            matches!(result, Err(StoreError::ProjectKeyExists(ref k)) if k.as_str() == "engineering"),
            "expected ProjectKeyExists(engineering), got {result:?}"
        );
    }

    /// A soft-deleted project frees its key for re-use — the partial
    /// unique index applies only to `is_latest AND NOT deleted`.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_project_after_delete_releases_key_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();
        store.delete_project(&id, &ActorRef::test()).await.unwrap();
        let result = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await;
        assert!(
            result.is_ok(),
            "expected re-add after delete, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_project_to_collide_with_another_returns_error_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::ProjectKey;
        let store = PostgresStoreV2::new(pool);
        let mut a = sample_project_pg();
        a.key = ProjectKey::try_new("a").unwrap();
        let mut b = sample_project_pg();
        b.key = ProjectKey::try_new("b").unwrap();
        store.add_project(a, &ActorRef::test()).await.unwrap();
        let (b_id, _) = store.add_project(b, &ActorRef::test()).await.unwrap();
        let mut collide = sample_project_pg();
        collide.key = ProjectKey::try_new("a").unwrap();
        let result = store
            .update_project(&b_id, collide, &ActorRef::test())
            .await;
        assert!(
            matches!(result, Err(StoreError::ProjectKeyExists(ref k)) if k.as_str() == "a"),
            "expected ProjectKeyExists(a), got {result:?}"
        );
    }

    /// Updating a project to its current key must succeed even though the
    /// partial unique index is in place — only a *different* live row
    /// holding the same key counts as a collision.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_project_keeping_same_key_succeeds_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool);
        let (id, _) = store
            .add_project(sample_project_pg(), &ActorRef::test())
            .await
            .unwrap();
        let mut next = sample_project_pg();
        next.name = "Engineering Renamed".to_string();
        let result = store.update_project(&id, next, &ActorRef::test()).await;
        assert!(
            result.is_ok(),
            "expected ok keeping same key, got {result:?}"
        );
    }

    /// The `seed_default_project` migration inserts the default project
    /// as version 1; this round-trips every field through `get_project`
    /// so that any future drift in the SELECT projection is caught at
    /// the Postgres store layer.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn default_project_seeded_by_migration_round_trips_pg(pool: PgStorePool) {
        use crate::domain::projects::default_project_id;
        let store = PostgresStoreV2::new(pool);
        let fetched = store
            .get_project(&default_project_id(), false)
            .await
            .expect("default project must be seeded by migration");
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.key.as_str(), "default");
        assert_eq!(fetched.item.name, "Default");
        assert_eq!(fetched.item.statuses.len(), 5);
        assert_eq!(
            fetched.item.prompt_path.as_deref(),
            Some("/projects/default/prompt.md")
        );
        let keys: Vec<&str> = fetched
            .item
            .statuses
            .iter()
            .map(|s| s.key.as_str())
            .collect();
        assert_eq!(keys, ["open", "in-progress", "closed", "dropped", "failed"]);
        let closed = fetched
            .item
            .find_status(&hydra_common::api::v1::projects::StatusKey::try_new("closed").unwrap())
            .unwrap();
        assert!(closed.unblocks_parents);
        assert!(closed.unblocks_dependents);
        assert!(!closed.cascades_to_children);
    }

    /// Issues constructed via `Issue::new` go through the seeded
    /// default project — verify status resolves through the DB-backed
    /// project.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn issue_with_default_project_id_resolves_through_db_pg(pool: PgStorePool) {
        use crate::domain::projects::default_project_id;
        let store = PostgresStoreV2::new(pool);
        let (issue_id, _) = store
            .add_issue(sample_issue(Vec::new()), &ActorRef::test())
            .await
            .unwrap();

        let issue = store.get_issue(&issue_id, false).await.unwrap().item;
        assert_eq!(issue.project_id, default_project_id());

        let project = store
            .get_project(&issue.project_id, false)
            .await
            .unwrap()
            .item;
        let status = project
            .find_status(&issue.status)
            .expect("issue status must resolve to a default-project status");
        assert_eq!(status.key.as_str(), "open");
    }

    // ---- Per-status CRUD (post-cutover) ----

    fn cutover_empty_project_pg(name: &str) -> Project {
        use hydra_common::api::v1::projects::ProjectKey;
        use hydra_common::api::v1::users::Username as ApiUsername;
        Project::new(
            ProjectKey::try_new(name).unwrap(),
            name.to_string(),
            Vec::new(),
            ApiUsername::from("alice"),
            false,
            0.0,
        )
    }

    fn cutover_status_def_pg(key: &str) -> hydra_common::api::v1::projects::StatusDefinition {
        use hydra_common::api::v1::projects::{StatusDefinition, StatusKey};
        StatusDefinition::new(
            StatusKey::try_new(key).unwrap(),
            key.to_string(),
            "#cccccc".parse().unwrap(),
            false,
            false,
            false,
            None,
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn add_status_assigns_sequences_in_input_order_pg(pool: PgStorePool) {
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("abc"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b", "c"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT sequence, key FROM metis.statuses WHERE project_id = $1 ORDER BY sequence",
        )
        .bind(project_id.as_ref())
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                (1, "a".to_string()),
                (2, "b".to_string()),
                (3, "c".to_string()),
            ]
        );
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT next_status_sequence FROM metis.projects WHERE id = $1 AND is_latest = TRUE",
        )
        .bind(project_id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(next_seq, 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_status_edits_in_place_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("abc"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b", "c"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        let mut updated = cutover_status_def_pg("b");
        updated.label = "B Prime".to_string();
        store
            .update_status(
                &project_id,
                &StatusKey::try_new("b").unwrap(),
                updated,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let row: (i64, String) = sqlx::query_as(
            "SELECT sequence, label FROM metis.statuses WHERE project_id = $1 AND key = 'b'",
        )
        .bind(project_id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row, (2, "B Prime".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn delete_status_then_add_does_not_reuse_sequence_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("abc"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b", "c"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        store
            .delete_status(
                &project_id,
                &StatusKey::try_new("c").unwrap(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT next_status_sequence FROM metis.projects WHERE id = $1 AND is_latest = TRUE",
        )
        .bind(project_id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(next_seq, 4);

        store
            .add_status(&project_id, cutover_status_def_pg("x"), &ActorRef::test())
            .await
            .unwrap();
        let x_seq: i64 = sqlx::query_scalar(
            "SELECT sequence FROM metis.statuses WHERE project_id = $1 AND key = 'x'",
        )
        .bind(project_id.as_ref())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(x_seq, 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_status_rename_does_not_orphan_issues_pg(pool: PgStorePool) {
        use crate::domain::issues::{Issue, IssueType};
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("rename"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b", "c"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        let mut renamed = cutover_status_def_pg("bb");
        renamed.key = StatusKey::try_new("bb").unwrap();
        store
            .update_status(
                &project_id,
                &StatusKey::try_new("b").unwrap(),
                renamed,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let issue = Issue::new(
            IssueType::Task,
            "rename test".to_string(),
            "test".to_string(),
            hydra_common::api::v1::users::Username::from("alice").into(),
            String::new(),
            StatusKey::try_new("bb").unwrap(),
            project_id.clone(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.status.as_str(), "bb");
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn cutover_fk_rejects_unknown_status_sequence_pg(pool: PgStorePool) {
        let res = sqlx::query(
            "INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, project_id, status_sequence) \
             VALUES ('i-fkbadseq', 1, 'task', 'fk', 'alice', 'j-defaul', 9999)",
        )
        .execute(&pool)
        .await;
        assert!(res.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn delete_status_rejects_removal_with_active_issue_pg(pool: PgStorePool) {
        use crate::domain::issues::{Issue, IssueType};
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("rmproj"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        let issue = Issue::new(
            IssueType::Task,
            "test".to_string(),
            "test".to_string(),
            hydra_common::api::v1::users::Username::from("alice").into(),
            String::new(),
            StatusKey::try_new("b").unwrap(),
            project_id.clone(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        store.add_issue(issue, &ActorRef::test()).await.unwrap();

        let res = store
            .delete_status(
                &project_id,
                &StatusKey::try_new("b").unwrap(),
                &ActorRef::test(),
            )
            .await;
        assert!(
            matches!(res, Err(StoreError::InvalidIssueStatus(_))),
            "expected InvalidIssueStatus when removing a status with active issues, got {res:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_status_rename_to_existing_key_returns_invalid_status_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("rn2"), &ActorRef::test())
            .await
            .unwrap();
        for k in ["a", "b"] {
            store
                .add_status(&project_id, cutover_status_def_pg(k), &ActorRef::test())
                .await
                .unwrap();
        }
        let mut renamed = cutover_status_def_pg("b");
        renamed.key = StatusKey::try_new("b").unwrap();
        let res = store
            .update_status(
                &project_id,
                &StatusKey::try_new("a").unwrap(),
                renamed,
                &ActorRef::test(),
            )
            .await;
        assert!(matches!(res, Err(StoreError::InvalidIssueStatus(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_status_unknown_key_returns_invalid_status_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let (project_id, _) = store
            .add_project(cutover_empty_project_pg("rn3"), &ActorRef::test())
            .await
            .unwrap();
        let res = store
            .update_status(
                &project_id,
                &StatusKey::try_new("nope").unwrap(),
                cutover_status_def_pg("c"),
                &ActorRef::test(),
            )
            .await;
        assert!(matches!(res, Err(StoreError::InvalidIssueStatus(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore]
    async fn update_status_project_not_found_pg(pool: PgStorePool) {
        use hydra_common::api::v1::projects::StatusKey;
        let store = PostgresStoreV2::new(pool.clone());
        let bogus = hydra_common::ProjectId::new();
        let res = store
            .update_status(
                &bogus,
                &StatusKey::try_new("a").unwrap(),
                cutover_status_def_pg("a"),
                &ActorRef::test(),
            )
            .await;
        assert!(matches!(res, Err(StoreError::ProjectNotFound(_))));
    }
}
