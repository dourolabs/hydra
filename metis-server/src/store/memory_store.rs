use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use super::{Edge, Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use metis_common::MetisId;
use metis_common::artifacts::{Artifact, IssueDependency};
use metis_common::task_status::Event;

/// An in-memory implementation of the Store trait.
///
/// This store maintains a DAG of tasks using HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<MetisId, Task>,
    /// Maps artifact IDs to their Artifact data
    artifacts: HashMap<MetisId, Artifact>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<MetisId, TaskStatusLog>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            artifacts: HashMap::new(),
            status_logs: HashMap::new(),
        }
    }

    /// Checks if all parents of a task are complete (Complete or Failed).
    fn all_parents_complete(&self, dependencies: &[IssueDependency]) -> bool {
        dependencies.iter().all(|edge| {
            self.status_logs
                .get(&edge.issue_id)
                .map(|status_log| {
                    matches!(
                        status_log.current_status(),
                        Status::Complete | Status::Failed
                    )
                })
                .unwrap_or(false)
        })
    }

    fn children_of(&self, id: &MetisId) -> Vec<MetisId> {
        self.tasks
            .iter()
            .filter(|(_, task)| {
                task.dependencies()
                    .iter()
                    .any(|dependency| &dependency.issue_id == id)
            })
            .map(|(task_id, _)| task_id.clone())
            .collect()
    }

    fn edges_from_dependencies(dependencies: &[IssueDependency]) -> Vec<Edge> {
        dependencies
            .iter()
            .map(|dependency| Edge {
                id: dependency.issue_id.clone(),
                dependency_type: dependency.dependency_type,
            })
            .collect()
    }

    fn initial_status(&self, dependencies: &[IssueDependency]) -> Status {
        if dependencies.is_empty() || self.all_parents_complete(dependencies) {
            Status::Pending
        } else {
            Status::Blocked
        }
    }

    fn validate_dependencies(&self, dependencies: &[IssueDependency]) -> Result<(), StoreError> {
        for dependency in dependencies {
            if !self.tasks.contains_key(&dependency.issue_id) {
                return Err(StoreError::InvalidDependency(format!(
                    "Parent task not found: {}",
                    dependency.issue_id
                )));
            }
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
    async fn add_artifact(&mut self, artifact: Artifact) -> Result<MetisId, StoreError> {
        let id = Uuid::new_v4().hyphenated().to_string();
        self.artifacts.insert(id.clone(), artifact);
        self.status_logs
            .entry(id.clone())
            .or_insert_with(|| TaskStatusLog::new(Status::Pending, Utc::now()));
        Ok(id)
    }

    async fn get_artifact(&self, id: &MetisId) -> Result<Artifact, StoreError> {
        self.artifacts
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::ArtifactNotFound(id.clone()))
    }

    async fn update_artifact(
        &mut self,
        id: &MetisId,
        artifact: Artifact,
    ) -> Result<(), StoreError> {
        if !self.artifacts.contains_key(id) {
            return Err(StoreError::ArtifactNotFound(id.clone()));
        }

        self.artifacts.insert(id.clone(), artifact);
        self.status_logs
            .entry(id.clone())
            .or_insert_with(|| TaskStatusLog::new(Status::Pending, Utc::now()));
        Ok(())
    }

    async fn list_artifacts(&self) -> Result<Vec<(MetisId, Artifact)>, StoreError> {
        Ok(self
            .artifacts
            .iter()
            .map(|(id, artifact)| (id.clone(), artifact.clone()))
            .collect())
    }

    async fn add_task(
        &mut self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<MetisId, StoreError> {
        let id = Uuid::new_v4().hyphenated().to_string();

        self.validate_dependencies(task.dependencies())?;
        let initial_status = self.initial_status(task.dependencies());

        let session_artifact = task.to_session_artifact();
        self.tasks.insert(id.clone(), task);
        self.artifacts.insert(id.clone(), session_artifact);
        self.status_logs.insert(
            id.clone(),
            TaskStatusLog::new(initial_status, creation_time),
        );

        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: MetisId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        if self.tasks.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!(
                "Task already exists: {metis_id}"
            )));
        }

        self.validate_dependencies(task.dependencies())?;
        let initial_status = self.initial_status(task.dependencies());

        let session_artifact = task.to_session_artifact();
        self.tasks.insert(metis_id.clone(), task);
        self.artifacts.insert(metis_id.clone(), session_artifact);
        self.status_logs.insert(
            metis_id.clone(),
            TaskStatusLog::new(initial_status, creation_time),
        );

        Ok(())
    }

    async fn update_task(&mut self, metis_id: &MetisId, task: Task) -> Result<(), StoreError> {
        if !self.tasks.contains_key(metis_id) {
            return Err(StoreError::TaskNotFound(metis_id.clone()));
        }

        self.validate_dependencies(task.dependencies())?;

        let session_artifact = task.to_session_artifact();

        self.tasks.insert(metis_id.clone(), task);
        self.artifacts.insert(metis_id.clone(), session_artifact);
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

        let task = self
            .tasks
            .get(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;
        Ok(Self::edges_from_dependencies(task.dependencies()))
    }

    async fn get_children(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        Ok(self.children_of(id))
    }

    async fn remove_task(&mut self, id: &MetisId) -> Result<(), StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        self.tasks.remove(id);
        self.artifacts.remove(id);
        self.status_logs.remove(id);

        for (task_id, task) in self.tasks.iter_mut() {
            let dependencies = task.dependencies_mut();
            let initial_len = dependencies.len();
            dependencies.retain(|dependency| &dependency.issue_id != id);
            if dependencies.len() != initial_len {
                self.artifacts
                    .insert(task_id.clone(), task.to_session_artifact());
            }
        }

        Ok(())
    }

    async fn list_tasks(&self) -> Result<Vec<MetisId>, StoreError> {
        Ok(self.tasks.keys().cloned().collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<MetisId>, StoreError> {
        Ok(self
            .tasks
            .keys()
            .filter_map(|id| {
                self.status_logs
                    .get(id)
                    .filter(|status_log| status_log.current_status() == status)
                    .map(|_| id.clone())
            })
            .collect())
    }

    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError> {
        if !self.tasks.contains_key(id) && !self.artifacts.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        self.status_logs
            .get(id)
            .map(|status_log| status_log.current_status())
            .ok_or_else(|| {
                if self.artifacts.contains_key(id) {
                    StoreError::ArtifactNotFound(id.clone())
                } else {
                    StoreError::TaskNotFound(id.clone())
                }
            })
    }

    async fn get_status_log(&self, id: &MetisId) -> Result<TaskStatusLog, StoreError> {
        if !self.tasks.contains_key(id) && !self.artifacts.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        self.status_logs.get(id).cloned().ok_or_else(|| {
            if self.artifacts.contains_key(id) {
                StoreError::ArtifactNotFound(id.clone())
            } else {
                StoreError::TaskNotFound(id.clone())
            }
        })
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
            Ok(()) => Event::Completed { at: end_time },
            Err(error) => Event::Failed {
                at: end_time,
                error,
            },
        };

        status_log.events.push(event);

        // Check all children (dependents) and update their status if needed
        let child_ids = self.children_of(id);
        for child_id in child_ids {
            // If child is blocked, check if all its parents are now complete
            let should_unblock = matches!(
                self.status_logs
                    .get(&child_id)
                    .map(|status_log| status_log.current_status()),
                Some(Status::Blocked)
            ) && self
                .tasks
                .get(&child_id)
                .map(|task| self.all_parents_complete(task.dependencies()))
                .unwrap_or(false);

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
        artifacts::{Artifact, IssueDependency, IssueDependencyType, IssueStatus, IssueType},
        jobs::Bundle,
    };
    use std::collections::{HashMap, HashSet};

    fn spawn_task() -> Task {
        Task::Spawn {
            program: "0".to_string(),
            params: vec![],
            context: Bundle::None,
            image: "metis-worker:latest".to_string(),
            env_vars: HashMap::new(),
            dependencies: Vec::new(),
        }
    }

    fn sample_artifact() -> Artifact {
        Artifact::Patch {
            diff: "diff --git a/file b/file".to_string(),
            description: "sample patch".to_string(),
        }
    }

    fn edge(id: &str) -> Edge {
        Edge {
            id: id.to_string(),
            dependency_type: IssueDependencyType::BlockedOn,
        }
    }

    fn with_dependencies(mut task: Task, edges: Vec<Edge>) -> Task {
        let dependencies = edges.iter().map(IssueDependency::from).collect();
        *task.dependencies_mut() = dependencies;
        task
    }

    #[tokio::test]
    async fn add_and_get_artifact_assigns_id() {
        let mut store = MemoryStore::new();

        let artifact = sample_artifact();
        let id = store.add_artifact(artifact.clone()).await.unwrap();

        assert_eq!(store.get_artifact(&id).await.unwrap(), artifact);
    }

    #[tokio::test]
    async fn update_artifact_overwrites_existing_value() {
        let mut store = MemoryStore::new();

        let id = store.add_artifact(sample_artifact()).await.unwrap();
        let updated = Artifact::Issue {
            issue_type: IssueType::Task,
            description: "issue details".to_string(),
            status: IssueStatus::Open,
            dependencies: vec![],
        };

        store.update_artifact(&id, updated.clone()).await.unwrap();

        assert_eq!(store.get_artifact(&id).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn update_missing_artifact_returns_error() {
        let mut store = MemoryStore::new();
        let missing = "missing".to_string();

        let err = store
            .update_artifact(
                &missing,
                Artifact::Patch {
                    diff: "noop".to_string(),
                    description: "noop patch".to_string(),
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::ArtifactNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn add_and_retrieve_tasks_with_dependencies() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task.clone(), Utc::now()).await.unwrap();
        let child_task = with_dependencies(spawn_task(), vec![edge(&root_id)]);
        let child_id = store
            .add_task(child_task.clone(), Utc::now())
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
    async fn add_task_creates_session_artifact() {
        let mut store = MemoryStore::new();

        let parent_id = store.add_task(spawn_task(), Utc::now()).await.unwrap();
        let mut task = spawn_task();
        task.dependencies_mut().push(IssueDependency {
            dependency_type: IssueDependencyType::BlockedOn,
            issue_id: parent_id.clone(),
        });

        let task_id = store.add_task(task.clone(), Utc::now()).await.unwrap();

        match store.get_artifact(&task_id).await.unwrap() {
            Artifact::Session {
                program,
                dependencies,
                ..
            } => {
                assert_eq!(program, "0");
                assert_eq!(dependencies, task.dependencies().to_vec());
            }
            other => panic!("expected session artifact, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn add_artifact_records_status_log() {
        let mut store = MemoryStore::new();
        let artifact_id = store.add_artifact(sample_artifact()).await.unwrap();

        let status_log = store.get_status_log(&artifact_id).await.unwrap();
        assert_eq!(status_log.current_status(), Status::Pending);
    }

    #[tokio::test]
    async fn add_task_with_missing_parent_fails() {
        let mut store = MemoryStore::new();
        let missing_parent = "missing".to_string();

        let task = spawn_task();
        let err = store
            .add_task(
                with_dependencies(task, vec![edge(&missing_parent)]),
                Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidDependency(msg) if msg.contains(&missing_parent)));

        assert!(store.list_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_task_updates_relationships() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();
        let grandchild_task = with_dependencies(spawn_task(), vec![edge(&child_id)]);
        let grandchild_id = store.add_task(grandchild_task, Utc::now()).await.unwrap();

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
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn task_with_incomplete_parent_starts_as_blocked() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();

        // Root is pending, child should be blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);
    }

    #[tokio::test]
    async fn get_parents_returns_dependencies() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        let child_dependency = Edge {
            id: root_id.clone(),
            dependency_type: IssueDependencyType::ChildOf,
        };
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![child_dependency.clone()]),
                Utc::now(),
            )
            .await
            .unwrap();

        let parents = store.get_parents(&child_id).await.unwrap();
        assert_eq!(parents, vec![child_dependency]);
    }

    #[tokio::test]
    async fn task_with_complete_parents_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);

        // Add a child - it should start as pending since parent is complete
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
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
            .mark_task_complete(&root_id, Ok(()), Utc::now())
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
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();

        // Initially, child is blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), Utc::now())
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
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();
        let child1_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();
        let child2_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
            .await
            .unwrap();

        // All children should be blocked
        assert_eq!(store.get_status(&child1_id).await.unwrap(), Status::Blocked);
        assert_eq!(store.get_status(&child2_id).await.unwrap(), Status::Blocked);

        // Complete the root task (first mark as running, then complete)
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        store
            .mark_task_complete(&root_id, Ok(()), Utc::now())
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
        let root1_id = store.add_task(root1_task, Utc::now()).await.unwrap();

        let root2_task = spawn_task();
        let root2_id = store.add_task(root2_task, Utc::now()).await.unwrap();

        // Child depends on both parents
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root1_id), edge(&root2_id)]),
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
            .mark_task_complete(&root1_id, Ok(()), Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete second parent - child should now be pending
        store
            .mark_task_running(&root2_id, Utc::now())
            .await
            .unwrap();
        store
            .mark_task_complete(&root2_id, Ok(()), Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_running_from_blocked_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();
        let child_id = store
            .add_task(
                with_dependencies(spawn_task(), vec![edge(&root_id)]),
                Utc::now(),
            )
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
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as complete from pending should fail
        let err = store
            .mark_task_complete(&root_id, Ok(()), Utc::now())
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
                Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }
}
