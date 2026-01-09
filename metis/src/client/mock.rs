use super::{LogStream, MetisClientInterface};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream;
use metis_common::{
    artifacts::{
        ArtifactRecord, ListArtifactsResponse, SearchArtifactsQuery, UpsertArtifactRequest,
        UpsertArtifactResponse,
    },
    job_outputs::{JobOutputResponse, SetJobOutputResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobSummary, KillJobResponse, ListJobsResponse,
        WorkerContext,
    },
    logs::LogsQuery,
    MetisId,
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockMetisClient {
    pub create_job_responses: Mutex<VecDeque<CreateJobResponse>>,
    pub list_jobs_responses: Mutex<VecDeque<ListJobsResponse>>,
    pub log_responses: Mutex<VecDeque<Vec<String>>>,
    pub log_requests: Mutex<Vec<MetisId>>,
    pub artifact_upsert_responses: Mutex<VecDeque<UpsertArtifactResponse>>,
    pub get_artifact_responses: Mutex<VecDeque<ArtifactRecord>>,
    pub list_artifacts_responses: Mutex<VecDeque<ListArtifactsResponse>>,
    pub artifact_upsert_requests: Mutex<Vec<(Option<MetisId>, UpsertArtifactRequest)>>,
    pub artifact_get_requests: Mutex<Vec<MetisId>>,
    pub list_artifacts_queries: Mutex<Vec<SearchArtifactsQuery>>,
    pub recorded_requests: Mutex<Vec<CreateJobRequest>>,
}

impl MockMetisClient {
    pub fn push_create_job_response(&self, response: CreateJobResponse) {
        self.create_job_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_jobs_response(&self, response: ListJobsResponse) {
        self.list_jobs_responses.lock().unwrap().push_back(response);
    }

    pub fn push_log_lines<I, S>(&self, lines: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let lines = lines.into_iter().map(Into::into).collect();
        self.log_responses.lock().unwrap().push_back(lines);
    }

    pub fn recorded_requests(&self) -> Vec<CreateJobRequest> {
        self.recorded_requests.lock().unwrap().clone()
    }

    pub fn recorded_log_requests(&self) -> Vec<MetisId> {
        self.log_requests.lock().unwrap().clone()
    }

    pub fn push_upsert_artifact_response(&self, response: UpsertArtifactResponse) {
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_get_artifact_response(&self, response: ArtifactRecord) {
        self.get_artifact_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_artifacts_response(&self, response: ListArtifactsResponse) {
        self.list_artifacts_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn recorded_artifact_upserts(&self) -> Vec<(Option<MetisId>, UpsertArtifactRequest)> {
        self.artifact_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_get_artifact_requests(&self) -> Vec<MetisId> {
        self.artifact_get_requests.lock().unwrap().clone()
    }

    pub fn recorded_list_artifacts_queries(&self) -> Vec<SearchArtifactsQuery> {
        self.list_artifacts_queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl MetisClientInterface for MockMetisClient {
    async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse> {
        self.recorded_requests.lock().unwrap().push(request.clone());
        self.create_job_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_job"))
    }

    async fn list_jobs(&self) -> Result<ListJobsResponse> {
        self.list_jobs_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_jobs"))
    }

    async fn get_job(&self, _job_id: &MetisId) -> Result<JobSummary> {
        Err(anyhow!("get_job not implemented in MockMetisClient"))
    }

    async fn kill_job(&self, _job_id: &MetisId) -> Result<KillJobResponse> {
        Err(anyhow!("kill_job not implemented in MockMetisClient"))
    }

    async fn get_job_logs(&self, job_id: &MetisId, _query: &LogsQuery) -> Result<LogStream> {
        self.log_requests.lock().unwrap().push(job_id.to_string());
        let lines = self
            .log_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_job_logs"))?;
        let stream = stream::iter(lines.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }

    async fn get_job_output(&self, _job_id: &MetisId) -> Result<JobOutputResponse> {
        Err(anyhow!("get_job_output not implemented in MockMetisClient"))
    }

    async fn set_job_output(&self, _job_id: &MetisId) -> Result<SetJobOutputResponse> {
        Err(anyhow!("set_job_output not implemented in MockMetisClient"))
    }

    async fn get_job_context(&self, _job_id: &MetisId) -> Result<WorkerContext> {
        Err(anyhow!(
            "get_job_context not implemented in MockMetisClient"
        ))
    }

    async fn create_artifact(
        &self,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        self.artifact_upsert_requests
            .lock()
            .unwrap()
            .push((None, request.clone()));
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_artifact"))
    }

    async fn update_artifact(
        &self,
        artifact_id: &MetisId,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        self.artifact_upsert_requests
            .lock()
            .unwrap()
            .push((Some(artifact_id.clone()), request.clone()));
        self.artifact_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for update_artifact"))
    }

    async fn get_artifact(&self, artifact_id: &MetisId) -> Result<ArtifactRecord> {
        self.artifact_get_requests
            .lock()
            .unwrap()
            .push(artifact_id.clone());
        self.get_artifact_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_artifact"))
    }

    async fn list_artifacts(&self, query: &SearchArtifactsQuery) -> Result<ListArtifactsResponse> {
        self.list_artifacts_queries
            .lock()
            .unwrap()
            .push(query.clone());
        self.list_artifacts_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_artifacts"))
    }
}
