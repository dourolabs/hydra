use crate::config::{build_kube_client, AppConfig};
use anyhow::{anyhow, bail, Result};
use futures::io::AsyncReadExt;
use k8s_openapi::{
    api::{
        batch::v1::{Job, JobSpec},
        core::v1::{Container, EnvVar, Pod, PodSpec, PodTemplateSpec},
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    api::{ListParams, LogParams, PostParams},
    Api, Error as KubeError,
};
use std::{
    collections::BTreeMap,
    env,
    io::{self, Write},
    time::Duration,
};
use tokio::time::sleep;
use uuid::Uuid;

pub async fn run(config: &AppConfig, label: Option<String>, wait: bool) -> Result<()> {
    let namespace = config.metis.namespace.clone();
    let worker_image = config.metis.worker_image.clone();
    let job_name = format!("metis-worker-{}", Uuid::new_v4().hyphenated());
    let client = build_kube_client(&config.kubernetes).await?;

    let openai_api_key = env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| config.metis.openai_api_key.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("OPENAI_API_KEY is not set. Provide it via the environment or config.toml.")
        })?;

    let jobs: Api<Job> = Api::namespaced(client.clone(), &namespace);
    let pods: Api<Pod> = Api::namespaced(client, &namespace);

    let mut metadata_labels = BTreeMap::new();
    metadata_labels.insert("metis-worker".to_string(), job_name.clone());
    // TODO: this isn't really necessary but let's leave it for now.
    if let Some(custom_label) = label.filter(|value| !value.trim().is_empty()) {
        metadata_labels.insert("metis-label".to_string(), custom_label);
    }

    let job = Job {
        metadata: ObjectMeta {
            name: Some(job_name.clone()),
            labels: Some(metadata_labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(metadata_labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "metis-worker".to_string(),
                        image: Some(worker_image),
                        command: Some(vec![
                            "codex".into(),
                            "exec".into(),
                            "print hello world".into(),
                        ]),
                        env: Some(vec![EnvVar {
                            name: "OPENAI_API_KEY".to_string(),
                            value: Some(openai_api_key),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }],
                    restart_policy: Some("Never".into()),
                    ..Default::default()
                }),
            },
            backoff_limit: Some(0),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pp = PostParams::default();

    match jobs.create(&pp, &job).await {
        Ok(created) => {
            let display_name = created
                .metadata
                .name
                .clone()
                .unwrap_or_else(|| job_name.clone());
            println!(
                "Spawned Kubernetes job '{}' in namespace '{}'.",
                display_name, namespace
            );
        }
        Err(KubeError::Api(err)) if err.code == 409 => {
            println!(
                "Job '{}' already exists in namespace '{}'.",
                job_name, namespace
            );
        }
        Err(err) => return Err(err.into()),
    }

    if wait {
        wait_for_job_completion(&jobs, &pods, &job_name).await?;
    }

    Ok(())
}

async fn wait_for_job_completion(jobs: &Api<Job>, pods: &Api<Pod>, job_name: &str) -> Result<()> {
    println!("Waiting for job '{}' to start running...", job_name);
    let pod_name = wait_for_pod_name(pods, job_name).await?;

    stream_pod_logs(pods, &pod_name).await?;

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

async fn wait_for_pod_name(pods: &Api<Pod>, job_name: &str) -> Result<String> {
    let selector = format!("job-name={job_name}");
    let lp = ListParams::default().labels(&selector);

    loop {
        let pod_list = pods.list(&lp).await?;
        if let Some(pod_name) = pod_list.items.into_iter().find_map(|pod| pod.metadata.name) {
            println!("Streaming logs from pod '{}'...", pod_name);
            return Ok(pod_name);
        }

        sleep(Duration::from_secs(1)).await;
    }
}

async fn stream_pod_logs(pods: &Api<Pod>, pod_name: &str) -> Result<()> {
    let mut log_params = LogParams::default();
    log_params.follow = true;

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
