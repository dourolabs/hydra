use crate::{
    background::Spawner,
    config::AppConfig,
    job_engine::{JobEngine, JobEngineError},
    store::{Status, Store, StoreError, Task, TaskExt, TaskResolutionError},
};
use chrono::Utc;
use metis_common::{
    MetisId, PatchId, TaskId,
    constants::ENV_METIS_ID,
    issues::{IssueDependency, IssueId, IssueStatus, UpsertIssueRequest},
    job_status::{JobStatusUpdate, SetJobStatusResponse},
    jobs::CreateJobRequest,
    patches::UpsertPatchRequest,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info};

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
    #[error("issue dependency '{dependency_id}' could not be validated")]
    DependencyValidation {
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

    pub async fn upsert_patch(
        &self,
        patch_id: Option<PatchId>,
        request: UpsertPatchRequest,
    ) -> Result<PatchId, UpsertPatchError> {
        let UpsertPatchRequest { patch, job_id } = request;

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
        validate_issue_dependencies(store.as_mut(), &issue.dependencies).await?;

        let issue_id = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();

                match store.update_issue(&id, issue).await {
                    Ok(()) => {
                        if updated_issue.status == IssueStatus::Dropped {
                            tasks_to_kill = active_tasks_for_issue(store.as_mut(), &id)
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

                let id =
                    store
                        .add_issue(issue)
                        .await
                        .map_err(|source| UpsertIssueError::Store {
                            source,
                            issue_id: None,
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

async fn validate_issue_dependencies(
    store: &mut dyn Store,
    dependencies: &[IssueDependency],
) -> Result<(), UpsertIssueError> {
    for dependency in dependencies {
        let target_id = &dependency.issue_id;
        store
            .get_issue(target_id)
            .await
            .map_err(|source| match source {
                StoreError::IssueNotFound(_) => UpsertIssueError::MissingDependency {
                    dependency_id: target_id.clone(),
                    source,
                },
                other => UpsertIssueError::DependencyValidation {
                    dependency_id: target_id.clone(),
                    source: other,
                },
            })?;
    }

    Ok(())
}

async fn active_tasks_for_issue(
    store: &mut dyn Store,
    issue_id: &IssueId,
) -> Result<Vec<TaskId>, StoreError> {
    let mut task_ids = Vec::new();

    for task_id in store.list_tasks().await? {
        let task = store.get_task(&task_id).await?;
        if task.spawned_from.as_ref() != Some(issue_id) {
            continue;
        }

        let status = store.get_status(&task_id).await?;
        if matches!(status, Status::Pending | Status::Running) {
            task_ids.push(task_id);
        }
    }

    Ok(task_ids)
}
