use crate::{
    command::spawn::{stream_pod_logs, wait_for_pod_name},
    config::{build_kube_client, AppConfig},
};
use anyhow::{anyhow, bail, Result};
use k8s_openapi::api::{batch::v1::Job, core::v1::Pod};
use kube::{api::ListParams, Api};

pub async fn run(config: &AppConfig, job: String, watch: bool) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let job_id = job_id.to_string();
    let namespace = &config.metis.namespace;
    let client = build_kube_client(&config.kubernetes).await?;
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let pods: Api<Pod> = Api::namespaced(client, namespace);

    let job = find_job_by_metis_id(&jobs, &job_id).await?;
    let job_name = job
        .metadata
        .name
        .clone()
        .ok_or_else(|| anyhow!("Job '{}' is missing a Kubernetes name.", job_id))?;
    let follow_logs = watch && job_is_running(&job);

    if follow_logs {
        println!(
            "Streaming logs for running job '{}' in namespace '{}'.",
            job_name, namespace
        );
    } else {
        println!(
            "Displaying logs for job '{}' in namespace '{}'.",
            job_name, namespace
        );
    }

    let pod_name = wait_for_pod_name(&pods, &job_name, &job_id).await?;
    stream_pod_logs(&pods, &pod_name, follow_logs).await
}

async fn find_job_by_metis_id(jobs: &Api<Job>, job_id: &str) -> Result<Job> {
    let selector = format!("metis-id={job_id}");
    let lp = ListParams::default().labels(&selector);
    let items = jobs.list(&lp).await?.items;

    match items.len() {
        0 => bail!("No job found with Metis ID '{job_id}'."),
        1 => Ok(items.into_iter().next().expect("validated single job")),
        _ => bail!("Multiple jobs found with Metis ID '{job_id}'."),
    }
}

fn job_is_running(job: &Job) -> bool {
    job.status
        .as_ref()
        .map(|status| status.succeeded.unwrap_or(0) == 0 && status.failed.unwrap_or(0) == 0)
        .unwrap_or(true)
}
