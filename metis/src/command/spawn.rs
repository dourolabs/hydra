use crate::{
    client::MetisClient,
    config::{build_kube_client, AppConfig},
};
use anyhow::{bail, Result};
use futures::io::AsyncReadExt;
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{
    api::{ListParams, LogParams},
    Api,
};
use metis_common::jobs::CreateJobRequest;
use std::{io::{self, Write}, time::Duration};
use tokio::time::sleep;

pub async fn run(
    config: &AppConfig,
    wait: bool,
    from_git_rev_arg: Option<String>,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = if prompt_parts.is_empty() {
        bail!("prompt is required")
    } else {
        prompt_parts.join(" ")
    };

    let client = MetisClient::from_config(config)?;
    let request = CreateJobRequest {
        prompt,
        from_git_rev: from_git_rev_arg,
    };
    let response = client.create_job(&request).await?;
    let job_id = response.job_id;
    let job_name = response.job_name;
    let namespace = response.namespace;

    println!(
        "Requested Metis job '{}' (id {}) in namespace '{}'.",
        job_name, job_id, namespace
    );

    if wait {
        let kube_client = build_kube_client(&config.kubernetes).await?;
        let jobs: Api<Job> = Api::namespaced(kube_client.clone(), &namespace);
        let pods: Api<Pod> = Api::namespaced(kube_client, &namespace);

        wait_for_job_completion(&jobs, &pods, &job_name, &job_id).await?;
    }

    Ok(())
}

async fn wait_for_job_completion(
    jobs: &Api<Job>,
    pods: &Api<Pod>,
    job_name: &str,
    job_uuid: &str,
) -> Result<()> {
    println!("Waiting for job '{}' to start running...", job_name);
    let pod_name = wait_for_pod_name(pods, job_name, job_uuid).await?;

    stream_pod_logs(pods, &pod_name, true).await?;

    let job = wait_for_terminal_job_state(jobs, job_name).await?;
    if let Some(status) = job.status {
        let succeeded = status.succeeded.unwrap_or(0);
        if succeeded > 0 {
            println!("Job '{}' completed successfully.", job_name);
            return Ok(());
        }
        let failed = status.failed.unwrap_or(0);
        if failed > 0 {
            bail!("Job '{}' failed ({} failed pods).", job_name, failed);
        }
    }

    bail!("Job '{}' completed without a final status.", job_name);
}

pub(crate) async fn wait_for_pod_name(
    pods: &Api<Pod>,
    job_name: &str,
    job_uuid: &str,
) -> Result<String> {
    let selector = format!("job-name={job_name}");
    let lp = ListParams::default().labels(&selector);

    loop {
        let pod_list = pods.list(&lp).await?;
        if let Some(mut pod) = pod_list
            .items
            .into_iter()
            .find(|pod| pod.metadata.name.is_some())
        {
            let pod_name = pod.metadata.name.take().expect("pod name missing");
            println!("Found pod for job '{}'.", job_uuid);

            if let Some(phase) = pod.status.and_then(|status| status.phase) {
                match phase.as_str() {
                    "Running" => {
                        println!("Job '{}' pod is running.", job_uuid);
                        return Ok(pod_name);
                    }
                    "Failed" | "Succeeded" => {
                        bail!(
                            "Pod '{}' reached terminal phase '{}' before running.",
                            pod_name,
                            phase
                        );
                    }
                    _ => {
                        println!(
                            "Job '{}' pod is currently in phase '{}'. Waiting for it to run...",
                            job_uuid, phase
                        );
                    }
                }
            } else {
                println!(
                    "Job '{}' pod status not yet available. Waiting for it to run...",
                    job_uuid
                );
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

pub(crate) async fn stream_pod_logs(pods: &Api<Pod>, pod_name: &str, follow: bool) -> Result<()> {
    let mut log_params = LogParams::default();
    log_params.follow = follow;

    let mut log_stream = pods.log_stream(pod_name, &log_params).await?;
    let mut buffer = vec![0u8; 1024];

    loop {
        let read = log_stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }

        io::stdout().write_all(&buffer[..read])?;
        io::stdout().flush()?;
    }

    Ok(())
}

async fn wait_for_terminal_job_state(jobs: &Api<Job>, job_name: &str) -> Result<Job> {
    loop {
        let job = jobs.get(job_name).await?;
        if let Some(status) = job.status.as_ref() {
            if status.succeeded.unwrap_or(0) > 0 || status.failed.unwrap_or(0) > 0 {
                return Ok(job);
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}
