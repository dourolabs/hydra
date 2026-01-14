use crate::{
    AppState,
    store::{Status, Task, TaskError},
};
use chrono::Utc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// Background task that periodically processes pending jobs.
///
/// This function runs in a loop, checking for pending tasks every few seconds
/// and starting them by:
/// 1. Setting their status to Running
/// 2. Creating the Kubernetes job via the job engine
pub async fn process_pending_jobs(state: AppState) {
    loop {
        // Check every 2 seconds
        sleep(Duration::from_secs(2)).await;

        // Get pending tasks
        let pending_ids = {
            let store = state.store.read().await;
            match store.list_tasks_with_status(Status::Pending).await {
                Ok(ids) => ids,
                Err(err) => {
                    error!(error = %err, "failed to list pending tasks");
                    continue;
                }
            }
        };

        if pending_ids.is_empty() {
            continue;
        }

        info!(count = pending_ids.len(), "found pending tasks to process");

        // Process each pending task
        for metis_id in pending_ids {
            let (image, env_vars) = {
                let store = state.store.read().await;
                match store.get_task(&metis_id).await {
                    Ok(Task {
                        image, env_vars, ..
                    }) => (image, env_vars),
                    Err(err) => {
                        warn!(metis_id = %metis_id, error = %err, "failed to load task for spawning");
                        continue;
                    }
                }
            };

            // Spawn the job
            match state
                .job_engine
                .create_job(&metis_id, &image, &env_vars)
                .await
            {
                Ok(()) => {
                    let mut store = state.store.write().await;
                    match store.mark_task_running(&metis_id, Utc::now()).await {
                        Ok(()) => {
                            info!(metis_id = %metis_id, "set task status to Running (spawned)");
                        }
                        Err(err) => {
                            warn!(metis_id = %metis_id, error = %err, "failed to set task to Running after spawn");
                        }
                    }
                }
                Err(err) => {
                    let mut store = state.store.write().await;
                    let failure_reason = format!("Failed to create Kubernetes job: {err}");
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
                        error!(metis_id = %metis_id, error = %update_err, "failed to set task status to Failed (spawn failed)");
                    } else {
                        info!(metis_id = %metis_id, "set task status to Failed (spawn failed)");
                    }
                }
            }
        }
    }
}
