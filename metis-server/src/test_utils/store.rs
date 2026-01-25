use crate::{
    domain::{
        actors::Actor,
        issues::{Issue, IssueGraphFilter},
        patches::Patch,
        users::{User, Username},
    },
    store::{Status, Store, StoreError, Task, TaskError, TaskStatusLog},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, PatchId, RepoName, TaskId, repositories::Repository};
use std::collections::HashSet;

/// Store implementation that always fails; useful for exercising error paths in tests.
#[derive(Default)]
pub struct FailingStore;

fn fail<T>() -> Result<T, StoreError> {
    Err(StoreError::Internal("forced failure".to_string()))
}

#[async_trait]
impl Store for FailingStore {
    async fn add_repository(
        &mut self,
        _name: RepoName,
        _config: Repository,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn get_repository(&self, _name: &RepoName) -> Result<Repository, StoreError> {
        fail()
    }

    async fn update_repository(
        &mut self,
        _name: RepoName,
        _config: Repository,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn list_repositories(&self) -> Result<Vec<(RepoName, Repository)>, StoreError> {
        fail()
    }

    async fn add_issue(&mut self, _issue: Issue) -> Result<IssueId, StoreError> {
        fail()
    }

    async fn get_issue(&self, _id: &IssueId) -> Result<Issue, StoreError> {
        fail()
    }

    async fn update_issue(&mut self, _id: &IssueId, _issue: Issue) -> Result<(), StoreError> {
        fail()
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError> {
        fail()
    }

    async fn search_issue_graph(
        &self,
        _filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        fail()
    }

    async fn add_patch(&mut self, _patch: Patch) -> Result<PatchId, StoreError> {
        fail()
    }

    async fn get_patch(&self, _id: &PatchId) -> Result<Patch, StoreError> {
        fail()
    }

    async fn update_patch(&mut self, _id: &PatchId, _patch: Patch) -> Result<(), StoreError> {
        fail()
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError> {
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
        &mut self,
        _task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        fail()
    }

    async fn add_task_with_id(
        &mut self,
        _metis_id: TaskId,
        _task: Task,
        _creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn update_task(&mut self, _metis_id: &TaskId, _task: Task) -> Result<(), StoreError> {
        fail()
    }

    async fn get_task(&self, _id: &TaskId) -> Result<Task, StoreError> {
        fail()
    }

    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        fail()
    }

    async fn list_tasks_with_status(&self, _status: Status) -> Result<Vec<TaskId>, StoreError> {
        fail()
    }

    async fn get_status(&self, _id: &TaskId) -> Result<Status, StoreError> {
        fail()
    }

    async fn get_status_log(&self, _id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        fail()
    }

    async fn mark_task_running(
        &mut self,
        _id: &TaskId,
        _start_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn mark_task_complete(
        &mut self,
        _id: &TaskId,
        _result: Result<(), TaskError>,
        _last_message: Option<String>,
        _completion_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        fail()
    }

    async fn create_actor_for_github_token(
        &mut self,
        _github_token: String,
        _github_refresh_token: String,
    ) -> Result<(User, Actor, String), StoreError> {
        fail()
    }

    async fn create_actor_for_task(
        &mut self,
        _task_id: TaskId,
    ) -> Result<(Actor, String), StoreError> {
        fail()
    }

    async fn add_actor(&mut self, _actor: Actor) -> Result<(), StoreError> {
        fail()
    }

    async fn get_actor(&self, _name: &str) -> Result<Actor, StoreError> {
        crate::store::validate_actor_name(_name)?;
        fail()
    }

    async fn list_actors(&self) -> Result<Vec<(String, Actor)>, StoreError> {
        fail()
    }

    async fn add_user(&mut self, _user: User) -> Result<(), StoreError> {
        fail()
    }

    async fn list_users(&self) -> Result<Vec<User>, StoreError> {
        fail()
    }

    async fn set_user_github_token(
        &mut self,
        _username: &Username,
        _github_token: String,
        _github_user_id: u64,
        _github_refresh_token: String,
    ) -> Result<User, StoreError> {
        fail()
    }

    async fn get_user(&self, _username: &Username) -> Result<User, StoreError> {
        fail()
    }
}
