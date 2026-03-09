use crate::{
    config::non_empty,
    domain::{actors::ActorRef, issues::JobSettings, jobs::BundleSpec, users::Username},
    job_engine::{JobEngineError, JobStatus},
    store::{ReadOnlyStore, Status, StoreError, Task, TaskError, TaskStatusLog},
};
use chrono::{DateTime, Duration, Utc};
use metis_common::{
    TaskId, Versioned,
    api::v1 as api,
    api::v1::jobs::SearchJobsQuery,
    issues::IssueId,
    job_status::{JobStatusUpdate, SetJobStatusResponse},
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use tracing::{error, info, warn};

use super::TaskResolutionError;
use super::app_state::AppState;

pub(crate) const WORKER_NAME_TASK_LIFECYCLE: &str = "task_lifecycle";
pub(crate) const WORKER_NAME_CLEANUP_ORPHANED_TASKS: &str = "cleanup_orphaned_tasks";

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
    #[error("failed to store job")]
    Store {
        #[source]
        source: StoreError,
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
    #[error("invalid status transition for job '{job_id}'")]
    InvalidStatusTransition { job_id: TaskId },
    #[error("failed to update status for job '{job_id}'")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
}

impl AppState {
    pub async fn add_task(
        &self,
        task: Task,
        created_at: DateTime<Utc>,
        actor: ActorRef,
    ) -> Result<TaskId, StoreError> {
        let (task_id, _version) = self
            .store
            .add_task_with_actor(task, created_at, actor)
            .await?;
        Ok(task_id)
    }

    pub async fn create_job(
        &self,
        request: api::jobs::CreateJobRequest,
        actor: ActorRef,
        creator: Username,
    ) -> Result<TaskId, CreateJobError> {
        let env_vars = request.variables;

        let issue = match request.issue_id.as_ref() {
            Some(issue_id) => {
                let store = self.store.as_ref();
                Some(store.get_issue(issue_id, false).await.map_err(|source| {
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
            .map(|issue| self.apply_job_settings_defaults(issue.item.job_settings.clone()))
            .filter(|settings| !JobSettings::is_default(settings));

        let mut context: BundleSpec = request.context.into();
        let image = job_settings
            .as_ref()
            .and_then(|settings| settings.image.clone())
            .or(request.image);
        let model = job_settings
            .as_ref()
            .and_then(|settings| settings.model.clone());
        let cpu_limit = job_settings
            .as_ref()
            .and_then(|settings| settings.cpu_limit.clone());
        let memory_limit = job_settings
            .as_ref()
            .and_then(|settings| settings.memory_limit.clone());

        if let Some(settings) = job_settings {
            if let Some(remote_url) = settings.remote_url.clone() {
                let rev = settings
                    .branch
                    .clone()
                    .or_else(|| match &context {
                        BundleSpec::GitRepository { rev, .. } => Some(rev.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "main".to_string());
                context = BundleSpec::GitRepository {
                    url: remote_url,
                    rev,
                };
            } else if let Some(repo_name) = settings.repo_name.clone() {
                context = BundleSpec::ServiceRepository {
                    name: repo_name,
                    rev: settings.branch.clone(),
                };
            } else if let (Some(branch), BundleSpec::GitRepository { url, .. }) =
                (settings.branch.clone(), &context)
            {
                context = BundleSpec::GitRepository {
                    url: url.clone(),
                    rev: branch,
                };
            }
        }

        let task = Task::new(
            request.prompt,
            context,
            request.issue_id.clone(),
            creator,
            image,
            model,
            env_vars,
            cpu_limit,
            memory_limit,
            None,
            Status::Created,
            None,
            None,
        );

        self.resolve_task(&task).await?;

        let (job_id, _version) = self
            .store
            .add_task_with_actor(task, Utc::now(), actor)
            .await
            .map_err(|source| CreateJobError::Store { source })?;

        Ok(job_id)
    }

    pub(crate) fn apply_job_settings_defaults(&self, mut settings: JobSettings) -> JobSettings {
        if settings.model.is_none() {
            if let Some(default_model) =
                self.config.job.default_model.as_deref().and_then(non_empty)
            {
                settings.model = Some(default_model.to_string());
            }
        }

        settings
    }

    pub async fn set_job_status(
        &self,
        job_id: TaskId,
        status: JobStatusUpdate,
        actor: ActorRef,
    ) -> Result<SetJobStatusResponse, SetJobStatusError> {
        {
            let store = self.store.as_ref();

            store
                .get_task(&job_id, false)
                .await
                .map(|_| ())
                .map_err(|source| SetJobStatusError::NotFound {
                    source,
                    job_id: job_id.clone(),
                })?;

            self.transition_task_to_completion(
                &job_id,
                status.to_result().map_err(TaskError::from),
                status.last_message(),
                actor,
            )
            .await
            .map_err(|source| match source {
                StoreError::InvalidStatusTransition => SetJobStatusError::InvalidStatusTransition {
                    job_id: job_id.clone(),
                },
                other => SetJobStatusError::Store {
                    source: other,
                    job_id: job_id.clone(),
                },
            })?;
        }

        Ok(SetJobStatusResponse::new(job_id, status.as_status()))
    }

    /// Loads all user secrets and injects them as env vars, then falls back to config
    /// values for system secrets (OPENAI_API_KEY, ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN)
    /// not already set.
    pub(crate) async fn resolve_secrets_into_env_vars(
        &self,
        creator: &Username,
        env_vars: &mut HashMap<String, String>,
    ) {
        use metis_common::constants::{
            ENV_ANTHROPIC_API_KEY, ENV_CLAUDE_CODE_OAUTH_TOKEN, ENV_OPENAI_API_KEY,
        };

        // 1. Load ALL user secrets and inject them as env vars.
        let user_secret_names = match self.store.list_user_secret_names(creator).await {
            Ok(names) => names,
            Err(err) => {
                warn!(
                    username = %creator,
                    error = %err,
                    "failed to list user secret names"
                );
                Vec::new()
            }
        };

        for secret_name in &user_secret_names {
            match self.store.get_user_secret(creator, secret_name).await {
                Ok(Some(encrypted)) => match self.secret_manager.decrypt(&encrypted) {
                    Ok(value) if !value.trim().is_empty() => {
                        env_vars.insert(secret_name.clone(), value);
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(
                            username = %creator,
                            secret = %secret_name,
                            error = %err,
                            "failed to decrypt user secret, skipping"
                        );
                    }
                },
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        username = %creator,
                        secret = %secret_name,
                        error = %err,
                        "failed to look up user secret, skipping"
                    );
                }
            }
        }

        // 2. For system secrets not already set by user secrets, fall back to config.
        let system_entries: [(&str, Option<&str>); 3] = [
            (
                ENV_OPENAI_API_KEY,
                self.config.metis.openai_api_key.as_deref(),
            ),
            (
                ENV_ANTHROPIC_API_KEY,
                self.config.metis.anthropic_api_key.as_deref(),
            ),
            (
                ENV_CLAUDE_CODE_OAUTH_TOKEN,
                self.config.metis.claude_code_oauth_token.as_deref(),
            ),
        ];

        for (secret_name, config_fallback) in system_entries {
            if env_vars.contains_key(secret_name) {
                continue;
            }

            let global_value = config_fallback
                .map(str::to_string)
                .filter(|v| !v.trim().is_empty());

            if let Some(value) = global_value {
                env_vars.insert(secret_name.to_string(), value);
            }
        }
    }

    pub async fn start_pending_task(&self, task_id: TaskId, actor: ActorRef) {
        let job_config = self.config.job.clone();
        let (mut resolved, cpu_limit, memory_limit, creator) = {
            let store = self.store.as_ref();
            match store.get_task(&task_id, false).await {
                Ok(task) => match self.resolve_task(&task.item).await {
                    Ok(resolved) => (
                        resolved,
                        task.item.cpu_limit.clone(),
                        task.item.memory_limit.clone(),
                        task.item.creator.clone(),
                    ),
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

        // Resolve per-user secrets with global fallback and inject into env_vars.
        self.resolve_secrets_into_env_vars(&creator, &mut resolved.env_vars)
            .await;

        let cpu_limit = cpu_limit.unwrap_or_else(|| job_config.cpu_limit.clone());
        let memory_limit = memory_limit.unwrap_or_else(|| job_config.memory_limit.clone());
        let cpu_request = job_config.cpu_request.clone();
        let memory_request = job_config.memory_request.clone();

        let (task_actor, auth_token) = match self
            .create_actor_for_job(task_id.clone(), actor.clone())
            .await
        {
            Ok(values) => values,
            Err(err) => {
                let failure_reason = format!("Failed to create actor for task: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        actor,
                    )
                    .await
                {
                    error!(
                        metis_id = %task_id,
                        error = %update_err,
                        "failed to set task status to Failed (actor creation failed)"
                    );
                } else {
                    info!(
                        metis_id = %task_id,
                        "set task status to Failed (actor creation failed)"
                    );
                }
                return;
            }
        };

        match self
            .job_engine
            .create_job(
                &task_id,
                &task_actor,
                &auth_token,
                &resolved.image,
                &resolved.env_vars,
                cpu_limit,
                memory_limit,
                cpu_request,
                memory_request,
                resolved.secrets.as_deref(),
            )
            .await
        {
            Ok(()) => match self
                .transition_task_to_pending(&task_id, actor.clone())
                .await
            {
                Ok(_) => {
                    info!(
                        metis_id = %task_id,
                        "set task status to Pending (spawned)"
                    );
                }
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to set task to Pending after spawn"
                    );
                }
            },
            Err(err) => {
                // For non-AlreadyExists errors (e.g. etcdserver timeouts), the job
                // may have actually been created despite the error. Wait briefly for
                // etcd to settle, then check whether the job exists in K8s before
                // marking the task as Failed.
                if !matches!(err, JobEngineError::AlreadyExists(_)) {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    match self.job_engine.find_job_by_metis_id(&task_id).await {
                        Ok(job)
                            if job.status == JobStatus::Pending
                                || job.status == JobStatus::Running =>
                        {
                            warn!(
                                metis_id = %task_id,
                                create_error = %err,
                                job_status = %job.status,
                                "create_job failed but job exists in K8s; treating as successful"
                            );
                            match self
                                .transition_task_to_pending(&task_id, actor.clone())
                                .await
                            {
                                Ok(_) => {
                                    info!(
                                        metis_id = %task_id,
                                        "set task status to Pending (job found after create error)"
                                    );
                                }
                                Err(transition_err) => {
                                    warn!(
                                        metis_id = %task_id,
                                        error = %transition_err,
                                        "failed to set task to Pending after finding existing job"
                                    );
                                }
                            }
                            return;
                        }
                        _ => {
                            // Job not found or in a terminal state — fall through
                            // to the existing failure path below.
                        }
                    }
                }

                let failure_reason = format!("Failed to create Kubernetes job: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        actor,
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

        let store_task_ids: Vec<TaskId> = {
            let store = self.store.as_ref();
            match store.list_tasks(&SearchJobsQuery::default()).await {
                Ok(tasks) => tasks.into_iter().map(|(id, _)| id).collect(),
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

    /// Cleans up tasks whose `spawned_from` issue has been soft-deleted.
    ///
    /// For each non-deleted task that references a `spawned_from` issue, checks
    /// whether that issue still exists. If it does not (i.e., it has been
    /// soft-deleted), the task is soft-deleted and any running/pending job is
    /// killed in the engine.
    pub async fn cleanup_orphaned_tasks(&self, actor: ActorRef) {
        let store = self.store.as_ref();
        let tasks = match store.list_tasks(&SearchJobsQuery::default()).await {
            Ok(tasks) => tasks,
            Err(err) => {
                error!(error = %err, "failed to list tasks for orphaned task cleanup");
                return;
            }
        };

        for (task_id, versioned_task) in tasks {
            let issue_id = match &versioned_task.item.spawned_from {
                Some(id) => id.clone(),
                None => continue,
            };

            let issue_deleted = match store.get_issue(&issue_id, false).await {
                Ok(_) => false,
                Err(StoreError::IssueNotFound(_)) => true,
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        issue_id = %issue_id,
                        error = %err,
                        "failed to check spawned_from issue for orphaned task cleanup"
                    );
                    continue;
                }
            };

            if !issue_deleted {
                continue;
            }

            info!(
                metis_id = %task_id,
                issue_id = %issue_id,
                "soft-deleting orphaned task whose spawned_from issue was deleted"
            );

            if let Err(err) = self
                .store
                .delete_task_with_actor(&task_id, actor.clone())
                .await
            {
                warn!(
                    metis_id = %task_id,
                    error = %err,
                    "failed to soft-delete orphaned task"
                );
                continue;
            }

            if matches!(
                versioned_task.item.status,
                Status::Pending | Status::Running
            ) {
                if let Err(err) = self.job_engine.kill_job(&task_id).await {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to kill job for orphaned task"
                    );
                }
            }
        }
    }

    pub async fn reconcile_running_task(&self, task_id: TaskId, actor: ActorRef) {
        let current_status = {
            let store = self.store.as_ref();
            match store.get_task(&task_id, false).await {
                Ok(task) => task.item.status,
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to load task while reconciling status"
                    );
                    return;
                }
            }
        };

        match self.job_engine.find_job_by_metis_id(&task_id).await {
            Ok(job) => match job.status {
                JobStatus::Pending => {}
                JobStatus::Running => {
                    if current_status == Status::Pending {
                        match self
                            .transition_task_to_running(&task_id, actor.clone())
                            .await
                        {
                            Ok(_) => {
                                info!(
                                    metis_id = %task_id,
                                    "set task status to Running (pod started)"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    metis_id = %task_id,
                                    error = %err,
                                    "failed to set task to Running after pod start"
                                );
                            }
                        }
                    }
                }
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
                    match self
                        .transition_task_to_completion(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            actor.clone(),
                        )
                        .await
                    {
                        Ok(_) => {
                            warn!(metis_id = %task_id, "task marked failed due to missing results after job completion timeout");
                        }
                        Err(err) => {
                            warn!(metis_id = %task_id, error = %err, "failed to mark task failed after missing results timeout");
                        }
                    }
                }
                JobStatus::Failed => {
                    let failure_reason = job
                        .failure_message
                        .unwrap_or_else(|| "Job failed for an undetermined reason".to_string());
                    match self
                        .transition_task_to_completion(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            actor.clone(),
                        )
                        .await
                    {
                        Ok(_) => {
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

                let failure_reason = "Job not found in job engine".to_string();
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        actor,
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

    pub async fn transition_task_to_pending(
        &self,
        task_id: &TaskId,
        actor: ActorRef,
    ) -> Result<Versioned<Task>, StoreError> {
        let latest = self.store.get_task(task_id, false).await?;
        if latest.item.status != Status::Created {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Pending;
        updated.last_message = None;
        updated.error = None;

        self.store
            .update_task_with_actor(task_id, updated, actor)
            .await
    }

    pub async fn transition_task_to_running(
        &self,
        task_id: &TaskId,
        actor: ActorRef,
    ) -> Result<Versioned<Task>, StoreError> {
        let latest = self.store.get_task(task_id, false).await?;
        if !matches!(latest.item.status, Status::Created | Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Running;
        updated.last_message = None;
        updated.error = None;
        if updated.start_time.is_none() {
            updated.start_time = Some(Utc::now());
        }

        self.store
            .update_task_with_actor(task_id, updated, actor)
            .await
    }

    pub async fn transition_task_to_completion(
        &self,
        task_id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        actor: ActorRef,
    ) -> Result<Versioned<Task>, StoreError> {
        let store = self.store.as_ref();
        let latest = store.get_task(task_id, false).await?;
        let can_transition = match latest.item.status {
            Status::Created => result.is_err(),
            Status::Pending | Status::Running => true,
            // Idempotent: if already in the target terminal state, return Ok
            Status::Complete => result.is_ok(),
            Status::Failed => result.is_err(),
        };
        if !can_transition {
            return Err(StoreError::InvalidStatusTransition);
        }

        // Already in the target terminal state — return existing version unchanged
        if matches!(latest.item.status, Status::Complete | Status::Failed) {
            return Ok(latest);
        }

        let mut updated = latest.item;
        match result {
            Ok(()) => {
                updated.status = Status::Complete;
                updated.last_message = last_message;
                updated.error = None;
            }
            Err(error) => {
                updated.status = Status::Failed;
                updated.last_message = None;
                updated.error = Some(error);
            }
        }
        if updated.end_time.is_none() {
            updated.end_time = Some(Utc::now());
        }

        self.store
            .update_task_with_actor(task_id, updated, actor)
            .await
    }

    pub async fn get_task(&self, task_id: &TaskId) -> Result<Task, StoreError> {
        let store = self.store.as_ref();
        store.get_task(task_id, false).await.map(|task| task.item)
    }

    pub async fn get_task_versions(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<Versioned<Task>>, StoreError> {
        let store = self.store.as_ref();
        store.get_task_versions(task_id).await
    }

    pub async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        let store = self.store.as_ref();
        store.get_tasks_for_issue(issue_id).await
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        let store = self.store.as_ref();
        store
            .list_tasks(&SearchJobsQuery::default())
            .await
            .map(|tasks| tasks.into_iter().map(|(id, _)| id).collect())
    }

    pub async fn list_tasks_with_query(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_tasks(query).await
    }

    pub async fn get_status_log(&self, task_id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        let store = self.store.as_ref();
        store.get_status_log(task_id).await
    }

    pub async fn get_status_logs(
        &self,
        task_ids: &[TaskId],
    ) -> Result<HashMap<TaskId, TaskStatusLog>, StoreError> {
        let store = self.store.as_ref();
        store.get_status_logs(task_ids).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::test_helpers::{
            issue_with_status, sample_task, state_with_default_model, task_for_issue,
        },
        domain::actors::ActorRef,
        domain::issues::{Issue, IssueStatus, IssueType, JobSettings},
        domain::users::Username,
        job_engine::{JobEngine, JobStatus},
        store::{ReadOnlyStore, Status, StoreError, TaskError},
        test_utils::{MockJobEngine, test_state_with_engine},
    };
    use chrono::{Duration, Utc};
    use metis_common::TaskId;
    use std::sync::Arc;

    #[tokio::test]
    async fn start_pending_task_spawns_and_marks_pending() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let config = state.config.clone();
        let task = sample_task();

        let (task_id, _) = {
            let store = state.store.as_ref();
            store
                .add_task_with_actor(task, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        state
            .start_pending_task(task_id.clone(), ActorRef::test())
            .await;

        {
            let store = state.store.as_ref();
            let status = store.get_task(&task_id, false).await.unwrap().item.status;
            assert_eq!(status, Status::Pending);
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
    async fn start_pending_task_uses_task_resource_limits() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let job_settings = JobSettings {
            cpu_limit: Some("750m".to_string()),
            memory_limit: Some("2Gi".to_string()),
            ..Default::default()
        };

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    Issue {
                        issue_type: IssueType::Task,
                        title: String::new(),
                        description: "with limits".to_string(),
                        creator: Username::from("creator"),
                        progress: String::new(),
                        status: IssueStatus::Open,
                        assignee: None,
                        job_settings: job_settings.clone(),
                        todo_list: Vec::new(),
                        dependencies: Vec::new(),
                        patches: Vec::new(),
                        deleted: false,
                    },
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        let (task_id, _) = {
            let store = state.store.as_ref();
            let mut task = task_for_issue(&issue_id);
            task.cpu_limit = job_settings.cpu_limit.clone();
            task.memory_limit = job_settings.memory_limit.clone();
            store
                .add_task_with_actor(task, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        state
            .start_pending_task(task_id.clone(), ActorRef::test())
            .await;

        let limits = job_engine
            .resource_limits_for_job(&task_id)
            .expect("resource limits should be recorded");
        assert_eq!(limits, ("750m".to_string(), "2Gi".to_string()));
    }

    #[tokio::test]
    async fn start_pending_task_timeout_but_job_exists_transitions_to_pending() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let task = sample_task();

        let (task_id, _) = {
            let store = state.store.as_ref();
            store
                .add_task_with_actor(task, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        // Pre-insert the job so find_job_by_metis_id finds it, and configure
        // create_job to fail (simulating an etcdserver timeout where the job
        // was actually created).
        job_engine.insert_job(&task_id, JobStatus::Running).await;
        job_engine.set_create_job_error(Some("etcdserver: request timed out".to_string()));

        state
            .start_pending_task(task_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        let status = store.get_task(&task_id, false).await.unwrap().item.status;
        assert_eq!(status, Status::Pending);
    }

    #[tokio::test]
    async fn start_pending_task_timeout_and_job_missing_transitions_to_failed() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let task = sample_task();

        let (task_id, _) = {
            let store = state.store.as_ref();
            store
                .add_task_with_actor(task, Utc::now(), ActorRef::test())
                .await
                .unwrap()
        };

        // Configure create_job to fail without pre-inserting the job, so
        // find_job_by_metis_id will return NotFound.
        job_engine.set_create_job_error(Some("etcdserver: request timed out".to_string()));

        state
            .start_pending_task(task_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        let status = store.get_task(&task_id, false).await.unwrap().item.status;
        assert_eq!(status, Status::Failed);
    }

    #[test]
    fn apply_job_settings_defaults_sets_model() {
        let state = state_with_default_model("gpt-4o");
        let job_settings = JobSettings::default();

        let resolved = state.apply_job_settings_defaults(job_settings);

        assert_eq!(resolved.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn apply_job_settings_defaults_preserves_explicit_model() {
        let state = state_with_default_model("gpt-4o");
        let job_settings = JobSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let resolved = state.apply_job_settings_defaults(job_settings);

        assert_eq!(resolved.model.as_deref(), Some("custom-model"));
    }

    #[tokio::test]
    async fn reap_orphaned_jobs_kills_jobs_missing_from_store() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let (tracked_task_id, _) = {
            let store = state.store.as_ref();
            store
                .add_task_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap()
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
            let store = state.store.as_ref();
            let (task_id, _) = store
                .add_task_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            state
                .transition_task_to_pending(&task_id, ActorRef::test())
                .await
                .expect("task should transition to pending");
            task_id
        };

        state
            .reconcile_running_task(task_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        assert_eq!(
            store.get_task(&task_id, false).await.unwrap().item.status,
            Status::Failed
        );

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
            let store = state.store.as_ref();
            let (task_id, _) = store
                .add_task_with_actor(sample_task(), Utc::now(), ActorRef::test())
                .await
                .unwrap();
            state
                .transition_task_to_pending(&task_id, ActorRef::test())
                .await
                .expect("task should transition to pending");
            task_id
        };

        job_engine
            .insert_job_with_metadata(&task_id, JobStatus::Complete, Some(completion_time), None)
            .await;

        state
            .reconcile_running_task(task_id.clone(), ActorRef::test())
            .await;

        let store = state.store.as_ref();
        assert_eq!(
            store.get_task(&task_id, false).await.unwrap().item.status,
            Status::Failed
        );
        let status_log = store.get_status_log(&task_id).await.unwrap();
        assert!(status_log.end_time().is_some());

        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => assert_eq!(
                reason,
                "Job completed without submitting results (timeout after 1 minute)"
            ),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_deletes_task_with_deleted_issue() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (task_id, _) = store
            .add_task_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let result = store.get_task(&task_id, false).await;
        assert!(
            matches!(result, Err(StoreError::TaskNotFound(_))),
            "orphaned task should be soft-deleted"
        );

        let deleted_task = store.get_task(&task_id, true).await.unwrap();
        assert!(deleted_task.item.deleted);
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_leaves_task_with_existing_issue() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (task_id, _) = store
            .add_task_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let task = store.get_task(&task_id, false).await.unwrap();
        assert!(
            !task.item.deleted,
            "task with existing issue should not be deleted"
        );
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_leaves_task_with_no_spawned_from() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let store = state.store.as_ref();

        let (task_id, _) = store
            .add_task_with_actor(sample_task(), Utc::now(), ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let task = store.get_task(&task_id, false).await.unwrap();
        assert!(
            !task.item.deleted,
            "task without spawned_from should not be deleted"
        );
    }

    #[tokio::test]
    async fn cleanup_orphaned_tasks_kills_running_job() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let store = state.store.as_ref();

        let (issue_id, _) = store
            .add_issue_with_actor(
                issue_with_status("parent", IssueStatus::Open, vec![]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        let (task_id, _) = store
            .add_task_with_actor(task_for_issue(&issue_id), Utc::now(), ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await
            .expect("task should transition to pending");

        job_engine.insert_job(&task_id, JobStatus::Running).await;

        store
            .delete_issue_with_actor(&issue_id, ActorRef::test())
            .await
            .unwrap();

        state.cleanup_orphaned_tasks(ActorRef::test()).await;

        let result = store.get_task(&task_id, false).await;
        assert!(
            matches!(result, Err(StoreError::TaskNotFound(_))),
            "orphaned running task should be soft-deleted"
        );

        let job = job_engine
            .find_job_by_metis_id(&task_id)
            .await
            .expect("job should still exist in engine");
        assert_eq!(
            job.status,
            JobStatus::Failed,
            "running job for orphaned task should be killed"
        );
    }
}
