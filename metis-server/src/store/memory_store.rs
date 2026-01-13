use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use super::{Edge, Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use metis_common::MetisId;
use metis_common::task_status::Event;
use metis_common::{
    issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus},
    patches::Patch,
};

/// An in-memory implementation of the Store trait.
///
/// This store maintains a DAG of tasks using HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<MetisId, Task>,
    /// Maps issue IDs to their Issue data
    issues: HashMap<MetisId, Issue>,
    /// Maps patch IDs to their Patch data
    patches: HashMap<MetisId, Patch>,
    /// Maps parent issue IDs to their child issue IDs declared via child-of dependencies
    issue_children: HashMap<MetisId, Vec<MetisId>>,
    /// Maps blocking issue IDs to the issues that are blocked on them
    issue_blocked_on: HashMap<MetisId, Vec<MetisId>>,
    /// Maps task IDs to their parent task edges (dependencies)
    parents: HashMap<MetisId, Vec<Edge>>,
    /// Maps task IDs to their child task IDs (dependents)
    children: HashMap<MetisId, Vec<MetisId>>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<MetisId, TaskStatusLog>,
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
            parents: HashMap::new(),
            children: HashMap::new(),
            status_logs: HashMap::new(),
        }
    }

    /// Checks if all parents of a task are complete (Complete or Failed).
    fn all_parents_complete(&self, id: &MetisId) -> bool {
        let parent_edges = self.parents.get(id).cloned().unwrap_or_default();
        parent_edges.iter().all(|edge| {
            self.status_logs
                .get(&edge.id)
                .map(|status_log| {
                    matches!(
                        status_log.current_status(),
                        Status::Complete | Status::Failed
                    )
                })
                .unwrap_or(false)
        })
    }

    /// Updates issue adjacency indexes to match the provided dependency list.
    fn apply_issue_dependency_delta(
        &mut self,
        issue_id: &MetisId,
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
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn add_issue(&mut self, issue: Issue) -> Result<MetisId, StoreError> {
        let id = Uuid::new_v4().hyphenated().to_string();
        let new_dependencies = issue.dependencies.clone();

        self.issues.insert(id.clone(), issue);

        if !new_dependencies.is_empty() {
            self.apply_issue_dependency_delta(&id, &[], &new_dependencies);
        }
        Ok(id)
    }

    async fn get_issue(&self, id: &MetisId) -> Result<Issue, StoreError> {
        self.issues
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn update_issue(&mut self, id: &MetisId, issue: Issue) -> Result<(), StoreError> {
        if !self.issues.contains_key(id) {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let previous_dependencies = self
            .issues
            .get(id)
            .map(|issue| issue.dependencies.clone())
            .unwrap_or_default();
        let updated_dependencies = issue.dependencies.clone();

        self.issues.insert(id.clone(), issue);

        if !previous_dependencies.is_empty() || !updated_dependencies.is_empty() {
            self.apply_issue_dependency_delta(id, &previous_dependencies, &updated_dependencies);
        }
        Ok(())
    }

    async fn list_issues(&self) -> Result<Vec<(MetisId, Issue)>, StoreError> {
        Ok(self
            .issues
            .iter()
            .map(|(id, issue)| (id.clone(), issue.clone()))
            .collect())
    }

    async fn add_patch(&mut self, patch: Patch) -> Result<MetisId, StoreError> {
        let id = Uuid::new_v4().hyphenated().to_string();
        self.patches.insert(id.clone(), patch);
        Ok(id)
    }

    async fn get_patch(&self, id: &MetisId) -> Result<Patch, StoreError> {
        self.patches
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn update_patch(&mut self, id: &MetisId, patch: Patch) -> Result<(), StoreError> {
        if !self.patches.contains_key(id) {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        self.patches.insert(id.clone(), patch);
        Ok(())
    }

    async fn list_patches(&self) -> Result<Vec<(MetisId, Patch)>, StoreError> {
        Ok(self
            .patches
            .iter()
            .map(|(id, patch)| (id.clone(), patch.clone()))
            .collect())
    }

    async fn get_issue_children(&self, issue_id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_children
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_issue_blocked_on(&self, issue_id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_blocked_on
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn is_issue_ready(&self, issue_id: &MetisId) -> Result<bool, StoreError> {
        let issue = self
            .issues
            .get(issue_id)
            .ok_or_else(|| StoreError::IssueNotFound(issue_id.clone()))?;

        let status = &issue.status;
        let dependencies = issue.dependencies.as_slice();

        match status {
            IssueStatus::Closed => Ok(false),
            IssueStatus::Open => {
                for dependency in dependencies {
                    if dependency.dependency_type == IssueDependencyType::BlockedOn {
                        let blocker_status = match self.issues.get(&dependency.issue_id) {
                            Some(blocker) => &blocker.status,
                            None => {
                                if self.patches.contains_key(&dependency.issue_id) {
                                    return Err(StoreError::InvalidDependency(format!(
                                        "record '{}' is not an issue",
                                        dependency.issue_id
                                    )));
                                }

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
                            if self.patches.contains_key(&child_id) {
                                return Err(StoreError::InvalidDependency(format!(
                                    "record '{child_id}' is not an issue"
                                )));
                            }
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
        parent_edges: Vec<Edge>,
        creation_time: DateTime<Utc>,
    ) -> Result<MetisId, StoreError> {
        // Generate a unique ID for the new task
        let id = Uuid::new_v4().hyphenated().to_string();

        // Verify all parent tasks exist
        for parent_edge in &parent_edges {
            if !self.tasks.contains_key(&parent_edge.id) {
                return Err(StoreError::InvalidDependency(format!(
                    "Parent task not found: {}",
                    parent_edge.id
                )));
            }
        }

        // Determine initial status: blocked if any parent is not complete, otherwise pending
        let initial_status = if parent_edges.is_empty() {
            Status::Pending
        } else {
            // Check if all parents are complete (Complete or Failed)
            let all_complete = parent_edges.iter().all(|parent_edge| {
                self.status_logs
                    .get(&parent_edge.id)
                    .map(|status_log| {
                        matches!(
                            status_log.current_status(),
                            Status::Complete | Status::Failed
                        )
                    })
                    .unwrap_or(false)
            });
            if all_complete {
                Status::Pending
            } else {
                Status::Blocked
            }
        };

        // Add the task
        self.tasks.insert(id.clone(), task);

        // Initialize empty vectors if needed
        self.parents.insert(id.clone(), parent_edges.clone());
        self.children.insert(id.clone(), Vec::new());

        // Initialize status log
        self.status_logs.insert(
            id.clone(),
            TaskStatusLog::new(initial_status, creation_time),
        );

        // Update children of each parent
        for parent_edge in &parent_edges {
            self.children
                .get_mut(&parent_edge.id)
                .expect("Parent should exist")
                .push(id.clone());
        }

        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: MetisId,
        task: Task,
        parent_edges: Vec<Edge>,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Check if task already exists
        if self.tasks.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!(
                "Task already exists: {metis_id}"
            )));
        }

        // Verify all parent tasks exist
        for parent_edge in &parent_edges {
            if !self.tasks.contains_key(&parent_edge.id) {
                return Err(StoreError::InvalidDependency(format!(
                    "Parent task not found: {}",
                    parent_edge.id
                )));
            }
        }

        // Determine initial status: blocked if any parent is not complete, otherwise pending
        let initial_status = if parent_edges.is_empty() {
            Status::Pending
        } else {
            // Check if all parents are complete (Complete or Failed)
            let all_complete = parent_edges.iter().all(|parent_edge| {
                self.status_logs
                    .get(&parent_edge.id)
                    .map(|status_log| {
                        matches!(
                            status_log.current_status(),
                            Status::Complete | Status::Failed
                        )
                    })
                    .unwrap_or(false)
            });
            if all_complete {
                Status::Pending
            } else {
                Status::Blocked
            }
        };

        // Add the task with the specified ID
        self.tasks.insert(metis_id.clone(), task);

        // Initialize empty vectors if needed
        self.parents.insert(metis_id.clone(), parent_edges.clone());
        self.children.insert(metis_id.clone(), Vec::new());

        // Initialize status log
        self.status_logs.insert(
            metis_id.clone(),
            TaskStatusLog::new(initial_status, creation_time),
        );

        // Update children of each parent
        for parent_edge in &parent_edges {
            self.children
                .get_mut(&parent_edge.id)
                .expect("Parent should exist")
                .push(metis_id.clone());
        }

        Ok(())
    }

    async fn update_task(&mut self, metis_id: &MetisId, task: Task) -> Result<(), StoreError> {
        if !self.tasks.contains_key(metis_id) {
            return Err(StoreError::TaskNotFound(metis_id.clone()));
        }

        // Overwrite the existing task without modifying edge structure
        self.tasks.insert(metis_id.clone(), task);
        Ok(())
    }

    async fn get_task(&self, id: &MetisId) -> Result<Task, StoreError> {
        self.tasks
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_parents(&self, id: &MetisId) -> Result<Vec<Edge>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        Ok(self.parents.get(id).cloned().unwrap_or_default())
    }

    async fn get_children(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        Ok(self.children.get(id).cloned().unwrap_or_default())
    }

    async fn remove_task(&mut self, id: &MetisId) -> Result<(), StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Get parent and child IDs before removal
        let parent_edges = self.parents.get(id).cloned().unwrap_or_default();
        let child_ids = self.children.get(id).cloned().unwrap_or_default();

        // Remove the task
        self.tasks.remove(id);
        self.parents.remove(id);
        self.children.remove(id);
        self.status_logs.remove(id);

        // Remove this task from its parents' children lists
        for parent_edge in &parent_edges {
            if let Some(children) = self.children.get_mut(&parent_edge.id) {
                children.retain(|child_id| child_id != id);
            }
        }

        // Remove this task from its children's parent lists
        for child_id in &child_ids {
            if let Some(parents) = self.parents.get_mut(child_id) {
                parents.retain(|edge| edge.id != *id);
            }
        }

        Ok(())
    }

    async fn list_tasks(&self) -> Result<Vec<MetisId>, StoreError> {
        Ok(self.tasks.keys().cloned().collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<MetisId>, StoreError> {
        Ok(self
            .status_logs
            .iter()
            .filter(|(_, status_log)| status_log.current_status() == status)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError> {
        self.status_logs
            .get(id)
            .map(|status_log| status_log.current_status())
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_log(&self, id: &MetisId) -> Result<TaskStatusLog, StoreError> {
        self.status_logs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    fn get_result(&self, id: &MetisId) -> Option<Result<(), TaskError>> {
        self.status_logs.get(id).and_then(TaskStatusLog::result)
    }

    async fn mark_task_running(
        &mut self,
        id: &MetisId,
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
        id: &MetisId,
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
        id: &MetisId,
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

        // Check all children (dependents) and update their status if needed
        let child_ids = self.children.get(id).cloned().unwrap_or_default();
        for child_id in child_ids {
            // If child is blocked, check if all its parents are now complete
            let should_unblock = matches!(
                self.status_logs
                    .get(&child_id)
                    .map(|status_log| status_log.current_status()),
                Some(Status::Blocked)
            ) && self.all_parents_complete(&child_id);

            if should_unblock {
                if let Some(child_log) = self.status_logs.get_mut(&child_id) {
                    child_log.events.push(Event::Created {
                        at: end_time,
                        status: Status::Pending,
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use metis_common::{
        issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType},
        jobs::Bundle,
        patches::Patch,
    };
    use std::collections::HashSet;

    fn spawn_task() -> Task {
        Task::Spawn {
            program: "0".to_string(),
            params: vec![],
            context: Bundle::None,
            image: "metis-worker:latest".to_string(),
            env_vars: HashMap::new(),
        }
    }

    fn sample_patch() -> Patch {
        Patch {
            title: "sample patch".to_string(),
            diff: "diff --git a/file b/file".to_string(),
            description: "sample patch".to_string(),
            is_automatic_backup: false,
            reviews: Vec::new(),
        }
    }

    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            status: IssueStatus::Open,
            assignee: None,
            dependencies,
        }
    }

    fn issue_with_status(status: IssueStatus, dependencies: Vec<IssueDependency>) -> Issue {
        Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            status,
            assignee: None,
            dependencies,
        }
    }

    fn edge(id: &str) -> Edge {
        Edge {
            id: id.to_string(),
            name: None,
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
            is_automatic_backup: false,
            reviews: Vec::new(),
        };

        store.update_patch(&id, updated.clone()).await.unwrap();

        assert_eq!(store.get_patch(&id).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn update_missing_patch_returns_error() {
        let mut store = MemoryStore::new();
        let missing = "missing".to_string();

        let err = store
            .update_patch(
                &missing,
                Patch {
                    title: "noop patch".to_string(),
                    diff: "noop".to_string(),
                    description: "noop patch".to_string(),
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
    async fn closed_issue_is_not_ready() {
        let mut store = MemoryStore::new();

        let issue_id = store
            .add_issue(issue_with_status(IssueStatus::Closed, vec![]))
            .await
            .unwrap();

        assert!(!store.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn add_and_retrieve_tasks_with_dependencies() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store
            .add_task(root_task.clone(), vec![], Utc::now())
            .await
            .unwrap();
        let child_task = spawn_task();
        let child_id = store
            .add_task(child_task.clone(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();

        assert_eq!(store.get_task(&root_id).await.unwrap(), root_task);
        assert_eq!(store.get_task(&child_id).await.unwrap(), child_task);
        assert_eq!(
            store.get_parents(&child_id).await.unwrap(),
            vec![edge(&root_id)]
        );
        assert_eq!(
            store.get_children(&root_id).await.unwrap(),
            vec![child_id.clone()]
        );

        // Check initial statuses
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        let tasks: HashSet<_> = store.list_tasks().await.unwrap().into_iter().collect();
        assert_eq!(tasks, HashSet::from([root_id, child_id]));
    }

    #[tokio::test]
    async fn add_task_with_missing_parent_fails() {
        let mut store = MemoryStore::new();
        let missing_parent = "missing".to_string();

        let task = spawn_task();
        let err = store
            .add_task(task, vec![edge(&missing_parent)], Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidDependency(msg) if msg.contains(&missing_parent)));

        assert!(store.list_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_task_updates_relationships() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();
        let child_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();
        let grandchild_task = Task::Spawn {
            program: "0".to_string(),
            params: vec![],
            context: Bundle::None,
            image: "metis-worker:latest".to_string(),
            env_vars: HashMap::new(),
        };
        let grandchild_id = store
            .add_task(grandchild_task, vec![edge(&child_id)], Utc::now())
            .await
            .unwrap();

        store.remove_task(&child_id).await.unwrap();

        assert!(matches!(
            store.get_task(&child_id).await,
            Err(StoreError::TaskNotFound(id)) if id == child_id
        ));
        assert!(store.get_children(&root_id).await.unwrap().is_empty());
        assert!(store.get_parents(&grandchild_id).await.unwrap().is_empty());

        let tasks: HashSet<_> = store.list_tasks().await.unwrap().into_iter().collect();
        assert_eq!(tasks, HashSet::from([root_id, grandchild_id]));
    }

    #[tokio::test]
    async fn removing_unknown_task_returns_error() {
        let mut store = MemoryStore::new();
        let missing = "does-not-exist".to_string();

        let err = store.remove_task(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::TaskNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn task_without_parents_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn task_with_incomplete_parent_starts_as_blocked() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();
        let child_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();

        // Root is pending, child should be blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);
    }

    #[tokio::test]
    async fn get_parents_returns_edge_names() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

        let child_id = store
            .add_task(
                spawn_task(),
                vec![Edge {
                    id: root_id.clone(),
                    name: Some("root".to_string()),
                }],
                Utc::now(),
            )
            .await
            .unwrap();

        let parents = store.get_parents(&child_id).await.unwrap();
        assert_eq!(
            parents,
            vec![Edge {
                id: root_id,
                name: Some("root".to_string())
            }]
        );
    }

    #[tokio::test]
    async fn task_with_complete_parents_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);

        // Add a child - it should start as pending since parent is complete
        let child_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_running_transitions_from_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);
    }

    #[tokio::test]
    async fn mark_task_complete_transitions_from_running() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

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
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

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
    async fn mark_task_complete_unblocks_dependents() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();
        let child_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();

        // Initially, child is blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap();

        // Child should now be pending
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_complete_with_multiple_dependents() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();
        let child1_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();
        let child2_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();

        // All children should be blocked
        assert_eq!(store.get_status(&child1_id).await.unwrap(), Status::Blocked);
        assert_eq!(store.get_status(&child2_id).await.unwrap(), Status::Blocked);

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap();

        // Both children should now be pending
        assert_eq!(store.get_status(&child1_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child2_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_complete_with_multiple_parents() {
        let mut store = MemoryStore::new();

        let root1_task = spawn_task();
        let root1_id = store
            .add_task(root1_task, vec![], Utc::now())
            .await
            .unwrap();

        let root2_task = spawn_task();
        let root2_id = store
            .add_task(root2_task, vec![], Utc::now())
            .await
            .unwrap();

        // Child depends on both parents
        let child_id = store
            .add_task(
                spawn_task(),
                vec![edge(&root1_id), edge(&root2_id)],
                Utc::now(),
            )
            .await
            .unwrap();

        // Child should be blocked since both parents are pending
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete first parent - child should still be blocked
        store
            .mark_task_running(&root1_id, Utc::now())
            .await
            .unwrap();
        store
            .mark_task_complete(&root1_id, Ok(()), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete second parent - child should now be pending
        store
            .mark_task_running(&root2_id, Utc::now())
            .await
            .unwrap();
        store
            .mark_task_complete(&root2_id, Ok(()), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_running_from_blocked_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();
        let child_id = store
            .add_task(spawn_task(), vec![edge(&root_id)], Utc::now())
            .await
            .unwrap();

        // Child is blocked, trying to mark it as running should fail
        let err = store
            .mark_task_running(&child_id, Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn mark_task_complete_from_pending_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

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
        let root_id = store.add_task(root_task, vec![], Utc::now()).await.unwrap();

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
