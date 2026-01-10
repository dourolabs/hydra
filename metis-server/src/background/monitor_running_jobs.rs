use crate::{
    AppState,
    job_engine::JobStatus,
    store::{Status, TaskError},
};
use chrono::Utc;
use metis_common::{MetisId, artifacts::ArtifactKind};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Background task that periodically monitors running jobs.
///
/// This function runs in a loop, checking for running tasks every few seconds
/// and updating their status based on the job engine state:
/// 1. Gets all running tasks from the store
/// 2. Checks each job's status in the job engine
/// 3. Updates the store status to Complete or Failed if the job has finished
pub async fn monitor_running_jobs(state: AppState) {
    loop {
        // Check every 5 seconds
        sleep(Duration::from_secs(5)).await;

        // Kill any jobs that are running in the engine but missing from the store
        let job_engine_jobs = match state.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                Vec::new()
            }
        };

        if !job_engine_jobs.is_empty() {
            let store_task_ids = {
                let store = state.store.read().await;
                match store.list_artifacts_with_type(ArtifactKind::Session).await {
                    Ok(artifacts) => {
                        Some(artifacts.into_iter().map(|(id, _)| id).collect::<Vec<_>>())
                    }
                    Err(err) => {
                        error!(error = %err, "failed to list tasks from store for job reconciliation");
                        None
                    }
                }
            };

            if let Some(store_task_ids) = store_task_ids {
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
                    match state.job_engine.kill_job(&job.id).await {
                        Ok(()) => {
                            info!(metis_id = %job.id, "killed job not present in store");
                        }
                        Err(err) => {
                            warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
                        }
                    }
                }
            }
        }

        // Get running tasks
        let running_ids: Vec<MetisId> = {
            let store = state.store.read().await;
            match store
                .list_artifacts_with_type_and_status(ArtifactKind::Session, Status::Running)
                .await
            {
                Ok(ids) => ids.into_iter().map(|(id, _)| id).collect(),
                Err(err) => {
                    error!(error = %err, "failed to list running tasks");
                    continue;
                }
            }
        };

        if running_ids.is_empty() {
            continue;
        }

        info!(count = running_ids.len(), "found running tasks to monitor");

        // Check each running job's status
        for metis_id in running_ids {
            match state.job_engine.find_job_by_metis_id(&metis_id).await {
                Ok(job) => {
                    match job.status {
                        JobStatus::Complete => {
                            warn!(metis_id = %metis_id, "Job completed in job engine without submitting results.");
                            // If job has been completed for at least 1 minute, mark as failed due to timeout for missing result
                            let mut store = state.store.write().await;
                            let completion_time = job.completion_time.unwrap_or_else(Utc::now);
                            let now = Utc::now();
                            let duration_since_completion =
                                now.signed_duration_since(completion_time);
                            // Check for a 1 minute (60s) timeout since completion
                            if duration_since_completion.num_seconds() >= 60 {
                                let failure_reason = "Job completed without submitting results (timeout after 1 minute)".to_string();
                                match store
                                    .mark_task_complete(
                                        &metis_id,
                                        Err(TaskError::JobEngineError {
                                            reason: failure_reason,
                                        }),
                                        completion_time,
                                    )
                                    .await
                                {
                                    Ok(()) => {
                                        warn!(metis_id = %metis_id, "task marked failed due to missing results after job completion timeout");
                                    }
                                    Err(err) => {
                                        warn!(metis_id = %metis_id, error = %err, "failed to mark task failed after missing results timeout");
                                    }
                                }
                            }
                        }
                        JobStatus::Failed => {
                            // Update status in store
                            let mut store = state.store.write().await;
                            let end_time = job.completion_time.unwrap_or_else(Utc::now);
                            let failure_reason = job.failure_message.unwrap_or_else(|| {
                                "Job failed for an undetermined reason".to_string()
                            });
                            match store
                                .mark_task_complete(
                                    &metis_id,
                                    Err(TaskError::JobEngineError {
                                        reason: failure_reason,
                                    }),
                                    end_time,
                                )
                                .await
                            {
                                Ok(()) => {
                                    info!(metis_id = %metis_id, "updated task status to Failed from job engine");
                                }
                                Err(err) => {
                                    warn!(metis_id = %metis_id, error = %err, "failed to update task status to Failed");
                                }
                            }
                        }
                        JobStatus::Running => {
                            // Still running, skip
                            continue;
                        }
                    }
                }
                Err(crate::job_engine::JobEngineError::NotFound(_)) => {
                    // Job not found in Kubernetes - might have been deleted or never created
                    // This could happen if the job was cleaned up externally
                    warn!(metis_id = %metis_id, "job not found in job engine, marking as failed");
                    let mut store = state.store.write().await;
                    let failure_reason = "Job not found in job engine".to_string();
                    if let Err(update_err) = store
                        .mark_task_complete(
                            &metis_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            Utc::now(),
                        )
                        .await
                    {
                        error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed");
                    }
                }
                Err(err) => {
                    error!(metis_id = %metis_id, error = %err, "failed to check job status in job engine");
                    // Don't update status on transient errors
                }
            }
        }
    }
}
