use crate::{
    app::AppState,
    job_engine::{JobEngineError, JobStatus, SessionId},
    routes::sessions::{ApiError, SessionIdPath},
};
use axum::{
    extract::{Query, State},
    http::{HeaderValue, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{StreamExt, channel::mpsc};
use hydra_common::api::v1::logs::LogsQuery;
use std::convert::Infallible;
use tracing::{error, info};

pub async fn get_session_logs(
    State(state): State<AppState>,
    SessionIdPath(session_id): SessionIdPath,
    Query(query): Query<LogsQuery>,
) -> Result<Response, ApiError> {
    let watch_requested = query.watch.unwrap_or(false);
    let tail_lines = query.tail_lines;
    info!(
        session_id = %session_id,
        watch = watch_requested,
        "get_session_logs invoked"
    );

    // Check if session exists and get its status to determine if we should follow logs
    let job = state
        .job_engine
        .find_job_by_hydra_id(&session_id)
        .await
        .map_err(|err| match err {
            JobEngineError::NotFound(hydra_id) => {
                let message = format!("Session '{hydra_id}' not found");
                error!(session_id = %session_id, error = %message, "session not found");
                ApiError::not_found(message)
            }
            JobEngineError::MultipleFound(hydra_id) => {
                let message = format!("Multiple sessions found for hydra-id '{hydra_id}'");
                error!(session_id = %session_id, error = %message, "multiple sessions found");
                ApiError::bad_request(message)
            }
            err => {
                error!(session_id = %session_id, error = ?err, "failed to find session");
                ApiError::internal(err)
            }
        })?;

    let follow = watch_requested && job.status == JobStatus::Running;

    if watch_requested {
        info!(
            session_id = %session_id,
            follow = follow,
            "streaming session logs via SSE"
        );
        let response = stream_logs_sse(state.job_engine.as_ref(), &session_id, follow).await?;
        info!(
            session_id = %session_id,
            follow = follow,
            "get_session_logs streaming response ready"
        );
        Ok(response)
    } else {
        info!(
            session_id = %session_id,
            "fetching session logs once"
        );
        let response = fetch_logs(state.job_engine.as_ref(), &session_id, tail_lines).await?;
        info!(
            session_id = %session_id,
            tail_lines = ?tail_lines,
            "get_session_logs returning log snapshot"
        );
        Ok(response)
    }
}

async fn fetch_logs(
    job_engine: &dyn crate::job_engine::JobEngine,
    session_id: &SessionId,
    tail_lines: Option<i64>,
) -> Result<Response, ApiError> {
    let logs = job_engine
        .get_logs(session_id, tail_lines)
        .await
        .map_err(|err| {
            error!(session_id = %session_id, error = ?err, "failed to fetch logs");
            match err {
                JobEngineError::NotFound(hydra_id) => {
                    ApiError::not_found(format!("Session '{hydra_id}' not found"))
                }
                err => ApiError::internal(err),
            }
        })?;

    let byte_len = logs.len();
    info!(
        session_id = %session_id,
        tail_lines = ?tail_lines,
        byte_len,
        "prepared single-shot log response"
    );

    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        logs,
    )
        .into_response())
}

async fn stream_logs_sse(
    job_engine: &dyn crate::job_engine::JobEngine,
    session_id: &SessionId,
    follow: bool,
) -> Result<Response, ApiError> {
    let mut receiver = job_engine
        .get_logs_stream(session_id, follow)
        .map_err(|err| {
            error!(session_id = %session_id, error = ?err, "failed to create log stream");
            match err {
                JobEngineError::NotFound(hydra_id) => {
                    ApiError::not_found(format!("Session '{hydra_id}' not found"))
                }
                err => ApiError::internal(err),
            }
        })?;

    let (tx, rx) = mpsc::unbounded::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        let sender = tx;
        while let Some(chunk) = receiver.next().await {
            if sender
                .unbounded_send(Ok(Event::default().data(chunk)))
                .is_err()
            {
                break;
            }
        }
    });

    let sse_stream = rx;
    let sse = Sse::new(sse_stream).keep_alive(KeepAlive::default());

    let response = sse.into_response();
    info!(
        session_id = %session_id,
        follow = follow,
        "prepared SSE log response"
    );
    Ok(response)
}
