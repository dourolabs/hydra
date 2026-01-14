use super::{MemoryStore, Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, MetisId, PatchId, TaskId};
use metis_common::{issues::Issue, patches::Patch};
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::fs;

/// File-backed implementation of the Store trait.
///
/// Persists the underlying MemoryStore to disk after every mutation so data
/// survives process restarts.
pub struct FileStore {
    path: PathBuf,
    inner: MemoryStore,
}

impl FileStore {
    /// Opens a file-backed store, creating the file if it does not already exist.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                StoreError::Internal(format!(
                    "failed to create store directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let inner = match fs::read(&path).await {
            Ok(bytes) => {
                if bytes.is_empty() {
                    MemoryStore::new()
                } else {
                    let mut store: MemoryStore = serde_json::from_slice(&bytes).map_err(|err| {
                        StoreError::Internal(format!(
                            "failed to parse store file '{}': {err}",
                            path.display()
                        ))
                    })?;
                    store.rebuild_issue_indexes();
                    store
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let store = MemoryStore::new();
                Self::write_snapshot(&path, &store).await?;
                store
            }
            Err(err) => {
                return Err(StoreError::Internal(format!(
                    "failed to read store file '{}': {err}",
                    path.display()
                )));
            }
        };

        Ok(Self { path, inner })
    }

    async fn persist(&self) -> Result<(), StoreError> {
        Self::write_snapshot(&self.path, &self.inner).await
    }

    async fn write_snapshot(path: &Path, store: &MemoryStore) -> Result<(), StoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                StoreError::Internal(format!(
                    "failed to create store directory '{}': {err}",
                    parent.display()
                ))
            })?;
        }

        let tmp_path = path.with_extension("tmp");
        let data = serde_json::to_vec(store).map_err(|err| {
            StoreError::Internal(format!(
                "failed to serialize store file '{}': {err}",
                path.display()
            ))
        })?;

        fs::write(&tmp_path, data).await.map_err(|err| {
            StoreError::Internal(format!(
                "failed to write store file '{}': {err}",
                tmp_path.display()
            ))
        })?;
        match fs::rename(&tmp_path, path).await {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                fs::remove_file(path).await.map_err(|err| {
                    StoreError::Internal(format!(
                        "failed to replace existing store file '{}': {err}",
                        path.display()
                    ))
                })?;
                fs::rename(&tmp_path, path).await.map_err(|err| {
                    StoreError::Internal(format!(
                        "failed to finalize store file '{}': {err}",
                        path.display()
                    ))
                })?;
            }
            Err(err) => {
                return Err(StoreError::Internal(format!(
                    "failed to finalize store file '{}': {err}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Store for FileStore {
    async fn add_issue(&mut self, issue: Issue) -> Result<IssueId, StoreError> {
        let id = self.inner.add_issue(issue).await?;
        self.persist().await?;
        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Issue, StoreError> {
        self.inner.get_issue(id).await
    }

    async fn update_issue(&mut self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        self.inner.update_issue(id, issue).await?;
        self.persist().await
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError> {
        self.inner.list_issues().await
    }

    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = self.inner.add_patch(patch).await?;
        self.persist().await?;
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError> {
        self.inner.get_patch(id).await
    }

    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        self.inner.update_patch(id, patch).await?;
        self.persist().await
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError> {
        self.inner.list_patches().await
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_children(issue_id).await
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        self.inner.get_issue_blocked_on(issue_id).await
    }

    async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        self.inner.is_issue_ready(issue_id).await
    }

    async fn add_task(
        &mut self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        let id = self.inner.add_task(task, creation_time).await?;
        self.persist().await?;
        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: TaskId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner
            .add_task_with_id(metis_id, task, creation_time)
            .await?;
        self.persist().await
    }

    async fn update_task(&mut self, metis_id: &TaskId, task: Task) -> Result<(), StoreError> {
        self.inner.update_task(metis_id, task).await?;
        self.persist().await
    }

    async fn get_task(&self, id: &TaskId) -> Result<Task, StoreError> {
        self.inner.get_task(id).await
    }

    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        self.inner.list_tasks().await
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        self.inner.list_tasks_with_status(status).await
    }

    async fn get_status(&self, id: &TaskId) -> Result<Status, StoreError> {
        self.inner.get_status(id).await
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.inner.get_status_log(id).await
    }

    fn get_result(&self, id: &TaskId) -> Option<Result<(), TaskError>> {
        self.inner.get_result(id)
    }

    async fn emit_task_artifacts(
        &mut self,
        id: &TaskId,
        artifact_ids: Vec<MetisId>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner.emit_task_artifacts(id, artifact_ids, at).await?;
        self.persist().await
    }

    async fn mark_task_running(
        &mut self,
        id: &TaskId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner.mark_task_running(id, start_time).await?;
        self.persist().await
    }

    async fn mark_task_complete(
        &mut self,
        id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.inner
            .mark_task_complete(id, result, last_message, end_time)
            .await?;
        self.persist().await
    }
}
