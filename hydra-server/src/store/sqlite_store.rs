use crate::domain::conversations::Conversation;
use crate::domain::{
    actors::{Actor, ActorId, ActorRef},
    agents::Agent,
    documents::Document,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, SessionSettings,
    },
    labels::Label,
    patches::{CommitRange, GithubPr, Patch, PatchStatus, Review},
    secrets::SecretRef,
    users::{User, Username},
};
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
    ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, ProjectId, RepoName, SessionId,
    TriggerId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    ids::random_len_for_count,
    repositories::{Repository, SearchRepositoriesQuery},
};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::OnceCell;

#[cfg(test)]
use super::{AgentConfig, SessionMode};
use super::{
    AuthTokenRow, ConversationEventSummary, ReadOnlyStore, Session, SessionEvent,
    SessionEventSummary, Status, Store, StoreError, TaskError, TaskStatusLog,
};

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
const TABLE_AUTH_TOKENS: &str = "auth_tokens";
const TABLE_USER_SECRETS: &str = "user_secrets";
const TABLE_OBJECT_RELATIONSHIPS: &str = "object_relationships";
const TABLE_CONVERSATIONS: &str = "conversations";
const TABLE_TRIGGERS: &str = "triggers";
const TABLE_PROJECTS: &str = "projects";
const TABLE_SESSION_EVENTS: &str = "session_events";
const TABLE_SESSION_STATE: &str = "session_state";

pub static MIGRATOR: Migrator = sqlx::migrate!("./sqlite-migrations");

/// Run the combined SQL+Rust migration sequence against `pool` up to (and
/// including) `up_to`, or to HEAD when `up_to == None`. The production
/// startup path calls this with `None`; the integration test passes
/// per-baseline pins. See `store/migrations/mod.rs` for the planning
/// helper and `/designs/migration-testing-redesign.md` §6 for the
/// semantics. Note for future migration authors: SQLite migrations that
/// reorder columns must NOT `INSERT INTO new_table SELECT * FROM
/// old_table` — column order in `SELECT *` is unstable across schema
/// changes and silently corrupts data ([[migrations]] memory).
pub async fn run_migrations(pool: &SqlitePool, up_to: Option<u64>) -> anyhow::Result<()> {
    use crate::store::migrations::{Backend, MigrationStep, plan_migrations, rust_migrations};
    use anyhow::Context;
    use sqlx::migrate::Migrate;

    let steps = plan_migrations(&MIGRATOR, rust_migrations(), up_to);

    let mut conn = pool
        .acquire()
        .await
        .context("acquire sqlite connection for migrations")?;
    let conn: &mut sqlx::SqliteConnection = &mut conn;

    conn.ensure_migrations_table()
        .await
        .context("ensure _sqlx_migrations table")?;
    if let Some(version) = conn.dirty_version().await? {
        anyhow::bail!("sqlite database is in a dirty state at migration version {version}");
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
                    conn.apply(migration)
                        .await
                        .with_context(|| format!("apply sqlite migration {}", migration.version))?;
                    applied.insert(migration.version);
                }
            }
            MigrationStep::Rust(rust) => {
                let name = rust.name();
                rust.run(&Backend::Sqlite(pool.clone()))
                    .await
                    .with_context(|| format!("apply rust migration {name}"))?;
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    row_counts: Arc<RowCountCache>,
}

/// In-memory row counts for the seven tables that drive `next_xxx_id`.
///
/// Each cell is lazily seeded with a single `SELECT COUNT(*)` and then
/// incremented in-process on every successful `add_*`. Assumes a single
/// writer to the SQLite database — diverges from disk if an external
/// process inserts directly.
#[derive(Default)]
struct RowCountCache {
    issues: OnceCell<AtomicI64>,
    patches: OnceCell<AtomicI64>,
    documents: OnceCell<AtomicI64>,
    tasks: OnceCell<AtomicI64>,
    labels: OnceCell<AtomicI64>,
    conversations: OnceCell<AtomicI64>,
    triggers: OnceCell<AtomicI64>,
    projects: OnceCell<AtomicI64>,
}

fn bump_count(cell: &OnceCell<AtomicI64>) {
    if let Some(atomic) = cell.get() {
        atomic.fetch_add(1, Ordering::Relaxed);
    }
}

fn decrement_count(cell: &OnceCell<AtomicI64>) {
    if let Some(atomic) = cell.get() {
        atomic.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: String,
    version_number: i64,
    remote_url: String,
    default_branch: Option<String>,
    default_image: Option<String>,
    deleted: bool,
    merge_policy: Option<String>,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct ActorRow {
    id: String,
    version_number: i64,
    actor_id: String,
    creator: String,
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
    created_at: String,
}

fn parse_relationship_row(
    r: ObjectRelationshipRow,
) -> Result<super::ObjectRelationship, StoreError> {
    let source_id: HydraId = r.source_id.parse().map_err(|_| {
        StoreError::Internal("invalid source_id in object_relationships".to_string())
    })?;
    let target_id: HydraId = r.target_id.parse().map_err(|_| {
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
    let created_at = parse_sqlite_timestamp(&r.created_at)?;
    Ok(super::ObjectRelationship {
        source_id,
        source_kind,
        target_id,
        target_kind,
        rel_type,
        created_at,
    })
}

#[derive(sqlx::FromRow)]
struct TriggerRow {
    id: String,
    version_number: i64,
    enabled: bool,
    creator: String,
    schedule: String,
    actions: String,
    last_fired_at: Option<String>,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ProjectRow {
    id: String,
    version_number: i64,
    key: String,
    name: String,
    default_status_key: String,
    statuses: String,
    creator: String,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    id: String,
    version_number: i64,
    title: Option<String>,
    agent_name: Option<String>,
    session_settings: String,
    status: String,
    creator: String,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConversationEventCountRow {
    conversation_id: String,
    event_count: i64,
}

#[derive(sqlx::FromRow)]
struct ConversationPreviewRow {
    conversation_id: String,
    event_data: String,
}

#[derive(sqlx::FromRow)]
struct SessionEventRow {
    version_number: i64,
    event_data: String,
    actor: Option<String>,
    created_at: String,
}

#[derive(sqlx::FromRow)]
struct SessionEventSummaryRow {
    session_id: String,
    event_count: i64,
    last_event_data: Option<String>,
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
    /// round-trips through `sqlx::FromRow`, but is no longer consumed at
    /// the Rust layer.
    #[allow(dead_code)]
    assignee: Option<String>,
    #[sqlx(default)]
    assignee_principal: Option<String>,
    #[sqlx(rename = "job_settings")]
    session_settings: String,
    deleted: bool,
    actor: Option<String>,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
    #[sqlx(default)]
    creation_time: Option<String>,
    #[sqlx(default)]
    form: Option<String>,
    #[sqlx(default)]
    form_response: Option<String>,
    #[sqlx(default)]
    feedback: Option<String>,
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
    creator: String,
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
    spawned_from: Option<String>,
    image: Option<String>,
    env_vars: String,
    cpu_limit: Option<String>,
    memory_limit: Option<String>,
    status: String,
    last_message: Option<String>,
    error: Option<String>,
    secrets: Option<String>,
    creator: String,
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
    // Denormalized from `mode.Interactive.conversation_id` at insert time
    // and never edited independently. Retained per design §6 step 16 as
    // the §3.4.1 single-query lookup index; SELECTed to keep the row
    // shape consistent with the table even though the read path reads
    // `mode` JSON.
    #[allow(dead_code)]
    #[sqlx(default)]
    conversation_id: Option<String>,
    #[sqlx(default)]
    usage: Option<String>,
    // Phase D step 12 (PR-1) populated these columns on every INSERT;
    // Phase E step 16 (PR-5) made them the sole source of session shape
    // after dropping the legacy `context` / `prompt` / `model` /
    // `mcp_config` / `interactive` columns.
    mount_spec: String,
    agent_config: String,
    mode: String,
    #[sqlx(default)]
    resumed_from: Option<String>,
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
    secrets: String,
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

fn row_to_agent(row: AgentRow) -> Result<Agent, StoreError> {
    let created_at = parse_sqlite_timestamp(&row.created_at)?;
    let updated_at = parse_sqlite_timestamp(&row.updated_at)?;
    let secrets: Vec<String> = serde_json::from_str(&row.secrets)
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
        Self {
            pool,
            row_counts: Arc::new(RowCountCache::default()),
        }
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

    /// Apply the SQL-only migration sequence. Kept for tests that need a
    /// fast schema-only bootstrap; production startup calls the free
    /// function [`run_migrations`] below to also drive Rust migrations.
    pub async fn run_migrations(pool: &SqlitePool) -> Result<(), anyhow::Error> {
        MIGRATOR.run(pool).await?;
        Ok(())
    }

    async fn cached_count_latest(
        &self,
        cell: &OnceCell<AtomicI64>,
        table: &str,
    ) -> Result<u64, StoreError> {
        let atomic = cell
            .get_or_try_init(|| async {
                let sql = format!("SELECT COUNT(*) FROM {table} WHERE is_latest = 1");
                let count = sqlx::query_scalar::<_, i64>(&sql)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                Ok::<_, StoreError>(AtomicI64::new(count.max(0)))
            })
            .await?;
        Ok(atomic.load(Ordering::Relaxed).max(0) as u64)
    }

    // Like `cached_count_latest`, but seeds the cell with `WHERE deleted = 0`
    // so the cache tracks the live row count. `add_label` increments it and
    // `delete_label` decrements it to keep it consistent with the soft-delete
    // semantics; soft-deleted rows do not inflate `next_label_id`'s suffix.
    async fn cached_count_undeleted(
        &self,
        cell: &OnceCell<AtomicI64>,
        table: &str,
    ) -> Result<u64, StoreError> {
        let atomic = cell
            .get_or_try_init(|| async {
                let sql = format!("SELECT COUNT(*) FROM {table} WHERE deleted = 0");
                let count = sqlx::query_scalar::<_, i64>(&sql)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                Ok::<_, StoreError>(AtomicI64::new(count.max(0)))
            })
            .await?;
        Ok(atomic.load(Ordering::Relaxed).max(0) as u64)
    }

    async fn next_issue_id(&self) -> Result<IssueId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.issues, TABLE_ISSUES_V2)
            .await?;
        let len = random_len_for_count(count);
        Ok(IssueId::generate(len).expect("length within bounds"))
    }

    async fn next_patch_id(&self) -> Result<PatchId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.patches, TABLE_PATCHES_V2)
            .await?;
        let len = random_len_for_count(count);
        Ok(PatchId::generate(len).expect("length within bounds"))
    }

    async fn next_document_id(&self) -> Result<DocumentId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.documents, TABLE_DOCUMENTS_V2)
            .await?;
        let len = random_len_for_count(count);
        Ok(DocumentId::generate(len).expect("length within bounds"))
    }

    async fn next_session_id(&self) -> Result<SessionId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.tasks, TABLE_TASKS_V2)
            .await?;
        let len = random_len_for_count(count);
        Ok(SessionId::generate(len).expect("length within bounds"))
    }

    async fn next_label_id(&self) -> Result<LabelId, StoreError> {
        let count = self
            .cached_count_undeleted(&self.row_counts.labels, TABLE_LABELS)
            .await?;
        let len = random_len_for_count(count);
        Ok(LabelId::generate(len).expect("length within bounds"))
    }

    async fn next_conversation_id(&self) -> Result<ConversationId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.conversations, TABLE_CONVERSATIONS)
            .await?;
        let len = random_len_for_count(count);
        Ok(ConversationId::generate(len).expect("length within bounds"))
    }

    async fn next_trigger_id(&self) -> Result<TriggerId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.triggers, TABLE_TRIGGERS)
            .await?;
        let len = random_len_for_count(count);
        Ok(TriggerId::generate(len).expect("length within bounds"))
    }

    async fn next_project_id(&self) -> Result<ProjectId, StoreError> {
        let count = self
            .cached_count_latest(&self.row_counts.projects, TABLE_PROJECTS)
            .await?;
        let len = random_len_for_count(count);
        Ok(ProjectId::generate(len).expect("length within bounds"))
    }

    #[cfg(test)]
    pub(super) fn bump_row_count_for_test(&self, table: &str, n: i64) {
        let cell = match table {
            TABLE_ISSUES_V2 => &self.row_counts.issues,
            TABLE_PATCHES_V2 => &self.row_counts.patches,
            TABLE_DOCUMENTS_V2 => &self.row_counts.documents,
            TABLE_TASKS_V2 => &self.row_counts.tasks,
            TABLE_LABELS => &self.row_counts.labels,
            TABLE_CONVERSATIONS => &self.row_counts.conversations,
            TABLE_TRIGGERS => &self.row_counts.triggers,
            TABLE_PROJECTS => &self.row_counts.projects,
            _ => panic!("unknown table for row-count cache: {table}"),
        };
        if let Some(atomic) = cell.get() {
            atomic.fetch_add(n, Ordering::Relaxed);
        } else {
            let _ = cell.set(AtomicI64::new(n));
        }
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

        let merge_policy_json = repo
            .merge_policy
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize merge_policy: {e}")))?;

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query("UPDATE repositories_v2 SET is_latest = 0 WHERE id = ?1 AND is_latest = 1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1
        sqlx::query(
            "INSERT INTO repositories_v2 (id, version_number, remote_url, default_branch, default_image, deleted, merge_policy, actor, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)"
        )
        .bind(id)
        .bind(version_number)
        .bind(&repo.remote_url)
        .bind(repo.default_branch.as_deref())
        .bind(repo.default_image.as_deref())
        .bind(repo.deleted)
        .bind(&merge_policy_json)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_repository(&self, row: &RepositoryRow) -> Result<Repository, StoreError> {
        let merge_policy = row
            .merge_policy
            .as_ref()
            .map(|v| {
                serde_json::from_str(v).map_err(|e| {
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

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query("UPDATE actors_v2 SET is_latest = 0 WHERE id = ?1 AND is_latest = 1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1.
        //
        // The `auth_token_hash` / `auth_token_salt` columns are vestigial
        // (the matching `Actor` fields and `verify_auth_token` consumer
        // are gone). We write empty strings to keep the NOT-NULL DB
        // schema satisfied until a follow-up migration drops the columns.
        sqlx::query(
            "INSERT INTO actors_v2 (id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor, is_latest)
             VALUES (?1, ?2, '', '', ?3, ?4, ?5, 1)"
        )
        .bind(id)
        .bind(version_number)
        .bind(&actor_id_json)
        .bind(&creator_str)
        .bind(acting_as)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_actor(&self, row: &ActorRow) -> Result<Actor, StoreError> {
        let actor_id: ActorId = serde_json::from_str(&row.actor_id)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize actor_id: {e}")))?;

        Ok(Actor {
            actor_id,
            creator: Username::from(row.creator.as_str()),
            session_id: None,
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

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query("UPDATE users_v2 SET is_latest = 0 WHERE id = ?1 AND is_latest = 1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1
        sqlx::query(
            "INSERT INTO users_v2 (id, version_number, username, github_user_id, deleted, actor, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        )
        .bind(id)
        .bind(version_number)
        .bind(user.username.as_str())
        .bind(user.github_user_id.map(|id| id as i64))
        .bind(user.deleted)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_user(&self, row: &UserRow) -> User {
        User::new(
            Username::from(row.username.clone()),
            row.github_user_id.map(|id| id as u64),
            row.deleted,
        )
    }

    // ---- Conversation helpers ----

    fn row_to_conversation(row: &ConversationRow) -> Result<Conversation, StoreError> {
        use hydra_common::api::v1::agents::AgentName;
        let status = match row.status.as_str() {
            "active" => crate::domain::conversations::ConversationStatus::Active,
            "idle" => crate::domain::conversations::ConversationStatus::Idle,
            "closed" => crate::domain::conversations::ConversationStatus::Closed,
            other => {
                return Err(StoreError::Internal(format!(
                    "unknown conversation status: {other}"
                )));
            }
        };
        let session_settings: crate::domain::issues::SessionSettings =
            serde_json::from_str(&row.session_settings).map_err(|e| {
                StoreError::Internal(format!(
                    "failed to deserialize conversation session_settings: {e}"
                ))
            })?;
        // Re-validate the persisted `agent_name` on read. The SQLite
        // column stays `TEXT`; the type-tightening happens at the Rust
        // boundary so malformed legacy values surface as an internal
        // error rather than silently passing through as `String`.
        let agent_name = row
            .agent_name
            .as_ref()
            .map(|s| AgentName::try_new(s.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("invalid agent_name in conversation row: {e}"))
            })?;
        Ok(Conversation {
            title: row.title.clone(),
            agent_name,
            status,
            creator: Username::from(row.creator.clone()),
            session_settings,
            deleted: row.deleted,
        })
    }

    fn conversation_status_str(
        status: &crate::domain::conversations::ConversationStatus,
    ) -> &'static str {
        match status {
            crate::domain::conversations::ConversationStatus::Active => "active",
            crate::domain::conversations::ConversationStatus::Idle => "idle",
            crate::domain::conversations::ConversationStatus::Closed => "closed",
        }
    }

    async fn insert_conversation_in_tx<'e, E>(
        executor: E,
        id: &ConversationId,
        version_number: VersionNumber,
        conversation: &Conversation,
        actor: Option<&str>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for conversation '{id}'"))
        })?;

        let session_settings_json =
            serde_json::to_string(&conversation.session_settings).map_err(|e| {
                StoreError::Internal(format!(
                    "failed to serialize conversation session_settings: {e}"
                ))
            })?;

        sqlx::query(&format!(
            "INSERT INTO {TABLE_CONVERSATIONS} (id, version_number, title, agent_name, session_settings, status, creator, deleted, actor, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)"
        ))
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&conversation.title)
        .bind(conversation.agent_name.as_ref().map(|n| n.as_str()))
        .bind(&session_settings_json)
        .bind(Self::conversation_status_str(&conversation.status))
        .bind(conversation.creator.as_str())
        .bind(conversation.deleted)
        .bind(actor)
        .execute(executor)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    // ---- Trigger helpers ----

    fn row_to_trigger(row: &TriggerRow) -> Result<Trigger, StoreError> {
        let schedule: hydra_common::triggers::Schedule = serde_json::from_str(&row.schedule)
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize trigger schedule: {e}"))
            })?;
        let actions: Vec<hydra_common::triggers::Action> = serde_json::from_str(&row.actions)
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize trigger actions: {e}"))
            })?;
        let last_fired_at = row
            .last_fired_at
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?;
        Ok(Trigger::new(
            row.enabled,
            schedule,
            actions,
            hydra_common::api::v1::users::Username::from(row.creator.clone()),
            last_fired_at,
            row.deleted,
        ))
    }

    async fn insert_trigger_in_tx<'e, E>(
        executor: E,
        id: &TriggerId,
        version_number: VersionNumber,
        trigger: &Trigger,
        actor: Option<&str>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for trigger '{id}'"))
        })?;
        let schedule_json = serde_json::to_string(&trigger.schedule).map_err(|e| {
            StoreError::Internal(format!("failed to serialize trigger schedule: {e}"))
        })?;
        let actions_json = serde_json::to_string(&trigger.actions).map_err(|e| {
            StoreError::Internal(format!("failed to serialize trigger actions: {e}"))
        })?;
        let last_fired_at = trigger.last_fired_at.map(|dt| dt.to_rfc3339());

        sqlx::query(&format!(
            "INSERT INTO {TABLE_TRIGGERS} (id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)"
        ))
        .bind(id.as_ref())
        .bind(version_number)
        .bind(trigger.enabled)
        .bind(trigger.creator.as_str())
        .bind(&schedule_json)
        .bind(&actions_json)
        .bind(last_fired_at.as_deref())
        .bind(trigger.deleted)
        .bind(actor)
        .execute(executor)
        .await
        .map_err(map_sqlx_error)?;

        Ok(())
    }

    // ---- Project helpers ----

    fn row_to_project(row: &ProjectRow) -> Result<Project, StoreError> {
        let statuses: Vec<StatusDefinition> = serde_json::from_str(&row.statuses).map_err(|e| {
            StoreError::Internal(format!("failed to deserialize project statuses: {e}"))
        })?;
        let key = ProjectKey::try_new(row.key.clone()).map_err(|e| {
            StoreError::Internal(format!("invalid project key stored for project: {e}"))
        })?;
        let default_status_key =
            StatusKey::try_new(row.default_status_key.clone()).map_err(|e| {
                StoreError::Internal(format!(
                    "invalid default_status_key stored for project: {e}"
                ))
            })?;
        Ok(Project::new(
            key,
            row.name.clone(),
            statuses,
            default_status_key,
            hydra_common::api::v1::users::Username::from(row.creator.clone()),
            row.deleted,
        ))
    }

    async fn insert_project_in_tx<'e, E>(
        executor: E,
        id: &ProjectId,
        version_number: VersionNumber,
        project: &Project,
        actor: Option<&str>,
    ) -> Result<(), StoreError>
    where
        E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
    {
        let version_number = i64::try_from(version_number).map_err(|_| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;
        let statuses_json = serde_json::to_string(&project.statuses).map_err(|e| {
            StoreError::Internal(format!("failed to serialize project statuses: {e}"))
        })?;

        sqlx::query(&format!(
            "INSERT INTO {TABLE_PROJECTS} (id, version_number, key, name, default_status_key, statuses, creator, deleted, actor, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)"
        ))
        .bind(id.as_ref())
        .bind(version_number)
        .bind(project.key.as_str())
        .bind(&project.name)
        .bind(project.default_status_key.as_str())
        .bind(&statuses_json)
        .bind(project.creator.as_str())
        .bind(project.deleted)
        .bind(actor)
        .execute(executor)
        .await
        .map_err(|err| {
            if is_project_key_unique_violation_sqlite(&err) {
                StoreError::ProjectKeyExists(project.key.clone())
            } else {
                map_sqlx_error(err)
            }
        })?;

        Ok(())
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
        let form_json = issue
            .form
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize form: {e}")))?;
        let form_response_json = issue
            .form_response
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to serialize form_response: {e}")))?;
        let assignee_principal_json = issue
            .assignee
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to serialize assignee_principal: {e}"))
            })?;
        // Dual-write the legacy `assignee TEXT` column from the typed
        // Principal's canonical path form so out-of-band readers of the
        // old column keep working.
        let assignee_path = issue.assignee.as_ref().map(|p| p.to_path());
        sqlx::query(
            "INSERT INTO issues_v2 (id, version_number, issue_type, title, description, creator, progress, status, assignee, assignee_principal, job_settings, deleted, actor, form, form_response, feedback, is_latest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 1)"
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(issue.issue_type.as_str())
        .bind(&issue.title)
        .bind(&issue.description)
        .bind(issue.creator.as_str())
        .bind(&issue.progress)
        .bind(issue.status.as_str())
        .bind(assignee_path.as_deref())
        .bind(assignee_principal_json.as_deref())
        .bind(&session_settings_json)
        .bind(issue.deleted)
        .bind(actor)
        .bind(&form_json)
        .bind(&form_response_json)
        .bind(issue.feedback.as_deref())
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
        // Delete only the relationships managed by this function. Other
        // rel_types (e.g. has-document) are owned by other code paths and
        // must not be stomped by issue updates.
        let delete_sql = format!(
            "DELETE FROM {TABLE_OBJECT_RELATIONSHIPS} \
             WHERE source_id = ?1 \
               AND rel_type IN ('child-of', 'blocked-on', 'has-patch')"
        );
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

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query(&format!(
            "UPDATE {TABLE_PATCHES_V2} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1
        sqlx::query(
            &format!(
                "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor, is_latest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 1)"
            )
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&patch.title)
        .bind(&patch.description)
        .bind(&patch.diff)
        .bind(patch.status.as_str())
        .bind(patch.is_automatic_backup)
        .bind(patch.creator.as_str())
        .bind(patch.base_branch.as_deref())
        .bind(patch.branch_name.as_deref())
        .bind(&commit_range_json)
        .bind(&reviews_json)
        .bind(patch.service_repo_name.as_str())
        .bind(&github_json)
        .bind(patch.deleted)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

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
        let commit_range: Option<CommitRange> = row
            .commit_range
            .as_ref()
            .map(|cr| {
                serde_json::from_str(cr).map_err(|e| {
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

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query(&format!(
            "UPDATE {TABLE_DOCUMENTS_V2} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1
        sqlx::query(
            &format!(
                "INSERT INTO {TABLE_DOCUMENTS_V2} (id, version_number, title, body_markdown, path, deleted, actor, is_latest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)"
            )
        )
        .bind(id.as_ref())
        .bind(version_number)
        .bind(&document.title)
        .bind(&document.body_markdown)
        .bind(document.path.as_ref().map(|p| p.as_str()))
        .bind(document.deleted)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        tx.commit().await.map_err(map_sqlx_error)?;

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

        let legacy_conversation_id = session.conversation_id().cloned();

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
        let usage_json = session
            .usage
            .as_ref()
            .map(|u| {
                serde_json::to_string(u).map_err(|err| {
                    StoreError::Internal(format!("failed to serialize usage: {err}"))
                })
            })
            .transpose()?;
        let mount_spec_json = serde_json::to_string(&super::dual_write_mount_spec_json(session)?)
            .map_err(|e| {
            StoreError::Internal(format!("failed to serialize mount_spec: {e}"))
        })?;
        let agent_config_json =
            serde_json::to_string(&super::dual_write_agent_config_json(session)?).map_err(|e| {
                StoreError::Internal(format!("failed to serialize agent_config: {e}"))
            })?;
        let mode_json = serde_json::to_string(&super::dual_write_mode_json(session)?)
            .map_err(|e| StoreError::Internal(format!("failed to serialize mode: {e}")))?;
        let resumed_from_str = session
            .resumed_from
            .as_ref()
            .map(|s| s.as_ref().to_string());
        let status_str = super::status_to_db_str(session.status);
        let creation_time_str = session.creation_time.map(|t| t.to_rfc3339());
        let start_time_str = session.start_time.map(|t| t.to_rfc3339());
        let end_time_str = session.end_time.map(|t| t.to_rfc3339());

        // Use a transaction to atomically clear the old is_latest and set the new one
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Clear is_latest on the previous latest version
        sqlx::query(&format!(
            "UPDATE {TABLE_TASKS_V2} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        // Insert the new version with is_latest = 1
        if let Some(ts) = created_at {
            sqlx::query(
                &format!(
                    "INSERT INTO {TABLE_TASKS_V2} (id, version_number, spawned_from, creator, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets, created_at, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, is_latest)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, 1)"
                )
            )
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
            .bind(ts)
            .bind(creation_time_str.as_deref())
            .bind(start_time_str.as_deref())
            .bind(end_time_str.as_deref())
            .bind(legacy_conversation_id.as_ref().map(|c| c.as_ref()))
            .bind(&usage_json)
            .bind(&mount_spec_json)
            .bind(&agent_config_json)
            .bind(&mode_json)
            .bind(resumed_from_str.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        } else {
            sqlx::query(
                &format!(
                    "INSERT INTO {TABLE_TASKS_V2} (id, version_number, spawned_from, creator, image, env_vars, cpu_limit, memory_limit, status, last_message, error, deleted, actor, secrets, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, is_latest)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, 1)"
                )
            )
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
            .bind(creation_time_str.as_deref())
            .bind(start_time_str.as_deref())
            .bind(end_time_str.as_deref())
            .bind(legacy_conversation_id.as_ref().map(|c| c.as_ref()))
            .bind(&usage_json)
            .bind(&mount_spec_json)
            .bind(&agent_config_json)
            .bind(&mode_json)
            .bind(resumed_from_str.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        }

        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(())
    }

    fn row_to_session(&self, row: &TaskRow) -> Result<Session, StoreError> {
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
        let creator = Username::from(row.creator.as_str());

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

        let usage = row
            .usage
            .as_deref()
            .map(|s| {
                serde_json::from_str(s)
                    .map_err(|e| StoreError::Internal(format!("failed to deserialize usage: {e}")))
            })
            .transpose()?;

        // Phase E step 16: the legacy `context` / `prompt` / `model` /
        // `mcp_config` / `interactive` columns are gone. `mount_spec`,
        // `agent_config`, and `mode` are now NOT NULL in every row
        // (backfilled by PR-1, dual-written since PR-2).
        let mount_spec = serde_json::from_str(&row.mount_spec)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize mount_spec: {e}")))?;
        let agent_config = serde_json::from_str(&row.agent_config).map_err(|e| {
            StoreError::Internal(format!("failed to deserialize agent_config: {e}"))
        })?;
        let mode = serde_json::from_str(&row.mode)
            .map_err(|e| StoreError::Internal(format!("failed to deserialize mode: {e}")))?;
        let resumed_from = row
            .resumed_from
            .as_deref()
            .map(|s| {
                s.parse::<SessionId>()
                    .map_err(|e| StoreError::Internal(format!("invalid resumed_from: {e}")))
            })
            .transpose()?;

        Ok(Session {
            creator,
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
            creation_time,
            start_time,
            end_time,
            usage,
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
        let form = row
            .form
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| StoreError::Internal(format!("failed to deserialize form: {e}")))?;
        let form_response = row
            .form_response
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize form_response: {e}"))
            })?;
        // `assignee_principal` is the source of truth for `Issue.assignee`.
        // The legacy `assignee TEXT` column is still dual-written for soak
        // but is no longer read here.
        let assignee = row
            .assignee_principal
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| {
                StoreError::Internal(format!("failed to deserialize assignee_principal: {e}"))
            })?;
        Ok(Issue {
            issue_type,
            title: row.title.clone(),
            description: row.description.clone(),
            creator: Username::from(row.creator.clone()),
            progress: row.progress.clone(),
            status,
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
}

/// Build WHERE predicates and bindings for conversations queries (SQLite `?N` placeholders).
fn build_conversations_predicates_sqlite(
    query: &SearchConversationsQuery,
) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(status) = &query.status {
        let status_str = match status {
            hydra_common::api::v1::conversations::ConversationStatus::Active => "active",
            hydra_common::api::v1::conversations::ConversationStatus::Idle => "idle",
            hydra_common::api::v1::conversations::ConversationStatus::Closed => "closed",
        };
        bindings.push(status_str.to_string());
        predicates.push(format!("status = ?{}", bindings.len()));
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
        bindings.push(pattern); // agent_name
        predicates.push(format!(
            "(LOWER(id) LIKE ?{s0} OR LOWER(COALESCE(title,'')) LIKE ?{s1} OR LOWER(COALESCE(agent_name,'')) LIKE ?{s2})",
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

/// Build WHERE predicates and bindings for issues queries (SQLite `?N` placeholders).
fn build_issues_predicates_sqlite(query: &SearchIssuesQuery) -> (Vec<String>, Vec<String>) {
    let mut predicates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    // When `ids` is provided, filter by ID (intersected with other filters).
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
    }

    if let Some(issue_type) = query.issue_type.as_ref() {
        bindings.push(issue_type.as_str().to_string());
        predicates.push(format!("issue_type = ?{}", bindings.len()));
    }

    if !query.status.is_empty() {
        let placeholders: Vec<String> = query
            .status
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", bindings.len() + i + 1))
            .collect();
        predicates.push(format!("status IN ({})", placeholders.join(", ")));
        for s in &query.status {
            bindings.push(s.as_str().to_string());
        }
    }

    if let Some(assignee) = query.assignee.as_ref() {
        // Filter against the typed `assignee_principal` column (JSON
        // text) using canonical serialization, not lowercased free-text
        // against the legacy `assignee TEXT`. The serialization is fixed
        // by serde so a binary `=` predicate is sufficient.
        let serialized = serde_json::to_string(assignee).unwrap_or_default();
        bindings.push(serialized);
        predicates.push(format!("assignee_principal = ?{}", bindings.len()));
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

    // When `ids` is provided, filter by ID (intersected with other filters).
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
    }

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

    if let Some(branch) = query
        .branch_name
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        bindings.push(branch.to_string());
        predicates.push(format!("branch_name = ?{}", bindings.len()));
    }

    if let Some(repo_name) = query
        .repo_name
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        bindings.push(repo_name.to_string());
        predicates.push(format!("service_repo_name = ?{}", bindings.len()));
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

    // When `ids` is provided, filter by ID (intersected with other filters).
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
    }

    if let Some(path) = query.path_prefix.as_ref() {
        if query.path_is_exact.unwrap_or(false) {
            bindings.push(path.clone());
            predicates.push(format!("COALESCE(path,'') = ?{}", bindings.len()));
        } else {
            bindings.push(format!("{path}%"));
            predicates.push(format!("COALESCE(path,'') LIKE ?{}", bindings.len()));
        }
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

    if let Some(has_path) = query.has_path {
        if has_path {
            predicates.push("path IS NOT NULL".to_string());
        } else {
            predicates.push("path IS NULL".to_string());
        }
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

    if let Some(creator) = query.creator.as_deref() {
        bindings.push(creator.to_string());
        predicates.push(format!("t.creator = ?{}", bindings.len()));
    }

    if let Some(conversation_id) = query.conversation_id.as_ref() {
        bindings.push(conversation_id.as_ref().to_string());
        predicates.push(format!("t.conversation_id = ?{}", bindings.len()));
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
             OR LOWER(COALESCE(json_extract(t.agent_config, '$.system_prompt'), '')) LIKE ?{idx_prompt} \
             OR LOWER(t.status) LIKE ?{idx_status})"
        ));
    }

    if !query.status.is_empty() {
        let status_strings: Vec<String> = query
            .status
            .iter()
            .filter_map(|s| {
                let server_status: Status = (*s).try_into().ok()?;
                Some(super::status_to_db_str(server_status).to_string())
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
    if let sqlx::Error::Database(ref db_err) = err {
        // SQLite unique constraint violation (SQLITE_CONSTRAINT_UNIQUE = 2067)
        if db_err.code().as_deref() == Some("2067") {
            let msg = db_err.message();
            if msg.contains("documents_v2.path") {
                // Message format: "UNIQUE constraint failed: documents_v2.path"
                return StoreError::DocumentPathConflict;
            }
        }
    }
    StoreError::Internal(err.to_string())
}

/// True iff `err` is a SQLite unique-violation on the partial
/// `projects_key_unique_active_idx` index (the `projects.key` column).
/// Used by `add_project` / `update_project` to translate the raw sqlx
/// error into a [`StoreError::ProjectKeyExists`] that carries the
/// colliding key.
fn is_project_key_unique_violation_sqlite(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        if db_err.code().as_deref() == Some("2067") {
            // SQLite reports either the column path (`projects.key`) or
            // the index name depending on the index kind; match both so
            // we are robust to the exact message format.
            let msg = db_err.message();
            return msg.contains("projects.key") || msg.contains("projects_key_unique_active_idx");
        }
    }
    false
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
            "SELECT id, version_number, remote_url, default_branch, default_image, deleted, merge_policy, actor, created_at, updated_at
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
        let normalized_needle = query
            .remote_url
            .as_deref()
            .map(Repository::normalize_remote_url);
        let rows = sqlx::query_as::<_, RepositoryRow>(
            "SELECT r.id, r.version_number, r.remote_url, r.default_branch, r.default_image, r.deleted, r.merge_policy, r.actor, r.created_at, r.updated_at
             FROM repositories_v2 r
             WHERE r.is_latest = 1
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
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, assignee_principal, job_settings, deleted, actor, created_at, updated_at, form, form_response, feedback,
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
            "SELECT id, version_number, issue_type, title, description, creator, progress, status, assignee, assignee_principal, job_settings, deleted, actor, created_at, updated_at, form, form_response, feedback, NULL AS creation_time
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
        let subquery = format!(
            "SELECT i.id, i.version_number, i.issue_type, i.title, i.description, i.creator, i.progress, i.status, i.assignee, i.assignee_principal, i.job_settings, i.deleted, i.actor, i.created_at, i.updated_at, i.form, i.form_response, i.feedback,
             (SELECT MIN(created_at) FROM {TABLE_ISSUES_V2} WHERE id = i.id) AS creation_time
             FROM {TABLE_ISSUES_V2} i
             WHERE i.is_latest = 1"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let (mut predicates, mut bindings) = build_issues_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "updated_at",
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
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_ISSUES_V2} i");
        let (mut predicates, bindings) = build_issues_predicates_sqlite(query);
        predicates.push("i.is_latest = 1".to_string());

        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));

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
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor, created_at, updated_at,
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
            "SELECT id, version_number, title, description, diff, status, is_automatic_backup, creator, base_branch, branch_name, commit_range, reviews, service_repo_name, github, deleted, actor, created_at, updated_at, NULL AS creation_time
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
            "SELECT p.id, p.version_number, p.title, p.description, p.diff, p.status, p.is_automatic_backup, p.creator, p.base_branch, p.branch_name, p.commit_range, p.reviews, p.service_repo_name, p.github, p.deleted, p.actor, p.created_at, p.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_PATCHES_V2} WHERE id = p.id) AS creation_time
             FROM {TABLE_PATCHES_V2} p
             WHERE p.is_latest = 1"
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
        let mut sql = format!("SELECT COUNT(*) FROM {TABLE_PATCHES_V2} p");
        let (mut predicates, bindings) = build_patches_predicates_sqlite(query);
        predicates.push("p.is_latest = 1".to_string());

        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));

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
            "SELECT id, version_number, title, body_markdown, path, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = ?1) AS creation_time
             FROM {TABLE_DOCUMENTS_V2}
             WHERE id = ?1 AND is_latest = 1"
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
            "SELECT id, version_number, title, body_markdown, path, deleted, actor, created_at, updated_at, NULL AS creation_time
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
            "SELECT d.id, d.version_number, d.title, d.body_markdown, d.path, d.deleted, d.actor, d.created_at, d.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_DOCUMENTS_V2} WHERE id = d.id) AS creation_time
             FROM {TABLE_DOCUMENTS_V2} d
             WHERE d.is_latest = 1"
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
            "SELECT id, title, body_markdown, path, deleted
             FROM {TABLE_DOCUMENTS_V2}
             WHERE is_latest = 1"
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

    async fn find_non_deleted_document_by_exact_path(
        &self,
        path: &str,
    ) -> Result<Option<DocumentId>, StoreError> {
        let row = sqlx::query_as::<_, (String,)>(&format!(
            "SELECT id FROM {TABLE_DOCUMENTS_V2}
                 WHERE path = ?1 AND is_latest = 1 AND COALESCE(deleted, 0) = 0
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

        // Deduplicate inputs (preserve first-seen order) so we don't bind the
        // same path twice and so we never emit duplicates in the result.
        let mut deduped: Vec<&str> = Vec::with_capacity(paths.len());
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for path in paths {
            if seen.insert(path.as_str()) {
                deduped.push(path.as_str());
            }
        }

        let placeholders: Vec<String> = (1..=deduped.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT path, id, title FROM {TABLE_DOCUMENTS_V2} \
             WHERE is_latest = 1 \
               AND COALESCE(deleted, 0) = 0 \
               AND path IN ({})",
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
        let prefix_len = prefix.len() as i64;

        let sql = format!(
            "SELECT
                CASE
                    WHEN INSTR(SUBSTR(path, ?1 + 1), '/') > 0
                    THEN SUBSTR(path, ?1 + 1, INSTR(SUBSTR(path, ?1 + 1), '/') - 1)
                    ELSE SUBSTR(path, ?1 + 1)
                END AS segment,
                COUNT(*) AS child_count,
                MAX(CASE WHEN path = ?3 || CASE
                    WHEN INSTR(SUBSTR(path, ?1 + 1), '/') > 0
                    THEN SUBSTR(path, ?1 + 1, INSTR(SUBSTR(path, ?1 + 1), '/') - 1)
                    ELSE SUBSTR(path, ?1 + 1)
                END THEN 1 ELSE 0 END) AS is_doc
             FROM {TABLE_DOCUMENTS_V2}
             WHERE is_latest = 1
               AND COALESCE(deleted, 0) = 0
               AND path IS NOT NULL
               AND path LIKE ?2
               AND LENGTH(path) > ?1
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

    async fn get_session(
        &self,
        id: &SessionId,
        include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError> {
        let row = sqlx::query_as::<_, TaskRow>(
            &format!(
                "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at,
                 creation_time, start_time, end_time, conversation_id, usage,
                 mount_spec, agent_config, mode, resumed_from
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
                "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at, creation_time, start_time, end_time, conversation_id, usage,
                 mount_spec, agent_config, mode, resumed_from
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
            "SELECT t.id, t.version_number, t.spawned_from, t.image, t.env_vars, t.cpu_limit, t.memory_limit, t.status, t.last_message, t.error, t.secrets, t.creator, t.deleted, t.actor, t.created_at, t.updated_at, \
             t.creation_time, t.start_time, t.end_time, t.conversation_id, t.usage, \
             t.mount_spec, t.agent_config, t.mode, t.resumed_from \
             FROM {TABLE_TASKS_V2} t"
        );
        let (mut predicates, mut bindings) = build_tasks_predicates_sqlite(query);
        predicates.insert(0, "t.is_latest = 1".to_string());

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
             FROM {TABLE_TASKS_V2} t"
        );
        let (mut predicates, bindings) = build_tasks_predicates_sqlite(query);
        predicates.insert(0, "t.is_latest = 1".to_string());

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
            "SELECT id, version_number, spawned_from, image, env_vars, cpu_limit, memory_limit, status, last_message, error, secrets, creator, deleted, actor, created_at, updated_at, creation_time, start_time, end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from \
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
            "SELECT id, version_number, actor_id, creator, actor, created_at, updated_at
             FROM actors_v2
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1",
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
            "SELECT a.id, a.version_number, a.actor_id, a.creator, a.actor, a.created_at, a.updated_at
             FROM actors_v2 a
             WHERE a.is_latest = 1
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
             WHERE u.is_latest = 1
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

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        let sql = format!(
            "SELECT name, prompt_path, mcp_config_path, max_tries, max_simultaneous, \
                    is_assignment_agent, is_default_conversation_agent, secrets, deleted, \
                    created_at, updated_at \
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
            "SELECT name, prompt_path, mcp_config_path, max_tries, max_simultaneous, \
                    is_assignment_agent, is_default_conversation_agent, secrets, deleted, \
                    created_at, updated_at \
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
        object_id: &HydraId,
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
        object_ids: &[HydraId],
    ) -> Result<HashMap<HydraId, Vec<LabelSummary>>, StoreError> {
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

        let mut result: HashMap<HydraId, Vec<LabelSummary>> = HashMap::new();
        for (obj_id_str, label_id_str, name, color, recurse, hidden) in rows {
            let obj_id = obj_id_str.parse::<HydraId>().map_err(|err| {
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

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<HydraId>, StoreError> {
        let sql = format!("SELECT object_id FROM {TABLE_LABEL_ASSOCIATIONS} WHERE label_id = ?1");
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
            "SELECT id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_TRIGGERS} WHERE id = ?1) AS creation_time
             FROM {TABLE_TRIGGERS}
             WHERE id = ?1
             ORDER BY version_number DESC
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
        let timestamp = parse_sqlite_timestamp(&row.created_at)?;
        let creation_time = row
            .creation_time
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?
            .unwrap_or(timestamp);

        Ok(Versioned::with_optional_actor(
            trigger,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn list_triggers(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(TriggerId, Versioned<Trigger>)>, StoreError> {
        let mut sql = format!(
            "SELECT t.id, t.version_number, t.enabled, t.creator, t.schedule, t.actions, t.last_fired_at, t.deleted, t.actor, t.created_at, t.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_TRIGGERS} WHERE id = t.id) AS creation_time
             FROM {TABLE_TRIGGERS} t
             WHERE t.is_latest = 1"
        );
        if !include_deleted {
            sql.push_str(" AND t.deleted = 0");
        }
        sql.push_str(" ORDER BY t.created_at DESC, t.id DESC");

        let rows = sqlx::query_as::<_, TriggerRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut triggers = Vec::with_capacity(rows.len());
        for row in &rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for trigger '{}'",
                    row.id
                ))
            })?;
            let trigger = Self::row_to_trigger(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            let trigger_id = TriggerId::from_str(&row.id).map_err(|err| {
                StoreError::Internal(format!("invalid trigger id stored '{}': {err}", row.id))
            })?;
            triggers.push((
                trigger_id,
                Versioned::with_optional_actor(
                    trigger,
                    version,
                    timestamp,
                    parse_actor_json_string(row.actor.as_deref())?,
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
            "SELECT id, version_number, key, name, default_status_key, statuses, creator, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = ?1) AS creation_time
             FROM {TABLE_PROJECTS}
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::ProjectNotFound(id.clone()))?;
        let project = Self::row_to_project(&row)?;

        if project.deleted && !include_deleted {
            return Err(StoreError::ProjectNotFound(id.clone()));
        }

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for project '{}'",
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

        Ok(Versioned::with_optional_actor(
            project,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn list_projects(
        &self,
        include_deleted: bool,
    ) -> Result<Vec<(ProjectId, Versioned<Project>)>, StoreError> {
        let mut sql = format!(
            "SELECT p.id, p.version_number, p.key, p.name, p.default_status_key, p.statuses, p.creator, p.deleted, p.actor, p.created_at, p.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_PROJECTS} WHERE id = p.id) AS creation_time
             FROM {TABLE_PROJECTS} p
             WHERE p.is_latest = 1"
        );
        if !include_deleted {
            sql.push_str(" AND p.deleted = 0");
        }
        sql.push_str(" ORDER BY p.created_at DESC, p.id DESC");

        let rows = sqlx::query_as::<_, ProjectRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut projects = Vec::with_capacity(rows.len());
        for row in &rows {
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
            let project = Self::row_to_project(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            projects.push((
                project_id,
                Versioned::with_optional_actor(
                    project,
                    version,
                    timestamp,
                    parse_actor_json_string(row.actor.as_deref())?,
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
            "SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
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
        ids: &[HydraId],
        direction: super::TransitiveDirection,
        rel_type: super::RelationshipType,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let rel_param = ids.len() + 1;

        let sql = match direction {
            super::TransitiveDirection::Forward => format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE source_id IN ({placeholders}) AND rel_type = ?{rel_param} \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type, r.created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.source_id = tr.target_id \
                     WHERE r.rel_type = ?{rel_param} \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                 FROM transitive_rels"
            ),
            super::TransitiveDirection::Backward => format!(
                "WITH RECURSIVE transitive_rels AS ( \
                     SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} \
                     WHERE target_id IN ({placeholders}) AND rel_type = ?{rel_param} \
                   UNION \
                     SELECT r.source_id, r.source_kind, r.target_id, r.target_kind, r.rel_type, r.created_at \
                     FROM {TABLE_OBJECT_RELATIONSHIPS} r \
                     INNER JOIN transitive_rels tr ON r.target_id = tr.source_id \
                     WHERE r.rel_type = ?{rel_param} \
                 ) \
                 SELECT source_id, source_kind, target_id, target_kind, rel_type, created_at \
                 FROM transitive_rels"
            ),
        };

        let mut query = sqlx::query_as::<_, ObjectRelationshipRow>(&sql);
        for id in ids {
            query = query.bind(id.as_ref());
        }
        query = query.bind(rel_type.as_str());

        let rows = query.fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        rows.into_iter().map(parse_relationship_row).collect()
    }

    async fn get_auth_token_hashes(&self, actor_name: &str) -> Result<Vec<String>, StoreError> {
        let sql = format!(
            "SELECT token_hash FROM {TABLE_AUTH_TOKENS} WHERE actor_name = ?1 ORDER BY created_at"
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
            "SELECT actor_name, session_id, is_revoked FROM {TABLE_AUTH_TOKENS} \
             WHERE token_hash = ?1 LIMIT 1"
        );
        let row = sqlx::query_as::<_, (String, Option<String>, i64)>(&sql)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        let Some((actor_name, session_id, is_revoked)) = row else {
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
            is_revoked: is_revoked != 0,
        }))
    }

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let sql = format!(
            "SELECT encrypted_value FROM {TABLE_USER_SECRETS} WHERE username = ?1 AND secret_name = ?2 ORDER BY internal ASC LIMIT 1"
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
            "SELECT secret_name, MIN(CAST(internal AS INTEGER)) as internal FROM {TABLE_USER_SECRETS} WHERE username = ?1 GROUP BY secret_name ORDER BY secret_name"
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
        let row = sqlx::query_as::<_, ConversationRow>(&format!(
            "SELECT id, version_number, title, agent_name, session_settings, status, creator, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_CONVERSATIONS} WHERE id = ?1) AS creation_time
             FROM {TABLE_CONVERSATIONS}
             WHERE id = ?1
             ORDER BY version_number DESC
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let row = row.ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        let conversation = Self::row_to_conversation(&row)?;

        if conversation.deleted && !include_deleted {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }

        let version = VersionNumber::try_from(row.version_number).map_err(|_| {
            StoreError::Internal(format!(
                "invalid version number stored for conversation '{}'",
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

        Ok(Versioned::with_optional_actor(
            conversation,
            version,
            timestamp,
            parse_actor_json_string(row.actor.as_deref())?,
            creation_time,
        ))
    }

    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<(ConversationId, Versioned<Conversation>)>, StoreError> {
        let subquery = format!(
            "SELECT c.id, c.version_number, c.title, c.agent_name, c.session_settings, c.status, c.creator, c.deleted, c.actor, c.created_at, c.updated_at,
             (SELECT MIN(created_at) FROM {TABLE_CONVERSATIONS} WHERE id = c.id) AS creation_time
             FROM {TABLE_CONVERSATIONS} c
             WHERE c.is_latest = 1"
        );
        let mut sql = format!("SELECT * FROM ({subquery}) AS latest");
        let (mut predicates, mut bindings) = build_conversations_predicates_sqlite(query);

        apply_pagination_sql_sqlite(
            &mut sql,
            &mut predicates,
            &mut bindings,
            &query.cursor,
            query.limit,
            "updated_at",
            "id",
        )?;

        let mut query_builder = sqlx::query_as::<_, ConversationRow>(&sql);
        for value in &bindings {
            query_builder = query_builder.bind(value);
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut conversations = Vec::with_capacity(rows.len());
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
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            let versioned = Versioned::with_optional_actor(
                conversation,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                creation_time,
            );
            conversations.push((conversation_id, versioned));
        }

        Ok(conversations)
    }

    async fn get_conversation_versions(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<Versioned<Conversation>>, StoreError> {
        let rows = sqlx::query_as::<_, ConversationRow>(&format!(
            "SELECT id, version_number, title, agent_name, session_settings, status, creator, deleted, actor, created_at, updated_at,
             (SELECT MIN(created_at) FROM {TABLE_CONVERSATIONS} WHERE id = ?1) AS creation_time
             FROM {TABLE_CONVERSATIONS}
             WHERE id = ?1
             ORDER BY version_number"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if rows.is_empty() {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }

        let mut results = Vec::with_capacity(rows.len());
        for row in &rows {
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal(format!(
                    "invalid version number stored for conversation '{}'",
                    row.id
                ))
            })?;
            let conversation = Self::row_to_conversation(row)?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            let creation_time = row
                .creation_time
                .as_deref()
                .map(parse_sqlite_timestamp)
                .transpose()?
                .unwrap_or(timestamp);
            results.push(Versioned::with_optional_actor(
                conversation,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                creation_time,
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

        // Query 1: Chat-text SessionEvent count per conversation_id —
        // summed across every live session linked to the conversation.
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let count_sql = format!(
            "SELECT t.conversation_id AS conversation_id, COUNT(*) AS event_count \
             FROM {TABLE_SESSION_EVENTS} e \
             JOIN {TABLE_TASKS_V2} t ON t.id = e.session_id \
                 AND t.is_latest = 1 \
                 AND t.deleted = 0 \
             WHERE t.conversation_id IN ({placeholders}) \
               AND e.event_type IN ('user_message', 'assistant_message') \
             GROUP BY t.conversation_id",
            placeholders = placeholders.join(", "),
        );
        let mut count_query = sqlx::query_as::<_, ConversationEventCountRow>(&count_sql);
        for id in ids {
            count_query = count_query.bind(id.as_ref());
        }
        let count_rows = count_query
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        // Query 2: latest chat-text SessionEvent (UserMessage / AssistantMessage)
        // per conversation_id, ordered latest session first then latest event
        // within that session. Joins through tasks_v2 to filter to live sessions
        // linked to the given conversations.
        let preview_sql = format!(
            "WITH ranked AS ( \
                SELECT t.conversation_id AS conversation_id, e.event_data AS event_data, \
                       ROW_NUMBER() OVER ( \
                           PARTITION BY t.conversation_id \
                           ORDER BY t.creation_time DESC, t.id DESC, e.rowid_seq DESC \
                       ) AS rn \
                FROM {TABLE_SESSION_EVENTS} e \
                JOIN {TABLE_TASKS_V2} t ON t.id = e.session_id \
                    AND t.is_latest = 1 \
                    AND t.deleted = 0 \
                WHERE t.conversation_id IN ({placeholders}) \
                  AND e.event_type IN ('user_message', 'assistant_message') \
             ) \
             SELECT conversation_id, event_data \
             FROM ranked \
             WHERE rn = 1",
            placeholders = placeholders.join(", "),
        );
        let mut preview_query = sqlx::query_as::<_, ConversationPreviewRow>(&preview_sql);
        for id in ids {
            preview_query = preview_query.bind(id.as_ref());
        }
        let preview_rows = preview_query
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
            let event: SessionEvent = serde_json::from_str(&row.event_data).map_err(|e| {
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
        // Verify session exists (including soft-deleted, mirroring memory store).
        let _ = self.get_session(id, true).await?;

        let rows = sqlx::query_as::<_, SessionEventRow>(&format!(
            "SELECT version_number, event_data, actor, created_at
             FROM {TABLE_SESSION_EVENTS}
             WHERE session_id = ?1
             ORDER BY rowid_seq ASC"
        ))
        .bind(id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let event: SessionEvent = serde_json::from_str(&row.event_data).map_err(|e| {
                StoreError::Internal(format!("failed to deserialize session event: {e}"))
            })?;
            let version = VersionNumber::try_from(row.version_number).map_err(|_| {
                StoreError::Internal("invalid version number for session event".to_string())
            })?;
            let timestamp = parse_sqlite_timestamp(&row.created_at)?;
            events.push(Versioned::with_optional_actor(
                event,
                version,
                timestamp,
                parse_actor_json_string(row.actor.as_deref())?,
                timestamp,
            ));
        }

        Ok(events)
    }

    async fn list_session_ids_by_conversation_id(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<SessionId>, StoreError> {
        let rows = sqlx::query_as::<_, (String,)>(&format!(
            "SELECT id FROM {TABLE_TASKS_V2}
             WHERE is_latest = 1
               AND deleted = 0
               AND conversation_id = ?1
             ORDER BY creation_time ASC, id ASC"
        ))
        .bind(conversation_id.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        let mut ids = Vec::with_capacity(rows.len());
        for (id_str,) in rows {
            ids.push(Self::row_to_session_id(&id_str)?);
        }
        Ok(ids)
    }

    async fn get_session_event_summaries(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, SessionEventSummary>, StoreError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT e.session_id AS session_id, COUNT(*) AS event_count, \
             (SELECT e2.event_data FROM {TABLE_SESSION_EVENTS} e2 \
              WHERE e2.session_id = e.session_id ORDER BY e2.rowid_seq DESC LIMIT 1) AS last_event_data \
             FROM {TABLE_SESSION_EVENTS} e \
             WHERE e.session_id IN ({}) \
             GROUP BY e.session_id",
            placeholders.join(", ")
        );

        let mut query_builder = sqlx::query_as::<_, SessionEventSummaryRow>(&sql);
        for id in ids {
            query_builder = query_builder.bind(id.as_ref());
        }

        let rows = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut result = HashMap::new();
        for row in rows {
            let sid = Self::row_to_session_id(&row.session_id)?;
            let last_event_preview = row
                .last_event_data
                .as_deref()
                .map(|data| {
                    serde_json::from_str::<SessionEvent>(data)
                        .map(|event| event.preview())
                        .map_err(|e| {
                            StoreError::Internal(format!(
                                "failed to deserialize session event: {e}"
                            ))
                        })
                })
                .transpose()?;
            result.insert(
                sid,
                SessionEventSummary {
                    event_count: row.event_count as usize,
                    last_event_preview,
                },
            );
        }

        Ok(result)
    }

    async fn get_session_state(&self, id: &SessionId) -> Result<Option<Vec<u8>>, StoreError> {
        // Verify session exists (including soft-deleted, mirroring memory store).
        let _ = self.get_session(id, true).await?;

        let row = sqlx::query_scalar::<_, Vec<u8>>(&format!(
            "SELECT data FROM {TABLE_SESSION_STATE} WHERE session_id = ?1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        Ok(row)
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
        let id = self.next_issue_id().await?;
        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Clear is_latest on any previous version (no-op for new entities)
        sqlx::query(&format!(
            "UPDATE {TABLE_ISSUES_V2} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_issue_in_tx(&mut *tx, &id, 1, &issue, Some(&actor_json)).await?;
        Self::sync_issue_relationships_in_tx(&mut tx, &id, &issue).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        bump_count(&self.row_counts.issues);
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
        // Clear is_latest on the previous latest version
        sqlx::query(&format!(
            "UPDATE {TABLE_ISSUES_V2} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
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
        let id = self.next_patch_id().await?;
        let actor_json = actor_to_json_string(actor);
        self.insert_patch(&id, 1, &patch, Some(&actor_json)).await?;
        bump_count(&self.row_counts.patches);
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
        let id = self.next_document_id().await?;
        let actor_json = actor_to_json_string(actor);
        self.insert_document(&id, 1, &document, Some(&actor_json))
            .await?;
        bump_count(&self.row_counts.documents);
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
        let id = self.next_session_id().await?;

        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        session.creation_time = Some(creation_time);
        let actor_json = actor_to_json_string(actor);
        let created_at = creation_time.to_rfc3339();
        self.insert_task(&id, 1, &session, Some(&actor_json), Some(&created_at))
            .await?;
        bump_count(&self.row_counts.tasks);
        Ok((id, 1))
    }

    async fn update_session(
        &self,
        hydra_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.ensure_issue_exists(issue_id).await?;
        }

        let latest_version = self
            .fetch_latest_version_number(TABLE_TASKS_V2, hydra_id.as_ref())
            .await?
            .ok_or_else(|| StoreError::SessionNotFound(hydra_id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for task '{hydra_id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);
        self.insert_task(hydra_id, next_version, &session, Some(&actor_json), None)
            .await?;
        self.get_session(hydra_id, true).await
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
                if agent.is_default_conversation_agent {
                    let has_default = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_default_conversation_agent = 1 AND deleted = 0)"
                    ))
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                    if has_default {
                        return Err(StoreError::ConversationAgentAlreadyExists);
                    }
                }

                let now = Utc::now().to_rfc3339();
                let secrets_json = serde_json::to_string(&agent.secrets).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize secrets: {e}"))
                })?;
                let sql = format!(
                    "UPDATE {TABLE_AGENTS} \
                     SET prompt_path = ?1, mcp_config_path = ?2, max_tries = ?3, max_simultaneous = ?4, \
                         is_assignment_agent = ?5, is_default_conversation_agent = ?6, secrets = ?7, \
                         deleted = 0, created_at = ?8, updated_at = ?9 \
                     WHERE name = ?10"
                );
                sqlx::query(&sql)
                    .bind(&agent.prompt_path)
                    .bind(agent.mcp_config_path.as_deref())
                    .bind(agent.max_tries)
                    .bind(agent.max_simultaneous)
                    .bind(agent.is_assignment_agent)
                    .bind(agent.is_default_conversation_agent)
                    .bind(&secrets_json)
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
                if agent.is_default_conversation_agent {
                    let has_default = sqlx::query_scalar::<_, bool>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                         WHERE is_default_conversation_agent = 1 AND deleted = 0)"
                    ))
                    .fetch_one(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
                    if has_default {
                        return Err(StoreError::ConversationAgentAlreadyExists);
                    }
                }

                let secrets_json = serde_json::to_string(&agent.secrets).map_err(|e| {
                    StoreError::Internal(format!("failed to serialize secrets: {e}"))
                })?;
                let sql = format!(
                    "INSERT INTO {TABLE_AGENTS} \
                     (name, prompt_path, mcp_config_path, max_tries, max_simultaneous, is_assignment_agent, \
                      is_default_conversation_agent, secrets, deleted, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
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
        if agent.is_default_conversation_agent {
            let conflict = sqlx::query_scalar::<_, bool>(&format!(
                "SELECT EXISTS(SELECT 1 FROM {TABLE_AGENTS} \
                 WHERE is_default_conversation_agent = 1 AND deleted = 0 AND name != ?1)"
            ))
            .bind(&agent.name)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
            if conflict {
                return Err(StoreError::ConversationAgentAlreadyExists);
            }
        }

        let secrets_json = serde_json::to_string(&agent.secrets)
            .map_err(|e| StoreError::Internal(format!("failed to serialize secrets: {e}")))?;
        let sql = format!(
            "UPDATE {TABLE_AGENTS} \
             SET prompt_path = ?1, mcp_config_path = ?2, max_tries = ?3, max_simultaneous = ?4, \
                 is_assignment_agent = ?5, is_default_conversation_agent = ?6, secrets = ?7, \
                 updated_at = ?8 \
             WHERE name = ?9"
        );
        sqlx::query(&sql)
            .bind(&agent.prompt_path)
            .bind(agent.mcp_config_path.as_deref())
            .bind(agent.max_tries)
            .bind(agent.max_simultaneous)
            .bind(agent.is_assignment_agent)
            .bind(agent.is_default_conversation_agent)
            .bind(&secrets_json)
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

        let id = self.next_label_id().await?;

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

        bump_count(&self.row_counts.labels);
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

        decrement_count(&self.row_counts.labels);
        Ok(())
    }

    async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
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
        object_id: &HydraId,
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
        source_id: &HydraId,
        target_id: &HydraId,
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
        source_id: &HydraId,
        target_id: &HydraId,
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

    // ---- Auth token mutations ----

    async fn add_auth_token(
        &self,
        actor_name: &str,
        token_hash: &str,
        session_id: Option<&SessionId>,
    ) -> Result<(), StoreError> {
        let now = Utc::now().to_rfc3339();
        let sql = format!(
            "INSERT OR IGNORE INTO {TABLE_AUTH_TOKENS} (actor_name, token_hash, created_at, session_id) \
             VALUES (?1, ?2, ?3, ?4)"
        );
        sqlx::query(&sql)
            .bind(actor_name)
            .bind(token_hash)
            .bind(&now)
            .bind(session_id.map(|s| s.to_string()))
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;
        Ok(())
    }

    async fn delete_auth_tokens_for_actor(&self, actor_name: &str) -> Result<(), StoreError> {
        let sql = format!("DELETE FROM {TABLE_AUTH_TOKENS} WHERE actor_name = ?1");
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
            "UPDATE {TABLE_AUTH_TOKENS} SET is_revoked = 1 \
             WHERE session_id = ?1 AND is_revoked = 0"
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
        let now = Utc::now().to_rfc3339();
        let sql = format!(
            "INSERT INTO {TABLE_USER_SECRETS} (username, secret_name, encrypted_value, internal, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
             ON CONFLICT (username, secret_name, internal) \
             DO UPDATE SET encrypted_value = ?3, updated_at = ?5"
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
        let sql = format!(
            "DELETE FROM {TABLE_USER_SECRETS} WHERE username = ?1 AND secret_name = ?2 AND internal = FALSE"
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
        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Clear is_latest on any previous version (no-op for new entities)
        sqlx::query(&format!(
            "UPDATE {TABLE_CONVERSATIONS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_conversation_in_tx(&mut *tx, &id, 1, &conversation, Some(&actor_json)).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        bump_count(&self.row_counts.conversations);
        Ok((id, 1))
    }

    async fn update_conversation(
        &self,
        id: &ConversationId,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let latest_version = self
            .fetch_latest_version_number(TABLE_CONVERSATIONS, id.as_ref())
            .await?
            .ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for conversation '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        sqlx::query(&format!(
            "UPDATE {TABLE_CONVERSATIONS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_conversation_in_tx(
            &mut *tx,
            id,
            next_version,
            &conversation,
            Some(&actor_json),
        )
        .await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        Ok(next_version)
    }

    async fn append_session_event(
        &self,
        id: &SessionId,
        event: SessionEvent,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        // Verify session exists (including soft-deleted, mirroring memory store).
        let _ = self.get_session(id, true).await?;

        let event_data = serde_json::to_string(&event)
            .map_err(|e| StoreError::Internal(format!("failed to serialize session event: {e}")))?;
        let event_type = match &event {
            SessionEvent::UserMessage { .. } => "user_message",
            SessionEvent::AssistantMessage { .. } => "assistant_message",
            SessionEvent::ToolUse { .. } => "tool_use",
            SessionEvent::Suspending { .. } => "suspending",
            SessionEvent::Resumed { .. } => "resumed",
            SessionEvent::Closed { .. } => "closed",
        };
        let actor_json = actor_to_json_string(actor);

        // Single transaction: compute next version_number for this session and
        // insert the row atomically.
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let latest_version: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT MAX(version_number) FROM {TABLE_SESSION_EVENTS} WHERE session_id = ?1"
        ))
        .bind(id.as_ref())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let next_version_i64 = latest_version.unwrap_or(0).checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("event version number overflow for session '{id}'"))
        })?;
        let next_version = VersionNumber::try_from(next_version_i64).map_err(|_| {
            StoreError::Internal(format!("event version number overflow for session '{id}'"))
        })?;

        sqlx::query(&format!(
            "INSERT INTO {TABLE_SESSION_EVENTS} (session_id, version_number, event_type, event_data, actor)
             VALUES (?1, ?2, ?3, ?4, ?5)"
        ))
        .bind(id.as_ref())
        .bind(next_version_i64)
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
        // Verify session exists (including soft-deleted, mirroring memory store).
        let _ = self.get_session(id, true).await?;

        sqlx::query(&format!(
            "INSERT INTO {TABLE_SESSION_STATE} (session_id, data, updated_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(session_id) DO UPDATE SET
                data = excluded.data,
                updated_at = excluded.updated_at"
        ))
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
        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Clear is_latest on any previous version (no-op for new entities).
        sqlx::query(&format!(
            "UPDATE {TABLE_TRIGGERS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_trigger_in_tx(&mut *tx, &id, 1, &trigger, Some(&actor_json)).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        bump_count(&self.row_counts.triggers);
        Ok((id, 1))
    }

    async fn update_trigger(
        &self,
        id: &TriggerId,
        mut trigger: Trigger,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        // Read the current latest row inside the transaction so a concurrent
        // record_trigger_fire's last_fired_at is carried forward.
        let latest_row = sqlx::query_as::<_, TriggerRow>(&format!(
            "SELECT id, version_number, enabled, creator, schedule, actions, last_fired_at, deleted, actor, created_at, updated_at
             FROM {TABLE_TRIGGERS}
             WHERE id = ?1 AND is_latest = 1
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let latest_row = latest_row.ok_or_else(|| StoreError::TriggerNotFound(id.clone()))?;

        // Always overwrite the supplied `last_fired_at` with the latest
        // row's value (Some or None) — `record_trigger_fire` mutates the
        // latest row in place, so a stale snapshot round-tripped by the
        // caller must not regress it.
        trigger.last_fired_at = latest_row
            .last_fired_at
            .as_deref()
            .map(parse_sqlite_timestamp)
            .transpose()?;

        let latest_version = VersionNumber::try_from(latest_row.version_number).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for trigger '{id}'"))
        })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for trigger '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);

        sqlx::query(&format!(
            "UPDATE {TABLE_TRIGGERS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
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
        let fired_at_str = fired_at.to_rfc3339();
        let now_str = Utc::now().to_rfc3339();
        let result = sqlx::query(&format!(
            "UPDATE {TABLE_TRIGGERS} SET last_fired_at = ?1, updated_at = ?2 WHERE id = ?3 AND is_latest = 1"
        ))
        .bind(&fired_at_str)
        .bind(&now_str)
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
        let actor_json = actor_to_json_string(actor);

        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        // Clear is_latest on any previous version (no-op for new entities).
        sqlx::query(&format!(
            "UPDATE {TABLE_PROJECTS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_project_in_tx(&mut *tx, &id, 1, &project, Some(&actor_json)).await?;
        tx.commit().await.map_err(map_sqlx_error)?;

        bump_count(&self.row_counts.projects);
        Ok((id, 1))
    }

    async fn update_project(
        &self,
        id: &ProjectId,
        project: Project,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;

        let latest_version = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT version_number FROM {TABLE_PROJECTS}
             WHERE id = ?1 AND is_latest = 1
             LIMIT 1"
        ))
        .bind(id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;

        let latest_version =
            latest_version.ok_or_else(|| StoreError::ProjectNotFound(id.clone()))?;
        let latest_version = VersionNumber::try_from(latest_version).map_err(|_| {
            StoreError::Internal(format!("invalid version number stored for project '{id}'"))
        })?;
        let next_version = latest_version.checked_add(1).ok_or_else(|| {
            StoreError::Internal(format!("version number overflow for project '{id}'"))
        })?;

        let actor_json = actor_to_json_string(actor);

        sqlx::query(&format!(
            "UPDATE {TABLE_PROJECTS} SET is_latest = 0 WHERE id = ?1 AND is_latest = 1"
        ))
        .bind(id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        Self::insert_project_in_tx(&mut *tx, id, next_version, &project, Some(&actor_json)).await?;
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
    use chrono::Duration;
    use hydra_common::SessionId;
    use std::collections::HashSet;

    async fn create_test_store() -> SqliteStore {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();
        SqliteStore::new(pool)
    }

    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://github.com/dourolabs/hydra".to_string(),
            Some("main".to_string()),
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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();

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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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

        let query = SearchRepositoriesQuery::new(Some(true), None);
        let list = store.list_repositories(&query).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_recreates_over_soft_deleted_repo() {
        let store = create_test_store().await;
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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

    #[tokio::test]
    async fn list_repositories_filters_by_remote_url() {
        let store = create_test_store().await;
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

    #[tokio::test]
    async fn repository_round_trip_merge_policy_some() {
        use hydra_common::Principal;
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::repositories::{AssigneeRef, MergePolicy, MergerRule, ReviewerGroup};

        let store = create_test_store().await;
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

    #[tokio::test]
    async fn repository_round_trip_merge_policy_none() {
        let store = create_test_store().await;
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

    #[tokio::test]
    async fn migration_adds_merge_policy_column_to_repositories_v2() {
        let store = create_test_store().await;
        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> = sqlx::query_as(
            "SELECT cid, name, type, \"notnull\", dflt_value, pk \
             FROM pragma_table_info('repositories_v2')",
        )
        .fetch_all(&store.pool)
        .await
        .unwrap();

        let column = rows
            .iter()
            .find(|(_, name, _, _, _, _)| name == "merge_policy")
            .expect("merge_policy column should exist after migrations");
        assert_eq!(column.2, "TEXT", "merge_policy should be TEXT");
        assert_eq!(column.3, 0, "merge_policy should be nullable");
        assert_eq!(column.5, 0, "merge_policy should not be part of the PK");
    }

    // ---- Actor tests ----

    #[tokio::test]
    async fn add_and_get_actor_by_name() {
        let store = create_test_store().await;
        let actor = Actor {
            actor_id: ActorId::User(Username::from("ada").into()),
            creator: Username::from("ada"),
            session_id: None,
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
            actor_id: ActorId::Adhoc(SessionId::new()),
            creator: Username::from("creator"),
            session_id: None,
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
            actor_id: ActorId::Adhoc(task_id),
            creator: Username::from("creator"),
            session_id: None,
        };
        let mut updated = actor.clone();
        updated.creator = Username::from("rotated-creator");

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
            actor_id: ActorId::User(Username::from("ada").into()),
            creator: Username::from("ada"),
            session_id: None,
        };

        let err = store
            .update_actor(actor, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorNotFound(name) if name == "users/ada"
        ));
    }

    #[tokio::test]
    async fn get_actor_missing_returns_not_found() {
        let store = create_test_store().await;
        let task_id = SessionId::new();
        let name = format!("adhoc/{task_id}");

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
            actor_id: ActorId::User(Username::from("alice").into()),
            creator: Username::from("alice"),
            session_id: None,
        };
        let actor2 = Actor {
            actor_id: ActorId::User(Username::from("bob").into()),
            creator: Username::from("bob"),
            session_id: None,
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
            dependencies,
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn sample_issue_all_fields(dependencies: Vec<IssueDependency>, patches: Vec<PatchId>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "full description".to_string(),
            Username::from("issue-creator"),
            "50%".to_string(),
            IssueStatus::Open,
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
            None,
            None,
            Some("some feedback text".to_string()),
        )
    }

    // ---- Issue tests ----

    #[tokio::test]
    async fn issue_serialization_round_trip_all_fields() {
        let store = create_test_store().await;

        let (parent_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let issue = sample_issue_all_fields(
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id,
            )],
            Vec::new(),
        );

        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item, issue,
            "Issue must round-trip all fields (assignee, job_settings, dependencies, feedback)"
        );
    }

    /// Verifies the inline-SQL backfill clause in
    /// `20260530000000_add_assignee_principal_to_issues.sql` still
    /// produces a typed principal that the Phase-4b read path surfaces as
    /// `Issue.assignee`. We bypass `add_issue` (which now dual-writes both
    /// columns) and use a raw `UPDATE` to simulate a pre-migration row.
    #[tokio::test]
    async fn migration_backfill_populates_assignee_principal_for_users_path() {
        use hydra_common::principal::Principal as ActorPrincipal;
        let store = create_test_store().await;
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Reset both columns so the row looks pre-Phase-4a: typed NULL,
        // legacy string populated.
        sqlx::query("UPDATE issues_v2 SET assignee = ?1, assignee_principal = NULL WHERE id = ?2")
            .bind("users/alice")
            .bind(issue_id.as_ref())
            .execute(&store.pool)
            .await
            .unwrap();

        // Re-run just the UPDATE clause from the migration.
        let update_sql = r#"
            UPDATE issues_v2
            SET assignee_principal = CASE
                    WHEN substr(assignee, 1, 6) = 'users/'
                         AND length(assignee) > 6
                         AND substr(assignee, 7) NOT LIKE '%/%'
                         AND substr(assignee, 7) NOT LIKE '% %'
                         AND substr(assignee, 7) NOT LIKE '%' || char(9) || '%'
                         AND substr(assignee, 7) NOT LIKE '%' || char(10) || '%'
                         AND substr(assignee, 7) NOT LIKE '%' || char(13) || '%'
                        THEN json_object('User', json_object('name', substr(assignee, 7)))
                    WHEN substr(assignee, 1, 7) = 'agents/'
                         AND length(assignee) > 7
                         AND substr(assignee, 8) NOT LIKE '%/%'
                         AND substr(assignee, 8) NOT LIKE '% %'
                         AND substr(assignee, 8) NOT LIKE '%' || char(9) || '%'
                         AND substr(assignee, 8) NOT LIKE '%' || char(10) || '%'
                         AND substr(assignee, 8) NOT LIKE '%' || char(13) || '%'
                        THEN json_object('Agent', json_object('name', substr(assignee, 8)))
                    WHEN assignee != ''
                         AND assignee NOT LIKE '%/%'
                         AND assignee NOT LIKE '% %'
                         AND assignee NOT LIKE '%' || char(9) || '%'
                         AND assignee NOT LIKE '%' || char(10) || '%'
                         AND assignee NOT LIKE '%' || char(13) || '%'
                        THEN json_object('User', json_object('name', assignee))
                    ELSE NULL
                END
            WHERE assignee IS NOT NULL AND assignee_principal IS NULL
        "#;
        sqlx::query(update_sql).execute(&store.pool).await.unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item.assignee,
            Some(ActorPrincipal::User {
                name: hydra_common::api::v1::users::Username::try_new("alice").unwrap(),
            })
        );
    }

    #[tokio::test]
    async fn issue_round_trips_assignee_principal_user() {
        use hydra_common::principal::Principal as ActorPrincipal;
        let store = create_test_store().await;
        let mut issue = sample_issue(vec![]);
        let alice = ActorPrincipal::User {
            name: hydra_common::api::v1::users::Username::try_new("alice").unwrap(),
        };
        issue.assignee = Some(alice.clone());
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.assignee, Some(alice));

        // The legacy `assignee TEXT` column is still populated with the
        // canonical path form so out-of-band readers keep working.
        let assignee_text: Option<String> =
            sqlx::query_scalar("SELECT assignee FROM issues_v2 WHERE id = ?1")
                .bind(issue_id.as_ref())
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(assignee_text.as_deref(), Some("users/alice"));
    }

    #[tokio::test]
    async fn issue_round_trips_assignee_none() {
        let store = create_test_store().await;
        let mut issue = sample_issue(vec![]);
        issue.assignee = None;
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.assignee, None);
    }

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
        query.status = vec![IssueStatus::Open.into()];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_multiple_statuses() {
        let store = create_test_store().await;

        // Create one issue per status: Open (default), InProgress, Closed
        let (open_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut in_progress_issue = sample_issue(vec![]);
        in_progress_issue.status = IssueStatus::InProgress;
        let (ip_id, _) = store
            .add_issue(in_progress_issue, &ActorRef::test())
            .await
            .unwrap();

        let mut closed_issue = sample_issue(vec![]);
        closed_issue.status = IssueStatus::Closed;
        store
            .add_issue(closed_issue, &ActorRef::test())
            .await
            .unwrap();

        // Filter by open + in-progress should return 2 issues
        let mut query = SearchIssuesQuery::default();
        query.status = vec![IssueStatus::Open.into(), IssueStatus::InProgress.into()];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        let result_ids: HashSet<_> = results.iter().map(|(id, _)| id.clone()).collect();
        assert!(result_ids.contains(&open_id));
        assert!(result_ids.contains(&ip_id));

        // Empty status filter should return all 3
        let mut query = SearchIssuesQuery::default();
        query.status = vec![];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 3);

        // Single status filter should still work
        let mut query = SearchIssuesQuery::default();
        query.status = vec![IssueStatus::Closed.into()];
        let results = store.list_issues(&query).await.unwrap();
        assert_eq!(results.len(), 1);
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
    async fn list_issues_sorted_by_update_time() {
        let store = create_test_store().await;

        // Create issue A, then issue B (B has a later creation time).
        let (id_a, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (id_b, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Sleep to ensure the update gets a distinct timestamp (SQLite has ms precision).
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        // Update issue A so its updated_at becomes the most recent.
        let mut updated_a = sample_issue(vec![]);
        updated_a.description = "updated A".to_string();
        store
            .update_issue(&id_a, updated_a, &ActorRef::test())
            .await
            .unwrap();

        // List should return A first (most recently updated), then B.
        let results = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, id_a);
        assert_eq!(results[1].0, id_b);
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
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn sample_document(path: Option<&str>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: path.map(|p| p.parse().unwrap()),
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
            vec![hydra_common::api::v1::patches::PatchStatus::Open],
            None,
        );
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].1.item.status, PatchStatus::Open);
    }

    #[tokio::test]
    async fn list_patches_filters_by_repo_name() {
        use std::str::FromStr;
        let store = create_test_store().await;

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
        let mut filtered: Vec<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        filtered.sort();
        let mut expected = vec![patch_a_id.clone(), patch_b_id.clone()];
        expected.sort();
        assert_eq!(filtered, expected);

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

    #[tokio::test]
    async fn list_patches_filters_by_creator() {
        let store = create_test_store().await;

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
        let mut filtered: Vec<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        filtered.sort();
        let mut expected = vec![patch_a_id.clone(), patch_b_id.clone()];
        expected.sort();
        assert_eq!(filtered, expected);

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

    #[tokio::test]
    async fn list_patches_empty_string_filter_is_noop() {
        use std::str::FromStr;
        let store = create_test_store().await;

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

    /// Patch with every optional field set so serialization round-trip can assert full equality.
    fn sample_patch_all_fields() -> Patch {
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

    #[tokio::test]
    async fn patch_serialization_round_trip_all_fields() {
        let store = create_test_store().await;
        let patch = sample_patch_all_fields();

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

        let patch = sample_patch_all_fields();
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
                sample_document(Some("docs/guides/intro.md")),
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
            .add_document(sample_document(Some("docs/howto.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("notes/todo.md")), &ActorRef::test())
            .await
            .unwrap();

        let by_path = store.get_documents_by_path("/docs/").await.unwrap();
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].0, doc1);
    }

    #[tokio::test]
    async fn list_document_path_children_returns_segments() {
        let store = create_test_store().await;

        // Create documents under various paths
        store
            .add_document(
                sample_document(Some("agents/swe/memory.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("agents/swe/plan.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("agents/pm/notes.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("docs/readme.md")), &ActorRef::test())
            .await
            .unwrap();

        // Top-level segments at prefix "/"
        let children = store.list_document_path_children("/").await.unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].0, "agents");
        assert_eq!(children[0].1, "/agents");
        assert_eq!(children[0].2, 3); // 3 docs under /agents/
        assert!(!children[0].3); // /agents is not a document
        assert_eq!(children[1].0, "docs");
        assert_eq!(children[1].1, "/docs");
        assert_eq!(children[1].2, 1);
        assert!(!children[1].3); // /docs is not a document

        // Nested prefix "/agents/" returns child segments
        let children = store.list_document_path_children("/agents/").await.unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].0, "pm");
        assert_eq!(children[0].1, "/agents/pm");
        assert_eq!(children[0].2, 1);
        assert!(!children[0].3); // /agents/pm is not a document
        assert_eq!(children[1].0, "swe");
        assert_eq!(children[1].1, "/agents/swe");
        assert_eq!(children[1].2, 2);
        assert!(!children[1].3); // /agents/swe is not a document

        // Prefix without trailing slash works the same
        let children = store.list_document_path_children("/agents").await.unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].0, "pm");
        assert_eq!(children[1].0, "swe");

        // Prefix with no matching documents returns empty
        let children = store
            .list_document_path_children("/nonexistent/")
            .await
            .unwrap();
        assert!(children.is_empty());
    }

    #[tokio::test]
    async fn list_document_path_children_excludes_deleted() {
        let store = create_test_store().await;

        let (doc_id, _) = store
            .add_document(
                sample_document(Some("agents/swe/memory.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("agents/pm/notes.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // Delete one document
        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        // Only the non-deleted document's segment should appear
        let children = store.list_document_path_children("/agents/").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, "pm");
        assert_eq!(children[0].2, 1);
    }

    #[tokio::test]
    async fn list_document_path_children_reports_is_document() {
        let store = create_test_store().await;

        // Create a leaf document at /agents/pm/notes.md
        store
            .add_document(
                sample_document(Some("agents/pm/notes.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Create a document whose path is also a prefix for other docs
        store
            .add_document(sample_document(Some("agents/pm")), &ActorRef::test())
            .await
            .unwrap();

        // At prefix "/agents/", "pm" should be is_document=true (path /agents/pm exists as a doc)
        let children = store.list_document_path_children("/agents/").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, "pm");
        assert_eq!(children[0].2, 2); // 2 docs: /agents/pm and /agents/pm/notes.md
        assert!(children[0].3); // /agents/pm IS a document

        // At prefix "/agents/pm/", "notes.md" should be is_document=true
        let children = store
            .list_document_path_children("/agents/pm/")
            .await
            .unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, "notes.md");
        assert!(children[0].3); // /agents/pm/notes.md IS a document

        // At prefix "/", "agents" is NOT a document
        let children = store.list_document_path_children("/").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, "agents");
        assert!(!children[0].3); // /agents is NOT a document
    }

    #[tokio::test]
    async fn document_filters_apply_query() {
        let store = create_test_store().await;

        let (first, _) = store
            .add_document(sample_document(Some("docs/howto.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("notes/todo.md")), &ActorRef::test())
            .await
            .unwrap();

        let query = SearchDocumentsQuery::new(
            Some("how".to_string()),
            Some("/docs/".to_string()),
            None,
            None,
        );

        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, first);
    }

    #[tokio::test]
    async fn list_documents_filters_by_ids() {
        let store = create_test_store().await;

        let (a, _) = store
            .add_document(sample_document(Some("docs/a.md")), &ActorRef::test())
            .await
            .unwrap();
        let (b, _) = store
            .add_document(sample_document(Some("docs/b.md")), &ActorRef::test())
            .await
            .unwrap();
        let (_c, _) = store
            .add_document(sample_document(Some("notes/c.md")), &ActorRef::test())
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

    #[tokio::test]
    async fn delete_document_sets_deleted_flag_and_filters_from_list() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(None), &ActorRef::test())
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
            .list_documents(&SearchDocumentsQuery::new(None, None, None, Some(true)))
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
        let doc = sample_document(Some("docs/roundtrip.md"));

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

    // ---- Task helpers ----

    fn spawn_task() -> Session {
        spawn_task_with_prompt("test prompt")
    }

    fn spawn_task_with_prompt(prompt: &str) -> Session {
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::new(None, None, Some(prompt.to_string()), None),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("hydra-worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
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

        let task = spawn_task_with_prompt("v1");
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let updated = spawn_task_with_prompt("v2");
        store
            .update_session(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        let expected = updated.clone();
        assert_versioned(&fetched, &expected, 2);
    }

    #[tokio::test]
    async fn task_get_versions_returns_ordered_entries() {
        let store = create_test_store().await;

        let task = spawn_task_with_prompt("v1");
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let v2 = spawn_task_with_prompt("v2");
        store
            .update_session(&task_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_session_versions(&task_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
        assert_eq!(versions[0].item.mode, SessionMode::Headless);
        assert_eq!(
            versions[0].item.agent_config.system_prompt.as_deref(),
            Some("v1")
        );
        assert_eq!(versions[1].item.mode, SessionMode::Headless);
        assert_eq!(
            versions[1].item.agent_config.system_prompt.as_deref(),
            Some("v2")
        );
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
    async fn task_list_filters_by_creator() {
        let store = create_test_store().await;

        let mut task_alice = spawn_task();
        task_alice.creator = Username::from("alice");
        let (alice_id, _) = store
            .add_session(task_alice, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_bob = spawn_task();
        task_bob.creator = Username::from("bob");
        store
            .add_session(task_bob, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.creator = Some("alice".to_string());
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, alice_id);
        assert_eq!(tasks[0].1.item.creator, Username::from("alice"));
    }

    #[tokio::test]
    async fn task_list_filters_by_conversation_id() {
        let store = create_test_store().await;

        let conv_a = ConversationId::new();
        let conv_b = ConversationId::new();

        let (sid_a, _) = store
            .add_session(
                interactive_session(Some(conv_a.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_session(
                interactive_session(Some(conv_b.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Non-interactive (no `interactive`, so no conversation link).
        store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(conv_a.clone());
        let tasks = store.list_sessions(&query).await.unwrap();
        let ids: Vec<_> = tasks.into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![sid_a]);

        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(ConversationId::new());
        let tasks = store.list_sessions(&query).await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn task_list_filters_by_text_search() {
        let store = create_test_store().await;

        let task1 = spawn_task_with_prompt("deploy to production");
        store
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let task2 = spawn_task_with_prompt("run tests");
        store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let query = SearchSessionsQuery::new(Some("deploy".to_string()), None, None, vec![]);
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(matches!(&tasks[0].1.item.mode, SessionMode::Headless));
        assert_eq!(
            tasks[0].1.item.agent_config.system_prompt.as_deref(),
            Some("deploy to production")
        );
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
            vec![hydra_common::task_status::Status::Running],
        );
        let tasks = store.list_sessions(&query).await.unwrap();
        assert_eq!(tasks.len(), 1);

        // Search for created tasks - should be empty since task is now running
        let query = SearchSessionsQuery::new(
            None,
            None,
            None,
            vec![hydra_common::task_status::Status::Created],
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
                hydra_common::task_status::Status::Created,
                hydra_common::task_status::Status::Running,
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
            vec![hydra_common::task_status::Status::Complete],
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
                hydra_common::task_status::Status::Running,
                hydra_common::task_status::Status::Complete,
                hydra_common::task_status::Status::Failed,
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
        let mcp_config = serde_json::json!({"mcpServers": {"playwright": {"command": "npx", "args": ["@anthropic/mcp-playwright"]}}});
        let mut task = Session::new(
            Username::from("alice"),
            None,
            None,
            AgentConfig::new(
                None,
                Some("claude-3".to_string()),
                Some("full test".to_string()),
                Some(mcp_config),
            ),
            crate::routes::sessions::mount_spec_from_create_request(
                hydra_common::api::v1::sessions::Bundle::None,
                None,
            ),
            Some("my-image:v1".to_string()),
            HashMap::from([("KEY".to_string(), "VALUE".to_string())]),
            Some("2".to_string()),
            Some("4Gi".to_string()),
            Some(vec!["secret1".to_string(), "secret2".to_string()]),
            SessionMode::Headless,
            Status::Pending,
            Some("last msg".to_string()),
            Some(TaskError::JobEngineError {
                reason: "test error".to_string(),
            }),
        );
        task.usage = Some(hydra_common::sessions::TokenUsage {
            input_tokens: 4321,
            output_tokens: 765,
            cache_read_input_tokens: 21,
            cache_creation_input_tokens: 5,
        });

        let now = Utc::now();
        let (task_id, _) = store
            .add_session(task.clone(), now, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // add_task sets creation_time on the stored task.
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
            None,
            3,
            i32::MAX,
            false,
            false,
            Vec::new(),
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
        assert!(!fetched.is_default_conversation_agent);
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

    #[tokio::test]
    async fn default_conversation_agent_uniqueness_on_add() {
        let store = create_test_store().await;
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        let err = store.add_agent(chat2).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationAgentAlreadyExists));
    }

    #[tokio::test]
    async fn default_conversation_agent_uniqueness_on_update() {
        let store = create_test_store().await;
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_default_conversation_agent = true;
        let err = store.update_agent(swe_updated).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationAgentAlreadyExists));
    }

    #[tokio::test]
    async fn default_conversation_agent_can_update_itself() {
        let store = create_test_store().await;
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let mut chat_updated = sample_agent("chat");
        chat_updated.is_default_conversation_agent = true;
        chat_updated.max_tries = 10;
        store.update_agent(chat_updated).await.unwrap();

        let fetched = store.get_agent("chat").await.unwrap();
        assert_eq!(fetched.max_tries, 10);
        assert!(fetched.is_default_conversation_agent);
    }

    #[tokio::test]
    async fn deleted_default_conversation_agent_allows_new_one() {
        let store = create_test_store().await;
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();
        store.delete_agent("chat").await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        store.add_agent(chat2).await.unwrap();

        let fetched = store.get_agent("chat2").await.unwrap();
        assert!(fetched.is_default_conversation_agent);
    }

    #[tokio::test]
    async fn default_conversation_agent_survives_server_restart() {
        // Same SQLite file is reopened by a fresh pool, so the flag must persist.
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("agents.db");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());

        {
            let pool = SqliteStore::init_pool(&url).await.unwrap();
            SqliteStore::run_migrations(&pool).await.unwrap();
            let store = SqliteStore::new(pool);
            let mut chat = sample_agent("chat");
            chat.is_default_conversation_agent = true;
            store.add_agent(chat).await.unwrap();
        }

        let pool = SqliteStore::init_pool(&url).await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();
        let store = SqliteStore::new(pool);
        let fetched = store.get_agent("chat").await.unwrap();
        assert!(fetched.is_default_conversation_agent);
    }

    #[tokio::test]
    async fn agent_secrets_round_trip() {
        let store = create_test_store().await;
        let agent = Agent::new(
            "swe".to_string(),
            "/agents/swe/prompt.md".to_string(),
            Some("/agents/swe/mcp-config.json".to_string()),
            3,
            i32::MAX,
            false,
            false,
            vec!["OPENAI_API_KEY".to_string(), "GITHUB_TOKEN".to_string()],
        );
        store.add_agent(agent).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(
            fetched.secrets,
            vec!["OPENAI_API_KEY".to_string(), "GITHUB_TOKEN".to_string()]
        );

        // Update secrets
        let mut updated = fetched;
        updated.secrets = vec!["NEW_SECRET".to_string()];
        store.update_agent(updated).await.unwrap();

        let fetched2 = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched2.secrets, vec!["NEW_SECRET".to_string()]);
    }

    #[tokio::test]
    async fn agent_default_secrets_is_empty() {
        let store = create_test_store().await;
        let agent = sample_agent("swe");
        store.add_agent(agent).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert!(fetched.secrets.is_empty());
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

        let issue_id: HydraId = IssueId::new().into();

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

        let issue_id: HydraId = IssueId::new().into();

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

        let issue1: HydraId = IssueId::new().into();
        let issue2: HydraId = IssueId::new().into();
        let issue3: HydraId = IssueId::new().into();

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

    // ---- Auth token tests ----

    #[tokio::test]
    async fn auth_tokens_add_and_get() {
        let store = create_test_store().await;
        store
            .add_auth_token("users/alice", "hash1", None)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash2", None)
            .await
            .unwrap();

        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string(), "hash2".to_string()]);
    }

    #[tokio::test]
    async fn auth_tokens_get_empty() {
        let store = create_test_store().await;
        let hashes = store.get_auth_token_hashes("users/nobody").await.unwrap();
        assert!(hashes.is_empty());
    }

    #[tokio::test]
    async fn auth_tokens_delete_for_actor() {
        let store = create_test_store().await;
        store
            .add_auth_token("users/alice", "hash1", None)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash2", None)
            .await
            .unwrap();
        store
            .delete_auth_tokens_for_actor("users/alice")
            .await
            .unwrap();

        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert!(hashes.is_empty());
    }

    #[tokio::test]
    async fn auth_tokens_duplicate_insert_is_idempotent() {
        let store = create_test_store().await;
        store
            .add_auth_token("users/alice", "hash1", None)
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash1", None)
            .await
            .unwrap();

        let hashes = store.get_auth_token_hashes("users/alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string()]);
    }

    #[tokio::test]
    async fn auth_tokens_by_hash_with_session_id_round_trips() {
        let store = create_test_store().await;
        let sid = SessionId::new();
        store
            .add_auth_token("agents/swe", "hash-sess", Some(&sid))
            .await
            .unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("token row should exist");
        assert_eq!(row.actor_name, "agents/swe");
        assert_eq!(row.session_id, Some(sid));
    }

    #[tokio::test]
    async fn auth_tokens_by_hash_without_session_id_round_trips() {
        let store = create_test_store().await;
        store
            .add_auth_token("users/alice", "hash-user", None)
            .await
            .unwrap();

        let row = store
            .get_auth_token_by_hash("hash-user")
            .await
            .unwrap()
            .expect("token row should exist");
        assert_eq!(row.actor_name, "users/alice");
        assert_eq!(row.session_id, None);
    }

    #[tokio::test]
    async fn auth_tokens_by_hash_missing_returns_none() {
        let store = create_test_store().await;
        let row = store.get_auth_token_by_hash("nope").await.unwrap();
        assert!(row.is_none());
    }

    /// Fresh rows must come back with `is_revoked = false`, and
    /// `revoke_auth_tokens_for_session` must flip exactly the rows
    /// matching the given session id without touching siblings.
    #[tokio::test]
    async fn revoke_auth_tokens_flips_only_target_session() {
        let store = create_test_store().await;
        let sid = SessionId::new();
        let other_sid = SessionId::new();
        store
            .add_auth_token("agents/swe", "hash-sess", Some(&sid))
            .await
            .unwrap();
        store
            .add_auth_token("agents/swe", "hash-other", Some(&other_sid))
            .await
            .unwrap();
        store
            .add_auth_token("users/alice", "hash-user", None)
            .await
            .unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("session-scoped token should exist before revocation");
        assert!(
            !row.is_revoked,
            "fresh row must default to is_revoked=false"
        );

        store.revoke_auth_tokens_for_session(&sid).await.unwrap();

        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("session-scoped token row should still exist after revoke");
        assert!(row.is_revoked, "revoked row must be marked is_revoked=true");

        let other = store
            .get_auth_token_by_hash("hash-other")
            .await
            .unwrap()
            .expect("sibling session token should still exist");
        assert!(!other.is_revoked, "sibling session must not be revoked");

        let user = store
            .get_auth_token_by_hash("hash-user")
            .await
            .unwrap()
            .expect("user token should still exist");
        assert!(
            !user.is_revoked,
            "session-less user token must not be revoked"
        );
    }

    /// `revoke_auth_tokens_for_session` must be idempotent — calling it
    /// twice for the same session is harmless, and revoking a session
    /// with no minted tokens is a no-op.
    #[tokio::test]
    async fn revoke_auth_tokens_is_idempotent_and_handles_no_match() {
        let store = create_test_store().await;
        let sid = SessionId::new();
        store
            .add_auth_token("agents/swe", "hash-sess", Some(&sid))
            .await
            .unwrap();

        // Revoking a session with no rows is a no-op (doesn't error).
        let no_match = SessionId::new();
        store
            .revoke_auth_tokens_for_session(&no_match)
            .await
            .unwrap();

        // Two consecutive revocations of the same session leave the row
        // in the same state.
        store.revoke_auth_tokens_for_session(&sid).await.unwrap();
        store.revoke_auth_tokens_for_session(&sid).await.unwrap();
        let row = store
            .get_auth_token_by_hash("hash-sess")
            .await
            .unwrap()
            .expect("row should still exist after double revocation");
        assert!(row.is_revoked);
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
    async fn internal_and_external_secret_coexist() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        // Set internal then external version of the same secret
        store
            .set_user_secret(&username, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        // get_user_secret should return the external version
        let fetched = store.get_user_secret(&username, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"external_val".to_vec()));
    }

    #[tokio::test]
    async fn get_user_secret_returns_internal_when_only_internal_exists() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        store
            .set_user_secret(&username, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();

        let fetched = store.get_user_secret(&username, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"internal_val".to_vec()));
    }

    #[tokio::test]
    async fn delete_user_secret_only_removes_external() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        // Set both internal and external
        store
            .set_user_secret(&username, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        // Delete should only remove external
        store
            .delete_user_secret(&username, "MY_SECRET")
            .await
            .unwrap();

        // Should fall back to internal
        let fetched = store.get_user_secret(&username, "MY_SECRET").await.unwrap();
        assert_eq!(fetched, Some(b"internal_val".to_vec()));
    }

    #[tokio::test]
    async fn list_user_secret_names_deduplicates_coexisting() {
        let store = create_test_store().await;
        let username = Username::from("alice".to_string());

        // Set both internal and external for the same secret
        store
            .set_user_secret(&username, "MY_SECRET", b"internal_val", true)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "MY_SECRET", b"external_val", false)
            .await
            .unwrap();

        let refs = store.list_user_secret_names(&username).await.unwrap();
        // Should only appear once, reported as non-internal
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "MY_SECRET");
        assert!(!refs[0].internal);
    }

    /// Schema as of migration 20260316000000_add_internal_to_user_secrets — the state
    /// the broken 20260330000000 migration starts from.
    async fn setup_pre_composite_pk_user_secrets(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE user_secrets ( \
                username TEXT NOT NULL, \
                secret_name TEXT NOT NULL, \
                encrypted_value BLOB NOT NULL, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                PRIMARY KEY (username, secret_name))",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("ALTER TABLE user_secrets ADD COLUMN internal BOOLEAN NOT NULL DEFAULT FALSE")
            .execute(pool)
            .await
            .unwrap();
    }

    /// Verbatim body of 20260330000000_user_secrets_composite_pk.sql — kept inline so
    /// the regression test reproduces the original scrambling bug end-to-end.
    async fn apply_broken_composite_pk_migration(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS user_secrets_new ( \
                username TEXT NOT NULL, \
                secret_name TEXT NOT NULL, \
                encrypted_value BLOB NOT NULL, \
                internal BOOLEAN NOT NULL DEFAULT FALSE, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                PRIMARY KEY (username, secret_name, internal))",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO user_secrets_new SELECT * FROM user_secrets")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE user_secrets")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("ALTER TABLE user_secrets_new RENAME TO user_secrets")
            .execute(pool)
            .await
            .unwrap();
    }

    /// Apply the repair migration file directly so the test exercises the actual
    /// shipped SQL.
    async fn apply_repair_migration(pool: &SqlitePool) {
        let sql = include_str!(
            "../../sqlite-migrations/20260512100000_repair_scrambled_user_secrets.sql"
        );
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn repair_migration_unscrambles_user_secrets() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_composite_pk_user_secrets(&pool).await;

        // Column order matches the post-20260316 schema: internal is last.
        sqlx::query(
            "INSERT INTO user_secrets \
             (username, secret_name, encrypted_value, created_at, updated_at, internal) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind("alice")
        .bind("CLAUDE_CODE_OAUTH_TOKEN")
        .bind(&b"orig"[..])
        .bind("2026-03-01T10:00:00.000+00:00")
        .bind("2026-03-02T10:00:00.000+00:00")
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        apply_broken_composite_pk_migration(&pool).await;
        apply_repair_migration(&pool).await;

        let store = SqliteStore::new(pool.clone());
        let username = Username::from("alice".to_string());

        let value = store
            .get_user_secret(&username, "CLAUDE_CODE_OAUTH_TOKEN")
            .await
            .unwrap();
        assert_eq!(value, Some(b"orig".to_vec()));

        let refs = store.list_user_secret_names(&username).await.unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "CLAUDE_CODE_OAUTH_TOKEN");
        assert!(!refs[0].internal);

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_secrets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[tokio::test]
    async fn repair_migration_dedupes_zombie_rows() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_composite_pk_user_secrets(&pool).await;

        sqlx::query(
            "INSERT INTO user_secrets \
             (username, secret_name, encrypted_value, created_at, updated_at, internal) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind("alice")
        .bind("CLAUDE_CODE_OAUTH_TOKEN")
        .bind(&b"orig"[..])
        .bind("2026-03-01T10:00:00.000+00:00")
        .bind("2026-03-02T10:00:00.000+00:00")
        .bind(false)
        .execute(&pool)
        .await
        .unwrap();

        apply_broken_composite_pk_migration(&pool).await;

        // After the bad migration, simulate `set_user_secret`. The UPSERT's
        // ON CONFLICT (username, secret_name, internal) cannot match the scrambled
        // PK (whose `internal` holds a timestamp), so a second row is inserted.
        let store = SqliteStore::new(pool.clone());
        let username = Username::from("alice".to_string());
        store
            .set_user_secret(&username, "CLAUDE_CODE_OAUTH_TOKEN", b"updated", false)
            .await
            .unwrap();

        let pre_repair: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_secrets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pre_repair, 2, "expected zombie pair before repair");

        apply_repair_migration(&pool).await;

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_secrets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 1);

        // Dedupe should prefer the zombie row's (newer) value.
        let value = store
            .get_user_secret(&username, "CLAUDE_CODE_OAUTH_TOKEN")
            .await
            .unwrap();
        assert_eq!(value, Some(b"updated".to_vec()));

        let refs = store.list_user_secret_names(&username).await.unwrap();
        assert_eq!(refs.len(), 1);
        assert!(!refs[0].internal);
    }

    #[tokio::test]
    async fn repair_migration_is_noop_on_clean_install() {
        // On a database whose user_secrets table only ever saw the (correctly typed)
        // post-composite-PK schema and well-formed rows, the repair must not touch
        // any data.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        SqliteStore::run_migrations(&pool).await.unwrap();

        let store = SqliteStore::new(pool.clone());
        let username = Username::from("alice".to_string());
        store
            .set_user_secret(&username, "EXTERNAL_KEY", b"ext", false)
            .await
            .unwrap();
        store
            .set_user_secret(&username, "GITHUB_TOKEN", b"int", true)
            .await
            .unwrap();

        // The repair migration has already been applied by run_migrations above;
        // running it again must remain a no-op.
        apply_repair_migration(&pool).await;

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_secrets")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 2);

        let ext = store
            .get_user_secret(&username, "EXTERNAL_KEY")
            .await
            .unwrap();
        assert_eq!(ext, Some(b"ext".to_vec()));
        let int = store
            .get_user_secret(&username, "GITHUB_TOKEN")
            .await
            .unwrap();
        assert_eq!(int, Some(b"int".to_vec()));
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
            IssueStatus::Closed,
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
        let query =
            hydra_common::api::v1::issues::SearchIssuesQuery::new(None, vec![], None, None, None);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);

        // Count only bugs
        let query = hydra_common::api::v1::issues::SearchIssuesQuery::new(
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            None,
            None,
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);

        // Count only closed
        let query = hydra_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            vec![hydra_common::api::v1::issues::IssueStatus::Closed],
            None,
            None,
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
            hydra_common::api::v1::patches::SearchPatchesQuery::new(None, None, Vec::new(), None);
        assert_eq!(store.count_patches(&query).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn count_issues_filters_by_assignee_principal() {
        use hydra_common::api::v1::agents::AgentName;
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::principal::Principal;

        let store = create_test_store().await;
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

        let query = hydra_common::api::v1::issues::SearchIssuesQuery::new(
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

    #[tokio::test]
    async fn count_patches_filters_by_creator() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        let patch_a = Patch::new(
            "patch a".to_string(),
            "patch a".to_string(),
            dummy_diff(),
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
            dummy_diff(),
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

        let mut query =
            hydra_common::api::v1::patches::SearchPatchesQuery::new(None, None, Vec::new(), None);
        query.creator = Some("alice".to_string());
        assert_eq!(store.count_patches(&query).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn count_documents_returns_total_matching() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        store
            .add_document(sample_document(Some("docs/a.md")), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("docs/b.md")), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("other/c.md")), &actor)
            .await
            .unwrap();

        // Count all
        let query =
            hydra_common::api::v1::documents::SearchDocumentsQuery::new(None, None, None, None);
        assert_eq!(store.count_documents(&query).await.unwrap(), 3);

        // Count with path prefix filter
        let query = hydra_common::api::v1::documents::SearchDocumentsQuery::new(
            Some("docs/".to_string()),
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
            hydra_common::api::v1::sessions::SearchSessionsQuery::new(None, None, None, vec![]);
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
        let mut query =
            hydra_common::api::v1::issues::SearchIssuesQuery::new(None, vec![], None, None, None);
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
            .add_document(sample_document(None), &actor)
            .await
            .unwrap();

        let source = HydraId::from(issue_id.clone());
        let target = HydraId::from(doc_id.clone());

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
    async fn refers_to_relationship_round_trip_conversation_to_issue() {
        use crate::store::{ObjectKind, RelationshipType};

        let store = create_test_store().await;
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let conversation_id = hydra_common::ConversationId::new();

        let source = HydraId::from(conversation_id.clone());
        let target = HydraId::from(issue_id.clone());

        store
            .add_relationship(&source, &target, RelationshipType::RefersTo)
            .await
            .unwrap();

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

    #[tokio::test]
    async fn update_issue_preserves_has_document_relationships() {
        use crate::store::RelationshipType;

        let store = create_test_store().await;
        let actor = ActorRef::test();

        let (issue_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (doc_id, _) = store
            .add_document(sample_document(None), &actor)
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

    /// Regression test for the migration that dropped the `dependencies` and
    /// `patches` JSON columns from `issues_v2`. After the drop, the read path
    /// must still reconstitute these Vec fields from `object_relationships`.
    #[tokio::test]
    async fn drop_deps_patches_columns_preserves_relationships() {
        let store = create_test_store().await;
        let actor = ActorRef::test();

        let (parent_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (blocker_id, _) = store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        let (patch_id, _) = store.add_patch(sample_patch(), &actor).await.unwrap();

        let dependencies = vec![
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
            IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone()),
        ];
        let mut issue = sample_issue(dependencies.clone());
        issue.patches = vec![patch_id.clone()];

        let (issue_id, _) = store.add_issue(issue, &actor).await.unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();

        // Order from object_relationships isn't guaranteed; compare as sets.
        let mut fetched_deps = fetched.item.dependencies.clone();
        let mut expected_deps = dependencies.clone();
        fetched_deps.sort_by(|a, b| a.issue_id.as_ref().cmp(b.issue_id.as_ref()));
        expected_deps.sort_by(|a, b| a.issue_id.as_ref().cmp(b.issue_id.as_ref()));
        assert_eq!(
            fetched_deps, expected_deps,
            "dependencies must round-trip via object_relationships after column drop"
        );
        assert_eq!(
            fetched.item.patches,
            vec![patch_id],
            "patches must round-trip via object_relationships after column drop"
        );
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

    #[tokio::test]
    async fn get_relationships_transitive_follows_same_type_only() {
        use crate::store::{RelationshipType, TransitiveDirection};

        let store = create_test_store().await;
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

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_existing() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/unique.md")), &ActorRef::test())
            .await
            .unwrap();

        let found = store
            .find_non_deleted_document_by_exact_path("/docs/unique.md")
            .await
            .unwrap();
        assert_eq!(found, Some(doc_id));
    }

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_none_for_deleted() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/deleted.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        let found = store
            .find_non_deleted_document_by_exact_path("/docs/deleted.md")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_none_for_missing() {
        let store = create_test_store().await;
        let found = store
            .find_non_deleted_document_by_exact_path("/docs/nonexistent.md")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    /// Helper to query is_latest values for a given document id, ordered by version_number.
    async fn get_is_latest_flags(store: &SqliteStore, doc_id: &DocumentId) -> Vec<(i64, i64)> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT version_number, is_latest FROM documents_v2 WHERE id = ?1 ORDER BY version_number",
        )
        .bind(doc_id.as_ref())
        .fetch_all(&store.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn is_latest_set_on_new_document() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/test.md")), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_is_latest_flags(&store, &doc_id).await;
        assert_eq!(flags, vec![(1, 1)]);
    }

    #[tokio::test]
    async fn is_latest_updated_on_document_update() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/test.md")), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_document(Some("docs/test.md"));
        updated.body_markdown = "Updated body".to_string();
        store
            .update_document(&doc_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_is_latest_flags(&store, &doc_id).await;
        // Version 1 should have is_latest = 0, version 2 should have is_latest = 1
        assert_eq!(flags, vec![(1, 0), (2, 1)]);

        // A third update should only keep the newest as latest
        updated.body_markdown = "Third version".to_string();
        store
            .update_document(&doc_id, updated, &ActorRef::test())
            .await
            .unwrap();

        let flags = get_is_latest_flags(&store, &doc_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 0), (3, 1)]);
    }

    #[tokio::test]
    async fn is_latest_maintained_on_delete() {
        let store = create_test_store().await;
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/test.md")), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        let flags = get_is_latest_flags(&store, &doc_id).await;
        // Version 1 is the original, version 2 is the soft-delete; only version 2 should be latest
        assert_eq!(flags, vec![(1, 0), (2, 1)]);
    }

    #[tokio::test]
    async fn is_latest_independent_across_documents() {
        let store = create_test_store().await;
        let (doc1, _) = store
            .add_document(sample_document(Some("docs/one.md")), &ActorRef::test())
            .await
            .unwrap();
        let (doc2, _) = store
            .add_document(sample_document(Some("docs/two.md")), &ActorRef::test())
            .await
            .unwrap();

        // Update doc1 only
        let mut updated = sample_document(Some("docs/one.md"));
        updated.body_markdown = "Updated".to_string();
        store
            .update_document(&doc1, updated, &ActorRef::test())
            .await
            .unwrap();

        // doc1 should have version 1 not latest, version 2 latest
        let flags1 = get_is_latest_flags(&store, &doc1).await;
        assert_eq!(flags1, vec![(1, 0), (2, 1)]);

        // doc2 should still have version 1 as latest
        let flags2 = get_is_latest_flags(&store, &doc2).await;
        assert_eq!(flags2, vec![(1, 1)]);
    }

    // ---- is_latest tests for issues ----

    /// Helper to query is_latest values for a given issue id, ordered by version_number.
    async fn get_issue_is_latest_flags(store: &SqliteStore, issue_id: &IssueId) -> Vec<(i64, i64)> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT version_number, is_latest FROM issues_v2 WHERE id = ?1 ORDER BY version_number",
        )
        .bind(issue_id.as_ref())
        .fetch_all(&store.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn is_latest_set_on_new_issue() {
        let store = create_test_store().await;
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_issue_is_latest_flags(&store, &issue_id).await;
        assert_eq!(flags, vec![(1, 1)]);
    }

    #[tokio::test]
    async fn is_latest_updated_on_issue_update() {
        let store = create_test_store().await;
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_issue(vec![]);
        updated.progress = "50%".to_string();
        store
            .update_issue(&issue_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_issue_is_latest_flags(&store, &issue_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 1)]);

        // A third update should only keep the newest as latest
        updated.progress = "100%".to_string();
        store
            .update_issue(&issue_id, updated, &ActorRef::test())
            .await
            .unwrap();

        let flags = get_issue_is_latest_flags(&store, &issue_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 0), (3, 1)]);
    }

    #[tokio::test]
    async fn is_latest_maintained_on_issue_delete() {
        let store = create_test_store().await;
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        let flags = get_issue_is_latest_flags(&store, &issue_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 1)]);
    }

    // ---- is_latest tests for tasks ----

    /// Helper to query is_latest values for a given task id, ordered by version_number.
    async fn get_task_is_latest_flags(store: &SqliteStore, task_id: &SessionId) -> Vec<(i64, i64)> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT version_number, is_latest FROM tasks_v2 WHERE id = ?1 ORDER BY version_number",
        )
        .bind(task_id.as_ref())
        .fetch_all(&store.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn is_latest_set_on_new_task() {
        let store = create_test_store().await;
        let (task_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_task_is_latest_flags(&store, &task_id).await;
        assert_eq!(flags, vec![(1, 1)]);
    }

    #[tokio::test]
    async fn is_latest_updated_on_task_update() {
        let store = create_test_store().await;
        let (task_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = spawn_task();
        updated.status = Status::Running;
        store
            .update_session(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let flags = get_task_is_latest_flags(&store, &task_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 1)]);

        // A third update should only keep the newest as latest
        updated.status = Status::Complete;
        store
            .update_session(&task_id, updated, &ActorRef::test())
            .await
            .unwrap();

        let flags = get_task_is_latest_flags(&store, &task_id).await;
        assert_eq!(flags, vec![(1, 0), (2, 0), (3, 1)]);
    }

    // ---- Conversation tests ----

    fn sample_conversation() -> Conversation {
        use hydra_common::api::v1::agents::AgentName;
        Conversation {
            title: Some("Test conversation".to_string()),
            agent_name: Some(AgentName::try_new("test-agent").unwrap()),
            status: crate::domain::conversations::ConversationStatus::Active,
            creator: Username::from("testuser".to_string()),
            session_settings: Default::default(),
            deleted: false,
        }
    }

    #[tokio::test]
    async fn conversation_crud_round_trip() {
        let store = create_test_store().await;
        let conversation = sample_conversation();

        let (id, version) = store
            .add_conversation(conversation.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_versioned(&fetched, &conversation, 1);
    }

    #[tokio::test]
    async fn conversation_update_bumps_version() {
        let store = create_test_store().await;
        let conversation = sample_conversation();

        let (id, _) = store
            .add_conversation(conversation, &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_conversation();
        updated.title = Some("Updated title".to_string());
        updated.status = crate::domain::conversations::ConversationStatus::Idle;

        let v2 = store
            .update_conversation(&id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_versioned(&fetched, &updated, 2);
    }

    #[tokio::test]
    async fn conversation_not_found() {
        let store = create_test_store().await;
        let fake_id = ConversationId::new();
        let result = store.get_conversation(&fake_id, false).await;
        assert!(matches!(result, Err(StoreError::ConversationNotFound(_))));
    }

    #[tokio::test]
    async fn get_conversation_versions_returns_one_row_per_update() {
        use crate::domain::conversations::ConversationStatus;
        let store = create_test_store().await;
        let (id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        // After insert, exactly one version exists (the create row).
        let versions = store.get_conversation_versions(&id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].item.status, ConversationStatus::Active);

        // Each `update_conversation` adds a new versioned row carrying the
        // current status. This is the new lifecycle log.
        let mut updated = versions[0].item.clone();
        updated.status = ConversationStatus::Idle;
        store
            .update_conversation(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        let mut updated2 = store.get_conversation(&id, false).await.unwrap().item;
        updated2.status = ConversationStatus::Closed;
        store
            .update_conversation(&id, updated2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_conversation_versions(&id).await.unwrap();
        assert_eq!(versions.len(), 3);
        let statuses: Vec<ConversationStatus> = versions.iter().map(|v| v.item.status).collect();
        assert_eq!(
            statuses,
            vec![
                ConversationStatus::Active,
                ConversationStatus::Idle,
                ConversationStatus::Closed,
            ]
        );
        let version_numbers: Vec<_> = versions.iter().map(|v| v.version).collect();
        assert_eq!(version_numbers, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn get_conversation_versions_not_found_for_missing_conversation() {
        let store = create_test_store().await;
        let id = ConversationId::new();
        let err = store.get_conversation_versions(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationNotFound(_)));
    }

    #[tokio::test]
    async fn list_conversations_filters_by_status() {
        let store = create_test_store().await;
        let mut conv1 = sample_conversation();
        conv1.status = crate::domain::conversations::ConversationStatus::Active;
        let mut conv2 = sample_conversation();
        conv2.status = crate::domain::conversations::ConversationStatus::Closed;

        store
            .add_conversation(conv1, &ActorRef::test())
            .await
            .unwrap();
        store
            .add_conversation(conv2, &ActorRef::test())
            .await
            .unwrap();

        let query = SearchConversationsQuery {
            status: Some(hydra_common::api::v1::conversations::ConversationStatus::Active),
            ..Default::default()
        };
        let results = store.list_conversations(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].1.item.status,
            crate::domain::conversations::ConversationStatus::Active,
        );
    }

    #[tokio::test]
    async fn list_conversations_filters_by_creator() {
        let store = create_test_store().await;
        let mut conv1 = sample_conversation();
        conv1.creator = Username::from("alice".to_string());
        let mut conv2 = sample_conversation();
        conv2.creator = Username::from("bob".to_string());

        store
            .add_conversation(conv1, &ActorRef::test())
            .await
            .unwrap();
        store
            .add_conversation(conv2, &ActorRef::test())
            .await
            .unwrap();

        let query = SearchConversationsQuery {
            creator: Some("alice".to_string()),
            ..Default::default()
        };
        let results = store.list_conversations(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].1.item.creator,
            Username::from("alice".to_string())
        );
    }

    #[tokio::test]
    async fn list_conversations_text_search() {
        let store = create_test_store().await;
        let mut conv1 = sample_conversation();
        conv1.title = Some("Meeting notes".to_string());
        let mut conv2 = sample_conversation();
        conv2.title = Some("Code review".to_string());

        store
            .add_conversation(conv1, &ActorRef::test())
            .await
            .unwrap();
        store
            .add_conversation(conv2, &ActorRef::test())
            .await
            .unwrap();

        let query = SearchConversationsQuery {
            q: Some("meeting".to_string()),
            ..Default::default()
        };
        let results = store.list_conversations(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.title, Some("Meeting notes".to_string()),);
    }

    #[tokio::test]
    async fn list_conversations_filters_deleted() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        // Conversation should be visible in list initially
        let results = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1.item.deleted);

        // Soft-delete the conversation
        let mut deleted_conv = sample_conversation();
        deleted_conv.deleted = true;
        store
            .update_conversation(&id, deleted_conv, &ActorRef::test())
            .await
            .unwrap();

        // Deleted conversation should not appear in default list
        let results = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert!(results.is_empty());

        // Deleted conversation should appear with include_deleted=true
        let results = store
            .list_conversations(&SearchConversationsQuery {
                include_deleted: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.item.deleted);
    }

    #[tokio::test]
    async fn get_conversation_filters_deleted() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        // Conversation is accessible when not deleted
        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_eq!(fetched.item.title.as_deref(), Some("Test conversation"));

        // Soft-delete the conversation
        let mut deleted_conv = sample_conversation();
        deleted_conv.deleted = true;
        store
            .update_conversation(&id, deleted_conv, &ActorRef::test())
            .await
            .unwrap();

        // get_conversation with include_deleted=false should return ConversationNotFound
        let err = store.get_conversation(&id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationNotFound(_)));

        // get_conversation with include_deleted=true should return the deleted conversation
        let fetched = store.get_conversation(&id, true).await.unwrap();
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn get_documents_by_paths_returns_titles_for_live_docs() {
        let store = create_test_store().await;

        let mut doc_a = sample_document(Some("agents/swe/prompt.md"));
        doc_a.title = "SWE Prompt".to_string();
        let (id_a, _) = store.add_document(doc_a, &ActorRef::test()).await.unwrap();

        let mut doc_b = sample_document(Some("agents/pm/prompt.md"));
        doc_b.title = "PM Prompt".to_string();
        let (id_b, _) = store.add_document(doc_b, &ActorRef::test()).await.unwrap();

        // A document the caller will not ask about — ensures filtering works.
        store
            .add_document(sample_document(Some("notes/unused.md")), &ActorRef::test())
            .await
            .unwrap();

        let paths = vec![
            "/agents/swe/prompt.md".to_string(),
            "/agents/pm/prompt.md".to_string(),
            // Duplicate input path must not produce a duplicate result.
            "/agents/pm/prompt.md".to_string(),
            // Non-matching path must be silently skipped.
            "/agents/missing.md".to_string(),
        ];
        let mut results = store.get_documents_by_paths(&paths).await.unwrap();
        results.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "/agents/pm/prompt.md");
        assert_eq!(results[0].1, id_b);
        assert_eq!(results[0].2, "PM Prompt");
        assert_eq!(results[1].0, "/agents/swe/prompt.md");
        assert_eq!(results[1].1, id_a);
        assert_eq!(results[1].2, "SWE Prompt");
    }

    #[tokio::test]
    async fn get_documents_by_paths_excludes_deleted() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_document(
                sample_document(Some("docs/transient.md")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store.delete_document(&id, &ActorRef::test()).await.unwrap();

        let results = store
            .get_documents_by_paths(&["/docs/transient.md".to_string()])
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn get_documents_by_paths_empty_input_returns_empty() {
        let store = create_test_store().await;
        let results = store.get_documents_by_paths(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    async fn insert_dummy_latest_sessions(store: &SqliteStore, start: usize, count: usize) {
        // Insert minimal session rows with is_latest = 1 to inflate the count
        // cheaply without exercising the full add_session pipeline. The numeric
        // suffix keeps each id globally unique across calls. Bumps the in-memory
        // row-count cache to match, since these raw inserts bypass add_session.
        for i in start..(start + count) {
            let id = format!("s-dummyaa{i:08}");
            sqlx::query(&format!(
                "INSERT INTO {TABLE_TASKS_V2} (id, version_number, env_vars, status, deleted, creator, mount_spec, agent_config, mode, is_latest)
                 VALUES (?1, 1, '{{}}', 'complete', 0, '', '{{\"working_dir\":\"repo\",\"mounts\":[]}}', '{{}}', '{{\"type\":\"headless\",\"prompt\":\"\"}}', 1)"
            ))
            .bind(&id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        store.bump_row_count_for_test(TABLE_TASKS_V2, count as i64);
    }

    async fn insert_dummy_latest_patches(store: &SqliteStore, start: usize, count: usize) {
        // Inflate the patches_v2 latest count cheaply without exercising the
        // full add_patch pipeline. See `insert_dummy_latest_sessions` for the
        // pattern; this is the parallel for patches and is used by
        // `add_patch_grows_id_suffix_with_table_size`.
        for i in start..(start + count) {
            let id = format!("p-dumyaa{i:08}");
            sqlx::query(&format!(
                "INSERT INTO {TABLE_PATCHES_V2} (id, version_number, title, description, diff, status, is_automatic_backup, creator, service_repo_name, deleted, is_latest)
                 VALUES (?1, 1, '', '', '', 'Open', 0, '', 'dourolabs/sample', 0, 1)"
            ))
            .bind(&id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        store.bump_row_count_for_test(TABLE_PATCHES_V2, count as i64);
    }

    async fn insert_dummy_undeleted_labels(store: &SqliteStore, count: usize) -> Vec<LabelId> {
        // Insert minimal label rows with deleted = 0 to inflate the count
        // cheaply without exercising the full add_label pipeline. Generates
        // wide random suffixes so collisions across this many rows are
        // vanishingly unlikely. Bumps the in-memory row-count cache to match,
        // since these raw inserts bypass add_label.
        let mut ids = Vec::with_capacity(count);
        let now = Utc::now().to_rfc3339();
        for i in 0..count {
            let id = LabelId::generate(10).unwrap();
            sqlx::query(&format!(
                "INSERT INTO {TABLE_LABELS} (id, name, color, deleted, recurse, hidden, created_at, updated_at)
                 VALUES (?1, ?2, '#000000', 0, 0, 0, ?3, ?3)"
            ))
            .bind(id.as_ref())
            .bind(format!("dummy-{i}"))
            .bind(&now)
            .execute(&store.pool)
            .await
            .unwrap();
            ids.push(id);
        }
        store.bump_row_count_for_test(TABLE_LABELS, count as i64);
        ids
    }

    #[tokio::test]
    async fn delete_label_decrements_next_label_id_count() {
        let store = create_test_store().await;

        // Seed 677 live labels so the cache crosses the 6 → 7-char threshold.
        let dummies = insert_dummy_undeleted_labels(&store, 677).await;
        let pre = store
            .add_label(sample_label("live-pre", "#ffffff"))
            .await
            .unwrap();
        assert_eq!(
            pre.as_ref().len() - LabelId::prefix().len(),
            7,
            "677 live labels should bump suffix length to 7"
        );

        // Soft-delete every label; each delete_label call must decrement the
        // cache so subsequent next_label_id sees a live count of zero.
        for id in &dummies {
            store.delete_label(id).await.unwrap();
        }
        store.delete_label(&pre).await.unwrap();

        let post = store
            .add_label(sample_label("live-post", "#ffffff"))
            .await
            .unwrap();
        assert_eq!(
            post.as_ref().len() - LabelId::prefix().len(),
            6,
            "soft-deleted labels must not inflate the suffix length"
        );
    }

    #[tokio::test]
    async fn add_patch_grows_id_suffix_with_table_size() {
        // Mirrors `add_session_grows_id_suffix_with_table_size` for the
        // patches_v2 table. The dynamic-length HydraId rollout means
        // `next_patch_id` widens the random suffix once the live row count
        // crosses each `random_len_for_count` threshold; if it ever
        // regressed and panicked at the `.expect("length within bounds")`
        // inside `add_patch`, the only visible signal would be a hyper
        // RST-stream on the CLI side. This pins the boundary down.
        let store = create_test_store().await;

        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - PatchId::prefix().len(),
            6,
            "fresh table should use default suffix length"
        );

        // 27 patches → ceil(log_26) = 2 → still 6.
        insert_dummy_latest_patches(&store, 0, 26).await;
        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - PatchId::prefix().len(),
            6,
            "27 rows should still use default 6-char suffix"
        );

        // Inflate to 677 total → bumps to 7.
        insert_dummy_latest_patches(&store, 26, 649).await;
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

    #[tokio::test]
    async fn add_session_grows_id_suffix_with_table_size() {
        let store = create_test_store().await;

        // Empty table — next ID should use the default 6-char suffix.
        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "fresh table should use default suffix length"
        );

        // 27 sessions still fit within ceil(log_26) = 2 → suffix stays at 6.
        insert_dummy_latest_sessions(&store, 0, 26).await; // 26 dummies + 1 real = 27
        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "27 rows should still use default 6-char suffix"
        );

        // Inflate to 677 rows: 2 real sessions + dummies. We already have 28
        // rows; add 649 more to reach 677 total before the next call.
        insert_dummy_latest_sessions(&store, 26, 649).await;
        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            7,
            "677 rows should bump suffix length to 7"
        );
    }

    // ---- Session event log tests ----

    fn interactive_session(conversation_id: Option<ConversationId>) -> Session {
        let mut session = spawn_task();
        match conversation_id {
            Some(conv_id) => {
                session.mode = SessionMode::Interactive {
                    conversation_id: conv_id,
                    idle_timeout_secs: None,
                    greet_user: false,
                };
            }
            None => {
                // Tests previously passed `None` to mean "interactive but no
                // conversation". The new shape requires a conversation_id, so
                // collapse this case to Headless — same semantic effect (no
                // conversation linkage).
                session.mode = SessionMode::Headless;
            }
        }
        session
    }

    #[tokio::test]
    async fn append_and_get_session_events_returns_in_insertion_order() {
        let store = create_test_store().await;
        let (sid, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let events = store.get_session_events(&sid).await.unwrap();
        assert!(events.is_empty());

        let v1 = store
            .append_session_event(
                &sid,
                SessionEvent::UserMessage {
                    content: "hi".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(v1, 1);

        let v2 = store
            .append_session_event(
                &sid,
                SessionEvent::AssistantMessage {
                    content: "hello".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let events = store.get_session_events(&sid).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].version, 1);
        assert_eq!(events[1].version, 2);
        assert!(matches!(events[0].item, SessionEvent::UserMessage { .. }));
        assert!(matches!(
            events[1].item,
            SessionEvent::AssistantMessage { .. }
        ));
    }

    #[tokio::test]
    async fn append_session_event_fails_for_missing_session() {
        let store = create_test_store().await;
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

        let err = store.get_session_events(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn session_event_rowid_seq_is_monotonic_across_sessions() {
        let store = create_test_store().await;
        let (s1, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (s2, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Interleaved appends: s1, s2, s1.
        store
            .append_session_event(
                &s1,
                SessionEvent::UserMessage {
                    content: "a".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s2,
                SessionEvent::UserMessage {
                    content: "b".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s1,
                SessionEvent::AssistantMessage {
                    content: "c".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // rowid_seq is strictly monotonic across all sessions in insertion order.
        let rows = sqlx::query_as::<_, (i64, String)>(
            "SELECT rowid_seq, session_id FROM session_events ORDER BY rowid_seq ASC",
        )
        .fetch_all(&store.pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].1, s1.as_ref());
        assert_eq!(rows[1].1, s2.as_ref());
        assert_eq!(rows[2].1, s1.as_ref());
        assert!(rows[0].0 < rows[1].0);
        assert!(rows[1].0 < rows[2].0);
    }

    #[tokio::test]
    async fn session_state_round_trip_and_upsert() {
        let store = create_test_store().await;
        let (sid, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let state = store.get_session_state(&sid).await.unwrap();
        assert!(state.is_none());

        let data = vec![1u8, 2, 3, 4, 5];
        store
            .store_session_state(&sid, data.clone(), &ActorRef::test())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data));

        // Second write must overwrite the first.
        let data2 = vec![9u8, 8, 7];
        store
            .store_session_state(&sid, data2.clone(), &ActorRef::test())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data2));
    }

    #[tokio::test]
    async fn session_state_fails_for_missing_session() {
        let store = create_test_store().await;
        let missing = SessionId::generate(6).unwrap();

        let err = store.get_session_state(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));

        let err = store
            .store_session_state(&missing, vec![1, 2, 3], &ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn get_session_event_summaries_returns_counts_and_previews() {
        let store = create_test_store().await;
        let (s1, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (s2, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let (s3, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        store
            .append_session_event(
                &s1,
                SessionEvent::UserMessage {
                    content: "first".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s1,
                SessionEvent::AssistantMessage {
                    content: "second".to_string(),
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s2,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_session_event_summaries(&[s1.clone(), s2.clone(), s3.clone()])
            .await
            .unwrap();

        let s1_summary = summaries.get(&s1).expect("s1 summary");
        assert_eq!(s1_summary.event_count, 2);
        assert_eq!(
            s1_summary.last_event_preview.as_deref(),
            Some("Assistant: second")
        );

        let s2_summary = summaries.get(&s2).expect("s2 summary");
        assert_eq!(s2_summary.event_count, 1);
        assert_eq!(s2_summary.last_event_preview.as_deref(), Some("Closed"));

        // s3 has no events and is omitted from the result.
        assert!(!summaries.contains_key(&s3));

        // Empty input → empty output.
        let summaries = store.get_session_event_summaries(&[]).await.unwrap();
        assert!(summaries.is_empty());
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_sources_preview_from_chat_text() {
        let store = create_test_store().await;
        let (conv_user_only, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();
        let (conv_user_then_assistant, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();
        let (conv_cross_session, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();
        let (conv_empty, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        // Single UserMessage from a single session.
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
        // Chat-text in older session; tool-use + lifecycle in newer session.
        // The newer session's events are skipped — older session wins because
        // it has the only chat-text candidate.
        let t_old = Utc::now() - Duration::seconds(60);
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
        let t_new = Utc::now();
        let (sid_new, _) = store
            .add_session(
                interactive_session(Some(conv_cross_session.clone())),
                t_new,
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

        // No conversation events and no chat-text — omitted entirely.
        assert!(!summaries.contains_key(&conv_empty));

        // Empty input → empty output.
        let empty = store.get_conversation_event_summaries(&[]).await.unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_latest_session_wins_over_older() {
        let store = create_test_store().await;
        let (conv, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();
        let t_old = Utc::now() - Duration::seconds(60);
        let (sid_old, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                t_old,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Older session's event has a wall-clock timestamp *after* the newer
        // session's event — but session-creation order trumps per-event time.
        store
            .append_session_event(
                &sid_old,
                SessionEvent::UserMessage {
                    content: "from older session, written later".to_string(),
                    timestamp: Utc::now() + Duration::seconds(60),
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

    #[tokio::test]
    async fn get_conversation_event_summaries_sums_chat_text_across_sessions() {
        // Regression test for the chat-list "Messages" column: when a
        // conversation has multiple sessions (close → resume), the count
        // must sum chat-text events across every session, not just the
        // latest one. ToolUse / lifecycle events are excluded.
        let store = create_test_store().await;
        let (conv, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        let t_old = Utc::now() - Duration::seconds(60);
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
        // ToolUse and Closed are excluded.
        assert_eq!(s.event_count, 4);
        assert_eq!(s.last_event_preview.as_deref(), Some("Assistant: four"));
    }

    #[tokio::test]
    async fn list_session_ids_by_conversation_id_returns_linked_in_creation_order() {
        let store = create_test_store().await;
        let (conv_id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();
        let (other_conv_id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
            .await
            .unwrap();

        // Session A: linked to conv_id, earliest.
        let t1 = Utc::now();
        let (sid_a, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                t1,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Session B: linked to other conversation.
        let (_sid_b, _) = store
            .add_session(
                interactive_session(Some(other_conv_id.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Session C: linked to conv_id, later than A.
        let t3 = t1 + Duration::seconds(5);
        let (sid_c, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                t3,
                &ActorRef::test(),
            )
            .await
            .unwrap();
        // Session D: non-interactive.
        let (_sid_d, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let ids = store
            .list_session_ids_by_conversation_id(&conv_id)
            .await
            .unwrap();
        assert_eq!(ids, vec![sid_a.clone(), sid_c.clone()]);

        // Unrelated conversation returns no sessions.
        let unrelated = ConversationId::new();
        let ids = store
            .list_session_ids_by_conversation_id(&unrelated)
            .await
            .unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn list_session_ids_by_conversation_id_excludes_deleted_sessions() {
        let store = create_test_store().await;
        let (conv_id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
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
        store.delete_session(&sid, &ActorRef::test()).await.unwrap();

        let ids = store
            .list_session_ids_by_conversation_id(&conv_id)
            .await
            .unwrap();
        assert!(ids.is_empty());
    }

    // ---- Session-shape column dual-write + backfill tests ----
    //
    // These tests cover Phase D step 12: the `mount_spec`, `agent_config`,
    // `mode`, and `resumed_from` columns added in
    // `20260523020000_add_session_shape_columns.sql`. They assert both the
    // runtime dual-write path (via `add_session` / our updated INSERT) and
    // the migration backfill SQL (replayed against raw inserts that bypass
    // the application path and leave the new columns NULL).

    #[derive(sqlx::FromRow)]
    struct SessionShapeRow {
        mount_spec: Option<String>,
        agent_config: Option<String>,
        mode: Option<String>,
        resumed_from: Option<String>,
    }

    async fn fetch_session_shape(store: &SqliteStore, id: &SessionId) -> SessionShapeRow {
        sqlx::query_as::<_, SessionShapeRow>(
            "SELECT mount_spec, agent_config, mode, resumed_from \
             FROM tasks_v2 WHERE id = ?1 AND is_latest = 1",
        )
        .bind(id.as_ref())
        .fetch_one(&store.pool)
        .await
        .unwrap()
    }

    fn parse_json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("column should hold valid JSON")
    }

    #[tokio::test]
    async fn dual_write_headless_session_populates_mode_and_mount_spec() {
        let store = create_test_store().await;
        let (sid, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let row = fetch_session_shape(&store, &sid).await;

        let mode = parse_json(row.mode.as_deref().expect("mode is non-null"));
        assert_eq!(mode["type"], "headless");
        // Headless is unit-like — the prompt lives on agent_config.system_prompt.
        assert!(mode.get("prompt").is_none_or(|v| v.is_null()));

        let mount_spec = parse_json(row.mount_spec.as_deref().expect("mount_spec is non-null"));
        assert_eq!(mount_spec["working_dir"], "repo");
        let mounts = mount_spec["mounts"].as_array().expect("mounts is an array");
        assert_eq!(
            mounts.len(),
            2,
            "headless backfill emits Bundle + Documents"
        );
        assert_eq!(mounts[0]["type"], "bundle");
        assert_eq!(mounts[0]["target"], "repo");
        // PR-D: `session_id` no longer rides on `MountItem::Bundle`.
        assert!(mounts[0].get("session_id").is_none_or(|v| v.is_null()));
        assert_eq!(mounts[0]["bundle"]["type"], "none");
        assert_eq!(mounts[1]["type"], "documents");
        assert_eq!(mounts[1]["target"], "documents");

        let agent_config = parse_json(
            row.agent_config
                .as_deref()
                .expect("agent_config is non-null"),
        );
        assert!(agent_config["agent_name"].is_null());
        // PR-1: `spawn_task()` puts the prompt on `agent_config.system_prompt`.
        assert_eq!(agent_config["system_prompt"], "test prompt");
        // spawn_task() sets model: None, mcp_config: None
        assert!(agent_config["model"].is_null());
        assert!(agent_config["mcp_config"].is_null());

        assert!(
            row.resumed_from.is_none(),
            "fresh sessions have no predecessor"
        );
    }

    #[tokio::test]
    async fn dual_write_interactive_session_populates_mode_with_conversation_id() {
        let store = create_test_store().await;
        let (conv_id, _) = store
            .add_conversation(sample_conversation(), &ActorRef::test())
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

        let row = fetch_session_shape(&store, &sid).await;

        let mode = parse_json(row.mode.as_deref().expect("mode is non-null"));
        assert_eq!(mode["type"], "interactive");
        assert_eq!(mode["conversation_id"], conv_id.as_ref());
        // `idle_timeout_secs` is omitted when None (server applies default).
        assert!(mode.get("idle_timeout_secs").is_none_or(|v| v.is_null()));
    }

    #[tokio::test]
    async fn dual_write_session_with_git_bundle_carries_url_into_mount_spec() {
        use hydra_common::api::v1::sessions::{
            Bundle, MountItem, MountSpec as ApiMountSpec, RelativePath,
        };
        let store = create_test_store().await;
        let mut session = spawn_task();
        session.mount_spec = ApiMountSpec::new(
            RelativePath::new("repo").unwrap(),
            vec![
                MountItem::Bundle {
                    target: RelativePath::new("repo").unwrap(),
                    bundle: Bundle::GitRepository {
                        url: "https://github.com/example/repo".to_string(),
                        rev: "main".to_string(),
                    },
                },
                MountItem::Documents {
                    target: RelativePath::new("documents").unwrap(),
                },
            ],
        );
        session.agent_config.model = Some("gpt-4o".to_string());

        let (sid, _) = store
            .add_session(session, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let row = fetch_session_shape(&store, &sid).await;
        let mount_spec = parse_json(row.mount_spec.as_deref().expect("mount_spec is non-null"));
        let bundle = &mount_spec["mounts"][0]["bundle"];
        assert_eq!(bundle["type"], "git_repository");
        assert_eq!(bundle["url"], "https://github.com/example/repo");
        assert_eq!(bundle["rev"], "main");

        let agent_config = parse_json(
            row.agent_config
                .as_deref()
                .expect("agent_config is non-null"),
        );
        assert_eq!(agent_config["model"], "gpt-4o");
    }

    // ---- assignee_principal backfill ----

    /// Apply the `assignee_principal` backfill migration to a
    /// pre-migration schema. The test fixture creates `agents` and
    /// `issues_v2` (without `assignee_principal`) so the migration's
    /// `ALTER TABLE` runs against a realistic starting state.
    async fn apply_assignee_principal_backfill_migration(pool: &SqlitePool) {
        let sql = include_str!(
            "../../sqlite-migrations/20260530000000_add_assignee_principal_to_issues.sql"
        );
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }

    /// Create a minimal pre-migration schema: `agents` + `issues_v2`
    /// without the `assignee_principal` column. Both tables match the
    /// shape at the moment immediately before
    /// `20260530000000_add_assignee_principal_to_issues.sql` runs.
    async fn setup_pre_assignee_principal_schema(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE agents ( \
                name TEXT PRIMARY KEY, \
                prompt_path TEXT NOT NULL, \
                max_tries INTEGER NOT NULL DEFAULT 3, \
                max_simultaneous INTEGER NOT NULL DEFAULT 2147483647, \
                is_assignment_agent INTEGER NOT NULL DEFAULT 0, \
                deleted INTEGER NOT NULL DEFAULT 0, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')))",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE issues_v2 ( \
                id TEXT NOT NULL, \
                version_number INTEGER NOT NULL, \
                title TEXT NOT NULL DEFAULT '', \
                issue_type TEXT NOT NULL, \
                description TEXT NOT NULL, \
                creator TEXT NOT NULL, \
                progress TEXT NOT NULL DEFAULT '', \
                status TEXT NOT NULL DEFAULT 'open', \
                assignee TEXT, \
                job_settings TEXT NOT NULL DEFAULT '{}', \
                deleted INTEGER NOT NULL DEFAULT 0, \
                actor TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                PRIMARY KEY (id, version_number))",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn insert_agent(pool: &SqlitePool, name: &str) {
        sqlx::query("INSERT INTO agents (name, prompt_path) VALUES (?1, ?2)")
            .bind(name)
            .bind("/dev/null")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn insert_legacy_issue(pool: &SqlitePool, id: &str, assignee: Option<&str>) {
        sqlx::query(
            "INSERT INTO issues_v2 \
             (id, version_number, issue_type, description, creator, assignee) \
             VALUES (?1, 1, 'task', '', 'creator', ?2)",
        )
        .bind(id)
        .bind(assignee)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn fetch_assignee_principal(pool: &SqlitePool, id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT assignee_principal FROM issues_v2 WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn assignee_principal_backfill_classifies_bare_agent_names_as_agent() {
        // Bare-name agents: with the `agents` table populated, the legacy
        // `assignee = "swe"` row should backfill to
        // `Principal::Agent { name: "swe" }`, not `Principal::User`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        insert_agent(&pool, "swe").await;
        insert_agent(&pool, "reviewer").await;
        insert_legacy_issue(&pool, "issue-swe", Some("swe")).await;
        insert_legacy_issue(&pool, "issue-reviewer", Some("reviewer")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-swe").await,
            Some(r#"{"Agent":{"name":"swe"}}"#.to_string())
        );
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-reviewer").await,
            Some(r#"{"Agent":{"name":"reviewer"}}"#.to_string())
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_classifies_unknown_bare_names_as_user() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        insert_agent(&pool, "swe").await;
        insert_legacy_issue(&pool, "issue-alice", Some("alice")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-alice").await,
            Some(r#"{"User":{"name":"alice"}}"#.to_string())
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_preserves_canonical_path_forms() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        // Even with "swe" in the agents table, `users/swe` should be
        // honoured as a user (the canonical form wins over the
        // bare-name match).
        insert_agent(&pool, "swe").await;
        insert_legacy_issue(&pool, "issue-user-path", Some("users/alice")).await;
        insert_legacy_issue(&pool, "issue-agent-path", Some("agents/swe")).await;
        insert_legacy_issue(&pool, "issue-user-swe-path", Some("users/swe")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-user-path").await,
            Some(r#"{"User":{"name":"alice"}}"#.to_string())
        );
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-agent-path").await,
            Some(r#"{"Agent":{"name":"swe"}}"#.to_string())
        );
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-user-swe-path").await,
            Some(r#"{"User":{"name":"swe"}}"#.to_string())
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_empty_agents_table_lifts_all_bare_as_user() {
        // No agents registered → behaves like the pre-fix migration:
        // every well-formed bare name is a user.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        insert_legacy_issue(&pool, "issue-swe", Some("swe")).await;
        insert_legacy_issue(&pool, "issue-alice", Some("alice")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-swe").await,
            Some(r#"{"User":{"name":"swe"}}"#.to_string())
        );
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-alice").await,
            Some(r#"{"User":{"name":"alice"}}"#.to_string())
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_leaves_invalid_input_null() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        insert_agent(&pool, "swe").await;
        insert_legacy_issue(&pool, "issue-empty", Some("")).await;
        insert_legacy_issue(&pool, "issue-whitespace", Some("alice bob")).await;
        insert_legacy_issue(&pool, "issue-null", None).await;
        // External path is intentionally NOT backfilled by this
        // migration -- the dual-write path picks it up on next write.
        insert_legacy_issue(&pool, "issue-external", Some("external/github/jayantk")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(fetch_assignee_principal(&pool, "issue-empty").await, None);
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-whitespace").await,
            None
        );
        assert_eq!(fetch_assignee_principal(&pool, "issue-null").await, None);
        assert_eq!(
            fetch_assignee_principal(&pool, "issue-external").await,
            None
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_case_sensitive_against_agent_names() {
        // "SWE" (uppercase) doesn't match the lowercase "swe" agent row,
        // so it falls back to Principal::User -- mirroring
        // `Principal::parse_legacy_assignee_with_agents`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        insert_agent(&pool, "swe").await;
        insert_legacy_issue(&pool, "issue-upper", Some("SWE")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-upper").await,
            Some(r#"{"User":{"name":"SWE"}}"#.to_string())
        );
    }

    #[tokio::test]
    async fn assignee_principal_backfill_classifies_deleted_agent_as_agent() {
        // Once a name has been registered as an agent (even if later
        // soft-deleted), legacy strings for that name still refer to
        // the agent -- the agent row remains in the table with
        // `deleted = 1`, and the migration's predicate doesn't filter
        // on `deleted`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_assignee_principal_schema(&pool).await;

        sqlx::query("INSERT INTO agents (name, prompt_path, deleted) VALUES (?1, ?2, 1)")
            .bind("retired")
            .bind("/dev/null")
            .execute(&pool)
            .await
            .unwrap();
        insert_legacy_issue(&pool, "issue-retired", Some("retired")).await;

        apply_assignee_principal_backfill_migration(&pool).await;

        assert_eq!(
            fetch_assignee_principal(&pool, "issue-retired").await,
            Some(r#"{"Agent":{"name":"retired"}}"#.to_string())
        );
    }

    // ---- review_author_principal backfill ----

    /// Apply the `review_author_principal` backfill migration to a
    /// pre-migration schema. The test fixture creates `agents` and
    /// `patches_v2` so the migration's `UPDATE` over each row's
    /// `reviews` JSON array runs against a realistic starting state.
    async fn apply_review_author_principal_backfill_migration(pool: &SqlitePool) {
        let sql =
            include_str!("../../sqlite-migrations/20260601000000_review_author_principal.sql");
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }

    /// Create a minimal pre-migration schema: `agents` + `patches_v2`.
    /// `patches_v2` mirrors the shape at the moment immediately before
    /// `20260601000000_review_author_principal.sql` runs (i.e. after
    /// `20260527000000_drop_patches_created_by.sql` has removed the
    /// `created_by` column).
    async fn setup_pre_review_author_principal_schema(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE agents ( \
                name TEXT PRIMARY KEY, \
                prompt_path TEXT NOT NULL, \
                max_tries INTEGER NOT NULL DEFAULT 3, \
                max_simultaneous INTEGER NOT NULL DEFAULT 2147483647, \
                is_assignment_agent INTEGER NOT NULL DEFAULT 0, \
                deleted INTEGER NOT NULL DEFAULT 0, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')))",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE patches_v2 ( \
                id TEXT NOT NULL, \
                version_number INTEGER NOT NULL, \
                title TEXT NOT NULL DEFAULT '', \
                description TEXT NOT NULL, \
                diff TEXT NOT NULL, \
                status TEXT NOT NULL DEFAULT 'open', \
                is_automatic_backup INTEGER NOT NULL DEFAULT 0, \
                creator TEXT, \
                base_branch TEXT, \
                branch_name TEXT, \
                commit_range TEXT, \
                reviews TEXT NOT NULL DEFAULT '[]', \
                service_repo_name TEXT NOT NULL, \
                github TEXT, \
                deleted INTEGER NOT NULL DEFAULT 0, \
                actor TEXT, \
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')), \
                PRIMARY KEY (id, version_number))",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// Insert a patch row with the given `reviews` JSON. Callers shape
    /// the reviews array per-test.
    async fn insert_patch_with_reviews_json(pool: &SqlitePool, id: &str, reviews_json: &str) {
        sqlx::query(
            "INSERT INTO patches_v2 \
             (id, version_number, description, diff, reviews, service_repo_name) \
             VALUES (?1, 1, '', '', ?2, 'owner/repo')",
        )
        .bind(id)
        .bind(reviews_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn fetch_reviews_json(pool: &SqlitePool, id: &str) -> String {
        sqlx::query_scalar::<_, String>("SELECT reviews FROM patches_v2 WHERE id = ?1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Build a single-element `reviews` array with the given author
    /// value (string or object). `contents`, `is_approved`, and
    /// `submitted_at` are filled with placeholders; the migration
    /// passes those through, but the tests focus on the `author`
    /// rewrite.
    fn single_review(author: serde_json::Value) -> String {
        serde_json::json!([{
            "contents": "lgtm",
            "is_approved": true,
            "author": author,
            "submitted_at": "2026-05-01T00:00:00+00:00"
        }])
        .to_string()
    }

    /// Extract the `author` field of each review element from the
    /// stored JSON. Keeps assertions focused on classification rather
    /// than on the migration's pass-through reshaping of the other
    /// review fields.
    fn extract_authors(reviews_json: &str) -> Vec<serde_json::Value> {
        let arr: Vec<serde_json::Value> = serde_json::from_str(reviews_json).unwrap();
        arr.into_iter()
            .map(|r| r.get("author").cloned().unwrap_or(serde_json::Value::Null))
            .collect()
    }

    #[tokio::test]
    async fn review_author_backfill_classifies_bare_agent_names_as_agent() {
        // Bare-name agents: with the `agents` table populated, a legacy
        // review `author = "reviewer"` should backfill to
        // `Principal::Agent { name: "reviewer" }`, not `Principal::User`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "swe").await;
        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-reviewer",
            &single_review(serde_json::json!("reviewer")),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-swe",
            &single_review(serde_json::json!("swe")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-reviewer").await),
            vec![serde_json::json!({"Agent": {"name": "reviewer"}})],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-swe").await),
            vec![serde_json::json!({"Agent": {"name": "swe"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_classifies_unknown_bare_names_as_user() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-alice",
            &single_review(serde_json::json!("alice")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-alice").await),
            vec![serde_json::json!({"User": {"name": "alice"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_preserves_canonical_path_forms() {
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        // Even with "reviewer" in the agents table, `users/reviewer`
        // should be honoured as a user (the canonical form wins over
        // the bare-name match).
        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-user-path",
            &single_review(serde_json::json!("users/alice")),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-agent-path",
            &single_review(serde_json::json!("agents/reviewer")),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-user-reviewer-path",
            &single_review(serde_json::json!("users/reviewer")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-user-path").await),
            vec![serde_json::json!({"User": {"name": "alice"}})],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-agent-path").await),
            vec![serde_json::json!({"Agent": {"name": "reviewer"}})],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-user-reviewer-path").await),
            vec![serde_json::json!({"User": {"name": "reviewer"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_empty_agents_table_lifts_all_bare_as_user() {
        // No agents registered → behaves like the pre-fix migration:
        // every well-formed bare name is a user.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_patch_with_reviews_json(
            &pool,
            "patch-reviewer",
            &single_review(serde_json::json!("reviewer")),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-alice",
            &single_review(serde_json::json!("alice")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-reviewer").await),
            vec![serde_json::json!({"User": {"name": "reviewer"}})],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-alice").await),
            vec![serde_json::json!({"User": {"name": "alice"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_preserves_invalid_or_external_authors() {
        // Empty string, embedded whitespace, and `external/<sys>/<x>`
        // all fall to the `ELSE` branch of the migration, which keeps
        // the raw string value untouched. The runtime deserializer
        // logs a warning and falls through `parse_legacy_assignee`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(&pool, "patch-empty", &single_review(serde_json::json!("")))
            .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-whitespace",
            &single_review(serde_json::json!("alice bob")),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-external",
            &single_review(serde_json::json!("external/github/jayantk")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-empty").await),
            vec![serde_json::json!("")],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-whitespace").await),
            vec![serde_json::json!("alice bob")],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-external").await),
            vec![serde_json::json!("external/github/jayantk")],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_case_sensitive_against_agent_names() {
        // "Reviewer" (mixed case) doesn't match the lowercase
        // "reviewer" agent row, so it falls back to Principal::User --
        // mirroring `Principal::parse_legacy_assignee_with_agents`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-mixed",
            &single_review(serde_json::json!("Reviewer")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-mixed").await),
            vec![serde_json::json!({"User": {"name": "Reviewer"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_classifies_deleted_agent_as_agent() {
        // Once a name has been registered as an agent (even if later
        // soft-deleted), legacy review-author strings for that name
        // still refer to the agent -- the agent row remains in the
        // table with `deleted = 1`, and the migration's predicate
        // doesn't filter on `deleted`.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        sqlx::query("INSERT INTO agents (name, prompt_path, deleted) VALUES (?1, ?2, 1)")
            .bind("retired")
            .bind("/dev/null")
            .execute(&pool)
            .await
            .unwrap();
        insert_patch_with_reviews_json(
            &pool,
            "patch-retired",
            &single_review(serde_json::json!("retired")),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-retired").await),
            vec![serde_json::json!({"Agent": {"name": "retired"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_preserves_already_typed_author() {
        // If `author` is already an object (the Phase-5b shape), the
        // migration's first CASE arm matches and leaves it untouched.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-typed-agent",
            &single_review(serde_json::json!({"Agent": {"name": "reviewer"}})),
        )
        .await;
        insert_patch_with_reviews_json(
            &pool,
            "patch-typed-user",
            &single_review(serde_json::json!({"User": {"name": "alice"}})),
        )
        .await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-typed-agent").await),
            vec![serde_json::json!({"Agent": {"name": "reviewer"}})],
        );
        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-typed-user").await),
            vec![serde_json::json!({"User": {"name": "alice"}})],
        );
    }

    #[tokio::test]
    async fn review_author_backfill_skips_empty_reviews_array() {
        // The migration's WHERE clause excludes rows whose `reviews`
        // is NULL, `'[]'`, or otherwise has zero elements. Such rows
        // should be left exactly as-is.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;
        insert_patch_with_reviews_json(&pool, "patch-empty-array", "[]").await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(fetch_reviews_json(&pool, "patch-empty-array").await, "[]");
    }

    #[tokio::test]
    async fn review_author_backfill_rewrites_all_reviews_in_array() {
        // A single patch may have multiple reviews. The migration
        // walks the array element-by-element and rewrites every
        // `author`. Verify ordering is preserved and each element's
        // classification is independent.
        let pool = SqliteStore::init_pool("sqlite::memory:").await.unwrap();
        setup_pre_review_author_principal_schema(&pool).await;

        insert_agent(&pool, "reviewer").await;

        let reviews = serde_json::json!([
            {
                "contents": "first",
                "is_approved": false,
                "author": "alice",
                "submitted_at": "2026-05-01T00:00:00+00:00"
            },
            {
                "contents": "second",
                "is_approved": true,
                "author": "reviewer",
                "submitted_at": "2026-05-02T00:00:00+00:00"
            },
            {
                "contents": "third",
                "is_approved": true,
                "author": "agents/reviewer",
                "submitted_at": "2026-05-03T00:00:00+00:00"
            },
            {
                "contents": "fourth",
                "is_approved": false,
                "author": "external/github/jayantk",
                "submitted_at": "2026-05-04T00:00:00+00:00"
            }
        ])
        .to_string();
        insert_patch_with_reviews_json(&pool, "patch-multi", &reviews).await;

        apply_review_author_principal_backfill_migration(&pool).await;

        assert_eq!(
            extract_authors(&fetch_reviews_json(&pool, "patch-multi").await),
            vec![
                serde_json::json!({"User": {"name": "alice"}}),
                serde_json::json!({"Agent": {"name": "reviewer"}}),
                serde_json::json!({"Agent": {"name": "reviewer"}}),
                serde_json::json!("external/github/jayantk"),
            ],
        );
    }

    // ---- Trigger tests --------------------------------------------------

    fn sample_trigger() -> Trigger {
        use hydra_common::api::v1::issues::{IssueStatus, IssueType, SessionSettings};
        use hydra_common::api::v1::users::Username as ApiUsername;
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
                Some(IssueStatus::Open),
                SessionSettings::default(),
            ))],
            ApiUsername::from("alice"),
            None,
            false,
        )
    }

    #[tokio::test]
    async fn trigger_round_trip_create_get_list_update_delete_sqlite() {
        let store = create_test_store().await;
        let (id, version) = store
            .add_trigger(sample_trigger(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert!(fetched.item.enabled);
        assert_eq!(fetched.item.actions, sample_trigger().actions);

        let listed = store.list_triggers(false).await.unwrap();
        assert_eq!(listed.len(), 1);

        let mut updated = sample_trigger();
        updated.enabled = false;
        let v2 = store
            .update_trigger(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);
        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 2);
        assert!(!fetched.item.enabled);

        let v3 = store.delete_trigger(&id, &ActorRef::test()).await.unwrap();
        assert_eq!(v3, 3);
        assert!(store.list_triggers(false).await.unwrap().is_empty());
        assert_eq!(store.list_triggers(true).await.unwrap().len(), 1);
        assert!(matches!(
            store.get_trigger(&id, false).await,
            Err(StoreError::TriggerNotFound(_))
        ));
        let tombstoned = store.get_trigger(&id, true).await.unwrap();
        assert!(tombstoned.item.deleted);
    }

    #[tokio::test]
    async fn record_trigger_fire_does_not_bump_version_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_trigger(sample_trigger(), &ActorRef::test())
            .await
            .unwrap();

        let fired_at: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, fired_at).await.unwrap();

        let fetched = store.get_trigger(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.last_fired_at, Some(fired_at));
    }

    #[tokio::test]
    async fn update_after_record_trigger_fire_carries_forward_last_fired_at_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_trigger(sample_trigger(), &ActorRef::test())
            .await
            .unwrap();

        let fired_at: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, fired_at).await.unwrap();

        // Caller has a stale copy where last_fired_at = None. The store
        // must carry the most recent last_fired_at forward into the new
        // version row instead of clobbering it.
        let mut next = sample_trigger();
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

    #[tokio::test]
    async fn update_with_stale_last_fired_at_does_not_regress_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_trigger(sample_trigger(), &ActorRef::test())
            .await
            .unwrap();

        let t_new: DateTime<Utc> = "2026-06-03T15:00:00Z".parse().unwrap();
        store.record_trigger_fire(&id, t_new).await.unwrap();

        // Caller supplies a stale `Some(t_old)` on the update payload.
        // `update_trigger` must ignore it and overwrite with the latest
        // row's `Some(t_new)`.
        let t_old: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
        let mut next = sample_trigger();
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

    #[tokio::test]
    async fn record_trigger_fire_not_found_sqlite() {
        let store = create_test_store().await;
        let result = store
            .record_trigger_fire(&TriggerId::new(), Utc::now())
            .await;
        assert!(matches!(result, Err(StoreError::TriggerNotFound(_))));
    }

    #[tokio::test]
    async fn get_trigger_not_found_sqlite() {
        let store = create_test_store().await;
        let result = store.get_trigger(&TriggerId::new(), false).await;
        assert!(matches!(result, Err(StoreError::TriggerNotFound(_))));
    }

    // ---- Project tests --------------------------------------------------

    /// Fully-populated sample, including `on_enter` so the JSON serde
    /// path for `StatusOnEnter` is exercised end-to-end in the round-trip
    /// test.
    fn sample_project() -> Project {
        use hydra_common::api::v1::projects::{
            IconKey, ProjectKey, StatusDefinition, StatusKey, StatusOnEnter,
        };
        use hydra_common::api::v1::users::Username as ApiUsername;
        use hydra_common::principal::Principal;

        let statuses = vec![
            StatusDefinition::new(
                StatusKey::try_new("backlog").unwrap(),
                "Backlog".to_string(),
                IconKey::try_new("circle").unwrap(),
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
                IconKey::try_new("check-circle").unwrap(),
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
            StatusKey::try_new("backlog").unwrap(),
            ApiUsername::from("alice"),
            false,
        )
    }

    #[tokio::test]
    async fn project_round_trip_create_get_list_update_delete_sqlite() {
        let store = create_test_store().await;
        let (id, version) = store
            .add_project(sample_project(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_project(&id, false).await.unwrap();
        assert_eq!(fetched.version, 1);
        assert_eq!(fetched.item.name, "Engineering");
        assert_eq!(fetched.item.statuses.len(), 2);
        // `on_enter` must round-trip through the JSON column unchanged.
        assert_eq!(fetched.item.statuses, sample_project().statuses);

        let listed = store.list_projects(false).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, id);

        let mut updated = sample_project();
        updated.name = "Engineering Renamed".to_string();
        let v2 = store
            .update_project(&id, updated, &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(v2, 2);
        let fetched = store.get_project(&id, false).await.unwrap();
        assert_eq!(fetched.version, 2);
        assert_eq!(fetched.item.name, "Engineering Renamed");

        let v3 = store.delete_project(&id, &ActorRef::test()).await.unwrap();
        assert_eq!(v3, 3);
        assert!(store.list_projects(false).await.unwrap().is_empty());
        assert_eq!(store.list_projects(true).await.unwrap().len(), 1);
        assert!(matches!(
            store.get_project(&id, false).await,
            Err(StoreError::ProjectNotFound(_))
        ));
        let tombstoned = store.get_project(&id, true).await.unwrap();
        assert!(tombstoned.item.deleted);
    }

    #[tokio::test]
    async fn get_project_not_found_sqlite() {
        let store = create_test_store().await;
        let result = store.get_project(&ProjectId::new(), false).await;
        assert!(matches!(result, Err(StoreError::ProjectNotFound(_))));
    }

    #[tokio::test]
    async fn update_project_not_found_sqlite() {
        let store = create_test_store().await;
        let result = store
            .update_project(&ProjectId::new(), sample_project(), &ActorRef::test())
            .await;
        assert!(matches!(result, Err(StoreError::ProjectNotFound(_))));
    }

    /// `update_project` must flip the prior `is_latest` row to false and
    /// insert the new latest in one transaction. Verify there is exactly
    /// one `is_latest = 1` row after the second write.
    #[tokio::test]
    async fn update_project_maintains_single_is_latest_row_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_project(sample_project(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = sample_project();
        updated.name = "v2".to_string();
        store
            .update_project(&id, updated, &ActorRef::test())
            .await
            .unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE id = ?1 AND is_latest = 1")
                .bind(id.as_ref())
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "exactly one is_latest row per project id");
    }

    /// The migration must add `project_id` to existing `issues_v2` rows
    /// without losing data. Verify the new column defaults to NULL so
    /// legacy rows keep resolving through `DefaultProject` instead of a
    /// stored value.
    #[tokio::test]
    async fn issues_v2_has_nullable_project_id_column_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_issue(sample_issue(Vec::new()), &ActorRef::test())
            .await
            .unwrap();

        let project_id: Option<String> =
            sqlx::query_scalar("SELECT project_id FROM issues_v2 WHERE id = ?1 LIMIT 1")
                .bind(id.as_ref())
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert!(
            project_id.is_none(),
            "legacy rows must default project_id to NULL"
        );
    }

    #[tokio::test]
    async fn add_project_with_duplicate_key_returns_error_sqlite() {
        let store = create_test_store().await;
        store
            .add_project(sample_project(), &ActorRef::test())
            .await
            .unwrap();
        let result = store.add_project(sample_project(), &ActorRef::test()).await;
        assert!(
            matches!(result, Err(StoreError::ProjectKeyExists(ref k)) if k.as_str() == "engineering"),
            "expected ProjectKeyExists(engineering), got {result:?}"
        );
    }

    /// A soft-deleted project frees its key for re-use — the partial
    /// unique index applies only to `is_latest = 1 AND deleted = 0`.
    #[tokio::test]
    async fn add_project_after_delete_releases_key_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_project(sample_project(), &ActorRef::test())
            .await
            .unwrap();
        store.delete_project(&id, &ActorRef::test()).await.unwrap();
        let result = store.add_project(sample_project(), &ActorRef::test()).await;
        assert!(
            result.is_ok(),
            "expected re-add after delete, got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_project_to_collide_with_another_returns_error_sqlite() {
        use hydra_common::api::v1::projects::ProjectKey;
        let store = create_test_store().await;
        let mut a = sample_project();
        a.key = ProjectKey::try_new("a").unwrap();
        let mut b = sample_project();
        b.key = ProjectKey::try_new("b").unwrap();
        store.add_project(a, &ActorRef::test()).await.unwrap();
        let (b_id, _) = store.add_project(b, &ActorRef::test()).await.unwrap();
        let mut collide = sample_project();
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
    #[tokio::test]
    async fn update_project_keeping_same_key_succeeds_sqlite() {
        let store = create_test_store().await;
        let (id, _) = store
            .add_project(sample_project(), &ActorRef::test())
            .await
            .unwrap();
        let mut next = sample_project();
        next.name = "Engineering Renamed".to_string();
        let result = store.update_project(&id, next, &ActorRef::test()).await;
        assert!(
            result.is_ok(),
            "expected ok keeping same key, got {result:?}"
        );
    }
}
