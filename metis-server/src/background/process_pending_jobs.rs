use crate::{
    app::AppState,
    background::WorkerOutcome,
    store::{Status, TaskError, TaskExt},
};
use chrono::Utc;
use tracing::{error, info, warn};

/// Process pending jobs once, spawning Kubernetes jobs for tasks queued in the store.
pub async fn process_pending_jobs(state: &AppState) -> WorkerOutcome {
    let fallback_image = state.config.metis.worker_image.clone();
    let service_state = state.service_state.clone();

    let pending_ids = {
        let store = state.store.read().await;
        match store.list_tasks_with_status(Status::Pending).await {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "failed to list pending tasks");
                return WorkerOutcome::TransientError {
                    reason: "list_pending_failed".to_string(),
                };
            }
        }
    };

    if pending_ids.is_empty() {
        return WorkerOutcome::Idle;
    }

    info!(count = pending_ids.len(), "found pending tasks to process");

    let mut spawned = 0usize;
    let mut failed = 0usize;

    for metis_id in pending_ids {
        let resolved = {
            let store = state.store.read().await;
            match store.get_task(&metis_id).await {
                Ok(task) => match task.resolve(service_state.as_ref(), &fallback_image) {
                    Ok(resolved) => resolved,
                    Err(err) => {
                        failed += 1;
                        warn!(metis_id = %metis_id, error = %err, "failed to resolve task for spawning");
                        continue;
                    }
                },
                Err(err) => {
                    failed += 1;
                    warn!(metis_id = %metis_id, error = %err, "failed to load task for spawning");
                    continue;
                }
            }
        };

        match state
            .job_engine
            .create_job(&metis_id, &resolved.image, &resolved.env_vars)
            .await
        {
            Ok(()) => {
                let mut store = state.store.write().await;
                match store.mark_task_running(&metis_id, Utc::now()).await {
                    Ok(()) => {
                        spawned += 1;
                        info!(metis_id = %metis_id, "set task status to Running (spawned)");
                    }
                    Err(err) => {
                        failed += 1;
                        warn!(metis_id = %metis_id, error = %err, "failed to set task to Running after spawn");
                    }
                }
            }
            Err(err) => {
                failed += 1;
                match &err {
                    crate::job_engine::JobEngineError::Kubernetes(kube_err) => {
                        error!(
                            metis_id = %metis_id,
                            error = %kube_err,
                            "kubernetes error while creating job"
                        );
                    }
                    _ => {
                        warn!(metis_id = %metis_id, error = %err, "job engine failed to create job");
                    }
                }

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

    WorkerOutcome::Progress {
        processed: spawned,
        failed,
    }
}
