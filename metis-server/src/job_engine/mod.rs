use async_trait::async_trait;
use chrono::{DateTime, Utc};

mod kubernetes_job_engine;

pub use kubernetes_job_engine::KubernetesJobEngine;

// TODO: make this a uuid
pub type MetisId = String;

/// Represents the lifecycle state of a Metis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Complete,
    Failed,
    Running,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
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
    pub id: MetisId,
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
    NotFound(String),
    #[error("Multiple jobs found: {0}")]
    MultipleFound(String),
    #[error("Job already exists: {0}")]
    AlreadyExists(String),
    #[error("Kubernetes API error: {0}")]
    Kubernetes(#[from] kube::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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
    /// * `prompt` - The prompt/command to execute in the job
    ///
    /// # Returns
    /// The Metis ID of the created job, or an error if creation fails
    async fn create_job(&self, prompt: &str) -> Result<MetisId, JobEngineError>;

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
    async fn find_job_by_metis_id(&self, metis_id: &MetisId) -> Result<MetisJob, JobEngineError>;

    /// Gets logs for a job as a single string (batch mode).
    ///
    /// # Arguments
    /// * `job_id` - The Metis job ID
    ///
    /// # Returns
    /// The complete logs as a string, or an error if retrieval fails
    async fn get_logs(&self, job_id: &str) -> Result<String, JobEngineError>;

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
        job_id: &str,
        follow: bool,
    ) -> Result<futures::channel::mpsc::UnboundedReceiver<String>, JobEngineError>;
}

