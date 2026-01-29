use crate::{
    domain::{
        actors::Actor,
        issues::{Issue, IssueGraphFilter},
        patches::Patch,
        users::{User, Username},
    },
    store::{Status, Store, StoreError, Task, TaskStatusLog},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, PatchId, RepoName, TaskId, Versioned, repositories::Repository};
use std::collections::HashSet;

/// Store implementation that always fails; useful for exercising error paths in tests.
#[derive(Default)]
pub struct FailingStore;

fn fail<T>() -> Result<T, StoreError> {
    Err(StoreError::Internal("forced failure".to_string()))
}

#[async_trait]
impl Store for FailingStore {
    async fn add_repository(&self, _name: RepoName, _config: Repository) -> Result<(), StoreError> {
        fail()
    }

    async fn get_repository(&self, _name: &RepoName) -> Result<Versioned<Repository>, StoreError> {
        fail()
    }

    async fn update_repository(
        &self,
        _name: RepoName,
        _config: Repository,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn list_repositories(
        &self,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        fail()
    }

    async fn add_issue(&self, _issue: Issue) -> Result<IssueId, StoreError> {
        fail()
    }

    async fn get_issue(&self, _id: &IssueId) -> Result<Versioned<Issue>, StoreError> {
        fail()
    }

    async fn get_issue_versions(&self, _id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        fail()
    }

    async fn update_issue(&self, _id: &IssueId, _issue: Issue) -> Result<(), StoreError> {
        fail()
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        fail()
    }

    async fn search_issue_graph(
        &self,
        _filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        fail()
    }

    async fn add_patch(&self, _patch: Patch) -> Result<PatchId, StoreError> {
        fail()
    }

    async fn get_patch(&self, _id: &PatchId) -> Result<Versioned<Patch>, StoreError> {
        fail()
    }

    async fn get_patch_versions(&self, _id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        fail()
    }

    async fn update_patch(&self, _id: &PatchId, _patch: Patch) -> Result<(), StoreError> {
        fail()
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        fail()
    }

    async fn get_issues_for_patch(&self, _patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_issue_children(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_issue_blocked_on(&self, _issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        fail()
    }

    async fn get_tasks_for_issue(&self, _issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        fail()
    }

    async fn add_task(
        &self,
        _task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        fail()
    }

    async fn add_task_with_id(
        &self,
        _metis_id: TaskId,
        _task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn update_task(
        &self,
        _metis_id: &TaskId,
        _task: Task,
    ) -> Result<Versioned<Task>, StoreError> {
        fail()
    }

    async fn get_task(&self, _id: &TaskId) -> Result<Versioned<Task>, StoreError> {
        fail()
    }

    async fn get_task_versions(&self, _id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        fail()
    }

    async fn list_tasks(&self) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        fail()
    }

    async fn list_tasks_with_status(&self, _status: Status) -> Result<Vec<TaskId>, StoreError> {
        fail()
    }

    async fn get_status_log(&self, _id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        fail()
    }

    async fn add_actor(&self, _actor: Actor) -> Result<(), StoreError> {
        fail()
    }

    async fn update_actor(&self, _actor: Actor) -> Result<(), StoreError> {
        fail()
    }

    async fn get_actor(&self, _name: &str) -> Result<Versioned<Actor>, StoreError> {
        crate::store::validate_actor_name(_name)?;
        fail()
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        fail()
    }

    async fn add_user(&self, _user: User) -> Result<(), StoreError> {
        fail()
    }

    async fn update_user(&self, _user: User) -> Result<Versioned<User>, StoreError> {
        fail()
    }

    async fn get_user(&self, _username: &Username) -> Result<Versioned<User>, StoreError> {
        fail()
    }
}
