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
    api::{DeleteParams, ListParams, LogParams, PostParams},
    Api, Client,
};
use tokio::time::{sleep, Duration};
use tracing::{error, info};
use uuid::Uuid;

use super::{JobStatus, JobEngine, JobEngineError, MetisId, MetisJob};

pub struct KubernetesJobEngine {
    pub namespace: String,
    pub worker_image: String,
    pub openai_api_key: String,
    pub server_hostname: String,
    pub client: Client,
}

impl KubernetesJobEngine {
    fn build_metadata_labels(job_uuid: &str) -> BTreeMap<String, String> {
        let mut metadata_labels = BTreeMap::new();
        metadata_labels.insert("metis-id".to_string(), job_uuid.to_string());
        metadata_labels
    }

    fn build_env_vars(&self, job_uuid: &str) -> Option<Vec<EnvVar>> {
        let mut vars = vec![
            EnvVar {
                name: "OPENAI_API_KEY".to_string(),
                value: Some(self.openai_api_key.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "METIS_ID".to_string(),
                value: Some(job_uuid.to_string()),
                ..Default::default()
            },
        ];

        if !self.server_hostname.trim().is_empty() {
            vars.push(EnvVar {
                name: "METIS_SERVER_URL".to_string(),
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

    fn job_metis_id(job: &Job) -> Option<String> {
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
                return Some(completion_time.0.clone());
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
            .map(|time| time.0.clone())
    }

    fn job_failure_message(job: &Job) -> Option<String> {
        let status = job.status.as_ref()?;
        let conditions = status.conditions.as_ref()?;
        let failed_condition = conditions.iter().find(|condition| condition.type_ == "Failed")?;

        failed_condition
            .message
            .clone()
            .or_else(|| failed_condition.reason.clone())
    }

    async fn find_kubernetes_job_by_metis_id(&self, job_id: &str) -> Result<Job, JobEngineError> {
        find_kubernetes_job_by_metis_id_impl(&self.client, &self.namespace, job_id).await
    }

    async fn wait_for_pod_name(&self, job_name: &str, job_id: &str) -> Result<String, JobEngineError> {
        wait_for_pod_name_impl(&self.client, &self.namespace, job_name, job_id).await
    }
}

async fn find_kubernetes_job_by_metis_id_impl(
    client: &Client,
    namespace: &str,
    job_id: &str,
) -> Result<Job, JobEngineError> {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let selector = format!("metis-id={job_id}");
    let lp = ListParams::default().labels(&selector);
    let items = jobs.list(&lp).await.map_err(JobEngineError::Kubernetes)?.items;

    match items.len() {
        0 => Err(JobEngineError::NotFound(format!(
            "No job found with Metis ID '{job_id}'."
        ))),
        1 => Ok(items
            .into_iter()
            .next()
            .expect("validated single job response")),
        _ => Err(JobEngineError::MultipleFound(format!(
            "Multiple jobs found with Metis ID '{job_id}'."
        ))),
    }
}

async fn wait_for_pod_name_impl(
    client: &Client,
    namespace: &str,
    job_name: &str,
    job_id: &str,
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
                    "Running" => return Ok(pod_name),
                    "Failed" | "Succeeded" => {
                        return Err(JobEngineError::Internal(format!(
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

#[async_trait]
impl JobEngine for KubernetesJobEngine {
    async fn create_job(&self, prompt: &str) -> Result<MetisId, JobEngineError> {
        let job_uuid = Uuid::new_v4().hyphenated().to_string();
        let job_name = format!("metis-worker-{}", job_uuid);
        
        info!(job_uuid = %job_uuid, namespace = %self.namespace, "creating Kubernetes job");
        
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let metadata_labels = Self::build_metadata_labels(&job_uuid);

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
                            image: Some(self.worker_image.clone()),
                            image_pull_policy: Some("IfNotPresent".into()),
                            args: Some(vec![
                                "codex".into(),
                                "exec".into(),
                                "-o".into(),
                                "output.txt".into(),
                                "--dangerously-bypass-approvals-and-sandbox".into(),
                                prompt.to_string(),
                            ]),
                            env: self.build_env_vars(&job_uuid),
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
                    job_uuid = %job_uuid,
                    job_name = %display_name,
                    namespace = %self.namespace,
                    "job created successfully"
                );

                Ok(job_uuid)
            }
            Err(kube::Error::Api(err)) if err.code == 409 => {
                error!(
                    job_name = %job_name,
                    namespace = %self.namespace,
                    code = err.code,
                    "job already exists"
                );
                Err(JobEngineError::AlreadyExists(format!(
                    "Job '{}' already exists in namespace '{}'",
                    job_name, self.namespace
                )))
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
            .filter_map(|job| {
                let id = Self::job_metis_id(&job)?;
                let status = Self::job_status(&job);
                let creation_time = job.metadata.creation_timestamp.as_ref().map(|t| t.0.clone());
                let start_time = job.status.as_ref()
                    .and_then(|s| s.start_time.as_ref())
                    .map(|t| t.0.clone());
                let completion_time = Self::job_end_time(&job);
                let failure_message = Self::job_failure_message(&job);

                Some(MetisJob {
                    id,
                    status,
                    creation_time,
                    start_time,
                    completion_time,
                    failure_message,
                })
            })
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

    async fn find_job_by_metis_id(&self, metis_id: &MetisId) -> Result<MetisJob, JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(metis_id).await?;
        let id = Self::job_metis_id(&job).ok_or_else(|| {
            JobEngineError::Internal(format!("Job '{}' is missing metis-id label", metis_id))
        })?;
        let status = Self::job_status(&job);
        let creation_time = job.metadata.creation_timestamp.as_ref().map(|t| t.0.clone());
        let start_time = job.status.as_ref()
            .and_then(|s| s.start_time.as_ref())
            .map(|t| t.0.clone());
        let completion_time = Self::job_end_time(&job);
        let failure_message = Self::job_failure_message(&job);

        Ok(MetisJob {
            id,
            status,
            creation_time,
            start_time,
            completion_time,
            failure_message,
        })
    }

    async fn get_logs(&self, job_id: &str) -> Result<String, JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(job_id).await?;
        let job_name = job.metadata.name.ok_or_else(|| {
            JobEngineError::Internal(format!("Job '{}' is missing a Kubernetes name.", job_id))
        })?;

        let pod_name = self.wait_for_pod_name(&job_name, job_id).await?;
        
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let mut params = LogParams::default();
        params.follow = false;

        let mut reader = pods
            .log_stream(&pod_name, &params)
            .await
            .map_err(JobEngineError::Kubernetes)?;

        let mut buffer = Vec::new();
        let mut chunk = vec![0u8; 1024];

        loop {
            let read = reader.read(&mut chunk).await.map_err(|err| {
                JobEngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, err))
            })?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }

        Ok(String::from_utf8_lossy(&buffer).to_string())
    }

    fn get_logs_stream(
        &self,
        job_id: &str,
        follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let (tx, rx) = mpsc::unbounded();
        
        let namespace = self.namespace.clone();
        let client = self.client.clone();
        let job_id = job_id.to_string();
        let sender = tx;

        tokio::spawn(async move {
            match find_kubernetes_job_by_metis_id_impl(&client, &namespace, &job_id).await {
                Ok(job) => {
                    let job_name = match job.metadata.name {
                        Some(name) => name,
                        None => {
                            let _ = sender.unbounded_send(format!(
                                "Error: Job '{}' is missing a Kubernetes name.",
                                job_id
                            ));
                            return;
                        }
                    };

                    match wait_for_pod_name_impl(&client, &namespace, &job_name, &job_id).await {
                        Ok(pod_name) => {
                            let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
                            let mut params = LogParams::default();
                            params.follow = follow;

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

                                                let chunk = String::from_utf8_lossy(&buffer[..read]).to_string();
                                                if sender.unbounded_send(chunk).is_err() {
                                                    break;
                                                }
                                            }
                                            Err(err) => {
                                                let _ = sender.unbounded_send(format!("Error: {}", err));
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    let _ = sender.unbounded_send(format!("Error: {}", err));
                                }
                            }
                        }
                        Err(err) => {
                            let _ = sender.unbounded_send(format!("Error: {}", err));
                        }
                    }
                }
                Err(err) => {
                    let _ = sender.unbounded_send(format!("Error: {}", err));
                }
            }
        });

        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &MetisId) -> Result<(), JobEngineError> {
        let job = self.find_kubernetes_job_by_metis_id(metis_id.as_str()).await?;
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
                Err(JobEngineError::NotFound(format!(
                    "Job '{metis_id}' not found in namespace '{}'",
                    self.namespace
                )))
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
}
