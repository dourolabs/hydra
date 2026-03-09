use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
        StartContainerOptions, WaitContainerOptions,
    },
    image::CreateImageOptions,
    models::HostConfig,
    secret::ContainerStateStatusEnum,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::{StreamExt, channel::mpsc};
use metis_common::constants::{ENV_METIS_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN};
use tracing::{error, info, warn};

use super::{JobEngine, JobEngineError, JobStatus, MetisJob, TaskId};
use crate::domain::actors::Actor;

/// Metadata tracked in-memory for each container managed by this engine.
struct ContainerInfo {
    container_id: String,
    creation_time: DateTime<Utc>,
}

/// A job engine that runs containers on the local Docker daemon via the bollard crate.
pub struct LocalDockerJobEngine {
    docker: Docker,
    server_url: String,
    /// Maps metis_id → container info for tracking.
    containers: DashMap<TaskId, ContainerInfo>,
}

impl LocalDockerJobEngine {
    pub async fn new(server_url: String) -> Result<Self, JobEngineError> {
        let docker = Docker::connect_with_local_defaults().map_err(|e| {
            JobEngineError::Internal(format!("Failed to connect to Docker daemon: {e}"))
        })?;
        let engine = Self {
            docker,
            server_url,
            containers: DashMap::new(),
        };
        engine.recover_containers().await?;
        Ok(engine)
    }

    async fn recover_containers(&self) -> Result<(), JobEngineError> {
        let filters = HashMap::from([("label".to_string(), vec!["metis-id".to_string()])]);
        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| {
                JobEngineError::Internal(format!("Failed to list containers for recovery: {e}"))
            })?;

        let mut recovered = 0u64;
        for container in &containers {
            let task_id = container
                .labels
                .as_ref()
                .and_then(|labels| labels.get("metis-id"))
                .and_then(|id| TaskId::from_str(id).ok());

            let task_id = match task_id {
                Some(id) => id,
                None => {
                    // Fall back to parsing from container name.
                    let name = extract_task_id_from_names(container.names.as_deref());
                    match name {
                        Some(id) => id,
                        None => {
                            warn!(
                                container_id = container.id.as_deref().unwrap_or("unknown"),
                                "skipping container: could not extract task ID"
                            );
                            continue;
                        }
                    }
                }
            };

            let container_id = match &container.id {
                Some(id) => id.clone(),
                None => continue,
            };

            let creation_time = container
                .created
                .and_then(|ts| DateTime::from_timestamp(ts, 0))
                .unwrap_or_else(Utc::now);

            self.containers.insert(
                task_id,
                ContainerInfo {
                    container_id,
                    creation_time,
                },
            );
            recovered += 1;
        }

        info!(recovered, "recovered existing Docker containers on startup");
        Ok(())
    }

    fn build_env_vars(
        &self,
        metis_id: &TaskId,
        auth_token: &str,
        extra_env: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut env: HashMap<String, String> = extra_env.clone();
        env.insert(ENV_METIS_ID.to_string(), metis_id.to_string());
        env.insert(ENV_METIS_TOKEN.to_string(), auth_token.to_string());

        let server_url = self.server_url.trim();
        if !server_url.is_empty() {
            env.insert(ENV_METIS_SERVER_URL.to_string(), server_url.to_string());
        }

        env.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
    }

    fn container_name(metis_id: &TaskId) -> String {
        format!("metis-worker-{metis_id}")
    }

    async fn inspect_to_metis_job(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        let info = self
            .containers
            .get(metis_id)
            .ok_or_else(|| JobEngineError::NotFound(metis_id.clone()))?;

        let inspect = self
            .docker
            .inspect_container(&info.container_id, None)
            .await
            .map_err(|e| match &e {
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                } => JobEngineError::NotFound(metis_id.clone()),
                _ => JobEngineError::Internal(format!("Docker inspect error: {e}")),
            })?;

        let state = inspect.state.as_ref();

        let status = match state.and_then(|s| s.status) {
            Some(ContainerStateStatusEnum::CREATED) => JobStatus::Pending,
            Some(ContainerStateStatusEnum::RUNNING) => JobStatus::Running,
            Some(ContainerStateStatusEnum::EXITED) => {
                let exit_code = state.and_then(|s| s.exit_code).unwrap_or(-1);
                if exit_code == 0 {
                    JobStatus::Complete
                } else {
                    JobStatus::Failed
                }
            }
            Some(ContainerStateStatusEnum::DEAD) => JobStatus::Failed,
            Some(ContainerStateStatusEnum::PAUSED) => JobStatus::Running,
            Some(ContainerStateStatusEnum::RESTARTING) => JobStatus::Running,
            _ => JobStatus::Pending,
        };

        let start_time = state
            .and_then(|s| s.started_at.as_deref())
            .and_then(parse_docker_time);

        let completion_time = match status {
            JobStatus::Complete | JobStatus::Failed => state
                .and_then(|s| s.finished_at.as_deref())
                .and_then(parse_docker_time),
            _ => None,
        };

        let failure_message = if status == JobStatus::Failed {
            state.map(|s| {
                let exit_code = s.exit_code.unwrap_or(-1);
                let error = s.error.as_deref().unwrap_or("");
                if error.is_empty() {
                    format!("Container exited with code {exit_code}")
                } else {
                    format!("Container exited with code {exit_code}: {error}")
                }
            })
        } else {
            None
        };

        Ok(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(info.creation_time),
            start_time,
            completion_time,
            failure_message,
        })
    }
}

fn parse_docker_time(s: &str) -> Option<DateTime<Utc>> {
    // Docker returns RFC 3339 timestamps; skip zero-value timestamps.
    if s.starts_with("0001-") {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[async_trait]
impl JobEngine for LocalDockerJobEngine {
    async fn create_job(
        &self,
        metis_id: &TaskId,
        _actor: &Actor,
        auth_token: &str,
        image: &str,
        env_vars: &HashMap<String, String>,
        _cpu_limit: String,
        memory_limit: String,
        _cpu_request: String,
        _memory_request: String,
    ) -> Result<(), JobEngineError> {
        if self.containers.contains_key(metis_id) {
            return Err(JobEngineError::AlreadyExists(metis_id.clone()));
        }

        info!(metis_id = %metis_id, image = %image, "pulling Docker image");

        // Pull the image (best-effort; it may already exist locally).
        let mut pull_stream = self.docker.create_image(
            Some(CreateImageOptions {
                from_image: image,
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(result) = pull_stream.next().await {
            match result {
                Ok(_) => {}
                Err(e) => {
                    warn!(metis_id = %metis_id, error = %e, "image pull warning (may already exist locally)");
                }
            }
        }

        let container_name = Self::container_name(metis_id);
        let env = self.build_env_vars(metis_id, auth_token, env_vars);

        // Parse memory limit for Docker (best-effort).
        let memory = parse_memory_limit(&memory_limit);

        let host_config = HostConfig {
            memory,
            ..Default::default()
        };

        let config = Config {
            image: Some(image.to_string()),
            env: Some(env),
            cmd: Some(vec![
                "metis".to_string(),
                "jobs".to_string(),
                "worker-run".to_string(),
                metis_id.to_string(),
                ".".to_string(),
                "--tempdir".to_string(),
            ]),
            host_config: Some(host_config),
            labels: Some(HashMap::from([(
                "metis-id".to_string(),
                metis_id.to_string(),
            )])),
            ..Default::default()
        };

        let create_opts = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        let response = self
            .docker
            .create_container(Some(create_opts), config)
            .await
            .map_err(|e| match &e {
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 409, ..
                } => JobEngineError::AlreadyExists(metis_id.clone()),
                _ => JobEngineError::Internal(format!("Docker create container error: {e}")),
            })?;

        let creation_time = Utc::now();
        self.containers.insert(
            metis_id.clone(),
            ContainerInfo {
                container_id: response.id.clone(),
                creation_time,
            },
        );

        // Start the container.
        self.docker
            .start_container(&response.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| JobEngineError::Internal(format!("Docker start container error: {e}")))?;

        info!(
            metis_id = %metis_id,
            container_id = %response.id,
            "container created and started"
        );

        // Spawn a background task to monitor container completion and update status.
        let docker = self.docker.clone();
        let container_id = response.id.clone();
        tokio::spawn(async move {
            let opts = WaitContainerOptions {
                condition: "not-running",
            };
            let mut stream = docker.wait_container(&container_id, Some(opts));
            if let Some(Err(e)) = stream.next().await {
                error!(container_id = %container_id, error = %e, "error waiting for container");
            }
        });

        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        let mut jobs = Vec::new();

        for entry in self.containers.iter() {
            match self.inspect_to_metis_job(entry.key()).await {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    warn!(metis_id = %entry.key(), error = %e, "skipping container in list");
                }
            }
        }

        // Sort by start_time or creation_time, most recent first.
        jobs.sort_by(|a, b| {
            let time_a = a.start_time.or(a.creation_time);
            let time_b = b.start_time.or(b.creation_time);
            time_b.cmp(&time_a)
        });

        Ok(jobs)
    }

    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        self.inspect_to_metis_job(metis_id).await
    }

    async fn get_logs(
        &self,
        job_id: &TaskId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let info = self
            .containers
            .get(job_id)
            .ok_or_else(|| JobEngineError::NotFound(job_id.clone()))?;

        let tail = tail_lines
            .map(|n| n.to_string())
            .unwrap_or_else(|| "all".to_string());

        let opts = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            tail,
            ..Default::default()
        };

        let mut stream = self.docker.logs(&info.container_id, Some(opts));
        let mut output = String::new();

        while let Some(result) = stream.next().await {
            match result {
                Ok(chunk) => {
                    output.push_str(&chunk.to_string());
                }
                Err(e) => {
                    return Err(JobEngineError::Internal(format!("Docker logs error: {e}")));
                }
            }
        }

        Ok(output)
    }

    fn get_logs_stream(
        &self,
        job_id: &TaskId,
        follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let info = self
            .containers
            .get(job_id)
            .ok_or_else(|| JobEngineError::NotFound(job_id.clone()))?;

        let container_id = info.container_id.clone();
        let docker = self.docker.clone();

        let (tx, rx) = mpsc::unbounded();

        tokio::spawn(async move {
            let opts = LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow,
                tail: "all".to_string(),
                ..Default::default()
            };

            let mut stream = docker.logs(&container_id, Some(opts));

            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        if tx.unbounded_send(chunk.to_string()).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(format!("Error: {e}"));
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        let info = self
            .containers
            .get(metis_id)
            .ok_or_else(|| JobEngineError::NotFound(metis_id.clone()))?;

        let container_id = info.container_id.clone();
        drop(info);

        // Stop the container (with a short timeout before SIGKILL).
        match self.docker.stop_container(&container_id, None).await {
            Ok(()) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => {
                // Container already stopped.
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                self.containers.remove(metis_id);
                return Err(JobEngineError::NotFound(metis_id.clone()));
            }
            Err(e) => {
                return Err(JobEngineError::Internal(format!(
                    "Docker stop container error: {e}"
                )));
            }
        }

        // Remove the container.
        let remove_opts = RemoveContainerOptions {
            force: true,
            ..Default::default()
        };
        match self
            .docker
            .remove_container(&container_id, Some(remove_opts))
            .await
        {
            Ok(()) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(e) => {
                return Err(JobEngineError::Internal(format!(
                    "Docker remove container error: {e}"
                )));
            }
        }

        self.containers.remove(metis_id);

        info!(metis_id = %metis_id, container_id = %container_id, "container killed and removed");

        Ok(())
    }
}

/// Extracts a TaskId from Docker container names (e.g., ["/metis-worker-t-abcdef"]).
fn extract_task_id_from_names(names: Option<&[String]>) -> Option<TaskId> {
    const PREFIX: &str = "metis-worker-";
    names?.iter().find_map(|name| {
        // Docker prefixes names with '/'.
        let stripped = name.strip_prefix('/').unwrap_or(name);
        let id_str = stripped.strip_prefix(PREFIX)?;
        TaskId::from_str(id_str).ok()
    })
}

/// Best-effort parsing of Kubernetes-style memory limits (e.g., "512Mi", "1Gi") to bytes.
fn parse_memory_limit(limit: &str) -> Option<i64> {
    let limit = limit.trim();
    if limit.is_empty() {
        return None;
    }

    if let Some(mi) = limit.strip_suffix("Mi") {
        mi.parse::<i64>().ok().map(|v| v * 1024 * 1024)
    } else if let Some(gi) = limit.strip_suffix("Gi") {
        gi.parse::<i64>().ok().map(|v| v * 1024 * 1024 * 1024)
    } else if let Some(ki) = limit.strip_suffix("Ki") {
        ki.parse::<i64>().ok().map(|v| v * 1024)
    } else {
        limit.parse::<i64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_memory_limit_handles_mi() {
        assert_eq!(parse_memory_limit("512Mi"), Some(512 * 1024 * 1024));
    }

    #[test]
    fn parse_memory_limit_handles_gi() {
        assert_eq!(parse_memory_limit("2Gi"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_memory_limit_handles_ki() {
        assert_eq!(parse_memory_limit("1024Ki"), Some(1024 * 1024));
    }

    #[test]
    fn parse_memory_limit_handles_raw_bytes() {
        assert_eq!(parse_memory_limit("1048576"), Some(1048576));
    }

    #[test]
    fn parse_memory_limit_handles_empty() {
        assert_eq!(parse_memory_limit(""), None);
        assert_eq!(parse_memory_limit("   "), None);
    }

    #[test]
    fn parse_memory_limit_handles_invalid() {
        assert_eq!(parse_memory_limit("abc"), None);
    }

    #[test]
    fn parse_docker_time_parses_rfc3339() {
        let result = parse_docker_time("2024-01-15T10:30:00.000000000Z");
        assert!(result.is_some());
    }

    #[test]
    fn parse_docker_time_skips_zero_value() {
        let result = parse_docker_time("0001-01-01T00:00:00Z");
        assert!(result.is_none());
    }

    #[test]
    fn container_name_uses_metis_id() {
        let id: TaskId = "t-abcd".parse().unwrap();
        assert_eq!(
            LocalDockerJobEngine::container_name(&id),
            "metis-worker-t-abcd"
        );
    }

    #[test]
    fn extract_task_id_from_names_with_slash_prefix() {
        let names = vec!["/metis-worker-t-abcdef".to_string()];
        let result = extract_task_id_from_names(Some(&names));
        assert!(result.is_some());
        assert_eq!(result.unwrap().to_string(), "t-abcdef");
    }

    #[test]
    fn extract_task_id_from_names_without_slash() {
        let names = vec!["metis-worker-t-xyzabc".to_string()];
        let result = extract_task_id_from_names(Some(&names));
        assert!(result.is_some());
        assert_eq!(result.unwrap().to_string(), "t-xyzabc");
    }

    #[test]
    fn extract_task_id_from_names_no_match() {
        let names = vec!["/some-other-container".to_string()];
        let result = extract_task_id_from_names(Some(&names));
        assert!(result.is_none());
    }

    #[test]
    fn extract_task_id_from_names_none() {
        let result = extract_task_id_from_names(None);
        assert!(result.is_none());
    }
}
