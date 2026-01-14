use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use super::{Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use metis_common::task_status::Event;
use metis_common::{IssueId, MetisId, PatchId, TaskId};
use metis_common::{
    issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus},
    patches::Patch,
};

/// An in-memory implementation of the Store trait.
///
/// This store keeps tasks, issues, and patches in HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<TaskId, Task>,
    /// Maps issue IDs to their Issue data
    issues: HashMap<IssueId, Issue>,
    /// Maps patch IDs to their Patch data
    patches: HashMap<PatchId, Patch>,
    /// Maps parent issue IDs to their child issue IDs declared via child-of dependencies
    issue_children: HashMap<IssueId, Vec<IssueId>>,
    /// Maps blocking issue IDs to the issues that are blocked on them
    issue_blocked_on: HashMap<IssueId, Vec<IssueId>>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<TaskId, TaskStatusLog>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            issues: HashMap::new(),
            patches: HashMap::new(),
            issue_children: HashMap::new(),
            issue_blocked_on: HashMap::new(),
            status_logs: HashMap::new(),
        }
    }

    /// Updates issue adjacency indexes to match the provided dependency list.
    fn apply_issue_dependency_delta(
        &mut self,
        issue_id: &IssueId,
        previous: &[IssueDependency],
        updated: &[IssueDependency],
    ) {
        for dependency in previous {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    if let Some(children) = self.issue_children.get_mut(&dependency.issue_id) {
                        children.retain(|child_id| child_id != issue_id);
                        if children.is_empty() {
                            self.issue_children.remove(&dependency.issue_id);
                        }
                    }
                }
                IssueDependencyType::BlockedOn => {
                    if let Some(blocked) = self.issue_blocked_on.get_mut(&dependency.issue_id) {
                        blocked.retain(|blocked_id| blocked_id != issue_id);
                        if blocked.is_empty() {
                            self.issue_blocked_on.remove(&dependency.issue_id);
                        }
                    }
                }
            }
        }

        for dependency in updated {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    let children = self
                        .issue_children
                        .entry(dependency.issue_id.clone())
                        .or_default();
                    if !children.contains(issue_id) {
                        children.push(issue_id.clone());
                    }
                }
                IssueDependencyType::BlockedOn => {
                    let blocked = self
                        .issue_blocked_on
                        .entry(dependency.issue_id.clone())
                        .or_default();
                    if !blocked.contains(issue_id) {
                        blocked.push(issue_id.clone());
                    }
                }
            }
        }
    }

    fn join_issue_ids(ids: &[IssueId]) -> String {
        let mut values: Vec<String> = ids.iter().map(ToString::to_string).collect();
        values.sort();
        values.join(", ")
    }

    fn validate_issue_lifecycle(
        &self,
        issue_id: Option<&IssueId>,
        issue: &Issue,
    ) -> Result<(), StoreError> {
        if issue.status != IssueStatus::Closed {
            return Ok(());
        }

        let mut open_blockers = Vec::new();
        for dependency in issue
            .dependencies
            .iter()
            .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
        {
            let blocker = self
                .issues
                .get(&dependency.issue_id)
                .ok_or_else(|| StoreError::IssueNotFound(dependency.issue_id.clone()))?;

            if blocker.status != IssueStatus::Closed {
                open_blockers.push(dependency.issue_id.clone());
            }
        }

        if let Some(issue_id) = issue_id {
            let mut open_children = Vec::new();
            if let Some(children) = self.issue_children.get(issue_id) {
                for child_id in children {
                    let child = self
                        .issues
                        .get(child_id)
                        .ok_or_else(|| StoreError::IssueNotFound(child_id.clone()))?;

                    if child.status != IssueStatus::Closed {
                        open_children.push(child_id.clone());
                    }
                }
            }

            if !open_children.is_empty() {
                return Err(StoreError::InvalidIssueStatus(format!(
                    "cannot close issue with open child issues: {}",
                    Self::join_issue_ids(&open_children)
                )));
            }
        }

        if !open_blockers.is_empty() {
            return Err(StoreError::InvalidIssueStatus(format!(
                "blocked issues cannot close until blockers are closed: {}",
                Self::join_issue_ids(&open_blockers)
            )));
        }

        Ok(())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn add_issue(&mut self, issue: Issue) -> Result<IssueId, StoreError> {
        let id = IssueId::new();
        let new_dependencies = issue.dependencies.clone();

        self.validate_issue_lifecycle(None, &issue)?;
        self.issues.insert(id.clone(), issue);

        if !new_dependencies.is_empty() {
            self.apply_issue_dependency_delta(&id, &[], &new_dependencies);
        }
        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Issue, StoreError> {
        self.issues
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn update_issue(&mut self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        if !self.issues.contains_key(id) {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let previous_dependencies = self
            .issues
            .get(id)
            .map(|issue| issue.dependencies.clone())
            .unwrap_or_default();
        let updated_dependencies = issue.dependencies.clone();

        self.validate_issue_lifecycle(Some(id), &issue)?;
        self.issues.insert(id.clone(), issue);

        if !previous_dependencies.is_empty() || !updated_dependencies.is_empty() {
            self.apply_issue_dependency_delta(id, &previous_dependencies, &updated_dependencies);
        }
        Ok(())
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError> {
        Ok(self
            .issues
            .iter()
            .map(|(id, issue)| (id.clone(), issue.clone()))
            .collect())
    }

    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.patches.insert(id.clone(), patch);
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError> {
        self.patches
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        if !self.patches.contains_key(id) {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        self.patches.insert(id.clone(), patch);
        Ok(())
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError> {
        Ok(self
            .patches
            .iter()
            .map(|(id, patch)| (id.clone(), patch.clone()))
            .collect())
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_children
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_blocked_on
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let issue = self
            .issues
            .get(issue_id)
            .ok_or_else(|| StoreError::IssueNotFound(issue_id.clone()))?;

        let status = &issue.status;
        let dependencies = issue.dependencies.as_slice();

        match status {
            IssueStatus::Closed => Ok(false),
            IssueStatus::Dropped => Ok(false),
            IssueStatus::Open => {
                for dependency in dependencies {
                    if dependency.dependency_type == IssueDependencyType::BlockedOn {
                        let blocker_status = match self.issues.get(&dependency.issue_id) {
                            Some(blocker) => &blocker.status,
                            None => {
                                return Err(StoreError::IssueNotFound(dependency.issue_id.clone()));
                            }
                        };

                        if !matches!(blocker_status, IssueStatus::Closed) {
                            return Ok(false);
                        }
                    }
                }

                Ok(true)
            }
            IssueStatus::InProgress => {
                let children = self
                    .issue_children
                    .get(issue_id)
                    .cloned()
                    .unwrap_or_default();
                for child_id in children {
                    let child_status = match self.issues.get(&child_id) {
                        Some(child) => &child.status,
                        None => {
                            return Err(StoreError::IssueNotFound(child_id));
                        }
                    };

                    if !matches!(child_status, IssueStatus::Closed) {
                        return Ok(false);
                    }
                }

                Ok(true)
            }
        }
    }

    async fn add_task(
        &mut self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        // Generate a unique ID for the new task
        let id = TaskId::new();

        // Add the task
        self.tasks.insert(id.clone(), task);

        // Initialize status log
        self.status_logs.insert(
            id.clone(),
            TaskStatusLog::new(Status::Pending, creation_time),
        );

        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: TaskId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Check if task already exists
        if self.tasks.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!(
                "Task already exists: {metis_id}"
            )));
        }

        // Add the task with the specified ID
        self.tasks.insert(metis_id.clone(), task);

        // Initialize status log
        self.status_logs.insert(
            metis_id.clone(),
            TaskStatusLog::new(Status::Pending, creation_time),
        );

        Ok(())
    }

    async fn update_task(&mut self, metis_id: &TaskId, task: Task) -> Result<(), StoreError> {
        if !self.tasks.contains_key(metis_id) {
            return Err(StoreError::TaskNotFound(metis_id.clone()));
        }

        // Overwrite the existing task without modifying edge structure
        self.tasks.insert(metis_id.clone(), task);
        Ok(())
    }

    async fn get_task(&self, id: &TaskId) -> Result<Task, StoreError> {
        self.tasks
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        Ok(self.tasks.keys().cloned().collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        Ok(self
            .status_logs
            .iter()
            .filter(|(_, status_log)| status_log.current_status() == status)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn get_status(&self, id: &TaskId) -> Result<Status, StoreError> {
        self.status_logs
            .get(id)
            .map(|status_log| status_log.current_status())
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.status_logs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    fn get_result(&self, id: &TaskId) -> Option<Result<(), TaskError>> {
        self.status_logs.get(id).and_then(TaskStatusLog::result)
    }

    async fn mark_task_running(
        &mut self,
        id: &TaskId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Verify current status is Pending
        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log.events.push(Event::Started { at: start_time });

        Ok(())
    }

    async fn emit_task_artifacts(
        &mut self,
        id: &TaskId,
        artifact_ids: Vec<MetisId>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Running) {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log.events.push(Event::Emitted { at, artifact_ids });

        Ok(())
    }

    async fn mark_task_complete(
        &mut self,
        id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Running) {
            return Err(StoreError::InvalidStatusTransition);
        }

        let event = match result {
            Ok(()) => Event::Completed {
                at: end_time,
                last_message,
            },
            Err(error) => Event::Failed {
                at: end_time,
                error,
            },
        };

        status_log.events.push(event);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use metis_common::{
        issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType},
        jobs::BundleSpec,
        patches::{Patch, PatchStatus},
    };
    use std::collections::HashSet;

    fn spawn_task() -> Task {
        Task {
            prompt: "0".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: Some("metis-worker:latest".to_string()),
            env_vars: HashMap::new(),
        }
    }

    fn sample_patch() -> Patch {
        Patch {
            title: "sample patch".to_string(),
            diff: "diff --git a/file b/file".to_string(),
            description: "sample patch".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
        }
    }

    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            progress: String::new(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies,
            patches: Vec::new(),
        }
    }

    fn issue_with_status(status: IssueStatus, dependencies: Vec<IssueDependency>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            progress: String::new(),
            status,
            assignee: None,
            dependencies,
            patches: Vec::new(),
        }
    }

    #[tokio::test]
    async fn add_and_get_patch_assigns_id() {
        let mut store = MemoryStore::new();

        let patch = sample_patch();
        let id = store.add_patch(patch.clone()).await.unwrap();

        assert_eq!(store.get_patch(&id).await.unwrap(), patch);
    }

    #[tokio::test]
    async fn update_patch_overwrites_existing_value() {
        let mut store = MemoryStore::new();

        let id = store.add_patch(sample_patch()).await.unwrap();
        let updated = Patch {
            title: "new title".to_string(),
            diff: "noop".to_string(),
            description: "updated patch".to_string(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            reviews: Vec::new(),
        };

        store.update_patch(&id, updated.clone()).await.unwrap();

        assert_eq!(store.get_patch(&id).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn update_missing_patch_returns_error() {
        let mut store = MemoryStore::new();
        let missing: PatchId = "p-miss".parse().unwrap();

        let err = store
            .update_patch(
                &missing,
                Patch {
                    title: "noop patch".to_string(),
                    diff: "noop".to_string(),
                    description: "noop patch".to_string(),
                    status: PatchStatus::Open,
                    is_automatic_backup: false,
                    reviews: Vec::new(),
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::PatchNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn issue_dependency_indexes_populated_on_create() {
        let mut store = MemoryStore::new();

        let parent_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocker_id = store.add_issue(sample_issue(vec![])).await.unwrap();

        let child_id = store
            .add_issue(sample_issue(vec![
                IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: parent_id.clone(),
                },
                IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: blocker_id.clone(),
                },
            ]))
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
    async fn issue_dependency_indexes_updated_on_update_and_removal() {
        let mut store = MemoryStore::new();

        let original_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let new_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let original_blocker = store.add_issue(sample_issue(vec![])).await.unwrap();
        let new_blocker = store.add_issue(sample_issue(vec![])).await.unwrap();

        let issue_id = store
            .add_issue(sample_issue(vec![
                IssueDependency {
                    dependency_type: IssueDependencyType::ChildOf,
                    issue_id: original_parent.clone(),
                },
                IssueDependency {
                    dependency_type: IssueDependencyType::BlockedOn,
                    issue_id: original_blocker.clone(),
                },
            ]))
            .await
            .unwrap();

        assert_eq!(
            store.get_issue_children(&original_parent).await.unwrap(),
            vec![issue_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&original_blocker).await.unwrap(),
            vec![issue_id.clone()]
        );

        store
            .update_issue(
                &issue_id,
                sample_issue(vec![
                    IssueDependency {
                        dependency_type: IssueDependencyType::ChildOf,
                        issue_id: new_parent.clone(),
                    },
                    IssueDependency {
                        dependency_type: IssueDependencyType::BlockedOn,
                        issue_id: new_blocker.clone(),
                    },
                ]),
            )
            .await
            .unwrap();

        assert!(
            store
                .get_issue_children(&original_parent)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .get_issue_blocked_on(&original_blocker)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.get_issue_children(&new_parent).await.unwrap(),
            vec![issue_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&new_blocker).await.unwrap(),
            vec![issue_id.clone()]
        );

        store
            .update_issue(&issue_id, sample_issue(vec![]))
            .await
            .unwrap();

        assert!(
            store
                .get_issue_children(&new_parent)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .get_issue_blocked_on(&new_blocker)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn open_issue_ready_when_not_blocked() {
        let mut store = MemoryStore::new();

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();

        assert!(store.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_blocked_on_open_issue() {
        let mut store = MemoryStore::new();

        let blocker_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocked_issue_id = store
            .add_issue(sample_issue(vec![IssueDependency {
                dependency_type: IssueDependencyType::BlockedOn,
                issue_id: blocker_id.clone(),
            }]))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&blocked_issue_id).await.unwrap());

        store
            .update_issue(&blocker_id, issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap();

        assert!(store.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_issue_ready_after_children_closed() {
        let mut store = MemoryStore::new();

        let parent_id = store
            .add_issue(issue_with_status(IssueStatus::InProgress, vec![]))
            .await
            .unwrap();
        let child_dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: parent_id.clone(),
        }];
        let child_id = store
            .add_issue(issue_with_status(
                IssueStatus::Open,
                child_dependencies.clone(),
            ))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&parent_id).await.unwrap());

        store
            .update_issue(
                &child_id,
                issue_with_status(IssueStatus::Closed, child_dependencies),
            )
            .await
            .unwrap();

        assert!(store.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_is_not_ready() {
        let mut store = MemoryStore::new();

        let issue_id = store
            .add_issue(issue_with_status(IssueStatus::Dropped, vec![]))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let mut store = MemoryStore::new();

        let blocker_id = store
            .add_issue(issue_with_status(IssueStatus::Dropped, vec![]))
            .await
            .unwrap();
        let blocked_dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::BlockedOn,
            issue_id: blocker_id.clone(),
        }];
        let blocked_issue_id = store
            .add_issue(issue_with_status(IssueStatus::Open, blocked_dependencies))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn closed_issue_is_not_ready() {
        let mut store = MemoryStore::new();

        let issue_id = store
            .add_issue(issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn closing_issue_requires_closed_blockers() {
        let mut store = MemoryStore::new();

        let blocker_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocked_dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::BlockedOn,
            issue_id: blocker_id.clone(),
        }];
        let blocked_issue_id = store
            .add_issue(sample_issue(blocked_dependencies.clone()))
            .await
            .unwrap();

        let err = store
            .update_issue(
                &blocked_issue_id,
                issue_with_status(IssueStatus::Closed, blocked_dependencies.clone()),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidIssueStatus(message)
                if message.contains(&blocker_id.to_string())
        ));

        store
            .update_issue(&blocker_id, issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap();

        store
            .update_issue(
                &blocked_issue_id,
                issue_with_status(IssueStatus::Closed, blocked_dependencies),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closing_parent_requires_closed_children() {
        let mut store = MemoryStore::new();

        let parent_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let child_dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::ChildOf,
            issue_id: parent_id.clone(),
        }];
        let child_id = store
            .add_issue(sample_issue(child_dependencies.clone()))
            .await
            .unwrap();

        let err = store
            .update_issue(&parent_id, issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidIssueStatus(message)
                if message.contains(&child_id.to_string())
        ));

        store
            .update_issue(
                &child_id,
                issue_with_status(IssueStatus::Closed, child_dependencies),
            )
            .await
            .unwrap();

        store
            .update_issue(&parent_id, issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn add_and_retrieve_tasks() {
        let mut store = MemoryStore::new();

        let task = spawn_task();
        let task_id = store.add_task(task.clone(), Utc::now()).await.unwrap();

        assert_eq!(store.get_task(&task_id).await.unwrap(), task);
        assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Pending);

        let tasks: HashSet<_> = store.list_tasks().await.unwrap().into_iter().collect();
        assert_eq!(tasks, HashSet::from([task_id]));
    }

    #[tokio::test]
    async fn task_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_running_transitions_from_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);
    }

    #[tokio::test]
    async fn mark_task_complete_transitions_from_running() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        // First mark as running
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);

        // Then mark as complete
        store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);
    }

    #[tokio::test]
    async fn mark_task_failed_transitions_from_running() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        // First mark as running
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);

        // Then mark as failed
        store
            .mark_task_complete(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test failure".to_string(),
                }),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Failed);
    }

    #[tokio::test]
    async fn mark_task_complete_from_pending_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as complete from pending should fail
        let err = store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn mark_task_failed_from_pending_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as failed from pending should fail
        let err = store
            .mark_task_complete(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test".to_string(),
                }),
                None,
                Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }
}
