use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use super::{Status, Store, StoreError, TaskError, TaskStatusLog};
use metis_common::MetisId;
use metis_common::artifacts::Artifact;
use metis_common::task_status::Event;

/// An in-memory implementation of the Store trait.
///
/// This store maintains session artifacts and their status logs.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps artifact IDs to their Artifact data
    artifacts: HashMap<MetisId, Artifact>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<MetisId, TaskStatusLog>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            artifacts: HashMap::new(),
            status_logs: HashMap::new(),
        }
    }

    fn insert_artifact(
        &mut self,
        id: MetisId,
        artifact: Artifact,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        match artifact {
            Artifact::Session {
                program,
                params,
                context,
                image,
                env_vars,
                dependencies,
                ..
            } => {
                let status_log = TaskStatusLog::new(Status::Pending, creation_time);
                self.status_logs.insert(id.clone(), status_log.clone());
                self.artifacts.insert(
                    id,
                    Artifact::Session {
                        program,
                        params,
                        context,
                        image,
                        env_vars,
                        log: status_log,
                        dependencies,
                    },
                );
            }
            other => {
                self.artifacts.insert(id, other);
            }
        }

        Ok(())
    }

    fn sync_session_log(&mut self, id: &MetisId) {
        let Some(status_log) = self.status_logs.get(id).cloned() else {
            return;
        };

        if let Some(Artifact::Session { log, .. }) = self.artifacts.get_mut(id) {
            *log = status_log;
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
    async fn add_artifact(&mut self, artifact: Artifact) -> Result<MetisId, StoreError> {
        let id = Uuid::new_v4().hyphenated().to_string();
        self.insert_artifact(id.clone(), artifact, Utc::now())?;
        Ok(id)
    }

    async fn add_artifact_with_id(
        &mut self,
        metis_id: MetisId,
        artifact: Artifact,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        if self.artifacts.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!(
                "Artifact already exists: {metis_id}"
            )));
        }

        self.insert_artifact(metis_id, artifact, creation_time)
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

        match artifact {
            Artifact::Session {
                program,
                params,
                context,
                image,
                env_vars,
                dependencies,
                ..
            } => {
                let status_log = self
                    .status_logs
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| TaskStatusLog::new(Status::Pending, Utc::now()));
                self.status_logs.insert(id.clone(), status_log.clone());
                self.artifacts.insert(
                    id.clone(),
                    Artifact::Session {
                        program,
                        params,
                        context,
                        image,
                        env_vars,
                        log: status_log,
                        dependencies,
                    },
                );
            }
            other => {
                self.status_logs.remove(id);
                self.artifacts.insert(id.clone(), other);
            }
        }

        Ok(())
    }

    async fn list_artifacts(&self) -> Result<Vec<(MetisId, Artifact)>, StoreError> {
        Ok(self
            .artifacts
            .iter()
            .map(|(id, artifact)| (id.clone(), artifact.clone()))
            .collect())
    }

    async fn list_tasks(&self) -> Result<Vec<MetisId>, StoreError> {
        Ok(self.status_logs.keys().cloned().collect())
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

    async fn mark_task_running(
        &mut self,
        id: &MetisId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log.events.push(Event::Started { at: start_time });
        self.sync_session_log(id);

        Ok(())
    }

    async fn emit_task_artifacts(
        &mut self,
        id: &MetisId,
        artifact_ids: Vec<MetisId>,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Running) {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log.events.push(Event::Emitted { at, artifact_ids });
        self.sync_session_log(id);

        Ok(())
    }

    async fn mark_task_complete(
        &mut self,
        id: &MetisId,
        result: Result<(), TaskError>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
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
        self.sync_session_log(id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use metis_common::{
        artifacts::{Artifact, IssueDependency, IssueDependencyType, IssueStatus, IssueType},
        jobs::Bundle,
    };
    use std::collections::{HashMap, HashSet};

    fn session_artifact_with_dependencies(dependencies: Vec<IssueDependency>) -> Artifact {
        Artifact::Session {
            program: "0".to_string(),
            params: vec![],
            context: Bundle::None,
            image: "metis-worker:latest".to_string(),
            env_vars: HashMap::new(),
            log: TaskStatusLog::default(),
            dependencies,
        }
    }

    fn sample_artifact() -> Artifact {
        Artifact::Patch {
            diff: "diff --git a/file b/file".to_string(),
            description: "sample patch".to_string(),
            dependencies: vec![],
        }
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
        assert!(!store.status_logs.contains_key(&id));
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
                    dependencies: vec![],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::ArtifactNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn add_artifact_with_id_sets_pending_status_and_dependencies() {
        let mut store = MemoryStore::new();
        let dependencies = vec![IssueDependency {
            dependency_type: IssueDependencyType::BlockedOn,
            issue_id: "parent-1".to_string(),
        }];
        let creation_time = Utc::now() - Duration::seconds(30);

        store
            .add_artifact_with_id(
                "job-1".to_string(),
                session_artifact_with_dependencies(dependencies.clone()),
                creation_time,
            )
            .await
            .unwrap();

        let status_log = store.get_status_log(&"job-1".to_string()).await.unwrap();
        assert_eq!(status_log.current_status(), Status::Pending);
        assert_eq!(status_log.creation_time(), Some(creation_time));

        match store.get_artifact(&"job-1".to_string()).await.unwrap() {
            Artifact::Session {
                log,
                dependencies: stored_deps,
                ..
            } => {
                assert_eq!(log, status_log);
                assert_eq!(stored_deps, dependencies);
            }
            other => panic!("expected session artifact, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_artifact_log_tracks_task_status_changes() {
        let mut store = MemoryStore::new();
        let job_id = "job-xyz".to_string();
        store
            .add_artifact_with_id(
                job_id.clone(),
                session_artifact_with_dependencies(vec![]),
                Utc::now(),
            )
            .await
            .unwrap();

        let start_time = Utc::now();
        store.mark_task_running(&job_id, start_time).await.unwrap();

        let running_log = store.get_status_log(&job_id).await.unwrap();
        assert_eq!(running_log.current_status(), Status::Running);
        assert_eq!(running_log.start_time(), Some(start_time));

        let end_time = Utc::now();
        store
            .mark_task_complete(&job_id, Ok(()), end_time)
            .await
            .unwrap();

        let completed_log = store.get_status_log(&job_id).await.unwrap();
        assert_eq!(completed_log.current_status(), Status::Complete);
        assert_eq!(completed_log.end_time(), Some(end_time));

        match store.get_artifact(&job_id).await.unwrap() {
            Artifact::Session { log, .. } => assert_eq!(log, completed_log),
            other => panic!("expected session artifact, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_tasks_with_status_filters_correctly() {
        let mut store = MemoryStore::new();

        store
            .add_artifact_with_id(
                "job-1".to_string(),
                session_artifact_with_dependencies(vec![]),
                Utc::now(),
            )
            .await
            .unwrap();
        store
            .add_artifact_with_id(
                "job-2".to_string(),
                session_artifact_with_dependencies(vec![]),
                Utc::now(),
            )
            .await
            .unwrap();
        store
            .mark_task_running(&"job-2".to_string(), Utc::now())
            .await
            .unwrap();

        let pending: HashSet<_> = store
            .list_tasks_with_status(Status::Pending)
            .await
            .unwrap()
            .into_iter()
            .collect();
        let running: HashSet<_> = store
            .list_tasks_with_status(Status::Running)
            .await
            .unwrap()
            .into_iter()
            .collect();

        assert_eq!(pending, HashSet::from(["job-1".to_string()]));
        assert_eq!(running, HashSet::from(["job-2".to_string()]));
    }

    #[tokio::test]
    async fn emit_task_artifacts_requires_running_state() {
        let mut store = MemoryStore::new();
        let job_id = "job-emit".to_string();
        store
            .add_artifact_with_id(
                job_id.clone(),
                session_artifact_with_dependencies(vec![]),
                Utc::now(),
            )
            .await
            .unwrap();

        let err = store
            .emit_task_artifacts(&job_id, vec!["artifact-1".into()], Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));

        store.mark_task_running(&job_id, Utc::now()).await.unwrap();
        store
            .emit_task_artifacts(&job_id, vec!["artifact-1".into()], Utc::now())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mark_task_complete_from_pending_fails() {
        let mut store = MemoryStore::new();
        let job_id = "job-pending".to_string();
        store
            .add_artifact_with_id(
                job_id.clone(),
                session_artifact_with_dependencies(vec![]),
                Utc::now(),
            )
            .await
            .unwrap();

        let err = store
            .mark_task_complete(&job_id, Ok(()), Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }
}
