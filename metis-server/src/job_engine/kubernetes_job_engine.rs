use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{channel::mpsc, io::AsyncReadExt};
use k8s_openapi::{
    api::{
        batch::v1::{Job, JobSpec, JobStatus as KubeJobStatus},
        core::v1::{
            Container, EnvVar, LocalObjectReference, Pod, PodSpec, PodTemplateSpec,
            ResourceRequirements, Secret, Volume, VolumeMount,
        },
    },
    apimachinery::pkg::api::resource::Quantity,
    apimachinery::pkg::apis::meta::v1::ObjectMeta,
};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, LogParams, PostParams},
};
use metis_common::constants::{
    ENV_METIS_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN, ENV_OPENAI_API_KEY,
};
use tokio::{
    sync::RwLock,
    time::{Duration, sleep},
};
use tracing::{error, info};

use super::{JobEngine, JobEngineError, JobStatus, MetisJob, TaskId};
use crate::{domain::users::User, store::Store};

pub struct KubernetesJobEngine {
    pub namespace: String,
    pub openai_api_key: String,
    pub server_hostname: String,
    pub client: Client,
    pub image_pull_secret: Option<String>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
}

fn merge_env_vars(
    job_uuid: &TaskId,
    env_vars: &HashMap<String, String>,
    openai_api_key: &str,
    server_hostname: &str,
    auth_token: Option<&str>,
) -> Vec<EnvVar> {
    let mut merged_vars: BTreeMap<String, String> = env_vars
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    merged_vars.insert(ENV_OPENAI_API_KEY.to_string(), openai_api_key.to_string());
    merged_vars.insert(ENV_METIS_ID.to_string(), job_uuid.to_string());
    if let Some(token) = auth_token {
        merged_vars.insert(ENV_METIS_TOKEN.to_string(), token.to_string());
    }

    let hostname = server_hostname.trim();
    if !hostname.is_empty() {
        merged_vars.insert(
            ENV_METIS_SERVER_URL.to_string(),
            format!("http://{hostname}"),
        );
    }

    merged_vars
        .into_iter()
        .map(|(name, value)| EnvVar {
            name,
            value: Some(value),
            ..Default::default()
        })
        .collect()
}

fn build_image_pull_secrets(image_pull_secret: Option<&str>) -> Option<Vec<LocalObjectReference>> {
    image_pull_secret
        .and_then(|name| {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .map(|name| vec![LocalObjectReference { name: Some(name) }])
}

impl KubernetesJobEngine {
    fn build_metadata_labels(job_uuid: &TaskId) -> BTreeMap<String, String> {
        let mut metadata_labels = BTreeMap::new();
        metadata_labels.insert("metis-id".to_string(), job_uuid.to_string());
        metadata_labels
    }

    fn build_env_vars(
        &self,
        job_uuid: &TaskId,
        env_vars: &HashMap<String, String>,
        auth_token: Option<&str>,
    ) -> Vec<EnvVar> {
        merge_env_vars(
            job_uuid,
            env_vars,
            &self.openai_api_key,
            &self.server_hostname,
            auth_token,
        )
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

    fn job_metis_id(job: &Job) -> Option<TaskId> {
        job.metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("metis-id"))
            .and_then(|value| value.parse::<TaskId>().ok())
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

    fn pod_status(pod: &Pod) -> JobStatus {
        match pod
            .status
            .as_ref()
            .and_then(|status| status.phase.as_deref())
        {
            Some("Succeeded") => JobStatus::Complete,
            Some("Failed") => JobStatus::Failed,
            _ => JobStatus::Running,
        }
    }

    fn pod_metis_id(pod: &Pod) -> Option<TaskId> {
        pod.metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get("metis-id"))
            .and_then(|value| value.parse::<TaskId>().ok())
    }

    fn pod_completion_time(pod: &Pod) -> Option<DateTime<Utc>> {
        pod.status
            .as_ref()
            .and_then(|status| status.container_statuses.as_ref())
            .and_then(|containers| {
                containers
                    .iter()
                    .filter_map(|container_status| {
                        container_status
                            .state
                            .as_ref()
                            .and_then(|state| state.terminated.as_ref())
                            .and_then(|terminated| terminated.finished_at.as_ref())
                            .map(|time| time.0)
                    })
                    .max()
            })
    }

    fn pod_failure_message(pod: &Pod) -> Option<String> {
        let status = pod.status.as_ref()?;

        if let Some(container_statuses) = status.container_statuses.as_ref() {
            for container_status in container_statuses {
                if let Some(terminated) = container_status
                    .state
                    .as_ref()
                    .and_then(|state| state.terminated.as_ref())
                {
                    if terminated.exit_code != 0 {
                        if let Some(message) = terminated.message.clone() {
                            return Some(message);
                        }

                        if let Some(reason) = terminated.reason.clone() {
                            return Some(reason);
                        }
                    }
                }
            }
        }

        status.message.clone()
    }

    fn to_metis_job_from_pod(pod: &Pod) -> Result<MetisJob, JobEngineError> {
        let pod_name = pod
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string());
        let id = Self::pod_metis_id(pod).ok_or_else(|| {
            JobEngineError::Internal(format!("Pod '{pod_name}' is missing metis-id label"))
        })?;
        let status = Self::pod_status(pod);
        let creation_time = pod.metadata.creation_timestamp.as_ref().map(|t| t.0);
        let start_time = pod
            .status
            .as_ref()
            .and_then(|s| s.start_time.as_ref())
            .map(|t| t.0);
        let completion_time = Self::pod_completion_time(pod);
        let failure_message = Self::pod_failure_message(pod);

        Ok(MetisJob {
            id,
            status,
            creation_time,
            start_time,
            completion_time,
            failure_message,
        })
    }

    async fn find_kubernetes_job_by_metis_id(
        &self,
        job_id: &TaskId,
    ) -> Result<Job, JobEngineError> {
        find_kubernetes_job_by_metis_id_impl(&self.client, &self.namespace, job_id).await
    }

    async fn find_kubernetes_pod_by_metis_id(
        &self,
        job_id: &TaskId,
    ) -> Result<Pod, JobEngineError> {
        find_kubernetes_pod_by_metis_id_impl(&self.client, &self.namespace, job_id).await
    }

    async fn resolve_pod_name(&self, job_id: &TaskId) -> Result<String, JobEngineError> {
        resolve_pod_name_impl(&self.client, &self.namespace, job_id).await
    }

    async fn kill_pods_by_metis_id(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        let pods: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let selector = format!("metis-id={metis_id}");
        let lp = ListParams::default().labels(&selector);
        let pod_list = pods
            .list(&lp)
            .await
            .map_err(|err| {
                error!(metis_id = %metis_id, error = ?err, namespace = %self.namespace, "failed to list pods before deletion");
                JobEngineError::Kubernetes(err)
            })?
            .into_iter()
            .collect::<Vec<_>>();

        if pod_list.is_empty() {
            return Err(JobEngineError::NotFound(metis_id.clone()));
        }

        let dp = DeleteParams::default();
        let mut deleted_any = false;

        for mut pod in pod_list {
            if let Some(pod_name) = pod.metadata.name.take() {
                match pods.delete(&pod_name, &dp).await {
                    Ok(_) => {
                        deleted_any = true;
                        info!(
                            metis_id = %metis_id,
                            pod_name = %pod_name,
                            namespace = %self.namespace,
                            "pod deleted successfully"
                        );
                    }
                    Err(kube::Error::Api(err)) if err.code == 404 => {}
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
                }
            }
        }

        if deleted_any {
            Ok(())
        } else {
            Err(JobEngineError::NotFound(metis_id.clone()))
        }
    }
}

async fn find_kubernetes_job_by_metis_id_impl(
    client: &Client,
    namespace: &str,
    job_id: &TaskId,
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

async fn find_kubernetes_pod_by_metis_id_impl(
    client: &Client,
    namespace: &str,
    job_id: &TaskId,
) -> Result<Pod, JobEngineError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let selector = format!("metis-id={job_id}");
    let lp = ListParams::default().labels(&selector);
    let items = pods
        .list(&lp)
        .await
        .map_err(JobEngineError::Kubernetes)?
        .items;

    match items.len() {
        0 => Err(JobEngineError::NotFound(job_id.clone())),
        1 => Ok(items
            .into_iter()
            .next()
            .expect("validated single pod response")),
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

async fn resolve_pod_name_impl(
    client: &Client,
    namespace: &str,
    job_id: &TaskId,
) -> Result<String, JobEngineError> {
    match find_kubernetes_job_by_metis_id_impl(client, namespace, job_id).await {
        Ok(job) => {
            let job_name = job.metadata.name.ok_or_else(|| {
                JobEngineError::Internal(format!("Job '{job_id}' is missing a Kubernetes name."))
            })?;
            wait_for_pod_name_impl(client, namespace, &job_name).await
        }
        Err(JobEngineError::NotFound(_)) => {
            let mut pod = find_kubernetes_pod_by_metis_id_impl(client, namespace, job_id).await?;
            pod.metadata.name.take().ok_or_else(|| {
                JobEngineError::Internal(format!("Pod '{job_id}' is missing a Kubernetes name."))
            })
        }
        Err(err) => Err(err),
    }
}

// Default path for auth token in containers (expanded from ~/.local/share/metis/auth-token)
// The file will be mounted at this path via a secret volume
#[allow(dead_code)]
const AUTH_TOKEN_PATH: &str = "/home/worker/.local/share/metis/auth-token";
const AUTH_TOKEN_MOUNT_DIR: &str = "/home/worker/.local/share/metis";
const AUTH_TOKEN_SECRET_NAME_PREFIX: &str = "metis-auth-token-";

#[async_trait]
impl JobEngine for KubernetesJobEngine {
    async fn create_job(
        &self,
        metis_id: &TaskId,
        image: &str,
        env_vars: &HashMap<String, String>,
        cpu_limit: String,
        memory_limit: String,
        user: Option<&User>,
    ) -> Result<(), JobEngineError> {
        let job_name = format!("metis-worker-{metis_id}");

        info!(metis_id = %metis_id, namespace = %self.namespace, "creating Kubernetes job");

        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let metadata_labels = Self::build_metadata_labels(metis_id);

        let (actor, auth_token) = {
            let mut store = self.store.write().await;
            store.create_actor_for_task(metis_id.clone()).await?
        };
        info!(
            metis_id = %metis_id,
            actor = %actor.name(),
            job_name = %job_name,
            "created actor for job"
        );

        // Create secret for GitHub token if user is provided
        let secret_name = user.map(|_| format!("{AUTH_TOKEN_SECRET_NAME_PREFIX}{metis_id}"));
        if let Some(user) = user {
            let secret_name = secret_name.as_ref().unwrap();
            let secrets: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);
            let secret = Secret {
                metadata: ObjectMeta {
                    name: Some(secret_name.clone()),
                    labels: Some(metadata_labels.clone()),
                    ..Default::default()
                },
                string_data: Some({
                    let mut data = std::collections::BTreeMap::new();
                    data.insert("auth-token".to_string(), user.github_token.clone());
                    data
                }),
                ..Default::default()
            };

            let pp = PostParams::default();
            if let Err(err) = secrets.create(&pp, &secret).await {
                // If secret already exists, that's okay (idempotent)
                if !matches!(err, kube::Error::Api(ref api_err) if api_err.code == 409) {
                    error!(
                        secret_name = %secret_name,
                        error = ?err,
                        "failed to create auth token secret"
                    );
                    return Err(JobEngineError::Kubernetes(err));
                }
            }
        }

        // Build container with optional volume mount for auth token
        let mut container = Container {
            name: "metis-worker".to_string(),
            image: Some(image.to_string()),
            image_pull_policy: Some("IfNotPresent".into()),
            args: None,
            env: Some(self.build_env_vars(metis_id, env_vars, Some(&auth_token))),
            ..Default::default()
        };

        container.resources = Some(ResourceRequirements {
            requests: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity(cpu_limit.clone())),
                ("memory".to_string(), Quantity(memory_limit.clone())),
            ])),
            limits: Some(BTreeMap::from([
                ("cpu".to_string(), Quantity(cpu_limit)),
                ("memory".to_string(), Quantity(memory_limit)),
            ])),
            ..Default::default()
        });

        let mut volumes = Vec::new();
        if let Some(secret_name) = &secret_name {
            // Add volume mount for auth token
            // Mount to the parent directory so the file appears at AUTH_TOKEN_PATH
            container.volume_mounts = Some(vec![VolumeMount {
                name: "auth-token".to_string(),
                mount_path: AUTH_TOKEN_MOUNT_DIR.to_string(),
                read_only: Some(true),
                ..Default::default()
            }]);

            // Add volume referencing the secret
            // Use items to map the secret key to the exact file path
            volumes.push(Volume {
                name: "auth-token".to_string(),
                secret: Some(k8s_openapi::api::core::v1::SecretVolumeSource {
                    secret_name: Some(secret_name.clone()),
                    default_mode: Some(0o600),
                    items: Some(vec![k8s_openapi::api::core::v1::KeyToPath {
                        key: "auth-token".to_string(),
                        path: "auth-token".to_string(), // This will be at <mount_path>/auth-token
                        mode: Some(0o666),
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            });
        }

        let image_pull_secrets = build_image_pull_secrets(self.image_pull_secret.as_deref());

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
                        containers: vec![container],
                        volumes: if volumes.is_empty() {
                            None
                        } else {
                            Some(volumes)
                        },
                        restart_policy: Some("Never".into()),
                        image_pull_secrets,
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
        let mut seen_ids: HashSet<TaskId> = metis_jobs.iter().map(|job| job.id.clone()).collect();

        let pods_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);
        let pods = pods_api
            .list(&ListParams::default().labels("metis-id"))
            .await
            .map_err(|err| {
                error!(error = ?err, namespace = %self.namespace, "failed to list pods from Kubernetes");
                JobEngineError::Kubernetes(err)
            })?;

        for pod in pods {
            if let Ok(pod_job) = Self::to_metis_job_from_pod(&pod) {
                if seen_ids.insert(pod_job.id.clone()) {
                    metis_jobs.push(pod_job);
                }
            }
        }

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

    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        match self.find_kubernetes_job_by_metis_id(metis_id).await {
            Ok(job) => Self::to_metis_job(&job),
            Err(JobEngineError::NotFound(_)) => {
                let pod = self.find_kubernetes_pod_by_metis_id(metis_id).await?;
                Self::to_metis_job_from_pod(&pod)
            }
            Err(err) => Err(err),
        }
    }

    async fn get_logs(
        &self,
        job_id: &TaskId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let pod_name = self.resolve_pod_name(job_id).await?;

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
        job_id: &TaskId,
        follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let (tx, rx) = mpsc::unbounded();

        let namespace = self.namespace.clone();
        let client = self.client.clone();
        let job_id = job_id.clone();
        let sender = tx;

        tokio::spawn(async move {
            let pod_name = match resolve_pod_name_impl(&client, &namespace, &job_id).await {
                Ok(name) => name,
                Err(err) => {
                    let _ = sender.unbounded_send(format!("Error: {err}"));
                    return;
                }
            };

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

                                let chunk = String::from_utf8_lossy(&buffer[..read]).to_string();
                                if sender.unbounded_send(chunk).is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = sender.unbounded_send(format!("Error: {err}"));
                                break;
                            }
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

    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        match self.find_kubernetes_job_by_metis_id(metis_id).await {
            Ok(job) => {
                let job_name = job.metadata.name.ok_or_else(|| {
                    JobEngineError::Internal(format!(
                        "Job '{metis_id}' is missing a Kubernetes name."
                    ))
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
                        if let Err(err) = self.kill_pods_by_metis_id(metis_id).await {
                            if !matches!(err, JobEngineError::NotFound(_)) {
                                return Err(err);
                            }
                        }
                        Ok(())
                    }
                    Err(kube::Error::Api(err)) if err.code == 404 => {
                        self.kill_pods_by_metis_id(metis_id).await
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
            Err(JobEngineError::NotFound(_)) => self.kill_pods_by_metis_id(metis_id).await,
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use k8s_openapi::{
        api::core::v1::{ContainerState, ContainerStateTerminated, ContainerStatus, PodStatus},
        apimachinery::pkg::apis::meta::v1::Time,
    };
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn merge_env_vars_combines_task_and_system_values() {
        let job_id: TaskId = "t-abcd".parse().unwrap();
        let mut task_env = HashMap::from([("CUSTOM".to_string(), "1".to_string())]);
        task_env.insert(ENV_METIS_ID.to_string(), "override-me".to_string());
        task_env.insert(
            ENV_METIS_SERVER_URL.to_string(),
            "http://example.com".to_string(),
        );

        let merged = merge_env_vars(
            &job_id,
            &task_env,
            "openai-key",
            "metis.example.com",
            Some("auth-token"),
        );

        let merged_map: HashMap<_, _> = merged
            .into_iter()
            .map(|env| (env.name, env.value.unwrap_or_default()))
            .collect();

        assert_eq!(merged_map.get("CUSTOM"), Some(&"1".to_string()));
        assert_eq!(
            merged_map.get(ENV_OPENAI_API_KEY),
            Some(&"openai-key".to_string())
        );
        assert_eq!(merged_map.get(ENV_METIS_ID), Some(&job_id.to_string()));
        assert_eq!(
            merged_map.get(ENV_METIS_SERVER_URL),
            Some(&"http://metis.example.com".to_string())
        );
        assert_eq!(
            merged_map.get(ENV_METIS_TOKEN),
            Some(&"auth-token".to_string())
        );
    }

    #[test]
    fn merge_env_vars_skips_empty_server_hostname() {
        let job_id: TaskId = "t-abcd".parse().unwrap();
        let merged = merge_env_vars(&job_id, &HashMap::new(), "openai-key", "   ", None);

        let merged_map: HashMap<_, _> = merged
            .into_iter()
            .map(|env| (env.name, env.value.unwrap_or_default()))
            .collect();

        assert!(!merged_map.contains_key(ENV_METIS_SERVER_URL));
    }

    #[test]
    fn build_image_pull_secrets_includes_reference_when_set() {
        let secrets = build_image_pull_secrets(Some("ghcr-credentials"))
            .expect("expected image_pull_secrets");

        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].name.as_deref(), Some("ghcr-credentials"));
    }

    #[test]
    fn build_image_pull_secrets_omits_blank_and_none() {
        assert!(build_image_pull_secrets(None).is_none());
        assert!(build_image_pull_secrets(Some("   ")).is_none());
    }

    #[test]
    fn to_metis_job_from_pod_uses_pod_metadata() {
        let job_id: TaskId = "t-abcd".parse().unwrap();
        let creation = Time(Utc::now());
        let start = Time(Utc::now());
        let finished = Time(Utc::now());
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("pod-name".into()),
                creation_timestamp: Some(creation.clone()),
                labels: Some(BTreeMap::from([("metis-id".into(), job_id.to_string())])),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Succeeded".into()),
                start_time: Some(start.clone()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "container".into(),
                    state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            finished_at: Some(finished.clone()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let metis_job =
            KubernetesJobEngine::to_metis_job_from_pod(&pod).expect("conversion should succeed");

        assert_eq!(metis_job.id, job_id);
        assert_eq!(metis_job.status, JobStatus::Complete);
        assert_eq!(metis_job.creation_time, Some(creation.0));
        assert_eq!(metis_job.start_time, Some(start.0));
        assert_eq!(metis_job.completion_time, Some(finished.0));
    }

    #[test]
    fn pod_failure_message_prefers_terminated_message() {
        let job_id: TaskId = "t-abcd".parse().unwrap();
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some("pod-name".into()),
                labels: Some(BTreeMap::from([("metis-id".into(), job_id.to_string())])),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some("Failed".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "container".into(),
                    state: Some(ContainerState {
                        terminated: Some(ContainerStateTerminated {
                            exit_code: 1,
                            message: Some("boom".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }]),
                message: Some("unused".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let metis_job =
            KubernetesJobEngine::to_metis_job_from_pod(&pod).expect("conversion should succeed");

        assert_eq!(metis_job.failure_message.as_deref(), Some("boom"));
        assert_eq!(metis_job.status, JobStatus::Failed);
    }
}
