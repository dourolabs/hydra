use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use super::{Edge, Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use crate::job_engine::MetisId;
use metis_common::job_outputs::JobOutputPayload;

/// An in-memory implementation of the Store trait.
///
/// This store maintains a DAG of tasks using HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<MetisId, Task>,
    /// Maps task IDs to their parent task edges (dependencies)
    parents: HashMap<MetisId, Vec<Edge>>,
    /// Maps task IDs to their child task IDs (dependents)
    children: HashMap<MetisId, Vec<MetisId>>,
    /// Maps task IDs to their execution results
    results: HashMap<MetisId, Result<JobOutputPayload, TaskError>>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<MetisId, TaskStatusLog>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            parents: HashMap::new(),
            children: HashMap::new(),
            results: HashMap::new(),
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
                    matches!(status_log.current_status, Status::Complete | Status::Failed)
                })
                .unwrap_or(false)
        })
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Store for MemoryStore {
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
                        matches!(status_log.current_status, Status::Complete | Status::Failed)
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
            TaskStatusLog {
                creation_time,
                start_time: None,
                end_time: None,
                current_status: initial_status,
            },
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
                        matches!(status_log.current_status, Status::Complete | Status::Failed)
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
            TaskStatusLog {
                creation_time,
                start_time: None,
                end_time: None,
                current_status: initial_status,
            },
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

    async fn get_args(&self, id: &MetisId) -> Result<Vec<JobOutputPayload>, StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Get all parent IDs
        let parent_edges = self.parents.get(id).cloned().unwrap_or_default();

        // Collect results from all parents
        let mut payloads = Vec::new();
        for parent_edge in &parent_edges {
            match self.get_result(&parent_edge.id) {
                Some(Ok(payload)) => payloads.push(payload),
                Some(Err(e)) => {
                    return Err(StoreError::Internal(format!(
                        "Parent task {} has error result: {e:?}",
                        parent_edge.id
                    )));
                }
                None => {
                    return Err(StoreError::Internal(format!(
                        "Parent task {} has no result yet",
                        parent_edge.id
                    )));
                }
            }
        }

        Ok(payloads)
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
            .filter(|(_, status_log)| status_log.current_status == status)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError> {
        self.status_logs
            .get(id)
            .map(|status_log| status_log.current_status)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_log(&self, id: &MetisId) -> Result<TaskStatusLog, StoreError> {
        self.status_logs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    fn get_result(&self, id: &MetisId) -> Option<Result<JobOutputPayload, TaskError>> {
        self.results.get(id).cloned()
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

        if !matches!(status_log.current_status, Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        // Update status log
        status_log.current_status = Status::Running;
        status_log.start_time = Some(start_time);

        Ok(())
    }

    async fn mark_task_complete(
        &mut self,
        id: &MetisId,
        result: Result<JobOutputPayload, TaskError>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Verify current status is Running
        let status = match result {
            Ok(_) => Status::Complete,
            Err(_) => Status::Failed,
        };

        // Store the result
        self.results.insert(id.clone(), result.clone());

        {
            let status_log = self
                .status_logs
                .get_mut(id)
                .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

            if !matches!(status_log.current_status, Status::Running) {
                return Err(StoreError::InvalidStatusTransition);
            }

            // Update status log
            status_log.current_status = status;
            status_log.end_time = Some(end_time);
        }

        // Check all children (dependents) and update their status if needed
        let child_ids = self.children.get(id).cloned().unwrap_or_default();
        for child_id in child_ids {
            // If child is blocked, check if all its parents are now complete
            let should_unblock = matches!(
                self.status_logs
                    .get(&child_id)
                    .map(|status_log| status_log.current_status),
                Some(Status::Blocked)
            ) && self.all_parents_complete(&child_id);

            if should_unblock {
                if let Some(child_log) = self.status_logs.get_mut(&child_id) {
                    child_log.current_status = Status::Pending;
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
    use metis_common::jobs::Bundle;
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

    // Helper function for tests to create a dummy payload
    fn dummy_payload() -> JobOutputPayload {
        JobOutputPayload {
            last_message: String::new(),
            patch: String::new(),
            bundle: Bundle::None,
        }
    }

    fn edge(id: &str) -> Edge {
        Edge {
            id: id.to_string(),
            name: None,
        }
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
            .mark_task_complete(&root_id, Ok(dummy_payload()), Utc::now())
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
            .mark_task_complete(&root_id, Ok(dummy_payload()), Utc::now())
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
            .mark_task_complete(&root_id, Ok(dummy_payload()), Utc::now())
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
            .mark_task_complete(&root_id, Ok(dummy_payload()), Utc::now())
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
            .mark_task_complete(&root1_id, Ok(dummy_payload()), Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete second parent - child should now be pending
        store
            .mark_task_running(&root2_id, Utc::now())
            .await
            .unwrap();
        store
            .mark_task_complete(&root2_id, Ok(dummy_payload()), Utc::now())
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
            .mark_task_complete(&root_id, Ok(dummy_payload()), Utc::now())
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
                Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }
}
