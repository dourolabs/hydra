use super::AppState;
use crate::{
    background::AgentQueue,
    domain::{actors::Actor, issues::Issue, patches::Patch, users::User},
    store::{Store, StoreError, Task, TaskError},
};
use chrono::{DateTime, Utc};
use metis_common::{IssueId, PatchId, TaskId};
use std::sync::Arc;
use tokio::sync::RwLock;

impl AppState {
    pub async fn add_patch(&self, patch: Patch) -> Result<PatchId, StoreError> {
        let store = self.store.as_ref();
        store.add_patch(patch).await
    }

    pub fn set_store_for_tests(&mut self, store: Box<dyn Store>) {
        self.store = Arc::from(store);
    }

    pub fn set_agents_for_tests(&mut self, agents: Vec<Arc<AgentQueue>>) {
        self.agents = Arc::new(RwLock::new(agents));
    }

    pub async fn add_issue(&self, issue: Issue) -> Result<IssueId, StoreError> {
        let store = self.store.as_ref();
        store.add_issue(issue).await
    }

    pub async fn add_user(&self, user: User) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store.add_user(user).await
    }

    pub async fn add_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store.add_actor(actor).await
    }

    pub async fn list_actors(&self) -> Result<Vec<(String, Actor)>, StoreError> {
        let store = self.store.as_ref();
        store.list_actors().await
    }

    pub async fn update_issue(&self, issue_id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store.update_issue(issue_id, issue).await
    }

    pub async fn add_task_with_id(
        &self,
        task_id: TaskId,
        task: Task,
        created_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store.add_task_with_id(task_id, task, created_at).await
    }

    pub async fn mark_task_running(
        &self,
        task_id: &TaskId,
        started_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store.mark_task_running(task_id, started_at).await
    }

    pub async fn mark_task_complete(
        &self,
        task_id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        completed_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let store = self.store.as_ref();
        store
            .mark_task_complete(task_id, result, last_message, completed_at)
            .await
    }
}
