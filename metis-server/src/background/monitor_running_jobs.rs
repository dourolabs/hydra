use crate::{
    app::AppState,
    background::WorkerOutcome,
    job_engine::JobStatus,
    store::{Status, TaskError},
};
use chrono::Utc;
use std::collections::HashSet;
use tracing::{error, info, warn};

/// Monitor running jobs once, reconciling orphaned engine jobs and task completion.
pub async fn monitor_running_jobs(state: &AppState) -> WorkerOutcome {
    let mut processed = 0usize;
    let mut failed = 0usize;

    let job_engine_jobs = match state.job_engine.list_jobs().await {
        Ok(jobs) => jobs,
        Err(err) => {
            error!(error = %err, "failed to list jobs in job engine");
            return WorkerOutcome::TransientError {
                reason: "list_jobs_failed".to_string(),
            };
        }
    };

    if !job_engine_jobs.is_empty() {
        let store_task_ids = {
            let store = state.store.read().await;
            match store.list_tasks().await {
                Ok(ids) => Ok(ids),
                Err(err) => {
                    error!(error = %err, "failed to list tasks from store for job reconciliation");
                    Err(())
                }
            }
        };

        if let Ok(store_task_ids) = store_task_ids {
            let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
            let orphaned_jobs: Vec<_> = job_engine_jobs
                .iter()
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
                        processed += 1;
                        info!(metis_id = %job.id, "killed job not present in store");
                    }
                    Err(err) => {
                        failed += 1;
                        warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
                    }
                }
            }
        } else {
            return WorkerOutcome::TransientError {
                reason: "list_tasks_failed".to_string(),
            };
        }
    }

    // Get running tasks
    let running_ids = {
        let store = state.store.read().await;
        match store.list_tasks_with_status(Status::Running).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list running tasks");
                return WorkerOutcome::TransientError {
                    reason: "list_running_failed".to_string(),
                };
            }
        }
    };

    if running_ids.is_empty() && processed == 0 && failed == 0 {
        return WorkerOutcome::Idle;
    }

    if !running_ids.is_empty() {
        info!(count = running_ids.len(), "found running tasks to monitor");
    }

    // Check each running job's status
    for metis_id in running_ids {
        match state.job_engine.find_job_by_metis_id(&metis_id).await {
            Ok(job) => match job.status {
                JobStatus::Complete => {
                    warn!(metis_id = %metis_id, "Job completed in job engine without submitting results.");
                    let mut store = state.store.write().await;
                    let completion_time = job.completion_time.unwrap_or_else(Utc::now);
                    let now = Utc::now();
                    let duration_since_completion = now.signed_duration_since(completion_time);
                    if duration_since_completion.num_seconds() >= 60 {
                        let failure_reason =
                            "Job completed without submitting results (timeout after 1 minute)"
                                .to_string();
                        match store
                            .mark_task_complete(
                                &metis_id,
                                Err(TaskError::JobEngineError {
                                    reason: failure_reason,
                                }),
                                None,
                                completion_time,
                            )
                            .await
                        {
                            Ok(()) => {
                                processed += 1;
                                warn!(metis_id = %metis_id, "task marked failed due to missing results after job completion timeout");
                            }
                            Err(err) => {
                                failed += 1;
                                warn!(metis_id = %metis_id, error = %err, "failed to mark task failed after missing results timeout");
                            }
                        }
                    }
                }
                JobStatus::Failed => {
                    let mut store = state.store.write().await;
                    let end_time = job.completion_time.unwrap_or_else(Utc::now);
                    let failure_reason = job
                        .failure_message
                        .unwrap_or_else(|| "Job failed for an undetermined reason".to_string());
                    match store
                        .mark_task_complete(
                            &metis_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                            end_time,
                        )
                        .await
                    {
                        Ok(()) => {
                            processed += 1;
                            info!(metis_id = %metis_id, "updated task status to Failed from job engine");
                        }
                        Err(err) => {
                            failed += 1;
                            warn!(metis_id = %metis_id, error = %err, "failed to update task status to Failed");
                        }
                    }
                }
                JobStatus::Running => {
                    continue;
                }
            },
            Err(crate::job_engine::JobEngineError::NotFound(_)) => {
                warn!(metis_id = %metis_id, "job not found in job engine, marking as failed");
                let mut store = state.store.write().await;
                let failure_reason = "Job not found in job engine".to_string();
                if let Err(update_err) = store
                    .mark_task_complete(
                        &metis_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                        Utc::now(),
                    )
                    .await
                {
                    failed += 1;
                    error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed");
                } else {
                    processed += 1;
                }
            }
            Err(err) => {
                failed += 1;
                error!(metis_id = %metis_id, error = %err, "failed to check job status in job engine");
            }
        }
    }

    if processed == 0 && failed == 0 {
        WorkerOutcome::Idle
    } else {
        WorkerOutcome::Progress { processed, failed }
    }
}
