use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::{domain::actors::Actor, store::StoreError};

mod local_docker_job_engine;

#[cfg(feature = "kubernetes")]
pub use crate::ee::job_engine::KubernetesJobEngine;
pub use local_docker_job_engine::LocalDockerJobEngine;
pub use metis_common::TaskId;

/// Represents the lifecycle state of a Metis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Complete,
    Failed,
    Running,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Complete => "complete",
            JobStatus::Failed => "failed",
            JobStatus::Running => "running",
        }
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Represents a job in the Metis system, abstracting away Kubernetes-specific details.
#[derive(Debug, Clone)]
pub struct MetisJob {
    /// The unique Metis job ID (metis-id label)
    pub id: TaskId,
    /// Job status in the Metis lifecycle.
    pub status: JobStatus,
    /// When the job was created
    pub creation_time: Option<DateTime<Utc>>,
    /// When the job started running
    pub start_time: Option<DateTime<Utc>>,
    /// When the job completed (succeeded or failed)
    pub completion_time: Option<DateTime<Utc>>,
    /// Failure message from job conditions, if any
    pub failure_message: Option<String>,
}

/// Error type for job engine operations
#[derive(Debug, thiserror::Error)]
pub enum JobEngineError {
    #[error("Job not found: {0}")]
    NotFound(TaskId),
    #[error("Multiple jobs found: {0}")]
    MultipleFound(TaskId),
    #[error("Job already exists: {0}")]
    AlreadyExists(TaskId),
    #[cfg(feature = "kubernetes")]
    #[error("Kubernetes API error: {0}")]
    Kubernetes(#[from] kube::Error),
    #[error("Docker error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Store error: {0}")]
    Store(#[from] StoreError),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Trait that abstracts the interface between the HTTP API and Kubernetes state.
///
/// This trait allows the routes to interact with Kubernetes jobs and pods
/// without directly depending on the Kubernetes client implementation.
/// This enables easier testing and potential future support for different
/// job execution backends.
#[async_trait]
pub trait JobEngine: Send + Sync {
    /// Creates a new job with the given parameters.
    ///
    /// # Arguments
    /// * `metis_id` - The Metis ID to use for the job
    /// * `actor` - The actor assigned to the job
    /// * `auth_token` - The raw auth token for the actor
    /// * `image` - The container image the job should run
    /// * `env_vars` - Environment variables to inject into the job container
    /// * `cpu_limit` - CPU limit for the job container
    /// * `memory_limit` - Memory limit for the job container
    /// * `cpu_request` - CPU request for the job container
    /// * `memory_request` - Memory request for the job container
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if creation fails
    async fn create_job(
        &self,
        metis_id: &TaskId,
        actor: &Actor,
        auth_token: &str,
        image: &str,
        env_vars: &HashMap<String, String>,
        cpu_limit: String,
        memory_limit: String,
        cpu_request: String,
        memory_request: String,
    ) -> Result<(), JobEngineError>;

    /// Lists all jobs matching the given label selector.
    ///
    /// # Returns
    /// A vector of MetisJob resources matching the selector
    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError>;

    /// Finds a single job by its metis-id.
    ///
    /// # Arguments
    /// * `metis_id` - The metis-id to search for
    ///
    /// # Returns
    /// The MetisJob if exactly one is found, or an error if none or multiple are found
    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError>;

    /// Gets logs for a job as a single string (batch mode).
    ///
    /// # Arguments
    /// * `job_id` - The Metis job ID
    /// * `tail_lines` - When set, return only the last N lines from the log
    ///
    /// # Returns
    /// The complete logs as a string, or an error if retrieval fails
    async fn get_logs(
        &self,
        job_id: &TaskId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError>;

    /// Gets logs for a job as a stream (streaming mode).
    ///
    /// This method spawns a background task that reads logs from Kubernetes
    /// and sends them as strings through the returned receiver.
    ///
    /// # Arguments
    /// * `job_id` - The Metis job ID
    /// * `follow` - Whether to follow the log stream (true for running jobs)
    ///
    /// # Returns
    /// A receiver that yields log chunks as strings. The sender will close
    /// when the stream ends or encounters an error.
    fn get_logs_stream(
        &self,
        job_id: &TaskId,
        follow: bool,
    ) -> Result<futures::channel::mpsc::UnboundedReceiver<String>, JobEngineError>;

    /// Terminates a job if it exists.
    ///
    /// Implementations should delete the underlying job and any associated
    /// resources necessary to stop execution.
    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError>;
}
