use crate::{
    domain::actors::ActorRef,
    domain::issues::{Issue, IssueDependencyType, IssueGraphFilter, IssueStatus, TodoItem},
    store::{ReadOnlyStore, Status, StoreError},
};
use metis_common::{
    SessionId, VersionNumber, Versioned, api::v1 as api, api::v1::issues::SearchIssuesQuery,
    issues::IssueId,
};
use std::collections::HashSet;
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum UpsertIssueError {
    #[error("job_id may only be provided when creating an issue")]
    JobIdProvidedForUpdate,
    #[error("issue creator must be set")]
    MissingCreator,
    #[error("issue dependency '{dependency_id}' not found")]
    MissingDependency {
        #[source]
        source: StoreError,
        dependency_id: IssueId,
    },
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: Option<IssueId>,
    },
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: SessionId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: SessionId,
    },
    #[error("job_id must reference a running job")]
    JobNotRunning {
        job_id: SessionId,
        status: Option<Status>,
    },
    #[error("failed to read tasks for dropped issue '{issue_id}'")]
    TaskLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("failed to kill task '{job_id}' for dropped issue '{issue_id}'")]
    KillTask {
        #[source]
        source: crate::job_engine::JobEngineError,
        issue_id: IssueId,
        job_id: SessionId,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
}

#[derive(Debug, Error)]
pub enum UpdateTodoListError {
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("todo item number {item_number} is out of range for issue '{issue_id}'")]
    InvalidItemNumber {
        issue_id: IssueId,
        item_number: usize,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
}

impl AppState {
    pub async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue(issue_id, include_deleted).await
    }

    pub async fn get_issue_versions(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue_versions(issue_id).await
    }

    pub async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let store = self.store.as_ref();
        store.search_issue_graph(filters).await
    }

    pub async fn list_issues(&self) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_issues(&SearchIssuesQuery::default()).await
    }

    pub async fn list_issues_with_query(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_issues(query).await
    }

    pub async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        let store = self.store.as_ref();
        store.count_issues(query).await
    }

    pub async fn upsert_issue(
        &self,
        issue_id: Option<IssueId>,
        request: api::issues::UpsertIssueRequest,
        actor: ActorRef,
    ) -> Result<(IssueId, VersionNumber), UpsertIssueError> {
        let api::issues::UpsertIssueRequest {
            issue,
            session_id: job_id,
            label_ids,
            label_names,
            ..
        } = request;
        let issue: Issue = issue.into();
        let is_create = issue_id.is_none();
        let dependencies = issue.dependencies.clone();

        let store = self.store.as_ref();
        let label_actor = actor.clone();

        let (issue_id, version) = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();

                // Run restriction policies (require_creator, issue_lifecycle_validation)
                {
                    self.policy_engine
                        .check_update_issue(&id, &updated_issue, None, store, &actor)
                        .await?;
                }

                match self
                    .store
                    .update_issue_with_actor(&id, updated_issue, actor)
                    .await
                {
                    Ok(version) => (id, version),
                    Err(source @ StoreError::IssueNotFound(_)) => {
                        return Err(UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        });
                    }
                    Err(StoreError::InvalidDependency(dependency_id)) => {
                        return Err(UpsertIssueError::MissingDependency {
                            dependency_id: dependency_id.clone(),
                            source: StoreError::InvalidDependency(dependency_id),
                        });
                    }
                    Err(source) => {
                        return Err(UpsertIssueError::Store {
                            source,
                            issue_id: Some(id),
                        });
                    }
                }
            }
            None => {
                if let Some(ref job_id) = job_id {
                    let status = store
                        .get_session(job_id, false)
                        .await
                        .map_err(|source| match source {
                            StoreError::SessionNotFound(_) => UpsertIssueError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertIssueError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?
                        .item
                        .status;

                    if status != Status::Running {
                        return Err(UpsertIssueError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }
                }

                // Run restriction policies (require_creator, issue_lifecycle_validation)
                {
                    self.policy_engine
                        .check_create_issue(&issue, store, &actor)
                        .await?;
                }

                let (id, version) = self
                    .store
                    .add_issue_with_actor(issue, actor)
                    .await
                    .map_err(|source| match source {
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: None,
                        },
                    })?;
                (id, version)
            }
        };

        info!(issue_id = %issue_id, "issue stored successfully");

        // Sync label associations if requested
        if label_ids.is_some() || label_names.is_some() {
            let resolved = self
                .resolve_label_ids(label_ids, label_names, label_actor)
                .await
                .map_err(|e| UpsertIssueError::Store {
                    source: match e {
                        super::CreateLabelError::Store { source } => source,
                        other => StoreError::Internal(other.to_string()),
                    },
                    issue_id: Some(issue_id.clone()),
                })?;

            let object_id = metis_common::MetisId::from(issue_id.clone());

            // Get current labels and compute diff
            let current_labels =
                self.get_labels_for_object(&object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;

            let current_ids: HashSet<metis_common::LabelId> =
                current_labels.iter().map(|l| l.label_id.clone()).collect();
            let desired_ids: HashSet<metis_common::LabelId> = resolved.into_iter().collect();

            // Remove labels that are no longer desired
            for old_id in current_ids.difference(&desired_ids) {
                self.remove_label_association(old_id, &object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;
            }

            // Add newly desired labels
            for new_id in desired_ids.difference(&current_ids) {
                self.add_label_association(new_id, &object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;
            }
        }

        // Inherit labels from parent issues when creating a child issue
        if is_create {
            let parent_ids: Vec<IssueId> = dependencies
                .iter()
                .filter(|d| d.dependency_type == IssueDependencyType::ChildOf)
                .map(|d| d.issue_id.clone())
                .collect();

            if !parent_ids.is_empty() {
                let parent_metis_ids: Vec<metis_common::MetisId> = parent_ids
                    .iter()
                    .map(|id| metis_common::MetisId::from(id.clone()))
                    .collect();
                let parent_labels = self
                    .get_labels_for_objects(&parent_metis_ids)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;

                let child_object_id = metis_common::MetisId::from(issue_id.clone());
                let mut inherited = HashSet::new();
                for labels in parent_labels.values() {
                    for label in labels {
                        if !label.recurse {
                            continue;
                        }
                        if inherited.insert(label.label_id.clone()) {
                            self.add_label_association(&label.label_id, &child_object_id)
                                .await
                                .map_err(|source| UpsertIssueError::Store {
                                    source,
                                    issue_id: Some(issue_id.clone()),
                                })?;
                        }
                    }
                }
            }
        }

        Ok((issue_id, version))
    }

    pub async fn delete_issue(
        &self,
        issue_id: &IssueId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store.delete_issue_with_actor(issue_id, actor).await?;
        Ok(())
    }

    pub async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let store = self.store.as_ref();
        let mut visited = HashSet::new();
        issue_ready(store, issue_id, &mut visited).await
    }

    pub async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue_children(issue_id).await
    }

    pub async fn add_todo_item(
        &self,
        issue_id: IssueId,
        item: TodoItem,
        actor: ActorRef,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id, false).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        issue.todo_list.push(item);
        let todo_list = issue.todo_list.clone();
        self.store
            .update_issue_with_actor(&issue_id, issue, actor)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;
        Ok(todo_list)
    }

    pub async fn replace_todo_list(
        &self,
        issue_id: IssueId,
        todo_list: Vec<TodoItem>,
        actor: ActorRef,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id, false).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        issue.todo_list = todo_list.clone();
        self.store
            .update_issue_with_actor(&issue_id, issue, actor)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;
        Ok(todo_list)
    }

    pub async fn set_todo_item_status(
        &self,
        issue_id: IssueId,
        item_number: usize,
        is_done: bool,
        actor: ActorRef,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id, false).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        if item_number == 0 {
            return Err(UpdateTodoListError::InvalidItemNumber {
                issue_id,
                item_number,
            });
        }
        let index = item_number - 1;
        let item =
            issue
                .todo_list
                .get_mut(index)
                .ok_or(UpdateTodoListError::InvalidItemNumber {
                    issue_id: issue_id.clone(),
                    item_number,
                })?;
        item.is_done = is_done;

        let todo_list = issue.todo_list.clone();
        self.store
            .update_issue_with_actor(&issue_id, issue, actor)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;
        Ok(todo_list)
    }
}

fn issue_ready<'a>(
    store: &'a dyn ReadOnlyStore,
    issue_id: &'a IssueId,
    visited: &'a mut HashSet<IssueId>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, StoreError>> + Send + 'a>> {
    Box::pin(async move {
        if !visited.insert(issue_id.clone()) {
            // Cycle detected: treat as not ready to break the loop.
            return Ok(false);
        }

        let issue = store.get_issue(issue_id, false).await?;
        let issue = issue.item;

        match issue.status {
            IssueStatus::Closed
            | IssueStatus::Dropped
            | IssueStatus::Rejected
            | IssueStatus::Failed => Ok(false),
            IssueStatus::Open => {
                for dependency in issue.dependencies.iter().filter(|dependency| {
                    dependency.dependency_type == IssueDependencyType::BlockedOn
                }) {
                    let blocker = store.get_issue(&dependency.issue_id, false).await?;
                    if blocker.item.status != IssueStatus::Closed {
                        return Ok(false);
                    }
                }

                Ok(true)
            }
            IssueStatus::InProgress => {
                // Parent is ready when no issue in its entire child subtree is ready.
                // This enables re-planning: if all descendants are stuck, the parent can spawn.
                // We must check the full subtree, not just direct children, because a
                // non-ready child (InProgress) may still have ready descendants.
                for child_id in store.get_issue_children(issue_id).await? {
                    if subtree_has_ready_issue(store, &child_id, visited).await? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }
        }
    })
}

/// Returns true if any issue in the subtree rooted at `issue_id` is ready.
///
/// Unlike `issue_ready` (which answers "is this specific issue ready?"), this function
/// answers "does any ready issue exist anywhere in this subtree?". It mirrors the
/// status-based logic of `issue_ready` but recurses into children for InProgress nodes
/// to find ready descendants at any depth.
fn subtree_has_ready_issue<'a>(
    store: &'a dyn ReadOnlyStore,
    issue_id: &'a IssueId,
    visited: &'a mut HashSet<IssueId>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, StoreError>> + Send + 'a>> {
    Box::pin(async move {
        if !visited.insert(issue_id.clone()) {
            return Ok(false);
        }

        let issue = store.get_issue(issue_id, false).await?;
        let issue = issue.item;

        match issue.status {
            IssueStatus::Closed
            | IssueStatus::Dropped
            | IssueStatus::Rejected
            | IssueStatus::Failed => Ok(false),
            IssueStatus::Open => {
                // An Open issue is ready if all its blockers are closed.
                for dependency in issue.dependencies.iter().filter(|dependency| {
                    dependency.dependency_type == IssueDependencyType::BlockedOn
                }) {
                    let blocker = store.get_issue(&dependency.issue_id, false).await?;
                    if blocker.item.status != IssueStatus::Closed {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            IssueStatus::InProgress => {
                // An InProgress node's subtree always contains at least one ready issue:
                // either a ready descendant, or the InProgress node itself (which is ready
                // when all descendants are stuck). We still recurse into children so the
                // visited set is populated for cycle detection.
                for child_id in store.get_issue_children(issue_id).await? {
                    if subtree_has_ready_issue(store, &child_id, visited).await? {
                        return Ok(true);
                    }
                }
                // No child subtree has a ready issue, so this InProgress node itself is ready.
                Ok(true)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{
            ServerEvent,
            test_helpers::{issue_with_status, start_test_automation_runner, task_for_issue},
        },
        domain::actors::ActorRef,
        domain::issues::{IssueDependency, IssueDependencyType, IssueStatus},
        job_engine::{JobEngine, JobStatus},
        store::ReadOnlyStore,
        test_utils::{MockJobEngine, test_state, test_state_with_engine},
    };
    use chrono::Utc;
    use metis_common::api::v1 as api;
    use std::sync::Arc;

    /// Wait briefly for automations to process events.
    async fn wait_for_automations() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn open_issue_ready_when_not_blocked() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("open", IssueStatus::Open, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_blocked_on_open_issue() {
        let state = test_state();

        let (blocker_id, blocked_issue_id) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", IssueStatus::Open, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let (blocked_issue_id, _) = store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id.clone(),
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap();

            (blocker_id, blocked_issue_id)
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue_with_actor(
                    &blocker_id,
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_issue_ready_after_children_closed() {
        let state = test_state();

        let (parent_id, child_id, child_dependencies) = {
            let store = state.store.as_ref();
            let (parent_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("parent", IssueStatus::InProgress, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let child_dependencies = vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )];
            let (child_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("child", IssueStatus::Open, child_dependencies.clone()),
                    ActorRef::test(),
                )
                .await
                .unwrap();

            (parent_id, child_id, child_dependencies)
        };

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue_with_actor(
                    &child_id,
                    issue_with_status("child", IssueStatus::Closed, child_dependencies),
                    ActorRef::test(),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("dropped", IssueStatus::Dropped, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", IssueStatus::Dropped, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id,
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn closed_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("closed", IssueStatus::Closed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropping_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let grandchild_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, child_id.clone());
        let grandchild_issue = issue_with_status(
            "grandchild",
            IssueStatus::Open,
            vec![grandchild_dependency.clone()],
        );
        let (grandchild_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(grandchild_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Add a closed child -- should NOT be overwritten to Dropped
        let closed_child_issue = issue_with_status(
            "closed_child",
            IssueStatus::Closed,
            vec![child_dependency.clone()],
        );
        let (closed_child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(closed_child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Add a failed child -- should NOT be overwritten to Dropped
        let failed_child_issue = issue_with_status(
            "failed_child",
            IssueStatus::Failed,
            vec![child_dependency.clone()],
        );
        let (failed_child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(failed_child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let (parent_task_id, child_task_id, grandchild_task_id) = {
            let store = state.store.as_ref();
            let (parent_task_id, _) = store
                .add_session_with_actor(task_for_issue(&parent_id), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            let (child_task_id, _) = store
                .add_session_with_actor(task_for_issue(&child_id), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            let (grandchild_task_id, _) = store
                .add_session_with_actor(
                    task_for_issue(&grandchild_id),
                    Utc::now(),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            (parent_task_id, child_task_id, grandchild_task_id)
        };

        job_engine
            .insert_job(&parent_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&grandchild_task_id, JobStatus::Running)
            .await;

        let mut dropped_parent = parent_issue.clone();
        dropped_parent.status = IssueStatus::Dropped;
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(dropped_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            // Open children should be dropped
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                IssueStatus::Dropped
            );
            assert_eq!(
                store
                    .get_issue(&grandchild_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Dropped
            );
            // Terminal-state children should retain their original status
            assert_eq!(
                store
                    .get_issue(&closed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Closed
            );
            assert_eq!(
                store
                    .get_issue(&failed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Failed
            );
        }

        for task_id in [parent_task_id, child_task_id, grandchild_task_id] {
            let job = job_engine
                .find_job_by_metis_id(&task_id)
                .await
                .expect("job should exist");
            assert_eq!(job.status, JobStatus::Failed);
        }

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn event_bus_emits_issue_created_and_updated() {
        let state = test_state();
        let mut rx = state.subscribe();

        let issue = issue_with_status("test issue", IssueStatus::Open, Vec::new());
        let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .expect("create should succeed");

        let event = rx.recv().await.expect("should receive IssueCreated");
        assert!(
            matches!(&event, ServerEvent::IssueCreated { issue_id: id, .. } if *id == issue_id)
        );
        let first_seq = event.seq();
        assert!(first_seq > 0);

        let updated_issue = issue_with_status("updated issue", IssueStatus::InProgress, Vec::new());
        let update_request = api::issues::UpsertIssueRequest::new(updated_issue.into(), None);
        state
            .upsert_issue(Some(issue_id.clone()), update_request, ActorRef::test())
            .await
            .expect("update should succeed");

        let event = rx.recv().await.expect("should receive IssueUpdated");
        assert!(
            matches!(&event, ServerEvent::IssueUpdated { issue_id: id, .. } if *id == issue_id)
        );
        assert!(event.seq() > first_seq);
    }

    #[tokio::test]
    async fn event_bus_emits_issue_deleted() {
        let state = test_state();

        let issue = issue_with_status("doomed issue", IssueStatus::Open, Vec::new());
        let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .expect("create should succeed");

        let mut rx = state.subscribe();

        state
            .delete_issue(&issue_id, ActorRef::test())
            .await
            .expect("delete should succeed");

        let event = rx.recv().await.expect("should receive IssueDeleted");
        assert!(
            matches!(&event, ServerEvent::IssueDeleted { issue_id: id, .. } if *id == issue_id)
        );
    }

    #[tokio::test]
    async fn event_bus_seq_is_monotonically_increasing() {
        let state = test_state();
        let mut rx = state.subscribe();

        let mut seqs = Vec::new();
        for i in 0..5 {
            let issue = issue_with_status(&format!("issue {i}"), IssueStatus::Open, Vec::new());
            let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
            state
                .upsert_issue(None, request, ActorRef::test())
                .await
                .expect("create should succeed");
            let event = rx.recv().await.expect("should receive event");
            seqs.push(event.seq());
        }

        for window in seqs.windows(2) {
            assert!(
                window[0] < window[1],
                "seq numbers should be strictly increasing: {seqs:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejected_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("rejected", IssueStatus::Rejected, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn failed_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("failed", IssueStatus::Failed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_rejected() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", IssueStatus::Rejected, vec![child_dep]);
        store
            .add_issue_with_actor(child, ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_failed() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", IssueStatus::Failed, vec![child_dep]);
        store
            .add_issue_with_actor(child, ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_children_mixed_terminal() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("closed child", IssueStatus::Closed, vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status(
                    "dropped child",
                    IssueStatus::Dropped,
                    vec![child_dep.clone()],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status(
                    "rejected child",
                    IssueStatus::Rejected,
                    vec![child_dep.clone()],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_failed_and_sibling_blocked() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Child A: failed
        let (failed_child_id, _) = store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Child B: open, blocked on failed child A
        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, failed_child_id);
        store
            .add_issue_with_actor(
                issue_with_status(
                    "blocked child",
                    IssueStatus::Open,
                    vec![child_dep, blocked_dep],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Neither child is Ready: A is Failed (terminal), B is blocked on non-Closed A.
        // Parent should be ready.
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_not_ready_when_child_is_open_and_unblocked() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Closed child
        store
            .add_issue_with_actor(
                issue_with_status("closed child", IssueStatus::Closed, vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Open unblocked child — this child is Ready
        store
            .add_issue_with_actor(
                issue_with_status("open child", IssueStatus::Open, vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Parent should NOT be ready because the open child is Ready
        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_no_children() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        // No children — trivially, no child is Ready
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_with_nested_stuck_children() {
        let state = test_state();

        let store = state.store.as_ref();
        // Grandparent (InProgress) -> Parent (InProgress) -> Child (Failed)
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Parent is ready (child is Failed, not Ready).
        // But since parent IS ready, grandparent is NOT ready (has a ready child).
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_grandparent_not_ready_with_ready_grandchild() {
        let state = test_state();

        let store = state.store.as_ref();
        // Grandparent (InProgress) -> Parent (InProgress) -> Child (Open, unblocked)
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("open child", IssueStatus::Open, vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Child is ready (Open, no blockers).
        // Parent is NOT ready (has a ready child).
        // Grandparent is NOT ready (subtree contains a ready issue).
        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_grandparent_not_ready_when_parent_ready_due_to_blocked_child() {
        let state = test_state();

        let store = state.store.as_ref();
        // Grandparent (InProgress) -> Parent (InProgress) -> Child (Open, blocked)
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        // Create a blocker issue that is still open (not closed)
        let blocker = issue_with_status("blocker", IssueStatus::Open, vec![]);
        let (blocker_id, _) = store
            .add_issue_with_actor(blocker, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id);
        store
            .add_issue_with_actor(
                issue_with_status(
                    "blocked child",
                    IssueStatus::Open,
                    vec![child_dep, blocked_dep],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Child is NOT ready (blocked).
        // Parent IS ready (no ready children).
        // Grandparent is NOT ready (Parent is ready in its subtree).
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn rejected_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", IssueStatus::Rejected, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id,
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn failed_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", IssueStatus::Failed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id,
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn rejected_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let (child_task_id,) = {
            let store = state.store.as_ref();
            let (child_task_id, _) = store
                .add_session_with_actor(task_for_issue(&child_id), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            (child_task_id,)
        };

        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;

        let mut rejected_parent = parent_issue;
        rejected_parent.status = IssueStatus::Rejected;
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(rejected_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                IssueStatus::Dropped
            );
        }

        let job = job_engine
            .find_job_by_metis_id(&child_task_id)
            .await
            .expect("job should exist");
        assert_eq!(job.status, JobStatus::Failed);

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn failed_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut failed_parent = parent_issue;
        failed_parent.status = IssueStatus::Failed;
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(failed_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                IssueStatus::Dropped
            );
        }

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn rejected_blocker_does_not_auto_drop_dependents() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let blocker_issue = issue_with_status("blocker", IssueStatus::Open, vec![]);
        let (blocker_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(blocker_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone());
        let dependent_issue =
            issue_with_status("dependent", IssueStatus::Open, vec![blocked_dep.clone()]);
        let (dependent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(dependent_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut rejected_blocker = blocker_issue;
        rejected_blocker.status = IssueStatus::Rejected;
        state
            .upsert_issue(
                Some(blocker_id.clone()),
                api::issues::UpsertIssueRequest::new(rejected_blocker.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        // Dependent should remain Open (not dropped) — blocking is retained
        // but status is not changed
        {
            let store = state.store.as_ref();
            assert_eq!(
                store
                    .get_issue(&dependent_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Open
            );
        }

        // Dependent should not be ready (blocker is not Closed)
        assert!(!state.is_issue_ready(&dependent_id).await.unwrap());

        runner.shutdown().await;
    }
}
