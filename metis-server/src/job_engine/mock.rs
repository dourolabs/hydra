use super::{JobEngine, JobEngineError, JobStatus, MetisId, MetisJob};
use async_trait::async_trait;
use chrono::Utc;
use futures::channel::mpsc;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub struct MockJobEngine {
    jobs: Arc<Mutex<Vec<MetisJob>>>,
    logs: Arc<Mutex<HashMap<MetisId, Vec<String>>>>,
}

impl MockJobEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert_job(&self, metis_id: &MetisId, status: JobStatus) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.push(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(Utc::now()),
            start_time: Some(Utc::now()),
            completion_time: None,
            failure_message: None,
        });
    }

    pub async fn set_logs(&self, metis_id: &MetisId, chunks: Vec<String>) {
        let mut logs = self.logs.lock().unwrap();
        logs.insert(metis_id.to_string(), chunks);
    }
}

#[async_trait]
impl JobEngine for MockJobEngine {
    async fn create_job(&self, metis_id: &MetisId, _image: &str) -> Result<(), JobEngineError> {
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
        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        let jobs = self.jobs.lock().unwrap();
        Ok(jobs.clone())
    }

    async fn find_job_by_metis_id(&self, metis_id: &MetisId) -> Result<MetisJob, JobEngineError> {
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
        job_id: &MetisId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let exists = {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter().any(|job| job.id == job_id)
        };

        if !exists {
            return Err(JobEngineError::NotFound(job_id.clone()));
        }

        let logs = {
            let logs = self.logs.lock().unwrap();
            logs.get(&job_id).cloned().unwrap_or_default()
        };

        let tail_count = tail_lines.unwrap_or(logs.len() as i64).max(0) as usize;
        let start = logs.len().saturating_sub(tail_count);
        Ok(logs[start..].join("\n"))
    }

    fn get_logs_stream(
        &self,
        job_id: &MetisId,
        _follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let exists = {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter().any(|job| job.id == job_id)
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

    async fn kill_job(&self, metis_id: &MetisId) -> Result<(), JobEngineError> {
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
