use crate::{
    background::AgentQueue,
    config::AppConfig,
    domain::{
        issues::{
            Issue, IssueDependencyType, IssueStatus, IssueType, JobSettings, TodoItem,
            UpsertIssueRequest,
        },
        jobs::CreateJobRequest,
        login::LoginResponse,
        patches::{Patch, PatchStatus, UpsertPatchRequest},
        users::UserSummary,
    },
    job_engine::{JobEngine, JobEngineError, JobStatus},
    store::{Status, Store, StoreError, Task, TaskError},
};
use chrono::{Duration, Utc};
use metis_common::{
    PatchId, RepoName, TaskId,
    constants::ENV_METIS_ID,
    issues::IssueId,
    job_status::{JobStatusUpdate, SetJobStatusResponse},
    merge_queues::MergeQueue,
};
use octocrab::Octocrab;
use std::{collections::HashSet, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::{
    MergeQueueError, RepositoryError, ServiceRepositoryConfig, ServiceRepositoryInfo, ServiceState,
    TaskResolutionError,
};

/// Shared application state and application-specific coordination such as issue lifecycle validation.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub github_app: Option<Octocrab>,
    pub service_state: Arc<ServiceState>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
    pub job_engine: Arc<dyn JobEngine>,
    pub agents: Vec<Arc<AgentQueue>>,
}

#[derive(Debug, Error)]
pub enum CreateJobError {
    #[error(transparent)]
    TaskResolution(#[from] TaskResolutionError),
    #[error("failed to load issue '{issue_id}' for job creation")]
    IssueLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
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
    #[error("created_by must reference a running job")]
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
    #[error("failed to load merge-request issues for patch '{patch_id}'")]
    MergeRequestLookup {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("failed to update merge-request issue '{issue_id}' for patch '{patch_id}'")]
    MergeRequestUpdate {
        #[source]
        source: StoreError,
        patch_id: PatchId,
        issue_id: IssueId,
    },
}

#[derive(Debug, Error)]
pub enum UpsertIssueError {
    #[error("job_id may only be provided when creating an issue")]
    JobIdProvidedForUpdate,
    #[error("issue creator must be set")]
    MissingCreator,
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
    #[error("job_id must reference a running job")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
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

#[derive(Debug, Error)]
pub enum UpdateTodoListError {
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("todo item number {item_number} is out of range for issue '{issue_id}'")]
    InvalidItemNumber {
        issue_id: IssueId,
        item_number: usize,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
}

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("invalid github token: {0}")]
    InvalidGithubToken(String),
    #[error("login store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    pub async fn login_with_github_token(
        &self,
        github_token: String,
    ) -> Result<LoginResponse, LoginError> {
        let mut store = self.store.write().await;
        let (user, _actor, login_token) = store
            .create_actor_for_github_token(github_token)
            .await
            .map_err(|source| match source {
            StoreError::GithubTokenInvalid(message) => LoginError::InvalidGithubToken(message),
            other => LoginError::Store { source: other },
        })?;

        Ok(LoginResponse::new(login_token, UserSummary::from(user)))
    }

    pub async fn list_repositories(&self) -> Result<Vec<ServiceRepositoryInfo>, RepositoryError> {
        let store = self.store.read().await;
        let repositories = store
            .list_repositories()
            .await
            .map_err(|source| RepositoryError::Store { source })?;

        Ok(repositories
            .into_iter()
            .map(ServiceRepositoryInfo::from)
            .collect())
    }

    pub async fn create_repository(
        &self,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<ServiceRepositoryInfo, RepositoryError> {
        {
            let mut store = self.store.write().await;
            store
                .add_repository(
                    name.clone(),
                    ServiceRepositoryConfig::new(
                        config.remote_url.clone(),
                        config.default_branch.clone(),
                        config.default_image.clone(),
                    ),
                )
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryAlreadyExists(name) => {
                        RepositoryError::AlreadyExists(name)
                    }
                    other => RepositoryError::Store { source: other },
                })?;
        }

        Ok(ServiceRepositoryInfo::from((name, config)))
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: ServiceRepositoryConfig,
    ) -> Result<ServiceRepositoryInfo, RepositoryError> {
        {
            let mut store = self.store.write().await;
            store
                .update_repository(name.clone(), config.clone())
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                    StoreError::RepositoryAlreadyExists(_) => {
                        RepositoryError::AlreadyExists(name.clone())
                    }
                    other => RepositoryError::Store { source: other },
                })?;
        }

        self.service_state.clear_cache(&name).await;

        Ok(ServiceRepositoryInfo::from((name, config)))
    }

    pub async fn create_job(&self, request: CreateJobRequest) -> Result<TaskId, CreateJobError> {
        let job_id = TaskId::new();

        let mut env_vars = request.variables;
        env_vars.insert(ENV_METIS_ID.to_string(), job_id.to_string());

        let issue = match request.issue_id.as_ref() {
            Some(issue_id) => {
                let store = self.store.read().await;
                Some(store.get_issue(issue_id).await.map_err(|source| {
                    CreateJobError::IssueLookup {
                        source,
                        issue_id: issue_id.clone(),
                    }
                })?)
            }
            None => None,
        };
        let job_settings = issue
            .as_ref()
            .map(|issue| issue.job_settings.clone())
            .filter(|settings| !JobSettings::is_default(settings));

        let task = Task::new(
            request.prompt,
            request.context,
            request.issue_id.clone(),
            request.image,
            env_vars,
            job_settings.clone(),
        );

        self.resolve_task(&task).await?;

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
                    status.to_result().map_err(TaskError::from),
                    status.last_message(),
                    Utc::now(),
                )
                .await
                .map_err(|source| SetJobStatusError::Store {
                    source,
                    job_id: job_id.clone(),
                })?;
        }

        Ok(SetJobStatusResponse::new(job_id, status.as_status()))
    }

    pub async fn start_pending_task(&self, task_id: TaskId) {
        let job_config = self.config.job.clone();
        let (resolved, job_settings) = {
            let store = self.store.read().await;
            match store.get_task(&task_id).await {
                Ok(mut task) => {
                    let job_settings = match task.spawned_from.as_ref() {
                        Some(issue_id) => match store.get_issue(issue_id).await {
                            Ok(issue) => issue.job_settings,
                            Err(err) => {
                                warn!(
                                    metis_id = %task_id,
                                    issue_id = %issue_id,
                                    error = %err,
                                    "failed to load issue for spawning"
                                );
                                return;
                            }
                        },
                        None => JobSettings::default(),
                    };

                    let merged = JobSettings::merge(task.job_settings.clone(), job_settings);
                    task.job_settings = merged;
                    let merged_job_settings = task.job_settings.clone();

                    match self.resolve_task(&task).await {
                        Ok(resolved) => (resolved, merged_job_settings),
                        Err(err) => {
                            warn!(
                                metis_id = %task_id,
                                error = %err,
                                "failed to resolve task for spawning"
                            );
                            return;
                        }
                    }
                }
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

        let cpu_limit = job_settings
            .cpu_limit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| job_config.cpu_limit.clone());
        let memory_limit = job_settings
            .memory_limit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| job_config.memory_limit.clone());

        match self
            .job_engine
            .create_job(
                &task_id,
                &resolved.image,
                &resolved.env_vars,
                cpu_limit,
                memory_limit,
            )
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

    pub async fn reap_orphaned_jobs(&self) {
        let job_engine_jobs = match self.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                return;
            }
        };

        if job_engine_jobs.is_empty() {
            return;
        }

        let store_task_ids = {
            let store = self.store.read().await;
            match store.list_tasks().await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list tasks from store for job reconciliation");
                    return;
                }
            }
        };

        let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
        let orphaned_jobs: Vec<_> = job_engine_jobs
            .into_iter()
            .filter(|job| !store_task_set.contains(&job.id))
            .collect();

        if !orphaned_jobs.is_empty() {
            info!(
                count = orphaned_jobs.len(),
                "killing jobs present in engine but missing from store"
            );
        }

        for job in orphaned_jobs {
            match self.job_engine.kill_job(&job.id).await {
                Ok(()) => {
                    info!(metis_id = %job.id, "killed job not present in store");
                }
                Err(err) => {
                    warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
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
        let UpsertPatchRequest { mut patch, .. } = request;

        let mut should_close_merge_requests = false;
        let mut store = self.store.write().await;
        let patch_id = match patch_id {
            Some(id) => {
                let existing_patch = store.get_patch(&id).await.map_err(|source| match source {
                    StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                        patch_id: id.clone(),
                        source,
                    },
                    other => UpsertPatchError::Store { source: other },
                })?;
                let new_status = patch.status;
                should_close_merge_requests = matches!(existing_patch.status, PatchStatus::Open)
                    && matches!(new_status, PatchStatus::Closed | PatchStatus::Merged);

                patch.created_by = existing_patch.created_by;
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
                if let Some(ref job_id) = patch.created_by {
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

                store
                    .add_patch(patch)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(id) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source: StoreError::PatchNotFound(id),
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?
            }
        };

        if should_close_merge_requests {
            let merge_request_issue_ids =
                store
                    .get_issues_for_patch(&patch_id)
                    .await
                    .map_err(|source| UpsertPatchError::MergeRequestLookup {
                        patch_id: patch_id.clone(),
                        source,
                    })?;

            let mut closed_issue_ids = Vec::new();
            for issue_id in merge_request_issue_ids {
                let mut issue = store.get_issue(&issue_id).await.map_err(|source| {
                    UpsertPatchError::MergeRequestUpdate {
                        patch_id: patch_id.clone(),
                        issue_id: issue_id.clone(),
                        source,
                    }
                })?;

                if issue.issue_type != IssueType::MergeRequest
                    || matches!(issue.status, IssueStatus::Closed | IssueStatus::Dropped)
                {
                    continue;
                }

                issue.status = IssueStatus::Closed;
                store
                    .update_issue(&issue_id, issue)
                    .await
                    .map_err(|source| UpsertPatchError::MergeRequestUpdate {
                        patch_id: patch_id.clone(),
                        issue_id: issue_id.clone(),
                        source,
                    })?;
                closed_issue_ids.push(issue_id);
            }

            if !closed_issue_ids.is_empty() {
                let issues = closed_issue_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::info!(
                    patch_id = %patch_id,
                    issues = %issues,
                    "closed merge-request issues for patch"
                );
            }
        }

        tracing::info!(patch_id = %patch_id, "patch stored successfully");

        Ok(patch_id)
    }

    pub async fn upsert_issue(
        &self,
        issue_id: Option<IssueId>,
        request: UpsertIssueRequest,
    ) -> Result<IssueId, UpsertIssueError> {
        let UpsertIssueRequest {
            mut issue, job_id, ..
        } = request;
        let mut tasks_to_kill = Vec::new();

        let mut store = self.store.write().await;

        let issue_id = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();
                if updated_issue.creator.as_ref().trim().is_empty() {
                    return Err(UpsertIssueError::MissingCreator);
                }
                let is_dropping = updated_issue.status == IssueStatus::Dropped;

                if let Err(source) =
                    validate_issue_lifecycle(store.as_ref(), Some(&id), &updated_issue).await
                {
                    return Err(match source {
                        StoreError::IssueNotFound(_) => UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        },
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: Some(id),
                        },
                    });
                }

                match store.update_issue(&id, updated_issue).await {
                    Ok(()) => {
                        if is_dropping {
                            tasks_to_kill = active_tasks_for_issue(store.as_ref(), &id)
                                .await
                                .map_err(|source| UpsertIssueError::TaskLookup {
                                    source,
                                    issue_id: id.clone(),
                                })?;

                            let child_tasks = drop_issue_children(store.as_mut(), &id).await?;
                            tasks_to_kill.extend(child_tasks);
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

                if issue.creator.as_ref().trim().is_empty() {
                    if let Some(parent_dependency) = issue.dependencies.iter().find(|dependency| {
                        dependency.dependency_type == IssueDependencyType::ChildOf
                    }) {
                        match store.get_issue(&parent_dependency.issue_id).await {
                            Ok(parent_issue) => {
                                issue.creator = parent_issue.creator;
                            }
                            Err(source @ StoreError::IssueNotFound(_)) => {
                                return Err(UpsertIssueError::MissingDependency {
                                    dependency_id: parent_dependency.issue_id.clone(),
                                    source,
                                });
                            }
                            Err(source) => {
                                return Err(UpsertIssueError::Store {
                                    source,
                                    issue_id: None,
                                });
                            }
                        }
                    }
                }
                if issue.creator.as_ref().trim().is_empty() {
                    return Err(UpsertIssueError::MissingCreator);
                }

                if let Err(source) = validate_issue_lifecycle(store.as_ref(), None, &issue).await {
                    return Err(match source {
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
                    });
                }

                store
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
                    })?
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

    pub async fn add_todo_item(
        &self,
        issue_id: IssueId,
        item: TodoItem,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let mut store = self.store.write().await;
        let mut issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;

        issue.todo_list.push(item);
        let todo_list = issue.todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn replace_todo_list(
        &self,
        issue_id: IssueId,
        todo_list: Vec<TodoItem>,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let mut store = self.store.write().await;
        let mut issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;

        issue.todo_list = todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn set_todo_item_status(
        &self,
        issue_id: IssueId,
        item_number: usize,
        is_done: bool,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let mut store = self.store.write().await;
        let mut issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;

        if item_number == 0 {
            return Err(UpdateTodoListError::InvalidItemNumber {
                issue_id,
                item_number,
            });
        }
        let index = item_number - 1;
        let item =
            issue
                .todo_list
                .get_mut(index)
                .ok_or(UpdateTodoListError::InvalidItemNumber {
                    issue_id: issue_id.clone(),
                    item_number,
                })?;
        item.is_done = is_done;

        let todo_list = issue.todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .get_merge_queue(service_repo_name, &config, branch_name)
            .await
    }

    pub async fn enqueue_merge_queue_patch(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        let patch = self.load_patch(patch_id.clone()).await?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .add_patch_to_merge_queue(service_repo_name, &config, branch_name, patch_id, &patch)
            .await
    }

    pub async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let store = self.store.read().await;
        issue_ready(store.as_ref(), issue_id).await
    }

    pub(crate) async fn repository_from_store(
        &self,
        name: &RepoName,
    ) -> Result<ServiceRepositoryConfig, StoreError> {
        let store = self.store.read().await;
        store.get_repository(name).await
    }

    async fn load_patch(&self, patch_id: PatchId) -> Result<Patch, MergeQueueError> {
        let store = self.store.read().await;
        match store.get_patch(&patch_id).await {
            Ok(patch) => Ok(patch),
            Err(StoreError::PatchNotFound(_)) => Err(MergeQueueError::PatchNotFound { patch_id }),
            Err(source) => Err(MergeQueueError::PatchLookup { patch_id, source }),
        }
    }
}

fn join_issue_ids(ids: &[IssueId]) -> String {
    let mut values: Vec<String> = ids.iter().map(ToString::to_string).collect();
    values.sort();
    values.join(", ")
}

fn join_item_numbers(numbers: &[usize]) -> String {
    let mut values = numbers.to_vec();
    values.sort();
    values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<String>>()
        .join(", ")
}

async fn issue_ready(store: &dyn Store, issue_id: &IssueId) -> Result<bool, StoreError> {
    let issue = store.get_issue(issue_id).await?;

    match issue.status {
        IssueStatus::Closed | IssueStatus::Dropped => Ok(false),
        IssueStatus::Open => {
            for dependency in issue
                .dependencies
                .iter()
                .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
            {
                let blocker = store.get_issue(&dependency.issue_id).await?;
                if blocker.status != IssueStatus::Closed {
                    return Ok(false);
                }
            }

            Ok(true)
        }
        IssueStatus::InProgress => {
            for child_id in store.get_issue_children(issue_id).await? {
                let child = store.get_issue(&child_id).await?;
                if child.status != IssueStatus::Closed {
                    return Ok(false);
                }
            }

            Ok(true)
        }
    }
}

async fn validate_issue_lifecycle(
    store: &dyn Store,
    issue_id: Option<&IssueId>,
    issue: &Issue,
) -> Result<(), StoreError> {
    if issue.status != IssueStatus::Closed {
        return Ok(());
    }

    let mut open_blockers = Vec::new();
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
    {
        let blocker = store
            .get_issue(&dependency.issue_id)
            .await
            .map_err(|err| match err {
                StoreError::IssueNotFound(missing_id) => StoreError::InvalidDependency(missing_id),
                other => other,
            })?;

        if blocker.status != IssueStatus::Closed {
            open_blockers.push(dependency.issue_id.clone());
        }
    }

    let mut open_todos = Vec::new();
    for (index, item) in issue.todo_list.iter().enumerate() {
        if !item.is_done {
            open_todos.push(index + 1);
        }
    }

    if !open_todos.is_empty() {
        return Err(StoreError::InvalidIssueStatus(format!(
            "cannot close issue with incomplete todo items: {}",
            join_item_numbers(&open_todos)
        )));
    }

    if let Some(issue_id) = issue_id {
        let mut open_children = Vec::new();
        for child_id in store.get_issue_children(issue_id).await? {
            let child = store.get_issue(&child_id).await?;
            if child.status != IssueStatus::Closed {
                open_children.push(child_id);
            }
        }

        if !open_children.is_empty() {
            return Err(StoreError::InvalidIssueStatus(format!(
                "cannot close issue with open child issues: {}",
                join_issue_ids(&open_children)
            )));
        }
    }

    if !open_blockers.is_empty() {
        return Err(StoreError::InvalidIssueStatus(format!(
            "blocked issues cannot close until blockers are closed: {}",
            join_issue_ids(&open_blockers)
        )));
    }

    Ok(())
}

async fn drop_issue_children(
    store: &mut dyn Store,
    issue_id: &IssueId,
) -> Result<Vec<TaskId>, UpsertIssueError> {
    let mut tasks_to_kill = Vec::new();
    let mut to_visit =
        store
            .get_issue_children(issue_id)
            .await
            .map_err(|source| UpsertIssueError::Store {
                source,
                issue_id: Some(issue_id.clone()),
            })?;
    let mut visited: HashSet<IssueId> = HashSet::new();

    while let Some(child_id) = to_visit.pop() {
        if !visited.insert(child_id.clone()) {
            continue;
        }

        let mut child_issue =
            store
                .get_issue(&child_id)
                .await
                .map_err(|source| UpsertIssueError::Store {
                    source,
                    issue_id: Some(child_id.clone()),
                })?;

        if child_issue.status != IssueStatus::Dropped {
            child_issue.status = IssueStatus::Dropped;
            store
                .update_issue(&child_id, child_issue)
                .await
                .map_err(|source| UpsertIssueError::Store {
                    source,
                    issue_id: Some(child_id.clone()),
                })?;
        }

        let active_child_tasks =
            active_tasks_for_issue(store, &child_id)
                .await
                .map_err(|source| UpsertIssueError::TaskLookup {
                    source,
                    issue_id: child_id.clone(),
                })?;
        tasks_to_kill.extend(active_child_tasks);

        let grandchildren = store
            .get_issue_children(&child_id)
            .await
            .map_err(|source| UpsertIssueError::Store {
                source,
                issue_id: Some(child_id.clone()),
            })?;
        to_visit.extend(grandchildren);
    }

    Ok(tasks_to_kill)
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
    use super::{LoginError, UpsertIssueError};
    use crate::{
        domain::{
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, JobSettings,
                TodoItem, UpsertIssueRequest,
            },
            jobs::{BundleSpec, Task},
            users::Username,
        },
        job_engine::{JobEngine, JobStatus},
        store::{Status, StoreError, TaskError},
        test_utils::{
            MockJobEngine, test_state, test_state_with_engine, test_state_with_github_client,
        },
    };
    use chrono::{Duration, Utc};
    use httpmock::prelude::*;
    use metis_common::{IssueId, TaskId};
    use octocrab::Octocrab;
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};

    fn sample_task() -> Task {
        Task::new(
            "Spawn me".to_string(),
            BundleSpec::None,
            None,
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
        )
    }

    fn task_for_issue(issue_id: &IssueId) -> Task {
        Task::new(
            "Spawn me".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
        )
    }

    fn issue_with_status(
        description: &str,
        status: IssueStatus,
        dependencies: Vec<IssueDependency>,
    ) -> Issue {
        Issue::new(
            IssueType::Task,
            description.to_string(),
            Username::from("creator"),
            String::new(),
            status,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    fn github_user_response(login: &str, id: u64) -> serde_json::Value {
        json!({
            "login": login,
            "id": id,
            "node_id": "NODEID",
            "avatar_url": "https://example.com/avatar",
            "gravatar_id": "gravatar",
            "url": "https://example.com/user",
            "html_url": "https://example.com/user",
            "followers_url": "https://example.com/followers",
            "following_url": "https://example.com/following",
            "gists_url": "https://example.com/gists",
            "starred_url": "https://example.com/starred",
            "subscriptions_url": "https://example.com/subscriptions",
            "organizations_url": "https://example.com/orgs",
            "repos_url": "https://example.com/repos",
            "events_url": "https://example.com/events",
            "received_events_url": "https://example.com/received_events",
            "type": "User",
            "site_admin": false,
            "name": null,
            "patch_url": null,
            "email": null
        })
    }

    fn build_github_client(base_url: String) -> Octocrab {
        Octocrab::builder()
            .base_uri(base_url)
            .unwrap()
            .personal_token("gh-token".to_string())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn login_persists_user_and_actor() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let state = test_state_with_github_client(build_github_client(github_server.base_url()));
        let response = state
            .login_with_github_token("gh-token".to_string())
            .await
            .expect("login should succeed");

        assert!(!response.login_token.is_empty());
        assert_eq!(response.user.username.as_str(), "octo");

        let store_read = state.store.read().await;
        let users = store_read.list_users().await?;
        let actors = store_read.list_actors().await?;
        assert_eq!(users.len(), 1);
        assert_eq!(actors.len(), 1);

        Ok(())
    }

    #[tokio::test]
    async fn login_returns_error_for_invalid_token() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(401);
        });

        let state = test_state_with_github_client(build_github_client(github_server.base_url()));
        let err = state
            .login_with_github_token("bad-token".to_string())
            .await
            .expect_err("login should fail for invalid token");

        assert!(matches!(err, LoginError::InvalidGithubToken(_)));
        Ok(())
    }

    #[tokio::test]
    async fn open_issue_ready_when_not_blocked() {
        let state = test_state();

        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue_with_status("open", IssueStatus::Open, vec![]))
                .await
                .unwrap()
        };

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_blocked_on_open_issue() {
        let state = test_state();

        let (blocker_id, blocked_issue_id) = {
            let mut store = state.store.write().await;
            let blocker_id = store
                .add_issue(issue_with_status("blocker", IssueStatus::Open, vec![]))
                .await
                .unwrap();
            let blocked_issue_id = store
                .add_issue(issue_with_status(
                    "blocked",
                    IssueStatus::Open,
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id.clone(),
                    )],
                ))
                .await
                .unwrap();

            (blocker_id, blocked_issue_id)
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());

        {
            let mut store = state.store.write().await;
            store
                .update_issue(
                    &blocker_id,
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_issue_ready_after_children_closed() {
        let state = test_state();

        let (parent_id, child_id, child_dependencies) = {
            let mut store = state.store.write().await;
            let parent_id = store
                .add_issue(issue_with_status("parent", IssueStatus::InProgress, vec![]))
                .await
                .unwrap();
            let child_dependencies = vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )];
            let child_id = store
                .add_issue(issue_with_status(
                    "child",
                    IssueStatus::Open,
                    child_dependencies.clone(),
                ))
                .await
                .unwrap();

            (parent_id, child_id, child_dependencies)
        };

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());

        {
            let mut store = state.store.write().await;
            store
                .update_issue(
                    &child_id,
                    issue_with_status("child", IssueStatus::Closed, child_dependencies),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_is_not_ready() {
        let state = test_state();

        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue_with_status("dropped", IssueStatus::Dropped, vec![]))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let state = test_state();

        let blocked_issue_id = {
            let mut store = state.store.write().await;
            let blocker_id = store
                .add_issue(issue_with_status("blocker", IssueStatus::Dropped, vec![]))
                .await
                .unwrap();
            store
                .add_issue(issue_with_status(
                    "blocked",
                    IssueStatus::Open,
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id,
                    )],
                ))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn closed_issue_is_not_ready() {
        let state = test_state();

        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(issue_with_status("closed", IssueStatus::Closed, vec![]))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn start_pending_task_spawns_and_marks_running() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let config = state.config.clone();
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
        let limits = job_engine
            .resource_limits_for_job(&task_id)
            .expect("resource limits should be recorded");
        assert_eq!(
            limits,
            (
                config.job.cpu_limit.clone(),
                config.job.memory_limit.clone()
            )
        );
    }

    #[tokio::test]
    async fn start_pending_task_uses_issue_resource_limits() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let job_settings = JobSettings {
            cpu_limit: Some("750m".to_string()),
            memory_limit: Some("2Gi".to_string()),
            ..Default::default()
        };

        let issue_id = {
            let mut store = state.store.write().await;
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "with limits".to_string(),
                    creator: Username::from("creator"),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: None,
                    job_settings: job_settings.clone(),
                    todo_list: Vec::new(),
                    dependencies: Vec::new(),
                    patches: Vec::new(),
                })
                .await
                .unwrap()
        };

        let task_id = {
            let mut store = state.store.write().await;
            store
                .add_task(task_for_issue(&issue_id), Utc::now())
                .await
                .unwrap()
        };

        state.start_pending_task(task_id.clone()).await;

        let limits = job_engine
            .resource_limits_for_job(&task_id)
            .expect("resource limits should be recorded");
        assert_eq!(limits, ("750m".to_string(), "2Gi".to_string()));
    }

    #[tokio::test]
    async fn reap_orphaned_jobs_kills_jobs_missing_from_store() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let tracked_task_id = {
            let mut store = state.store.write().await;
            store.add_task(sample_task(), Utc::now()).await.unwrap()
        };
        let orphan_task_id = TaskId::new();

        job_engine
            .insert_job(&tracked_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&orphan_task_id, JobStatus::Running)
            .await;

        state.reap_orphaned_jobs().await;

        let tracked_status = job_engine
            .find_job_by_metis_id(&tracked_task_id)
            .await
            .expect("tracked job should exist")
            .status;
        assert_eq!(tracked_status, JobStatus::Running);

        let orphan_status = job_engine
            .find_job_by_metis_id(&orphan_task_id)
            .await
            .expect("orphan job should exist")
            .status;
        assert_eq!(orphan_status, JobStatus::Failed);
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

    #[tokio::test]
    async fn dropping_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue.clone(), None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue.clone(), None))
            .await
            .unwrap();

        let grandchild_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, child_id.clone());
        let grandchild_issue = issue_with_status(
            "grandchild",
            IssueStatus::Open,
            vec![grandchild_dependency.clone()],
        );
        let grandchild_id = state
            .upsert_issue(
                None,
                UpsertIssueRequest::new(grandchild_issue.clone(), None),
            )
            .await
            .unwrap();

        let (parent_task_id, child_task_id, grandchild_task_id) = {
            let mut store = state.store.write().await;
            let parent_task_id = store
                .add_task(task_for_issue(&parent_id), Utc::now())
                .await
                .unwrap();
            let child_task_id = store
                .add_task(task_for_issue(&child_id), Utc::now())
                .await
                .unwrap();
            let grandchild_task_id = store
                .add_task(task_for_issue(&grandchild_id), Utc::now())
                .await
                .unwrap();
            (parent_task_id, child_task_id, grandchild_task_id)
        };

        job_engine
            .insert_job(&parent_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&grandchild_task_id, JobStatus::Running)
            .await;

        let mut dropped_parent = parent_issue.clone();
        dropped_parent.status = IssueStatus::Dropped;
        state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(dropped_parent, None),
            )
            .await
            .unwrap();

        {
            let store = state.store.read().await;
            assert_eq!(
                store.get_issue(&child_id).await.unwrap().status,
                IssueStatus::Dropped
            );
            assert_eq!(
                store.get_issue(&grandchild_id).await.unwrap().status,
                IssueStatus::Dropped
            );
        }

        for task_id in [parent_task_id, child_task_id, grandchild_task_id] {
            let job = job_engine
                .find_job_by_metis_id(&task_id)
                .await
                .expect("job should exist");
            assert_eq!(job.status, JobStatus::Failed);
        }
    }

    #[tokio::test]
    async fn closing_issue_requires_closed_blockers() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let blocker_issue = issue_with_status("blocker", IssueStatus::Open, vec![]);
        let blocker_id = state
            .upsert_issue(None, UpsertIssueRequest::new(blocker_issue, None))
            .await
            .unwrap();

        let blocked_dependencies = vec![IssueDependency::new(
            IssueDependencyType::BlockedOn,
            blocker_id.clone(),
        )];
        let blocked_issue =
            issue_with_status("blocked", IssueStatus::Open, blocked_dependencies.clone());
        let blocked_issue_id = state
            .upsert_issue(None, UpsertIssueRequest::new(blocked_issue, None))
            .await
            .unwrap();

        let err = state
            .upsert_issue(
                Some(blocked_issue_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocked", IssueStatus::Closed, blocked_dependencies.clone()),
                    None,
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == blocked_issue_id && message.contains(&blocker_id.to_string())
        ));

        state
            .upsert_issue(
                Some(blocker_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap();

        state
            .upsert_issue(
                Some(blocked_issue_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocked", IssueStatus::Closed, blocked_dependencies),
                    None,
                ),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closing_parent_requires_closed_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let err = state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("parent", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == parent_id && message.contains(&child_id.to_string())
        ));

        state
            .upsert_issue(
                Some(child_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("child", IssueStatus::Closed, vec![child_dependency.clone()]),
                    None,
                ),
            )
            .await
            .unwrap();

        state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("parent", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closing_issue_requires_completed_todos() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut issue = issue_with_status("todo", IssueStatus::Open, vec![]);
        issue
            .todo_list
            .push(TodoItem::new("finish task".to_string(), false));
        let issue_id = state
            .upsert_issue(None, UpsertIssueRequest::new(issue.clone(), None))
            .await
            .unwrap();

        let mut closed_issue = issue.clone();
        closed_issue.status = IssueStatus::Closed;

        let err = state
            .upsert_issue(
                Some(issue_id.clone()),
                UpsertIssueRequest::new(closed_issue.clone(), None),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == issue_id && message.contains("incomplete todo items")
        ));

        state
            .set_todo_item_status(issue_id.clone(), 1, true)
            .await
            .unwrap();

        closed_issue
            .todo_list
            .iter_mut()
            .for_each(|item| item.is_done = true);

        state
            .upsert_issue(
                Some(issue_id.clone()),
                UpsertIssueRequest::new(closed_issue, None),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_issue_inherits_creator_from_parent_when_empty() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        parent_issue.creator = Username::from("parent-creator");
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let mut child_issue = issue_with_status("child", IssueStatus::Open, vec![child_dependency]);
        child_issue.creator = Username::from("");
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let store = state.store.read().await;
        let stored_child = store.get_issue(&child_id).await.unwrap();
        assert_eq!(stored_child.creator, Username::from("parent-creator"));
    }

    #[tokio::test]
    async fn create_issue_preserves_explicit_creator_with_parent() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        parent_issue.creator = Username::from("parent-creator");
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let mut child_issue = issue_with_status("child", IssueStatus::Open, vec![child_dependency]);
        child_issue.creator = Username::from("explicit-creator");
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let store = state.store.read().await;
        let stored_child = store.get_issue(&child_id).await.unwrap();
        assert_eq!(stored_child.creator, Username::from("explicit-creator"));
    }

    #[tokio::test]
    async fn create_issue_without_parent_rejects_empty_creator() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut issue = issue_with_status("solo", IssueStatus::Open, vec![]);
        issue.creator = Username::from("");
        let err = state
            .upsert_issue(None, UpsertIssueRequest::new(issue, None))
            .await
            .unwrap_err();
        assert!(matches!(err, UpsertIssueError::MissingCreator));
    }
}
