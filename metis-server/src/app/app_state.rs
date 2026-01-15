use crate::{
    background::Spawner,
    config::AppConfig,
    job_engine::{JobEngine, JobEngineError, JobStatus},
    store::{Status, Store, StoreError, Task, TaskError, TaskExt, TaskResolutionError},
};
use chrono::{Duration, Utc};
use metis_common::{
    MetisId, PatchId, TaskId,
    constants::ENV_METIS_ID,
    issues::{IssueId, IssueStatus, UpsertIssueRequest},
    job_status::{JobStatusUpdate, SetJobStatusResponse},
    jobs::{BundleSpec, CreateJobRequest},
    patches::UpsertPatchRequest,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::ServiceState;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub service_state: Arc<ServiceState>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
    pub job_engine: Arc<dyn JobEngine>,
    pub spawners: Vec<Arc<dyn Spawner>>,
}

#[derive(Debug, Error)]
pub enum CreateJobError {
    #[error(transparent)]
    TaskResolution(#[from] TaskResolutionError),
    #[error("failed to store job {job_id}")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum SetJobStatusError {
    #[error("job '{job_id}' not found in store")]
    NotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to update status for job '{job_id}'")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum UpsertPatchError {
    #[error("job_id may only be provided when creating a patch")]
    JobIdProvidedForUpdate,
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("job_id must reference a running job to record emitted artifacts")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("patch '{patch_id}' not found")]
    PatchNotFound {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("patch store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("failed to emit artifacts for '{job_id}'")]
    EmitArtifacts {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum UpsertIssueError {
    #[error("job_id may only be provided when creating an issue")]
    JobIdProvidedForUpdate,
    #[error("issue dependency '{dependency_id}' not found")]
    MissingDependency {
        #[source]
        source: StoreError,
        dependency_id: IssueId,
    },
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: Option<IssueId>,
    },
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("job_id must reference a running job to record emitted artifacts")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("failed to emit artifacts for '{job_id}'")]
    EmitArtifacts {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to read tasks for dropped issue '{issue_id}'")]
    TaskLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("failed to kill task '{job_id}' for dropped issue '{issue_id}'")]
    KillTask {
        #[source]
        source: JobEngineError,
        issue_id: IssueId,
        job_id: TaskId,
    },
}

impl AppState {
    pub async fn create_job(&self, request: CreateJobRequest) -> Result<TaskId, CreateJobError> {
        let job_id = TaskId::new();
        let fallback_image = self.config.metis.worker_image.clone();

        let mut env_vars = request.variables;
        env_vars.insert(ENV_METIS_ID.to_string(), job_id.to_string());

        let task = Task {
            prompt: request.prompt,
            context: request.context,
            spawned_from: None,
            image: request.image,
            env_vars,
        };

        task.resolve(self.service_state.as_ref(), &fallback_image)?;

        let mut store = self.store.write().await;
        store
            .add_task_with_id(job_id.clone(), task, Utc::now())
            .await
            .map_err(|source| CreateJobError::Store {
                source,
                job_id: job_id.clone(),
            })?;

        Ok(job_id)
    }

    pub async fn set_job_status(
        &self,
        job_id: TaskId,
        status: JobStatusUpdate,
    ) -> Result<SetJobStatusResponse, SetJobStatusError> {
        {
            let mut store = self.store.write().await;

            store
                .get_task(&job_id)
                .await
                .map_err(|source| SetJobStatusError::NotFound {
                    source,
                    job_id: job_id.clone(),
                })?;

            store
                .mark_task_complete(
                    &job_id,
                    status.to_result(),
                    status.last_message(),
                    Utc::now(),
                )
                .await
                .map_err(|source| SetJobStatusError::Store {
                    source,
                    job_id: job_id.clone(),
                })?;
        }

        Ok(SetJobStatusResponse {
            job_id,
            status: status.as_status(),
        })
    }

    pub async fn start_pending_task(&self, task_id: TaskId) {
        let fallback_image = self.config.metis.worker_image.clone();
        let resolved = {
            let store = self.store.read().await;
            match store.get_task(&task_id).await {
                Ok(task) => match task.resolve(self.service_state.as_ref(), &fallback_image) {
                    Ok(resolved) => resolved,
                    Err(err) => {
                        warn!(
                            metis_id = %task_id,
                            error = %err,
                            "failed to resolve task for spawning"
                        );
                        return;
                    }
                },
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to load task for spawning"
                    );
                    return;
                }
            }
        };

        match self
            .job_engine
            .create_job(&task_id, &resolved.image, &resolved.env_vars)
            .await
        {
            Ok(()) => {
                let mut store = self.store.write().await;
                match store.mark_task_running(&task_id, Utc::now()).await {
                    Ok(()) => {
                        info!(
                            metis_id = %task_id,
                            "set task status to Running (spawned)"
                        );
                    }
                    Err(err) => {
                        warn!(
                            metis_id = %task_id,
                            error = %err,
                            "failed to set task to Running after spawn"
                        );
                    }
                }
            }
            Err(err) => {
                let mut store = self.store.write().await;
                let failure_reason = format!("Failed to create Kubernetes job: {err}");
                if let Err(update_err) = store
                    .mark_task_complete(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        Utc::now(),
                    )
                    .await
                {
                    error!(
                        metis_id = %task_id,
                        error = %update_err,
                        "failed to set task status to Failed (spawn failed)"
                    );
                } else {
                    info!(
                        metis_id = %task_id,
                        "set task status to Failed (spawn failed)"
                    );
                }
            }
        }
    }

    pub async fn reconcile_running_task(&self, task_id: TaskId) {
        match self.job_engine.find_job_by_metis_id(&task_id).await {
            Ok(job) => match job.status {
                JobStatus::Running => {}
                JobStatus::Complete => {
                    warn!(
                        metis_id = %task_id,
                        "Job completed in job engine without submitting results."
                    );

                    let completion_time = job.completion_time.unwrap_or_else(Utc::now);
                    let duration_since_completion =
                        Utc::now().signed_duration_since(completion_time);

                    if duration_since_completion < Duration::seconds(60) {
                        return;
                    }

                    let failure_reason =
                        "Job completed without submitting results (timeout after 1 minute)"
                            .to_string();
                    let mut store = self.store.write().await;
                    match store
                        .mark_task_complete(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            completion_time,
                        )
                        .await
                    {
                        Ok(()) => {
                            warn!(metis_id = %task_id, "task marked failed due to missing results after job completion timeout");
                        }
                        Err(err) => {
                            warn!(metis_id = %task_id, error = %err, "failed to mark task failed after missing results timeout");
                        }
                    }
                }
                JobStatus::Failed => {
                    let mut store = self.store.write().await;
                    let end_time = job.completion_time.unwrap_or_else(Utc::now);
                    let failure_reason = job
                        .failure_message
                        .unwrap_or_else(|| "Job failed for an undetermined reason".to_string());
                    match store
                        .mark_task_complete(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            end_time,
                        )
                        .await
                    {
                        Ok(()) => {
                            info!(metis_id = %task_id, "updated task status to Failed from job engine");
                        }
                        Err(err) => {
                            warn!(metis_id = %task_id, error = %err, "failed to update task status to Failed");
                        }
                    }
                }
            },
            Err(JobEngineError::NotFound(_)) => {
                warn!(
                    metis_id = %task_id,
                    "job not found in job engine, marking as failed"
                );

                let mut store = self.store.write().await;
                let failure_reason = "Job not found in job engine".to_string();
                if let Err(update_err) = store
                    .mark_task_complete(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        Utc::now(),
                    )
                    .await
                {
                    error!(metis_id = %task_id, error = %update_err, "failed to set task status to Failed");
                }
            }
            Err(err) => {
                error!(
                    metis_id = %task_id,
                    error = %err,
                    "failed to check job status in job engine"
                );
            }
        }
    }

    pub async fn upsert_patch(
        &self,
        patch_id: Option<PatchId>,
        request: UpsertPatchRequest,
    ) -> Result<PatchId, UpsertPatchError> {
        let UpsertPatchRequest { mut patch, job_id } = request;

        let mut store = self.store.write().await;
        let patch_id = match patch_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertPatchError::JobIdProvidedForUpdate);
                }

                store
                    .update_patch(&id, patch)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source,
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?;

                id
            }
            None => {
                if let Some(ref job_id) = job_id {
                    let status = store
                        .get_status(job_id)
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertPatchError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertPatchError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?;

                    if status != Status::Running {
                        return Err(UpsertPatchError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }

                    let task = store
                        .get_task(job_id)
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertPatchError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertPatchError::Store { source: other },
                        })?;
                    patch.service_repo_name = match task.context {
                        BundleSpec::ServiceRepository { name, .. } => Some(name),
                        _ => None,
                    };
                }

                let id = store
                    .add_patch(patch)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(id) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source: StoreError::PatchNotFound(id),
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?;

                if let Some(job_id) = job_id {
                    store
                        .emit_task_artifacts(&job_id, vec![id.clone().into()], Utc::now())
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertPatchError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            StoreError::InvalidStatusTransition => {
                                UpsertPatchError::JobNotRunning {
                                    job_id: job_id.clone(),
                                    status: None,
                                }
                            }
                            other => UpsertPatchError::EmitArtifacts {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?;
                }

                id
            }
        };

        tracing::info!(patch_id = %patch_id, "patch stored successfully");

        Ok(patch_id)
    }

    pub async fn upsert_issue(
        &self,
        issue_id: Option<IssueId>,
        request: UpsertIssueRequest,
    ) -> Result<IssueId, UpsertIssueError> {
        let UpsertIssueRequest { issue, job_id } = request;
        let mut tasks_to_kill = Vec::new();

        let mut store = self.store.write().await;

        let issue_id = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();

                match store.update_issue(&id, issue).await {
                    Ok(()) => {
                        if updated_issue.status == IssueStatus::Dropped {
                            tasks_to_kill = active_tasks_for_issue(store.as_ref(), &id)
                                .await
                                .map_err(|source| UpsertIssueError::TaskLookup {
                                    source,
                                    issue_id: id.clone(),
                                })?;
                        }
                        id
                    }
                    Err(source @ StoreError::IssueNotFound(_)) => {
                        return Err(UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        });
                    }
                    Err(StoreError::InvalidDependency(dependency_id)) => {
                        return Err(UpsertIssueError::MissingDependency {
                            dependency_id: dependency_id.clone(),
                            source: StoreError::InvalidDependency(dependency_id),
                        });
                    }
                    Err(source) => {
                        return Err(UpsertIssueError::Store {
                            source,
                            issue_id: Some(id),
                        });
                    }
                }
            }
            None => {
                if let Some(ref job_id) = job_id {
                    let status = store
                        .get_status(job_id)
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertIssueError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertIssueError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?;

                    if status != Status::Running {
                        return Err(UpsertIssueError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }
                }

                let id = store
                    .add_issue(issue)
                    .await
                    .map_err(|source| match source {
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: None,
                        },
                    })?;

                if let Some(job_id) = job_id {
                    store
                        .emit_task_artifacts(&job_id, vec![MetisId::from(id.clone())], Utc::now())
                        .await
                        .map_err(|source| UpsertIssueError::EmitArtifacts {
                            job_id: job_id.clone(),
                            source,
                        })?;
                }

                id
            }
        };

        drop(store);

        for job_id in tasks_to_kill {
            match self.job_engine.kill_job(&job_id).await {
                Ok(()) => info!(
                    issue_id = %issue_id,
                    job_id = %job_id,
                    "killed task for dropped issue"
                ),
                Err(JobEngineError::NotFound(_)) => info!(
                    issue_id = %issue_id,
                    job_id = %job_id,
                    "task already missing while dropping issue"
                ),
                Err(source) => {
                    return Err(UpsertIssueError::KillTask {
                        issue_id: issue_id.clone(),
                        job_id,
                        source,
                    });
                }
            }
        }

        info!(issue_id = %issue_id, "issue stored successfully");

        Ok(issue_id)
    }
}

async fn active_tasks_for_issue(
    store: &dyn Store,
    issue_id: &IssueId,
) -> Result<Vec<TaskId>, StoreError> {
    let task_ids = store.get_tasks_for_issue(issue_id).await?;

    let mut active_task_ids = Vec::new();
    for task_id in task_ids {
        let status = store.get_status(&task_id).await?;
        if matches!(status, Status::Pending | Status::Running) {
            active_task_ids.push(task_id);
        }
    }

    Ok(active_task_ids)
}

#[cfg(test)]
mod tests {
    use crate::{
        job_engine::{JobStatus, MockJobEngine},
        store::{Status, TaskError},
        test::test_state_with_engine,
    };
    use chrono::{Duration, Utc};
    use metis_common::jobs::{BundleSpec, Task};
    use std::{collections::HashMap, sync::Arc};

    fn sample_task() -> Task {
        Task {
            prompt: "Spawn me".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: Some("worker:latest".to_string()),
            env_vars: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn start_pending_task_spawns_and_marks_running() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let task = sample_task();

        let task_id = {
            let mut store = state.store.write().await;
            store.add_task(task, Utc::now()).await.unwrap()
        };

        state.start_pending_task(task_id.clone()).await;

        {
            let store = state.store.read().await;
            let status = store.get_status(&task_id).await.unwrap();
            assert_eq!(status, Status::Running);
        }

        assert!(job_engine.env_vars_for_job(&task_id).is_some());
    }

    #[tokio::test]
    async fn reconcile_running_task_marks_missing_jobs_failed() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let task_id = {
            let mut store = state.store.write().await;
            let task_id = store.add_task(sample_task(), Utc::now()).await.unwrap();
            store
                .mark_task_running(&task_id, Utc::now())
                .await
                .expect("task should transition to running");
            task_id
        };

        state.reconcile_running_task(task_id.clone()).await;

        let store = state.store.read().await;
        assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Failed);

        let status_log = store.get_status_log(&task_id).await.unwrap();
        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => {
                assert_eq!(reason, "Job not found in job engine");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reconcile_running_task_times_out_completed_jobs_without_results() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let completion_time = Utc::now() - Duration::seconds(90);

        let task_id = {
            let mut store = state.store.write().await;
            let task_id = store.add_task(sample_task(), Utc::now()).await.unwrap();
            store
                .mark_task_running(&task_id, Utc::now())
                .await
                .expect("task should transition to running");
            task_id
        };

        job_engine
            .insert_job_with_metadata(&task_id, JobStatus::Complete, Some(completion_time), None)
            .await;

        state.reconcile_running_task(task_id.clone()).await;

        let store = state.store.read().await;
        assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Failed);
        let status_log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(status_log.end_time(), Some(completion_time));

        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => assert_eq!(
                reason,
                "Job completed without submitting results (timeout after 1 minute)"
            ),
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
