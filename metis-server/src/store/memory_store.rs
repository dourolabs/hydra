use async_trait::async_trait;
use std::collections::HashMap;
use uuid::Uuid;

use crate::job_engine::MetisId;
use super::{Store, StoreError, Status, Task};

/// An in-memory implementation of the Store trait.
///
/// This store maintains a DAG of tasks using HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<MetisId, Task>,
    /// Maps task IDs to their parent task IDs (dependencies)
    parents: HashMap<MetisId, Vec<MetisId>>,
    /// Maps task IDs to their child task IDs (dependents)
    children: HashMap<MetisId, Vec<MetisId>>,
    /// Maps task IDs to their Status
    statuses: HashMap<MetisId, Status>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            parents: HashMap::new(),
            children: HashMap::new(),
            statuses: HashMap::new(),
        }
    }

    /// Checks if all parents of a task are complete (Complete or Failed).
    fn all_parents_complete(&self, id: &MetisId) -> bool {
        let parent_ids = self.parents.get(id).cloned().unwrap_or_default();
        parent_ids.iter().all(|parent_id| {
            self.statuses
                .get(parent_id)
                .map(|status| matches!(status, Status::Complete | Status::Failed))
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
    async fn add_task(&mut self, task: Task, parent_ids: Vec<MetisId>) -> Result<MetisId, StoreError> {
        // Generate a unique ID for the new task
        let id = Uuid::new_v4().hyphenated().to_string();

        // Verify all parent tasks exist
        for parent_id in &parent_ids {
            if !self.tasks.contains_key(parent_id) {
                return Err(StoreError::InvalidDependency(
                    format!("Parent task not found: {}", parent_id)
                ));
            }
        }

        // Determine initial status: blocked if any parent is not complete, otherwise pending
        let initial_status = if parent_ids.is_empty() {
            Status::Pending
        } else {
            // Check if all parents are complete (Complete or Failed)
            let all_complete = parent_ids.iter().all(|parent_id| {
                self.statuses
                    .get(parent_id)
                    .map(|status| matches!(status, Status::Complete | Status::Failed))
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
        self.parents.insert(id.clone(), parent_ids.clone());
        self.children.insert(id.clone(), Vec::new());
        self.statuses.insert(id.clone(), initial_status);

        // Update children of each parent
        for parent_id in &parent_ids {
            self.children
                .get_mut(parent_id)
                .expect("Parent should exist")
                .push(id.clone());
        }

        Ok(id)
    }

    async fn add_task_with_id(&mut self, metis_id: MetisId, task: Task, parent_ids: Vec<MetisId>) -> Result<(), StoreError> {
        // Check if task already exists
        if self.tasks.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!("Task already exists: {}", metis_id)));
        }

        // Verify all parent tasks exist
        for parent_id in &parent_ids {
            if !self.tasks.contains_key(parent_id) {
                return Err(StoreError::InvalidDependency(
                    format!("Parent task not found: {}", parent_id)
                ));
            }
        }

        // Determine initial status: blocked if any parent is not complete, otherwise pending
        let initial_status = if parent_ids.is_empty() {
            Status::Pending
        } else {
            // Check if all parents are complete (Complete or Failed)
            let all_complete = parent_ids.iter().all(|parent_id| {
                self.statuses
                    .get(parent_id)
                    .map(|status| matches!(status, Status::Complete | Status::Failed))
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
        self.parents.insert(metis_id.clone(), parent_ids.clone());
        self.children.insert(metis_id.clone(), Vec::new());
        self.statuses.insert(metis_id.clone(), initial_status);

        // Update children of each parent
        for parent_id in &parent_ids {
            self.children
                .get_mut(parent_id)
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

    async fn get_parents(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        Ok(self.parents
            .get(id)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_children(&self, id: &MetisId) -> Result<Vec<MetisId>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        Ok(self.children
            .get(id)
            .cloned()
            .unwrap_or_default())
    }

    async fn remove_task(&mut self, id: &MetisId) -> Result<(), StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Get parent and child IDs before removal
        let parent_ids = self.parents.get(id).cloned().unwrap_or_default();
        let child_ids = self.children.get(id).cloned().unwrap_or_default();

        // Remove the task
        self.tasks.remove(id);
        self.parents.remove(id);
        self.children.remove(id);
        self.statuses.remove(id);

        // Remove this task from its parents' children lists
        for parent_id in &parent_ids {
            if let Some(children) = self.children.get_mut(parent_id) {
                children.retain(|child_id| child_id != id);
            }
        }

        // Remove this task from its children's parent lists
        for child_id in &child_ids {
            if let Some(parents) = self.parents.get_mut(child_id) {
                parents.retain(|parent_id| parent_id != id);
            }
        }

        Ok(())
    }

    async fn list_tasks(&self) -> Result<Vec<MetisId>, StoreError> {
        Ok(self.tasks.keys().cloned().collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<MetisId>, StoreError> {
        Ok(self.statuses
            .iter()
            .filter(|(_, s)| **s == status)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn get_status(&self, id: &MetisId) -> Result<Status, StoreError> {
        self.statuses
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn update_task_status(&mut self, id: &MetisId, new_status: Status) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Verify new_status is Running, Complete, or Failed
        if !matches!(new_status, Status::Running | Status::Complete | Status::Failed) {
            return Err(StoreError::Internal(
                "update_task_status can only set status to Running, Complete, or Failed".to_string()
            ));
        }

        // Verify current status is Pending or Running
        let current_status = self.statuses
            .get(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;
        
        // Allow transitions from Pending to Running/Complete/Failed
        // Allow transitions from Running to Complete/Failed
        let valid_transition = match (current_status, &new_status) {
            (Status::Pending, Status::Running | Status::Complete | Status::Failed) => true,
            (Status::Running, Status::Complete | Status::Failed) => true,
            _ => false,
        };
        
        if !valid_transition {
            return Err(StoreError::InvalidStatusTransition);
        }

        // Check if we need to update children before inserting the new status
        let should_update_children = matches!(new_status, Status::Complete | Status::Failed);

        // Update the status
        self.statuses.insert(id.clone(), new_status);

        // If transitioning to Complete or Failed, check all children (dependents) and update their status if needed
        if should_update_children {
            let child_ids = self.children.get(id).cloned().unwrap_or_default();
            for child_id in child_ids {
                // If child is blocked, check if all its parents are now complete
                if let Some(child_status) = self.statuses.get(&child_id) {
                    if matches!(child_status, Status::Blocked) {
                        if self.all_parents_complete(&child_id) {
                            self.statuses.insert(child_id, Status::Pending);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use metis_common::jobs::CreateJobRequestContext;

    #[tokio::test]
    async fn add_and_retrieve_tasks_with_dependencies() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task.clone(), vec![]).await.unwrap();
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();

        assert_eq!(store.get_task(&root_id).await.unwrap(), root_task);
        assert_eq!(store.get_task(&child_id).await.unwrap(), Task::Ask);
        assert_eq!(store.get_parents(&child_id).await.unwrap(), vec![root_id.clone()]);
        assert_eq!(store.get_children(&root_id).await.unwrap(), vec![child_id.clone()]);

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

        let spawn_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let err = store
            .add_task(spawn_task, vec![missing_parent.clone()])
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidDependency(msg) if msg.contains(&missing_parent)));

        assert!(store.list_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn remove_task_updates_relationships() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();
        let grandchild_task = Task::Spawn {
            prompt: "test2".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let grandchild_id = store.add_task(grandchild_task, vec![child_id.clone()]).await.unwrap();

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

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn task_with_incomplete_parent_starts_as_blocked() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();

        // Root is pending, child should be blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);
    }

    #[tokio::test]
    async fn task_with_complete_parents_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();

        // Complete the root task
        store.update_task_status(&root_id, Status::Complete).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);

        // Add a child - it should start as pending since parent is complete
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn update_task_status_completes_pending_task() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        store.update_task_status(&root_id, Status::Complete).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);
    }

    #[tokio::test]
    async fn update_task_status_fails_pending_task() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        store.update_task_status(&root_id, Status::Failed).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Failed);
    }

    #[tokio::test]
    async fn update_task_status_unblocks_dependents() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();

        // Initially, child is blocked
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete the root task
        store.update_task_status(&root_id, Status::Complete).await.unwrap();

        // Child should now be pending
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn update_task_status_with_multiple_dependents() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();
        let child1_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();
        let child2_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();

        // All children should be blocked
        assert_eq!(store.get_status(&child1_id).await.unwrap(), Status::Blocked);
        assert_eq!(store.get_status(&child2_id).await.unwrap(), Status::Blocked);

        // Complete the root task
        store.update_task_status(&root_id, Status::Complete).await.unwrap();

        // Both children should now be pending
        assert_eq!(store.get_status(&child1_id).await.unwrap(), Status::Pending);
        assert_eq!(store.get_status(&child2_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn update_task_status_with_multiple_parents() {
        let mut store = MemoryStore::new();

        let root1_task = Task::Spawn {
            prompt: "test1".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root1_id = store.add_task(root1_task, vec![]).await.unwrap();

        let root2_task = Task::Spawn {
            prompt: "test2".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root2_id = store.add_task(root2_task, vec![]).await.unwrap();

        // Child depends on both parents
        let child_id = store.add_task(Task::Ask, vec![root1_id.clone(), root2_id.clone()]).await.unwrap();

        // Child should be blocked since both parents are pending
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete first parent - child should still be blocked
        store.update_task_status(&root1_id, Status::Complete).await.unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Blocked);

        // Complete second parent - child should now be pending
        store.update_task_status(&root2_id, Status::Complete).await.unwrap();
        assert_eq!(store.get_status(&child_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn update_task_status_from_blocked_fails() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();
        let child_id = store.add_task(Task::Ask, vec![root_id.clone()]).await.unwrap();

        // Child is blocked, trying to update it should fail
        let err = store.update_task_status(&child_id, Status::Complete).await.unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn update_task_status_to_invalid_status_fails() {
        let mut store = MemoryStore::new();

        let root_task = Task::Spawn {
            prompt: "test".to_string(),
            context: CreateJobRequestContext::None,
            result: None,
        };
        let root_id = store.add_task(root_task, vec![]).await.unwrap();

        // Trying to set status to Blocked or Pending should fail
        let err = store.update_task_status(&root_id, Status::Blocked).await.unwrap_err();
        assert!(matches!(err, StoreError::Internal(_)));

        let err = store.update_task_status(&root_id, Status::Pending).await.unwrap_err();
        assert!(matches!(err, StoreError::Internal(_)));

        let err = store.update_task_status(&root_id, Status::Running).await.unwrap_err();
        assert!(matches!(err, StoreError::Internal(_)));
    }
}
