use crate::{AppState, config::build_kube_client, routes::jobs::ApiError};
use anyhow::anyhow;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::{channel::mpsc, io::AsyncReadExt};
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{
    Api,
    api::{ListParams, LogParams},
};
use metis_common::logs::LogsQuery;
use std::convert::Infallible;
use tokio::time::{Duration, sleep};
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
    let job_id = job_id.to_string();

    let config = state.config;
    let namespace = config.metis.namespace.clone();
    let client = build_kube_client(&config.kubernetes).await.map_err(|err| {
        error!(error = ?err, "failed to build Kubernetes client for get_job_logs");
        ApiError::internal(err)
    })?;

    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let pods: Api<Pod> = Api::namespaced(client, &namespace);

    let job = match find_job_by_metis_id(&jobs, &job_id).await {
        Ok(job) => job,
        Err(err) => {
            error!(job_id = %job_id, error = ?err, "failed to find job by Metis ID");
            return Err(err);
        }
    };
    let job_name = match job.metadata.name.clone() {
        Some(name) => name,
        None => {
            let err = ApiError::internal(anyhow!("Job '{}' is missing a Kubernetes name.", job_id));
            error!(job_id = %job_id, error = ?err, "job missing Kubernetes name");
            return Err(err);
        }
    };

    let pod_name = match wait_for_pod_name(&pods, &job_name, &job_id).await {
        Ok(name) => name,
        Err(err) => {
            error!(
                job_id = %job_id,
                job_name = %job_name,
                error = ?err,
                "failed while waiting for pod name"
            );
            return Err(err);
        }
    };

    if watch_requested {
        info!(
            job_id = %job_id,
            job_name = %job_name,
            pod_name = %pod_name,
            "streaming job logs via SSE"
        );
        match stream_logs_sse(pods, pod_name, job_is_running(&job)).await {
            Ok(response) => Ok(response),
            Err(err) => {
                error!(
                    job_id = %job_id,
                    job_name = %job_name,
                    error = ?err,
                    "failed to stream logs via SSE"
                );
                Err(err)
            }
        }
    } else {
        info!(
            job_id = %job_id,
            job_name = %job_name,
            pod_name = %pod_name,
            "fetching job logs once"
        );
        match fetch_logs(&pods, &pod_name).await {
            Ok(response) => Ok(response),
            Err(err) => {
                error!(
                    job_id = %job_id,
                    job_name = %job_name,
                    error = ?err,
                    "failed to fetch logs"
                );
                Err(err)
            }
        }
    }
}

async fn fetch_logs(pods: &Api<Pod>, pod_name: &str) -> Result<Response, ApiError> {
    let mut params = LogParams::default();
    params.follow = false;

    let mut reader = pods
        .log_stream(pod_name, &params)
        .await
        .map_err(ApiError::internal)?;

    let mut buffer = Vec::new();
    let mut chunk = vec![0u8; 1024];

    loop {
        let read = reader.read(&mut chunk).await.map_err(ApiError::internal)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let logs = String::from_utf8_lossy(&buffer).to_string();

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
    pods: Api<Pod>,
    pod_name: String,
    follow: bool,
) -> Result<Response, ApiError> {
    let mut params = LogParams::default();
    params.follow = follow;

    let log_stream = pods
        .log_stream(&pod_name, &params)
        .await
        .map_err(ApiError::internal)?;

    let (tx, rx) = mpsc::unbounded::<Result<Event, Infallible>>();

    tokio::spawn(async move {
        let mut reader = log_stream;
        let mut buffer = vec![0u8; 1024];
        let sender = tx;

        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    if read == 0 {
                        continue;
                    }

                    let chunk = String::from_utf8_lossy(&buffer[..read]).to_string();
                    if sender
                        .unbounded_send(Ok(Event::default().data(chunk)))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err) => {
                    if sender
                        .unbounded_send(Ok(Event::default().event("error").data(err.to_string())))
                        .is_err()
                    {
                        break;
                    }
                    break;
                }
            }
        }
    });

    let sse_stream = rx;
    let sse = Sse::new(sse_stream).keep_alive(KeepAlive::default());

    Ok(sse.into_response())
}

async fn find_job_by_metis_id(jobs: &Api<Job>, job_id: &str) -> Result<Job, ApiError> {
    let selector = format!("metis-id={job_id}");
    let lp = ListParams::default().labels(&selector);
    let items = jobs.list(&lp).await.map_err(ApiError::internal)?.items;

    match items.len() {
        0 => Err(ApiError::not_found(format!(
            "No job found with Metis ID '{job_id}'."
        ))),
        1 => Ok(items
            .into_iter()
            .next()
            .expect("validated single job response")),
        _ => Err(ApiError::bad_request(format!(
            "Multiple jobs found with Metis ID '{job_id}'."
        ))),
    }
}

fn job_is_running(job: &Job) -> bool {
    job.status
        .as_ref()
        .map(|status| status.succeeded.unwrap_or(0) == 0 && status.failed.unwrap_or(0) == 0)
        .unwrap_or(true)
}

async fn wait_for_pod_name(
    pods: &Api<Pod>,
    job_name: &str,
    job_id: &str,
) -> Result<String, ApiError> {
    let selector = format!("job-name={job_name}");
    let lp = ListParams::default().labels(&selector);

    loop {
        let pod_list = pods.list(&lp).await.map_err(ApiError::internal)?;

        if let Some(mut pod) = pod_list
            .items
            .into_iter()
            .find(|pod| pod.metadata.name.is_some())
        {
            let pod_name = pod.metadata.name.take().expect("pod name missing");

            if let Some(phase) = pod.status.and_then(|status| status.phase) {
                match phase.as_str() {
                    "Running" => return Ok(pod_name),
                    "Failed" | "Succeeded" => {
                        return Err(ApiError::bad_request(format!(
                            "Pod '{}' for job '{}' reached terminal phase '{}' before running.",
                            pod_name, job_id, phase
                        )));
                    }
                    _ => {}
                }
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

