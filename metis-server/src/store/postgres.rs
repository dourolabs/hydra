use crate::{
    config::DatabaseSection,
    store::{Status, Store, StoreError, Task, TaskError, TaskStatusLog},
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use metis_common::{
    IssueId, PatchId, TaskId,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueGraphFilterSide,
        IssueGraphWildcard,
    },
    patches::Patch,
    users::User,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{
    Pool, Postgres,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    str::FromStr,
    time::Duration,
};

pub type PgStorePool = Pool<Postgres>;

pub const ISSUE_SCHEMA_VERSION: i32 = 1;
pub const PATCH_SCHEMA_VERSION: i32 = 1;
pub const TASK_SCHEMA_VERSION: i32 = 1;
pub const TASK_STATUS_LOG_SCHEMA_VERSION: i32 = 1;
pub const USER_SCHEMA_VERSION: i32 = 1;

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

const TABLE_ISSUES: &str = "metis.issues";
const TABLE_PATCHES: &str = "metis.patches";
const TABLE_TASKS: &str = "metis.tasks";
const TABLE_TASK_STATUS_LOGS: &str = "metis.task_status_logs";
const TABLE_USERS: &str = "metis.users";

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

    async fn migrate_payload(
        &self,
        object_type: &str,
        from_version: i32,
        to_version: i32,
        payload: Value,
    ) -> Result<Value, StoreError> {
        sqlx::query_scalar::<_, Value>("SELECT metis.migrate_payload($1, $2, $3, $4) AS payload")
            .bind(object_type)
            .bind(from_version)
            .bind(to_version)
            .bind(payload)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_error)
    }

    async fn fetch_payload<T: DeserializeOwned>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        target_version: i32,
    ) -> Result<Option<T>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct PayloadRow {
            schema_version: i32,
            payload: Value,
        }

        let query = format!("SELECT schema_version, payload FROM {table} WHERE id = $1");
        let row = sqlx::query_as::<_, PayloadRow>(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let migrated = self
            .migrate_payload(
                object_type,
                row.schema_version,
                target_version,
                row.payload.clone(),
            )
            .await?;

        if row.schema_version != target_version {
            let update =
                format!("UPDATE {table} SET schema_version = $1, payload = $2 WHERE id = $3");
            sqlx::query(&update)
                .bind(target_version)
                .bind(&migrated)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_error)?;
        }

        serde_json::from_value(migrated)
            .map(Some)
            .map_err(map_serde_error(object_type))
    }

    async fn fetch_payloads_with_ids<T: DeserializeOwned>(
        &self,
        table: &str,
        object_type: &str,
        target_version: i32,
    ) -> Result<Vec<(String, T)>, StoreError> {
        #[derive(sqlx::FromRow)]
        struct PayloadWithId {
            id: String,
            schema_version: i32,
            payload: Value,
        }

        let query = format!("SELECT id, schema_version, payload FROM {table}");
        let rows = sqlx::query_as::<_, PayloadWithId>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let migrated = self
                .migrate_payload(
                    object_type,
                    row.schema_version,
                    target_version,
                    row.payload.clone(),
                )
                .await?;

            if row.schema_version != target_version {
                let update =
                    format!("UPDATE {table} SET schema_version = $1, payload = $2 WHERE id = $3");
                sqlx::query(&update)
                    .bind(target_version)
                    .bind(&migrated)
                    .bind(&row.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_error)?;
            }

            let value: T =
                serde_json::from_value(migrated).map_err(map_serde_error(object_type))?;
            results.push((row.id, value));
        }

        Ok(results)
    }

    async fn insert_payload<T: Serialize>(
        &self,
        table: &str,
        object_type: &str,
        id: &str,
        version: i32,
        payload: &T,
    ) -> Result<(), StoreError> {
        let payload_value = serde_json::to_value(payload).map_err(map_serde_error(object_type))?;

        let query =
            format!("INSERT INTO {table} (id, schema_version, payload) VALUES ($1, $2, $3)");
        sqlx::query(&query)
            .bind(id)
            .bind(version)
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
        version: i32,
        payload: &T,
    ) -> Result<(), StoreError> {
        let payload_value = serde_json::to_value(payload).map_err(map_serde_error(object_type))?;

        let query = format!("UPDATE {table} SET schema_version = $1, payload = $2 WHERE id = $3");
        let result = sqlx::query(&query)
            .bind(version)
            .bind(payload_value)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(StoreError::Internal(format!(
                "{object_type} '{id}' was missing during update"
            )));
        }

        Ok(())
    }
}

fn map_sqlx_error(err: sqlx::Error) -> StoreError {
    StoreError::Internal(err.to_string())
}

fn map_serde_error(object_type: &str) -> impl FnOnce(serde_json::Error) -> StoreError + '_ {
    move |err| StoreError::Internal(format!("failed to encode/decode {object_type}: {err}"))
}

#[derive(Clone)]
struct IssueGraphContext {
    known_issues: HashSet<IssueId>,
    forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
    reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>>,
}

impl IssueGraphContext {
    fn new(issues: &[(IssueId, Issue)]) -> Self {
        let mut forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();

        for (issue_id, issue) in issues {
            for dependency in &issue.dependencies {
                forward
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(dependency.issue_id.clone())
                    .or_default()
                    .push(issue_id.clone());

                reverse
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(issue_id.clone())
                    .or_default()
                    .push(dependency.issue_id.clone());
            }
        }

        Self {
            known_issues: issues.iter().map(|(id, _)| id.clone()).collect(),
            forward,
            reverse,
        }
    }

    fn contains_issue(&self, issue_id: &IssueId) -> bool {
        self.known_issues.contains(issue_id)
    }

    fn adjacency(
        &self,
        side: IssueGraphFilterSide,
        dependency_type: IssueDependencyType,
    ) -> Option<&HashMap<IssueId, Vec<IssueId>>> {
        match side {
            IssueGraphFilterSide::Left => self.forward.get(&dependency_type),
            IssueGraphFilterSide::Right => self.reverse.get(&dependency_type),
        }
    }
}

fn apply_graph_filters(
    context: &IssueGraphContext,
    filters: &[IssueGraphFilter],
) -> Result<HashSet<IssueId>, StoreError> {
    let mut intersection: Option<HashSet<IssueId>> = None;

    for filter in filters {
        let literal = filter.literal_issue_id();
        if !context.contains_issue(literal) {
            return Err(StoreError::IssueNotFound(literal.clone()));
        }

        let adjacency = context.adjacency(filter.wildcard_position(), filter.dependency_type);
        let matches = collect_matches(adjacency, literal, filter.wildcard_kind());

        match &mut intersection {
            Some(existing) => existing.retain(|id| matches.contains(id)),
            None => intersection = Some(matches),
        }

        if let Some(existing) = &intersection {
            if existing.is_empty() {
                break;
            }
        }
    }

    Ok(intersection.unwrap_or_default())
}

fn collect_matches(
    adjacency: Option<&HashMap<IssueId, Vec<IssueId>>>,
    literal: &IssueId,
    wildcard: IssueGraphWildcard,
) -> HashSet<IssueId> {
    let Some(map) = adjacency else {
        return HashSet::new();
    };

    match wildcard {
        IssueGraphWildcard::Immediate => map
            .get(literal)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        IssueGraphWildcard::Transitive => {
            let mut matches = HashSet::new();
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();

            visited.insert(literal.clone());
            queue.push_back(literal.clone());

            while let Some(current) = queue.pop_front() {
                if let Some(neighbors) = map.get(&current) {
                    for neighbor in neighbors {
                        if visited.insert(neighbor.clone()) {
                            queue.push_back(neighbor.clone());
                        }
                        matches.insert(neighbor.clone());
                    }
                }
            }

            matches
        }
    }
}

#[async_trait]
impl Store for PostgresStore {
    async fn add_issue(&mut self, issue: Issue) -> Result<IssueId, StoreError> {
        self.validate_issue_dependencies(&issue.dependencies)
            .await?;
        let id = IssueId::new();

        self.insert_payload(
            TABLE_ISSUES,
            "issue",
            id.as_ref(),
            ISSUE_SCHEMA_VERSION,
            &issue,
        )
        .await?;

        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Issue, StoreError> {
        self.fetch_payload(TABLE_ISSUES, "issue", id.as_ref(), ISSUE_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn update_issue(&mut self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
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

    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError> {
        let rows = self
            .fetch_payloads_with_ids::<Issue>(TABLE_ISSUES, "issue", ISSUE_SCHEMA_VERSION)
            .await?;

        rows.into_iter()
            .map(|(id, issue)| {
                id.parse::<IssueId>()
                    .map(|issue_id| (issue_id, issue))
                    .map_err(|err| {
                        StoreError::Internal(format!("invalid issue id stored in database: {err}"))
                    })
            })
            .collect()
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let issues = self.list_issues().await?;
        let context = IssueGraphContext::new(&issues);
        apply_graph_filters(&context, filters)
    }

    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.insert_payload(
            TABLE_PATCHES,
            "patch",
            id.as_ref(),
            PATCH_SCHEMA_VERSION,
            &patch,
        )
        .await?;
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError> {
        self.fetch_payload(TABLE_PATCHES, "patch", id.as_ref(), PATCH_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
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

    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError> {
        let rows = self
            .fetch_payloads_with_ids::<Patch>(TABLE_PATCHES, "patch", PATCH_SCHEMA_VERSION)
            .await?;

        rows.into_iter()
            .map(|(id, patch)| {
                id.parse::<PatchId>()
                    .map(|patch_id| (patch_id, patch))
                    .map_err(|err| {
                        StoreError::Internal(format!("invalid patch id stored in database: {err}"))
                    })
            })
            .collect()
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_patch_exists(patch_id).await?;
        let issues = self.list_issues().await?;

        Ok(issues
            .into_iter()
            .filter(|(_, issue)| issue.patches.contains(patch_id))
            .map(|(id, _)| id)
            .collect())
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.ensure_issue_exists(issue_id).await?;
        let issues = self.list_issues().await?;
        Ok(issues
            .into_iter()
            .filter_map(|(id, issue)| {
                issue
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
        let issues = self.list_issues().await?;
        Ok(issues
            .into_iter()
            .filter_map(|(id, issue)| {
                issue
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
        let tasks = self.list_tasks().await?;
        let mut results = Vec::new();

        for task_id in tasks {
            if let Ok(task) = self.get_task(&task_id).await {
                if task.spawned_from.as_ref() == Some(issue_id) {
                    results.push(task_id);
                }
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use metis_common::{
                RepoName,
                issues::{
                    Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, TodoItem,
                },
                jobs::BundleSpec,
                patches::{Patch, PatchStatus},
                users::User,
            };
            use std::{collections::HashSet, str::FromStr};

            #[allow(dead_code)]
            fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
                Issue {
                    issue_type: IssueType::Task,
                    description: "details".to_string(),
                    creator: String::new(),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: None,
                    todo_list: vec![TodoItem {
                        description: "todo".to_string(),
                        is_done: false,
                    }],
                    dependencies,
                    patches: Vec::new(),
                }
            }

            #[allow(dead_code)]
            fn sample_patch() -> Patch {
                Patch {
                    title: "patch title".to_string(),
                    description: "desc".to_string(),
                    diff: "diff".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    created_by: None,
                    reviews: Vec::new(),
                    service_repo_name: RepoName::from_str("dourolabs/sample").unwrap(),
                    github: None,
                }
            }

            #[allow(dead_code)]
            fn sample_task() -> Task {
                Task {
                    prompt: "prompt".to_string(),
                    context: BundleSpec::None,
                    spawned_from: None,
                    image: Some("metis-worker:latest".to_string()),
                    env_vars: Default::default(),
                }
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn issue_round_trip(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);

                let parent = store.add_issue(sample_issue(vec![])).await.unwrap();
                let issue = store
                    .add_issue(sample_issue(vec![IssueDependency {
                        dependency_type: IssueDependencyType::ChildOf,
                        issue_id: parent.clone(),
                    }]))
                    .await
                    .unwrap();

                let fetched = store.get_issue(&issue).await.unwrap();
                assert_eq!(fetched.dependencies.len(), 1);

                let issues: HashSet<_> = store
                    .list_issues()
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
                assert!(issues.contains(&issue));

                let children = store.get_issue_children(&parent).await.unwrap();
                assert_eq!(children, vec![issue.clone()]);

                let new_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
                let mut updated_issue = sample_issue(vec![IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: new_parent.clone(),
                }]);
                updated_issue.patches = Vec::new();
                store.update_issue(&issue, updated_issue).await.unwrap();

                assert!(store.get_issue_children(&parent).await.unwrap().is_empty());
                assert_eq!(
                    store.get_issue_children(&new_parent).await.unwrap(),
                    vec![issue]
                );
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn add_issue_rejects_missing_dependency(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);
                let missing = IssueId::new();

                let err = store
                    .add_issue(sample_issue(vec![IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: missing.clone(),
                    }]))
                    .await
                    .unwrap_err();

                assert!(matches!(err, StoreError::InvalidDependency(id) if id == missing));

                let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
                let err = store
                    .update_issue(
                        &issue_id,
                        sample_issue(vec![IssueDependency {
                            dependency_type: IssueDependencyType::ChildOf,
                            issue_id: missing.clone(),
                        }]),
                    )
                    .await
                    .unwrap_err();
                assert!(matches!(err, StoreError::InvalidDependency(id) if id == missing));
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn issue_graph_searches_blockers(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);
                let blocker = store.add_issue(sample_issue(vec![])).await.unwrap();
                let blocked = store
                    .add_issue(sample_issue(vec![IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: blocker.clone(),
                    }]))
                    .await
                    .unwrap();

                let blocked_list = store.get_issue_blocked_on(&blocker).await.unwrap();
                assert_eq!(blocked_list, vec![blocked.clone()]);

                let filter: IssueGraphFilter = format!("*:blocked-on:{blocker}").parse().unwrap();
                let matches = store.search_issue_graph(&[filter]).await.unwrap();
                assert_eq!(matches, HashSet::from([blocked]));
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn patch_associations_round_trip(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);
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
                assert_eq!(store.get_patch(&patch_id).await.unwrap().title, "updated");
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn task_lifecycle_updates_status(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);
                let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();

                let mut task = sample_task();
                task.spawned_from = Some(issue_id.clone());
                let task_id = store.add_task(task.clone(), Utc::now()).await.unwrap();
                assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Pending);

                store.mark_task_running(&task_id, Utc::now()).await.unwrap();
                assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Running);

                store
                    .mark_task_complete(&task_id, Ok(()), Some("done".into()), Utc::now())
                    .await
                    .unwrap();
                assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Complete);

                let tasks = store.get_tasks_for_issue(&issue_id).await.unwrap();
                assert_eq!(tasks, vec![task_id.clone()]);

                let mut updated_task = task.clone();
                updated_task.spawned_from = None;
                store
                    .update_task(&task_id, updated_task.clone())
                    .await
                    .unwrap();
                assert_eq!(store.get_task(&task_id).await.unwrap(), updated_task);
                assert!(
                    store
                        .get_tasks_for_issue(&issue_id)
                        .await
                        .unwrap()
                        .is_empty()
                );

                let complete = store
                    .list_tasks_with_status(Status::Complete)
                    .await
                    .unwrap();
                assert_eq!(complete, vec![task_id]);

                let explicit_id = TaskId::new();
                store
                    .add_task_with_id(explicit_id.clone(), sample_task(), Utc::now())
                    .await
                    .unwrap();
                let all_tasks = store.list_tasks().await.unwrap();
                assert!(all_tasks.contains(&explicit_id));
            }

            #[sqlx::test(migrations = "./migrations")]
            async fn user_management_round_trip(pool: PgStorePool) {
                let mut store = PostgresStore::new(pool);
                let user = User {
                    username: "alice".to_string(),
                    github_token: "token".to_string(),
                };
                store.add_user(user.clone()).await.unwrap();

                let users = store.list_users().await.unwrap();
                assert_eq!(users.len(), 1);
                assert_eq!(users[0], user);

                let updated = store
                    .set_user_github_token("alice", "new-token".to_string())
                    .await
                    .unwrap();
                assert_eq!(updated.github_token, "new-token");

                store.delete_user("alice").await.unwrap();
                assert!(store.list_users().await.unwrap().is_empty());

                let err = store.delete_user("alice").await.unwrap_err();
                assert!(matches!(err, StoreError::UserNotFound(name) if name == "alice"));
            }
        }

        Ok(results)
    }

    async fn add_task(
        &mut self,
        task: Task,
        creation_time: chrono::DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        let id = TaskId::new();
        self.add_task_with_id(id.clone(), task, creation_time)
            .await?;
        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: TaskId,
        task: Task,
        creation_time: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
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
            &task,
        )
        .await?;

        let status_log = TaskStatusLog::new(Status::Pending, creation_time);
        self.insert_payload(
            TABLE_TASK_STATUS_LOGS,
            "task_status_log",
            metis_id.as_ref(),
            TASK_STATUS_LOG_SCHEMA_VERSION,
            &status_log,
        )
        .await?;

        Ok(())
    }

    async fn update_task(&mut self, metis_id: &TaskId, task: Task) -> Result<(), StoreError> {
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
        .await
    }

    async fn get_task(&self, id: &TaskId) -> Result<Task, StoreError> {
        self.fetch_payload(TABLE_TASKS, "task", id.as_ref(), TASK_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        let rows = self
            .fetch_payloads_with_ids::<Task>(TABLE_TASKS, "task", TASK_SCHEMA_VERSION)
            .await?;

        rows.into_iter()
            .map(|(id, _)| {
                id.parse::<TaskId>().map_err(|err| {
                    StoreError::Internal(format!("invalid task id stored in database: {err}"))
                })
            })
            .collect()
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        let rows = self
            .fetch_payloads_with_ids::<TaskStatusLog>(
                TABLE_TASK_STATUS_LOGS,
                "task_status_log",
                TASK_STATUS_LOG_SCHEMA_VERSION,
            )
            .await?;

        let mut matches = Vec::new();
        for (id, log) in rows {
            if log.current_status() == status {
                matches.push(id.parse::<TaskId>().map_err(|err| {
                    StoreError::Internal(format!("invalid task id stored in status log: {err}"))
                })?);
            }
        }

        Ok(matches)
    }

    async fn get_status(&self, id: &TaskId) -> Result<Status, StoreError> {
        Ok(self.get_status_log(id).await?.current_status())
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.fetch_payload(
            TABLE_TASK_STATUS_LOGS,
            "task_status_log",
            id.as_ref(),
            TASK_STATUS_LOG_SCHEMA_VERSION,
        )
        .await?
        .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn mark_task_running(
        &mut self,
        id: &TaskId,
        start_time: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut status_log = self.get_status_log(id).await?;

        if status_log.current_status() != Status::Pending {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log
            .events
            .push(metis_common::task_status::Event::Started { at: start_time });

        self.update_payload(
            TABLE_TASK_STATUS_LOGS,
            "task_status_log",
            id.as_ref(),
            TASK_STATUS_LOG_SCHEMA_VERSION,
            &status_log,
        )
        .await
    }

    async fn mark_task_complete(
        &mut self,
        id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut status_log = self.get_status_log(id).await?;

        if status_log.current_status() != Status::Running {
            return Err(StoreError::InvalidStatusTransition);
        }

        let event = match result {
            Ok(()) => metis_common::task_status::Event::Completed {
                at: end_time,
                last_message,
            },
            Err(error) => metis_common::task_status::Event::Failed {
                at: end_time,
                error,
            },
        };
        status_log.events.push(event);

        self.update_payload(
            TABLE_TASK_STATUS_LOGS,
            "task_status_log",
            id.as_ref(),
            TASK_STATUS_LOG_SCHEMA_VERSION,
            &status_log,
        )
        .await
    }

    async fn add_user(&mut self, user: User) -> Result<(), StoreError> {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(1) FROM {TABLE_USERS} WHERE id = $1"
        ))
        .bind(user.username.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;

        if exists > 0 {
            return Err(StoreError::UserAlreadyExists(user.username));
        }

        self.insert_payload(
            TABLE_USERS,
            "user",
            user.username.as_str(),
            USER_SCHEMA_VERSION,
            &user,
        )
        .await
    }

    async fn list_users(&self) -> Result<Vec<User>, StoreError> {
        let mut users = self
            .fetch_payloads_with_ids::<User>(TABLE_USERS, "user", USER_SCHEMA_VERSION)
            .await?
            .into_iter()
            .map(|(_, user)| user)
            .collect::<Vec<_>>();
        users.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(users)
    }

    async fn delete_user(&mut self, username: &str) -> Result<(), StoreError> {
        let query = format!("DELETE FROM {TABLE_USERS} WHERE id = $1");
        let result = sqlx::query(&query)
            .bind(username)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_error)?;

        if result.rows_affected() == 0 {
            return Err(StoreError::UserNotFound(username.to_string()));
        }

        Ok(())
    }

    async fn set_user_github_token(
        &mut self,
        username: &str,
        github_token: String,
    ) -> Result<User, StoreError> {
        let mut user: User = self
            .fetch_payload(TABLE_USERS, "user", username, USER_SCHEMA_VERSION)
            .await?
            .ok_or_else(|| StoreError::UserNotFound(username.to_string()))?;

        user.github_token = github_token;

        self.update_payload(TABLE_USERS, "user", username, USER_SCHEMA_VERSION, &user)
            .await?;

        Ok(user)
    }
}
