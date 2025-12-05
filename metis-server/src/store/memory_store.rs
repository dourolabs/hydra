use async_trait::async_trait;
use std::collections::HashMap;
use uuid::Uuid;

use crate::job_engine::MetisId;
use super::{Store, StoreError, Task};

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
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            parents: HashMap::new(),
            children: HashMap::new(),
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

        // Add the task
        self.tasks.insert(id.clone(), task);

        // Initialize empty vectors if needed
        self.parents.insert(id.clone(), parent_ids.clone());
        self.children.insert(id.clone(), Vec::new());

        // Update children of each parent
        for parent_id in &parent_ids {
            self.children
                .get_mut(parent_id)
                .expect("Parent should exist")
                .push(id.clone());
        }

        Ok(id)
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
}
