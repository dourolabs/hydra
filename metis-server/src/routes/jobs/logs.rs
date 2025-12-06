use crate::{
    AppState,
    job_engine::{JobEngineError, JobStatus},
    routes::jobs::ApiError,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{StreamExt, channel::mpsc};
use metis_common::logs::LogsQuery;
use std::convert::Infallible;
use tracing::{error, info};

pub async fn get_job_logs(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Response, ApiError> {
    let watch_requested = query.watch.unwrap_or(false);
    let job_id = job_id.trim();
    info!(
        job_id = %job_id,
        watch = watch_requested,
        "get_job_logs invoked"
    );
    if job_id.is_empty() {
        error!("get_job_logs received an empty job_id");
        return Err(ApiError::bad_request("job_id must not be empty"));
    }

    // Check if job exists and get its status to determine if we should follow logs
    let job = state
        .job_engine
        .find_job_by_metis_id(&job_id.to_string())
        .await
        .map_err(|err| match err {
            JobEngineError::NotFound(msg) => {
                error!(job_id = %job_id, error = %msg, "job not found");
                ApiError::not_found(msg)
            }
            JobEngineError::MultipleFound(msg) => {
                error!(job_id = %job_id, error = %msg, "multiple jobs found");
                ApiError::bad_request(msg)
            }
            err => {
                error!(job_id = %job_id, error = ?err, "failed to find job");
                ApiError::internal(err)
            }
        })?;

    let follow = watch_requested && job.status == JobStatus::Running;

    if watch_requested {
        info!(
            job_id = %job_id,
            follow = follow,
            "streaming job logs via SSE"
        );
        stream_logs_sse(state.job_engine.as_ref(), job_id, follow).await
    } else {
        info!(
            job_id = %job_id,
            "fetching job logs once"
        );
        fetch_logs(state.job_engine.as_ref(), job_id, query.tail_lines).await
    }
}

async fn fetch_logs(
    job_engine: &dyn crate::job_engine::JobEngine,
    job_id: &str,
    tail_lines: Option<i64>,
) -> Result<Response, ApiError> {
    let logs = job_engine.get_logs(job_id, tail_lines).await.map_err(|err| {
        error!(job_id = %job_id, error = ?err, "failed to fetch logs");
        match err {
            JobEngineError::NotFound(msg) => ApiError::not_found(msg),
            err => ApiError::internal(err),
        }
    })?;

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
    job_id: &str,
    follow: bool,
) -> Result<Response, ApiError> {
    let mut receiver = job_engine.get_logs_stream(job_id, follow).map_err(|err| {
        error!(job_id = %job_id, error = ?err, "failed to create log stream");
        match err {
            JobEngineError::NotFound(msg) => ApiError::not_found(msg),
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

    Ok(sse.into_response())
}
