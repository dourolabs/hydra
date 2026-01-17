use crate::store::{Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, MetisId, PatchId, TaskId};
use metis_common::{
    issues::{Issue, IssueGraphFilter},
    patches::Patch,
};
use std::collections::HashSet;

/// Store implementation that always fails; useful for exercising error paths in tests.
#[derive(Default)]
pub struct FailingStore;

fn fail<T>() -> Result<T, StoreError> {
    Err(StoreError::Internal("forced failure".to_string()))
}

#[async_trait]
impl Store for FailingStore {
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

    async fn is_issue_ready(&self, _issue_id: &IssueId) -> Result<bool, StoreError> {
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

    fn get_result(&self, _id: &TaskId) -> Option<Result<(), TaskError>> {
        None
    }

    async fn emit_task_artifacts(
        &mut self,
        _id: &TaskId,
        _artifacts: Vec<MetisId>,
        _timestamp: DateTime<Utc>,
    ) -> Result<(), StoreError> {
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
}
