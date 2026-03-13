use crate::job_engine::{JobEngine, JobEngineError, JobStatus, MetisJob, TaskId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::channel::mpsc;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub struct MockJobEngine {
    jobs: Arc<Mutex<Vec<MetisJob>>>,
    logs: Arc<Mutex<HashMap<TaskId, Vec<String>>>>,
    env_vars: Arc<Mutex<HashMap<TaskId, HashMap<String, String>>>>,
    resource_limits: Arc<Mutex<HashMap<TaskId, (String, String)>>>,
    resource_requests: Arc<Mutex<HashMap<TaskId, (String, String)>>>,
    /// When set, `create_job` returns this error instead of creating the job.
    create_job_error: Arc<Mutex<Option<String>>>,
}

impl MockJobEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert_job(&self, metis_id: &TaskId, status: JobStatus) {
        let mut jobs = self.jobs.lock().unwrap();
        let start_time = if status == JobStatus::Pending {
            None
        } else {
            Some(Utc::now())
        };
        jobs.push(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(Utc::now()),
            start_time,
            completion_time: None,
            failure_message: None,
        });
    }

    pub async fn insert_job_with_metadata(
        &self,
        metis_id: &TaskId,
        status: JobStatus,
        completion_time: Option<DateTime<Utc>>,
        failure_message: Option<String>,
    ) {
        let mut jobs = self.jobs.lock().unwrap();
        let start_time = if status == JobStatus::Pending {
            None
        } else {
            Some(Utc::now())
        };
        jobs.push(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(Utc::now()),
            start_time,
            completion_time,
            failure_message,
        });
    }

    pub async fn set_logs(&self, metis_id: &TaskId, chunks: Vec<String>) {
        let mut logs = self.logs.lock().unwrap();
        logs.insert(metis_id.clone(), chunks);
    }

    pub fn env_vars_for_job(&self, metis_id: &TaskId) -> Option<HashMap<String, String>> {
        let env_vars = self.env_vars.lock().unwrap();
        env_vars.get(metis_id).cloned()
    }

    pub fn resource_limits_for_job(&self, metis_id: &TaskId) -> Option<(String, String)> {
        let limits = self.resource_limits.lock().unwrap();
        limits.get(metis_id).cloned()
    }

    pub fn resource_requests_for_job(&self, metis_id: &TaskId) -> Option<(String, String)> {
        let requests = self.resource_requests.lock().unwrap();
        requests.get(metis_id).cloned()
    }

    /// Configure `create_job` to fail with a `Kubernetes` error containing the
    /// given message. Pass `None` to restore normal behavior.
    pub fn set_create_job_error(&self, error_message: Option<String>) {
        *self.create_job_error.lock().unwrap() = error_message;
    }
}

#[async_trait]
impl JobEngine for MockJobEngine {
    async fn create_job(
        &self,
        metis_id: &TaskId,
        _actor: &crate::domain::actors::Actor,
        _auth_token: &str,
        _image: &str,
        env_vars: &HashMap<String, String>,
        cpu_limit: String,
        memory_limit: String,
        cpu_request: String,
        memory_request: String,
    ) -> Result<(), JobEngineError> {
        // If a create_job_error is configured, return it without creating a job.
        // This simulates transient K8s API errors (e.g. etcdserver timeouts)
        // where create_job fails but the job may or may not have been created.
        if let Some(msg) = self.create_job_error.lock().unwrap().clone() {
            return Err(JobEngineError::Internal(msg));
        }

        let mut jobs = self.jobs.lock().unwrap();
        if jobs.iter().any(|job| &job.id == metis_id) {
            return Err(JobEngineError::AlreadyExists(metis_id.clone()));
        }

        jobs.push(MetisJob {
            id: metis_id.clone(),
            status: JobStatus::Running,
            creation_time: Some(Utc::now()),
            start_time: Some(Utc::now()),
            completion_time: None,
            failure_message: None,
        });
        self.env_vars
            .lock()
            .unwrap()
            .insert(metis_id.clone(), env_vars.clone());
        self.resource_limits
            .lock()
            .unwrap()
            .insert(metis_id.clone(), (cpu_limit, memory_limit));
        self.resource_requests
            .lock()
            .unwrap()
            .insert(metis_id.clone(), (cpu_request, memory_request));
        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        let jobs = self.jobs.lock().unwrap();
        Ok(jobs.clone())
    }

    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        let mut matches: Vec<MetisJob> = {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter()
                .filter(|job| &job.id == metis_id)
                .cloned()
                .collect()
        };

        match matches.len() {
            0 => Err(JobEngineError::NotFound(metis_id.clone())),
            1 => Ok(matches.remove(0)),
            _ => Err(JobEngineError::MultipleFound(metis_id.clone())),
        }
    }

    async fn get_logs(
        &self,
        job_id: &TaskId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let exists = {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter().any(|job| job.id == *job_id)
        };

        if !exists {
            return Err(JobEngineError::NotFound(job_id.clone()));
        }

        let logs = {
            let logs = self.logs.lock().unwrap();
            logs.get(job_id).cloned().unwrap_or_default()
        };

        let tail_count = tail_lines.unwrap_or(logs.len() as i64).max(0) as usize;
        let start = logs.len().saturating_sub(tail_count);
        Ok(logs[start..].join("\n"))
    }

    fn get_logs_stream(
        &self,
        job_id: &TaskId,
        _follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let exists = {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter().any(|job| job.id == *job_id)
        };

        if !exists {
            return Err(JobEngineError::NotFound(job_id.clone()));
        }

        let logs = self.logs.clone();
        let (tx, rx) = mpsc::unbounded();
        let job_id = job_id.clone();

        tokio::spawn(async move {
            let chunks = {
                let guard = logs.lock().unwrap();
                guard.get(&job_id).cloned().unwrap_or_default()
            };

            for chunk in chunks {
                if tx.unbounded_send(chunk).is_err() {
                    return;
                }
            }
        });

        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        let mut jobs = self.jobs.lock().unwrap();
        let matching_indices: Vec<_> = jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| &job.id == metis_id)
            .map(|(idx, _)| idx)
            .collect();

        match matching_indices.len() {
            0 => Err(JobEngineError::NotFound(metis_id.clone())),
            1 => {
                let idx = matching_indices[0];
                if let Some(job) = jobs.get_mut(idx) {
                    job.status = JobStatus::Failed;
                    job.completion_time = Some(Utc::now());
                }
                Ok(())
            }
            _ => Err(JobEngineError::MultipleFound(metis_id.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_engine::JobEngine;
    use std::collections::HashMap;

    #[tokio::test]
    async fn create_job_records_env_vars() {
        let engine = MockJobEngine::new();
        let env_vars = HashMap::from([("FOO".to_string(), "bar".to_string())]);
        let metis_id = TaskId::new();
        let (actor, _) = crate::domain::actors::Actor::new_for_session(
            TaskId::new(),
            crate::domain::users::Username::from("creator"),
        );

        engine
            .create_job(
                &metis_id,
                &actor,
                "token",
                "image",
                &env_vars,
                "250m".to_string(),
                "128Mi".to_string(),
                "100m".to_string(),
                "64Mi".to_string(),
            )
            .await
            .expect("job creation should succeed");

        let recorded = engine
            .env_vars_for_job(&metis_id)
            .expect("env vars should be recorded");
        assert_eq!(recorded.get("FOO"), Some(&"bar".to_string()));

        let limits = engine
            .resource_limits_for_job(&metis_id)
            .expect("resource limits should be recorded");
        assert_eq!(limits, ("250m".to_string(), "128Mi".to_string()));

        let requests = engine
            .resource_requests_for_job(&metis_id)
            .expect("resource requests should be recorded");
        assert_eq!(requests, ("100m".to_string(), "64Mi".to_string()));
    }
}
