use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{channel::mpsc, io::AsyncReadExt};
use k8s_openapi::{
    api::{
        batch::v1::{Job, JobSpec, JobStatus as KubeJobStatus},
        core::v1::{Container, EnvVar, Pod, PodSpec, PodTemplateSpec},
    },
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, LogParams, PostParams},
};
use metis_common::constants::{ENV_METIS_ID, ENV_METIS_SERVER_URL, ENV_OPENAI_API_KEY};
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use super::{JobEngine, JobEngineError, JobStatus, MetisId, MetisJob, MetisPod};

pub struct KubernetesJobEngine {
    pub namespace: String,
    pub openai_api_key: String,
    pub server_hostname: String,
    pub client: Client,
}

impl KubernetesJobEngine {
    fn build_metadata_labels(job_uuid: &MetisId) -> BTreeMap<String, String> {
        let mut metadata_labels = BTreeMap::new();
        metadata_labels.insert("metis-id".to_string(), job_uuid.to_string());
        metadata_labels
    }

    fn build_env_vars(&self, job_uuid: &MetisId) -> Option<Vec<EnvVar>> {
        let mut vars = vec![
            EnvVar {
                name: ENV_OPENAI_API_KEY.to_string(),
                value: Some(self.openai_api_key.clone()),
                ..Default::default()
            },
            EnvVar {
                name: ENV_METIS_ID.to_string(),
                value: Some(job_uuid.to_string()),
                ..Default::default()
            },
        ];

        if !self.server_hostname.trim().is_empty() {
            vars.push(EnvVar {
                name: ENV_METIS_SERVER_URL.to_string(),
                value: Some(format!("http://{}", self.server_hostname.trim())),
                ..Default::default()
            });
        }

        Some(vars)
    }

    fn job_status(job: &Job) -> JobStatus {
        if let Some(status) = job.status.as_ref() {
            if status.succeeded.unwrap_or(0) > 0 {
                return JobStatus::Complete;
            }
            if status.failed.unwrap_or(0) > 0 {
                return JobStatus::Failed;
            }
        }

        JobStatus::Running
    }

    fn job_metis_id(job: &Job) -> Option<MetisId> {
        job.metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("metis-id"))
            .cloned()
    }

    fn job_end_time(job: &Job) -> Option<DateTime<Utc>> {
        let status = job.status.as_ref()?;

        if status.succeeded.unwrap_or(0) > 0 {
            if let Some(completion_time) = status.completion_time.as_ref() {
                return Some(completion_time.0);
            }

            if let Some(time) = Self::condition_time(status, "Complete") {
                return Some(time);
            }
        }

        if status.failed.unwrap_or(0) > 0 {
            if let Some(time) = Self::condition_time(status, "Failed") {
                return Some(time);
            }
        }

        None
    }

    fn condition_time(status: &KubeJobStatus, kind: &str) -> Option<DateTime<Utc>> {
        status
            .conditions
            .as_ref()
            .and_then(|conditions| {
                conditions
                    .iter()
                    .find(|condition| condition.type_ == kind)
                    .and_then(|condition| condition.last_transition_time.as_ref())
            })
            .map(|time| time.0)
    }

    fn job_failure_message(job: &Job) -> Option<String> {
        let status = job.status.as_ref()?;
        let conditions = status.conditions.as_ref()?;
        let failed_condition = conditions
            .iter()
            .find(|condition| condition.type_ == "Failed")?;

        failed_condition
            .message
            .clone()
            .or_else(|| failed_condition.reason.clone())
    }

    fn to_metis_job(job: &Job) -> Result<MetisJob, JobEngineError> {
        let job_name = job
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string());
        let id = Self::job_metis_id(job).ok_or_else(|| {
            JobEngineError::Internal(format!("Job '{job_name}' is missing metis-id label"))
        })?;
        let status = Self::job_status(job);
        let creation_time = job.metadata.creation_timestamp.as_ref().map(|t| t.0);
        let start_time = job
            .status
            .as_ref()
            .and_then(|s| s.start_time.as_ref())
            .map(|t| t.0);
        let completion_time = Self::job_end_time(job);
        let failure_message = Self::job_failure_message(job);

        Ok(MetisJob {
            id,
            status,
            creation_time,
            start_time,
            completion_time,
            failure_message,
        })
    }

    fn pod_metis_id(pod: &Pod) -> Option<MetisId> {
        pod.metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("metis-id"))
            .cloned()
    }

    fn to_metis_pod(pod: &Pod) -> Option<MetisPod> {
        let name = pod.metadata.name.clone()?;
        let metis_id = Self::pod_metis_id(pod)?;

        Some(MetisPod { name, metis_id })
    }

    async fn find_kubernetes_job_by_metis_id(
        &self,
        job_id: &MetisId,
    ) -> Result<Job, JobEngineError> {
        find_kubernetes_job_by_metis_id_impl(&self.client, &self.namespace, job_id).await
    }

    async fn wait_for_pod_name(&self, job_name: &str) -> Result<String, JobEngineError> {
        wait_for_pod_name_impl(&self.client, &self.namespace, job_name).await
    }
}

async fn find_kubernetes_job_by_metis_id_impl(
    client: &Client,
    namespace: &str,
    job_id: &MetisId,
) -> Result<Job, JobEngineError> {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let selector = format!("metis-id={job_id}");
    let lp = ListParams::default().labels(&selector);
    let items = jobs
        .list(&lp)
        .await
        .map_err(JobEngineError::Kubernetes)?
        .items;

    match items.len() {
        0 => Err(JobEngineError::NotFound(job_id.clone())),
        1 => Ok(items
            .into_iter()
            .next()
            .expect("validated single job response")),
        _ => Err(JobEngineError::MultipleFound(job_id.clone())),
    }
}

async fn wait_for_pod_name_impl(
    client: &Client,
    namespace: &str,
    job_name: &str,
) -> Result<String, JobEngineError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let selector = format!("job-name={job_name}");
    let lp = ListParams::default().labels(&selector);

    loop {
        let pod_list = pods.list(&lp).await.map_err(JobEngineError::Kubernetes)?;

        if let Some(mut pod) = pod_list
            .items
            .into_iter()
            .find(|pod| pod.metadata.name.is_some())
        {
            let pod_name = pod.metadata.name.take().expect("pod name missing");

            if let Some(phase) = pod.status.and_then(|status| status.phase) {
                match phase.as_str() {
                    // Allow terminal phases so completed jobs can still return logs.
                    "Running" | "Failed" | "Succeeded" => return Ok(pod_name),
                    _ => {}
                }
            }
        }

        sleep(Duration::from_secs(1)).await;
    }
}

#[async_trait]
impl JobEngine for KubernetesJobEngine {
    async fn create_job(&self, metis_id: &MetisId, image: &str) -> Result<(), JobEngineError> {
        let job_name = format!("metis-worker-{metis_id}");

        info!(metis_id = %metis_id, namespace = %self.namespace, "creating Kubernetes job");

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let metadata_labels = Self::build_metadata_labels(metis_id);

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
                            image: Some(image.to_string()),
                            image_pull_policy: Some("IfNotPresent".into()),
                            args: None,
                            env: self.build_env_vars(metis_id),
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
                let display_name = created.metadata.name.clone().unwrap_or(job_name.clone());
                info!(
                    metis_id = %metis_id,
                    job_name = %display_name,
                    namespace = %self.namespace,
                    "job created successfully"
                );

                Ok(())
            }
            Err(kube::Error::Api(err)) if err.code == 409 => {
                error!(
                    job_name = %job_name,
                    namespace = %self.namespace,
                    code = err.code,
                    "job already exists"
                );
                Err(JobEngineError::AlreadyExists(metis_id.clone()))
            }
            Err(err) => {
                error!(job_name = %job_name, error = ?err, "failed to create job in Kubernetes");
                Err(JobEngineError::Kubernetes(err))
            }
        }
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        info!(namespace = %self.namespace, "listing Kubernetes jobs");

        let jobs_api: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let jobs = jobs_api
            .list(&ListParams::default().labels("metis-id"))
            .await
            .map_err(|err| {
                error!(error = ?err, namespace = %self.namespace, "failed to list jobs from Kubernetes");
                JobEngineError::Kubernetes(err)
            })?;

        let mut metis_jobs: Vec<MetisJob> = jobs
            .into_iter()
            .filter_map(|job| Self::to_metis_job(&job).ok())
            .collect();

        // Sort by reference time (start_time or creation_time), most recent first
        metis_jobs.sort_by(|a, b| {
            let time_a = a.start_time.or(a.creation_time);
            let time_b = b.start_time.or(b.creation_time);
            time_b.cmp(&time_a)
        });

        info!(
            namespace = %self.namespace,
            job_count = metis_jobs.len(),
            "list_jobs completed successfully"
        );

        Ok(metis_jobs)
    }

    async fn list_pods(&self) -> Result<Vec<MetisPod>, JobEngineError> {
        info!(namespace = %self.namespace, "listing Kubernetes pods for metis jobs");

        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let pods = pods_api
            .list(&ListParams::default().labels("metis-id"))
            .await
            .map_err(|err| {
                error!(error = ?err, namespace = %self.namespace, "failed to list pods from Kubernetes");
                JobEngineError::Kubernetes(err)
            })?;

        let mut metis_pods: Vec<MetisPod> = Vec::new();

        for pod in pods {
            match Self::to_metis_pod(&pod) {
                Some(pod) => metis_pods.push(pod),
                None => warn!(
                    namespace = %self.namespace,
                    "skipping pod missing metis-id label or name"
                ),
            }
        }

        Ok(metis_pods)
    }

    async fn find_job_by_metis_id(&self, metis_id: &MetisId) -> Result<MetisJob, JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(metis_id).await?;
        Self::to_metis_job(&job)
    }

    async fn get_logs(
        &self,
        job_id: &MetisId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(job_id).await?;
        let job_name = job.metadata.name.ok_or_else(|| {
            JobEngineError::Internal(format!("Job '{job_id}' is missing a Kubernetes name."))
        })?;

        let pod_name = self.wait_for_pod_name(&job_name).await?;

        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let params = LogParams {
            follow: false,
            tail_lines,
            ..Default::default()
        };

        let mut reader = pods
            .log_stream(&pod_name, &params)
            .await
            .map_err(JobEngineError::Kubernetes)?;

        let mut buffer = Vec::new();
        let mut chunk = vec![0u8; 1024];

        loop {
            let read = reader
                .read(&mut chunk)
                .await
                .map_err(|err| JobEngineError::Io(std::io::Error::other(err)))?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }

        Ok(String::from_utf8_lossy(&buffer).to_string())
    }

    fn get_logs_stream(
        &self,
        job_id: &MetisId,
        follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let (tx, rx) = mpsc::unbounded();

        let namespace = self.namespace.clone();
        let client = self.client.clone();
        let job_id = job_id.clone();
        let sender = tx;

        tokio::spawn(async move {
            match find_kubernetes_job_by_metis_id_impl(&client, &namespace, &job_id).await {
                Ok(job) => {
                    let job_name = match job.metadata.name {
                        Some(name) => name,
                        None => {
                            let _ = sender.unbounded_send(format!(
                                "Error: Job '{job_id}' is missing a Kubernetes name."
                            ));
                            return;
                        }
                    };

                    match wait_for_pod_name_impl(&client, &namespace, &job_name).await {
                        Ok(pod_name) => {
                            let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
                            let params = LogParams {
                                follow,
                                ..Default::default()
                            };

                            match pods.log_stream(&pod_name, &params).await {
                                Ok(mut reader) => {
                                    let mut buffer = vec![0u8; 1024];

                                    loop {
                                        match reader.read(&mut buffer).await {
                                            Ok(0) => break,
                                            Ok(read) => {
                                                if read == 0 {
                                                    continue;
                                                }

                                                let chunk =
                                                    String::from_utf8_lossy(&buffer[..read])
                                                        .to_string();
                                                if sender.unbounded_send(chunk).is_err() {
                                                    break;
                                                }
                                            }
                                            Err(err) => {
                                                let _ =
                                                    sender.unbounded_send(format!("Error: {err}"));
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    let _ = sender.unbounded_send(format!("Error: {err}"));
                                }
                            }
                        }
                        Err(err) => {
                            let _ = sender.unbounded_send(format!("Error: {err}"));
                        }
                    }
                }
                Err(err) => {
                    let _ = sender.unbounded_send(format!("Error: {err}"));
                }
            }
        });

        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &MetisId) -> Result<(), JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(metis_id).await?;
        let job_name = job.metadata.name.ok_or_else(|| {
            JobEngineError::Internal(format!("Job '{metis_id}' is missing a Kubernetes name."))
        })?;

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let dp = DeleteParams::default();

        match jobs.delete(&job_name, &dp).await {
            Ok(_) => {
                info!(
                    metis_id = %metis_id,
                    job_name = %job_name,
                    namespace = %self.namespace,
                    "job deleted successfully"
                );
                Ok(())
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {
                Err(JobEngineError::NotFound(metis_id.clone()))
            }
            Err(err) => {
                error!(
                    metis_id = %metis_id,
                    job_name = %job_name,
                    namespace = %self.namespace,
                    error = ?err,
                    "failed to delete job"
                );
                Err(JobEngineError::Kubernetes(err))
            }
        }
    }

    async fn delete_pods_for_metis_id(&self, metis_id: &MetisId) -> Result<(), JobEngineError> {
        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let selector = format!("metis-id={metis_id}");
        let lp = ListParams::default().labels(&selector);
        let pod_list = pods_api
            .list(&lp)
            .await
            .map_err(|err| {
                error!(metis_id = %metis_id, namespace = %self.namespace, error = ?err, "failed to list pods for cleanup");
                JobEngineError::Kubernetes(err)
            })?;

        if pod_list.items.is_empty() {
            return Ok(());
        }

        let dp = DeleteParams::default();
        for mut pod in pod_list.items {
            match pod.metadata.name.take() {
                Some(pod_name) => match pods_api.delete(&pod_name, &dp).await {
                    Ok(_) => info!(
                        metis_id = %metis_id,
                        pod_name = %pod_name,
                        namespace = %self.namespace,
                        "pod deleted successfully"
                    ),
                    Err(kube::Error::Api(err)) if err.code == 404 => {
                        warn!(
                            metis_id = %metis_id,
                            pod_name = %pod_name,
                            namespace = %self.namespace,
                            "pod already deleted before cleanup"
                        );
                    }
                    Err(err) => {
                        error!(
                            metis_id = %metis_id,
                            pod_name = %pod_name,
                            namespace = %self.namespace,
                            error = ?err,
                            "failed to delete pod"
                        );
                        return Err(JobEngineError::Kubernetes(err));
                    }
                },
                None => warn!(
                    metis_id = %metis_id,
                    namespace = %self.namespace,
                    "encountered pod without name during cleanup"
                ),
            }
        }

        Ok(())
    }
}
