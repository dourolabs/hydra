use crate::{AppState, job_store::JobStoreError, routes::jobs::ApiError};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{channel::mpsc, StreamExt};
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
    let job = state.job_store.find_job_by_metis_id(&job_id.to_string()).await
        .map_err(|err| match err {
            JobStoreError::NotFound(msg) => {
                error!(job_id = %job_id, error = %msg, "job not found");
                ApiError::not_found(msg)
            }
            JobStoreError::MultipleFound(msg) => {
                error!(job_id = %job_id, error = %msg, "multiple jobs found");
                ApiError::bad_request(msg)
            }
            err => {
                error!(job_id = %job_id, error = ?err, "failed to find job");
                ApiError::internal(err)
            }
        })?;

    let follow = watch_requested && job.status == "running";

    if watch_requested {
        info!(
            job_id = %job_id,
            follow = follow,
            "streaming job logs via SSE"
        );
        stream_logs_sse(state.job_store.as_ref(), job_id, follow).await
    } else {
        info!(
            job_id = %job_id,
            "fetching job logs once"
        );
        fetch_logs(state.job_store.as_ref(), job_id).await
    }
}

async fn fetch_logs(
    job_store: &dyn crate::job_store::JobStore,
    job_id: &str,
) -> Result<Response, ApiError> {
    let logs = job_store.get_logs(job_id).await.map_err(|err| {
        error!(job_id = %job_id, error = ?err, "failed to fetch logs");
        match err {
            JobStoreError::NotFound(msg) => ApiError::not_found(msg),
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
    job_store: &dyn crate::job_store::JobStore,
    job_id: &str,
    follow: bool,
) -> Result<Response, ApiError> {
    let mut receiver = job_store.get_logs_stream(job_id, follow).map_err(|err| {
        error!(job_id = %job_id, error = ?err, "failed to create log stream");
        match err {
            JobStoreError::NotFound(msg) => ApiError::not_found(msg),
            err => ApiError::internal(err),
        }
    })?;

    let (tx, rx) = mpsc::unbounded::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        let sender = tx;
        loop {
            match receiver.next().await {
                Some(chunk) => {
                    if sender
                        .unbounded_send(Ok(Event::default().data(chunk)))
                        .is_err()
                    {
                        break;
                    }
                }
                None => break,
            }
        }
    });

    let sse_stream = rx;
    let sse = Sse::new(sse_stream).keep_alive(KeepAlive::default());

    Ok(sse.into_response())
}

