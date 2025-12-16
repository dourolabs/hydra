use super::{LogStream, MetisClientInterface};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream;
use metis_common::{
    job_outputs::{JobOutputPayload, JobOutputResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobSummary, KillJobResponse, ListJobsResponse,
        WorkerContext,
    },
    logs::LogsQuery,
    workflows::{
        CreateWorkflowRequest, CreateWorkflowResponse, ListWorkflowsResponse, WorkflowSummary,
    },
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockMetisClient {
    pub create_job_responses: Mutex<VecDeque<CreateJobResponse>>,
    pub list_jobs_responses: Mutex<VecDeque<ListJobsResponse>>,
    pub list_workflows_responses: Mutex<VecDeque<ListWorkflowsResponse>>,
    pub workflow_summaries: Mutex<VecDeque<WorkflowSummary>>,
    pub log_responses: Mutex<VecDeque<Vec<String>>>,
    pub log_requests: Mutex<Vec<String>>,
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

    pub fn push_list_workflows_response(&self, response: ListWorkflowsResponse) {
        self.list_workflows_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_workflow_summary(&self, summary: WorkflowSummary) {
        self.workflow_summaries.lock().unwrap().push_back(summary);
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

    pub fn recorded_log_requests(&self) -> Vec<String> {
        self.log_requests.lock().unwrap().clone()
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

    async fn list_workflows(&self) -> Result<ListWorkflowsResponse> {
        self.list_workflows_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_workflows"))
    }

    async fn get_job(&self, _job_id: &str) -> Result<JobSummary> {
        Err(anyhow!("get_job not implemented in MockMetisClient"))
    }

    async fn get_workflow(&self, _workflow_id: &str) -> Result<WorkflowSummary> {
        self.workflow_summaries
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_workflow"))
    }

    async fn kill_job(&self, _job_id: &str) -> Result<KillJobResponse> {
        Err(anyhow!("kill_job not implemented in MockMetisClient"))
    }

    async fn get_job_logs(&self, job_id: &str, _query: &LogsQuery) -> Result<LogStream> {
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

    async fn get_job_output(&self, _job_id: &str) -> Result<JobOutputResponse> {
        Err(anyhow!("get_job_output not implemented in MockMetisClient"))
    }

    async fn set_job_output(
        &self,
        _job_id: &str,
        _payload: &JobOutputPayload,
    ) -> Result<JobOutputResponse> {
        Err(anyhow!("set_job_output not implemented in MockMetisClient"))
    }

    async fn get_job_context(&self, _job_id: &str) -> Result<WorkerContext> {
        Err(anyhow!(
            "get_job_context not implemented in MockMetisClient"
        ))
    }

    async fn create_workflow(
        &self,
        _request: &CreateWorkflowRequest,
    ) -> Result<CreateWorkflowResponse> {
        Err(anyhow!(
            "create_workflow not implemented in MockMetisClient"
        ))
    }
}
