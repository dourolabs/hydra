use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::{Request, Response};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::{domain::actors::Actor, store::StoreError};

mod local_docker_job_engine;
mod local_job_engine;
pub mod proxy;

#[cfg(feature = "kubernetes")]
pub use crate::ee::job_engine::KubernetesJobEngine;
pub use hydra_common::SessionId;
pub use local_docker_job_engine::LocalDockerJobEngine;
pub use local_job_engine::LocalJobEngine;
pub use proxy::{ProxyError, WsPumpGuard, proxy_http_to_upstream, proxy_ws_to_upstream};

/// Represents the lifecycle state of a Hydra job.
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

/// Represents a job in the Hydra system, abstracting away Kubernetes-specific details.
#[derive(Debug, Clone)]
pub struct HydraJob {
    /// The unique Hydra job ID (hydra-id label)
    pub id: SessionId,
    /// Job status in the Hydra lifecycle.
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
    NotFound(SessionId),
    #[error("Multiple jobs found: {0}")]
    MultipleFound(SessionId),
    #[error("Job already exists: {0}")]
    AlreadyExists(SessionId),
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

/// Describes a filesystem bind mount for Docker containers.
///
/// Maps a host filesystem path into a container at a specified mount point.
/// Used to give Docker workers access to local filesystem git repos.
#[derive(Debug, Clone)]
pub struct BindMount {
    pub host_path: String,
    pub container_path: String,
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
    /// * `hydra_id` - The Hydra ID to use for the job
    /// * `actor` - The actor assigned to the job
    /// * `auth_token` - The raw auth token for the actor
    /// * `image` - The container image the job should run
    /// * `env_vars` - Environment variables to inject into the job container
    /// * `cpu_limit` - CPU limit for the job container
    /// * `memory_limit` - Memory limit for the job container
    /// * `cpu_request` - CPU request for the job container
    /// * `memory_request` - Memory request for the job container
    /// * `bind_mounts` - Optional filesystem bind mounts for Docker containers
    ///
    /// # Returns
    /// Ok(()) if successful, or an error if creation fails
    async fn create_job(
        &self,
        hydra_id: &SessionId,
        actor: &Actor,
        auth_token: &str,
        image: &str,
        env_vars: &HashMap<String, String>,
        cpu_limit: String,
        memory_limit: String,
        cpu_request: String,
        memory_request: String,
        bind_mounts: Vec<BindMount>,
    ) -> Result<(), JobEngineError>;

    /// Returns `true` when workers run inside containers and need host paths
    /// rewritten to container-side mount points (e.g. Docker engine).
    fn is_containerized(&self) -> bool {
        false
    }

    /// Lists all jobs matching the given label selector.
    ///
    /// # Returns
    /// A vector of HydraJob resources matching the selector
    async fn list_jobs(&self) -> Result<Vec<HydraJob>, JobEngineError>;

    /// Finds a single job by its hydra-id.
    ///
    /// # Arguments
    /// * `hydra_id` - The hydra-id to search for
    ///
    /// # Returns
    /// The HydraJob if exactly one is found, or an error if none or multiple are found
    async fn find_job_by_hydra_id(&self, hydra_id: &SessionId) -> Result<HydraJob, JobEngineError>;

    /// Gets logs for a job as a single string (batch mode).
    ///
    /// # Arguments
    /// * `job_id` - The Hydra job ID
    /// * `tail_lines` - When set, return only the last N lines from the log
    ///
    /// # Returns
    /// The complete logs as a string, or an error if retrieval fails
    async fn get_logs(
        &self,
        job_id: &SessionId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError>;

    /// Gets logs for a job as a stream (streaming mode).
    ///
    /// This method spawns a background task that reads logs from Kubernetes
    /// and sends them as strings through the returned receiver.
    ///
    /// # Arguments
    /// * `job_id` - The Hydra job ID
    /// * `follow` - Whether to follow the log stream (true for running jobs)
    ///
    /// # Returns
    /// A receiver that yields log chunks as strings. The sender will close
    /// when the stream ends or encounters an error.
    fn get_logs_stream(
        &self,
        job_id: &SessionId,
        follow: bool,
    ) -> Result<futures::channel::mpsc::UnboundedReceiver<String>, JobEngineError>;

    /// Stops a running job without deleting its underlying execution
    /// resources, so post-mortem logs remain available.
    ///
    /// For Kubernetes this signals the container's PID 1 via the pod
    /// exec API and leaves the Job/Pod objects in place; eventual GC is
    /// handled by `ttl_seconds_after_finished`. For local engines, the
    /// worker subprocess/container is signalled (SIGTERM, then SIGKILL
    /// after a grace period) but tracking metadata — and any log file
    /// or container needed to serve `get_logs` — is preserved.
    ///
    /// Returns `JobEngineError::NotFound` when no job exists for the
    /// given id; callers typically treat that as a no-op success.
    async fn stop_job(&self, hydra_id: &SessionId) -> Result<(), JobEngineError>;

    /// Hard-removes a job and any associated execution resources.
    ///
    /// Intended for reconciliation paths that need to GC orphans whose
    /// owning state has already vanished (e.g. the reaper's
    /// "missing from store" arm). Implementations should both stop the
    /// execution AND drop the underlying objects (Kubernetes Job + Pod,
    /// Docker container, local tracking entry). Post-deletion `get_logs`
    /// calls will not succeed.
    ///
    /// Returns `JobEngineError::NotFound` when no job exists for the
    /// given id.
    async fn delete_job(&self, hydra_id: &SessionId) -> Result<(), JobEngineError>;

    /// Proxy an HTTP request to `port` on the worker's container/pod/process.
    ///
    /// Implementations resolve the runtime-local address for the worker
    /// (cluster pod IP / docker network IP / localhost), forward the request
    /// body and headers, and return the upstream response as-is. The
    /// caller is responsible for stripping any headers that must not leak
    /// to the upstream (e.g. `Cookie`, `Authorization`) and for setting
    /// any response headers that must apply at the proxy edge (e.g. CSP).
    async fn proxy_http(
        &self,
        session_id: &SessionId,
        port: u16,
        req: Request<Body>,
    ) -> Result<Response<Body>, ProxyError>;

    /// Proxy a WebSocket upgrade to `port` on the worker's
    /// container/pod/process. Returns the response that completes the
    /// upgrade handshake; the implementation is responsible for spawning
    /// the bidirectional pump that relays frames between the client
    /// `WebSocket` and the upstream WebSocket.
    ///
    /// `pump_guard` is moved into the spawned pump task so its `Drop`
    /// runs when the pump exits, not when this method returns. Callers
    /// pass the per-target concurrency permit through here so a long-
    /// lived WS upgrade continues to hold its slot for the lifetime of
    /// the upgraded socket.
    async fn proxy_ws(
        &self,
        session_id: &SessionId,
        port: u16,
        upgrade: WebSocketUpgrade,
        pump_guard: WsPumpGuard,
    ) -> Result<Response<Body>, ProxyError>;
}
