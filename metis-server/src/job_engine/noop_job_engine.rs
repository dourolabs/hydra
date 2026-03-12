use std::collections::HashMap;

use async_trait::async_trait;
use futures::channel::mpsc;

use super::{JobEngine, JobEngineError, MetisJob, TaskId};
use crate::domain::actors::Actor;

/// A no-op job engine that does not run any jobs.
///
/// This is used when no real job engine (Docker, Kubernetes) is available.
/// It returns descriptive errors for job creation and empty/not-found results
/// for all other operations.
pub struct NoOpJobEngine;

#[async_trait]
impl JobEngine for NoOpJobEngine {
    async fn create_job(
        &self,
        _metis_id: &TaskId,
        _actor: &Actor,
        _auth_token: &str,
        _image: &str,
        _env_vars: &HashMap<String, String>,
        _cpu_limit: String,
        _memory_limit: String,
        _cpu_request: String,
        _memory_request: String,
    ) -> Result<(), JobEngineError> {
        Err(JobEngineError::Internal(
            "No job engine configured. Install Docker or configure a job engine.".to_string(),
        ))
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        Ok(vec![])
    }

    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        Err(JobEngineError::NotFound(metis_id.clone()))
    }

    async fn get_logs(
        &self,
        job_id: &TaskId,
        _tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        Err(JobEngineError::NotFound(job_id.clone()))
    }

    fn get_logs_stream(
        &self,
        job_id: &TaskId,
        _follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        Err(JobEngineError::NotFound(job_id.clone()))
    }

    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        Err(JobEngineError::NotFound(metis_id.clone()))
    }
}
