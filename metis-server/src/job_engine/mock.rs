use super::{JobEngine, JobEngineError, JobStatus, MetisId, MetisJob};
use async_trait::async_trait;
use chrono::Utc;
use futures::channel::mpsc;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct MockJobEngine {
    jobs: Arc<Mutex<Vec<MetisJob>>>,
}

impl MockJobEngine {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    async fn insert_job(&self, metis_id: &MetisId, status: JobStatus) {
        let mut jobs = self.jobs.lock().await;
        jobs.push(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(Utc::now()),
            start_time: Some(Utc::now()),
            completion_time: None,
            failure_message: None,
        });
    }
}

#[async_trait]
impl JobEngine for MockJobEngine {
    async fn create_job(&self, metis_id: &MetisId, _prompt: &str) -> Result<(), JobEngineError> {
        let mut jobs = self.jobs.lock().await;
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
        let jobs = self.jobs.lock().await;
        Ok(jobs.clone())
    }

    async fn find_job_by_metis_id(&self, metis_id: &MetisId) -> Result<MetisJob, JobEngineError> {
        let mut matches: Vec<MetisJob> = {
            let jobs = self.jobs.lock().await;
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
        _job_id: &str,
        _tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        Ok(String::new())
    }

    fn get_logs_stream(
        &self,
        _job_id: &str,
        _follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let (_tx, rx) = mpsc::unbounded();
        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &MetisId) -> Result<(), JobEngineError> {
        let mut jobs = self.jobs.lock().await;
        if let Some(index) = jobs.iter().position(|job| &job.id == metis_id) {
            let mut job = jobs.remove(index);
            job.status = JobStatus::Failed;
            job.completion_time = Some(Utc::now());
            jobs.push(job);
            return Ok(());
        }

        Err(JobEngineError::NotFound(metis_id.clone()))
    }
}
