use crate::{
    app::AppState,
    domain::{
        actors::{Actor, ActorRef},
        task_status::TaskError,
    },
    job_engine::JobEngineError,
    store::StoreError,
};
use axum::{Extension, Json, extract::State};
use hydra_common::api::v1;
use tracing::{error, info, warn};

use super::{ApiError, SessionIdPath};

pub async fn kill_session(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    SessionIdPath(session_id): SessionIdPath,
) -> Result<Json<v1::sessions::KillSessionResponse>, ApiError> {
    info!(session_id = %session_id, "kill_session invoked");

    // Short-circuit if the DB row is already terminal. `transition_task_to_completion`
    // is idempotent and returns Ok unchanged in that case, which we couldn't
    // otherwise distinguish from a real transition — so we look up the row
    // first to produce the `already_terminal` response status.
    let already_terminal = match state.get_session(&session_id).await {
        Ok(session) => session.status.is_terminal(),
        Err(StoreError::SessionNotFound(_)) => {
            return Err(ApiError::not_found(format!(
                "Session '{session_id}' not found"
            )));
        }
        Err(err) => {
            error!(session_id = %session_id, error = %err, "failed to load session for kill");
            return Err(ApiError::internal(err));
        }
    };

    // Attempt synchronous K8s pod stop (exec SIGTERM → grace → SIGKILL).
    // Whatever the outcome, we still mark the DB session Failed below —
    // this keeps the kill button reliable when the K8s job is stuck or
    // absent (e.g. `Created` sessions whose job hasn't been created
    // yet). Non-NotFound engine errors get logged and left to the
    // reaper. `stop_job` deliberately leaves the Pod object intact so
    // post-mortem logs survive.
    let cleanup_pending = match state.job_engine.stop_job(&session_id).await {
        Ok(()) => false,
        Err(JobEngineError::NotFound(_)) => {
            info!(session_id = %session_id, "no K8s pod to stop; proceeding to DB transition");
            false
        }
        Err(JobEngineError::MultipleFound(_)) => {
            warn!(
                session_id = %session_id,
                "multiple K8s pods found while stopping session; reaper will clean up"
            );
            true
        }
        Err(err) => {
            error!(
                session_id = %session_id,
                error = %err,
                "K8s stop_job failed; marking DB Failed and leaving cleanup to reaper"
            );
            true
        }
    };

    let kill_status = if already_terminal {
        "already_terminal"
    } else {
        let actor_ref = ActorRef::from(&actor);
        match state
            .transition_task_to_completion(
                &session_id,
                Err(TaskError::Killed {
                    reason: "killed by user".to_string(),
                }),
                None,
                None,
                actor_ref,
            )
            .await
        {
            Ok(_) => {
                if cleanup_pending {
                    "stop_pending_cleanup"
                } else {
                    "stopped"
                }
            }
            Err(StoreError::SessionNotFound(_)) => {
                return Err(ApiError::not_found(format!(
                    "Session '{session_id}' not found"
                )));
            }
            Err(err) => {
                error!(
                    session_id = %session_id,
                    error = %err,
                    "failed to transition session to Failed"
                );
                return Err(ApiError::internal(err));
            }
        }
    };

    // Revoke every auth token minted by this session so any request
    // still in flight from the dying container fails at `require_auth`
    // (401). Idempotent — safe to retry.
    state
        .store
        .revoke_auth_tokens_for_session(&session_id)
        .await
        .map_err(|err| {
            error!(session_id = %session_id, error = %err, "failed to revoke session tokens after kill");
            ApiError::internal(err)
        })?;

    info!(
        session_id = %session_id,
        status = kill_status,
        "kill_session completed successfully"
    );

    Ok(Json(v1::sessions::KillSessionResponse::new(
        session_id,
        kill_status.to_string(),
    )))
}
