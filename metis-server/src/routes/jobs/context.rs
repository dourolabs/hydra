use crate::{
    AppState,
    routes::jobs::{ApiError, JobIdPath, payload_from_artifact},
    store::Task,
};
use anyhow::anyhow;
use axum::{Json, extract::State};
use metis_common::{
    MetisId,
    jobs::{ParentContext, WorkerContext},
};
use std::collections::HashMap;
use tracing::{error, info};

pub async fn get_job_context(
    State(state): State<AppState>,
    JobIdPath(job_id): JobIdPath,
) -> Result<Json<WorkerContext>, ApiError> {
    info!(job_id = %job_id, "get_job_context invoked");

    let store = state.store.read().await;
    let task = store.get_task(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get task");
        ApiError::not_found(format!("Job '{job_id}' not found"))
    })?;

    // Get parent task IDs and their results
    let parent_edges = store.get_parents(&job_id).await.map_err(|err| {
        error!(error = %err, job_id = %job_id, "failed to get parent tasks");
        ApiError::internal(anyhow!("Failed to get parent tasks: {err}"))
    })?;

    let mut parents: HashMap<MetisId, ParentContext> = HashMap::new();
    for parent_edge in parent_edges {
        if matches!(store.get_result(&parent_edge.id), Some(Ok(()))) {
            if let Ok(Some(artifact_ids)) = store.latest_emitted_artifact_ids(&parent_edge.id).await
            {
                for artifact_id in artifact_ids {
                    if let Ok(artifact) = store.get_artifact(&artifact_id).await {
                        if let Some(output) = payload_from_artifact(&artifact) {
                            parents.insert(
                                parent_edge.id.clone(),
                                ParentContext {
                                    name: parent_edge.name.clone(),
                                    output,
                                },
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    match task {
        Task::Spawn {
            program,
            params,
            context,
            env_vars,
            ..
        } => Ok(Json(WorkerContext {
            request_context: context.clone(),
            parents,
            program: program.clone(),
            params: params.clone(),
            variables: env_vars.clone(),
        })),
    }
}
