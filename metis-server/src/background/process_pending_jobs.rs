use crate::{
    AppState,
    store::{Event, Status, TaskError},
};
use chrono::Utc;
use metis_common::artifacts::{Artifact, ArtifactKind};
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
        let pending_sessions = {
            let store = state.store.read().await;
            match store
                .list_artifacts_with_type_and_status(ArtifactKind::Session, Status::Pending)
                .await
            {
                Ok(sessions) => sessions,
                Err(err) => {
                    error!(error = %err, "failed to list pending tasks");
                    continue;
                }
            }
        };

        if pending_sessions.is_empty() {
            continue;
        }

        info!(
            count = pending_sessions.len(),
            "found pending tasks to process"
        );

        // Process each pending task
        for (metis_id, artifact) in pending_sessions {
            let Artifact::Session { image, .. } = artifact else {
                warn!(metis_id = %metis_id, "artifact for pending task was not a session");
                continue;
            };

            // Spawn the job
            match state.job_engine.create_job(&metis_id, &image).await {
                Ok(()) => {
                    let mut store = state.store.write().await;
                    match store
                        .append_status_event(&metis_id, Event::Started { at: Utc::now() })
                        .await
                    {
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
                    let failure_event = Event::Failed {
                        at: Utc::now(),
                        error: TaskError::JobEngineError {
                            reason: failure_reason,
                        },
                    };
                    if let Err(update_err) =
                        store.append_status_event(&metis_id, failure_event).await
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
