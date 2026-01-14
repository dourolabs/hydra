use super::{LogStream, MetisClientInterface};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream;
use metis_common::{
    issues::{
        IssueRecord, ListIssuesResponse, SearchIssuesQuery, UpsertIssueRequest, UpsertIssueResponse,
    },
    job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobRecord, KillJobResponse, ListJobsResponse,
        SearchJobsQuery, WorkerContext,
    },
    logs::LogsQuery,
    patches::{
        ListPatchesResponse, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    IssueId, PatchId, TaskId,
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
pub struct MockMetisClient {
    pub create_job_responses: Mutex<VecDeque<CreateJobResponse>>,
    pub list_jobs_responses: Mutex<VecDeque<ListJobsResponse>>,
    pub log_responses: Mutex<VecDeque<Vec<String>>>,
    pub log_requests: Mutex<Vec<TaskId>>,
    pub issue_upsert_responses: Mutex<VecDeque<UpsertIssueResponse>>,
    pub patch_upsert_responses: Mutex<VecDeque<UpsertPatchResponse>>,
    pub get_issue_responses: Mutex<VecDeque<IssueRecord>>,
    pub get_patch_responses: Mutex<VecDeque<PatchRecord>>,
    pub list_issue_responses: Mutex<VecDeque<ListIssuesResponse>>,
    pub list_patch_responses: Mutex<VecDeque<ListPatchesResponse>>,
    pub issue_upsert_requests: Mutex<Vec<(Option<IssueId>, UpsertIssueRequest)>>,
    pub patch_upsert_requests: Mutex<Vec<(Option<PatchId>, UpsertPatchRequest)>>,
    pub issue_get_requests: Mutex<Vec<IssueId>>,
    pub patch_get_requests: Mutex<Vec<PatchId>>,
    pub list_job_queries: Mutex<Vec<SearchJobsQuery>>,
    pub list_issue_queries: Mutex<Vec<SearchIssuesQuery>>,
    pub list_patch_queries: Mutex<Vec<SearchPatchesQuery>>,
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

    pub fn recorded_log_requests(&self) -> Vec<TaskId> {
        self.log_requests.lock().unwrap().clone()
    }

    pub fn push_upsert_issue_response(&self, response: UpsertIssueResponse) {
        self.issue_upsert_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_upsert_patch_response(&self, response: UpsertPatchResponse) {
        self.patch_upsert_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_get_issue_response(&self, response: IssueRecord) {
        self.get_issue_responses.lock().unwrap().push_back(response);
    }

    pub fn push_get_patch_response(&self, response: PatchRecord) {
        self.get_patch_responses.lock().unwrap().push_back(response);
    }

    pub fn push_list_issues_response(&self, response: ListIssuesResponse) {
        self.list_issue_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_patches_response(&self, response: ListPatchesResponse) {
        self.list_patch_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn recorded_issue_upserts(&self) -> Vec<(Option<IssueId>, UpsertIssueRequest)> {
        self.issue_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_patch_upserts(&self) -> Vec<(Option<PatchId>, UpsertPatchRequest)> {
        self.patch_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_get_issue_requests(&self) -> Vec<IssueId> {
        self.issue_get_requests.lock().unwrap().clone()
    }

    pub fn recorded_get_patch_requests(&self) -> Vec<PatchId> {
        self.patch_get_requests.lock().unwrap().clone()
    }

    pub fn recorded_list_issue_queries(&self) -> Vec<SearchIssuesQuery> {
        self.list_issue_queries.lock().unwrap().clone()
    }

    pub fn recorded_list_patch_queries(&self) -> Vec<SearchPatchesQuery> {
        self.list_patch_queries.lock().unwrap().clone()
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

    async fn list_jobs(&self, query: &SearchJobsQuery) -> Result<ListJobsResponse> {
        self.list_job_queries.lock().unwrap().push(query.clone());
        self.list_jobs_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_jobs"))
    }

    async fn get_job(&self, _job_id: &TaskId) -> Result<JobRecord> {
        Err(anyhow!("get_job not implemented in MockMetisClient"))
    }

    async fn kill_job(&self, _job_id: &TaskId) -> Result<KillJobResponse> {
        Err(anyhow!("kill_job not implemented in MockMetisClient"))
    }

    async fn get_job_logs(&self, job_id: &TaskId, _query: &LogsQuery) -> Result<LogStream> {
        self.log_requests.lock().unwrap().push(job_id.clone());
        let lines = self
            .log_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_job_logs"))?;
        let stream = stream::iter(lines.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }

    async fn set_job_status(
        &self,
        _job_id: &TaskId,
        _status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse> {
        Err(anyhow!("set_job_status not implemented in MockMetisClient"))
    }

    async fn get_job_status(&self, _job_id: &TaskId) -> Result<GetJobStatusResponse> {
        Err(anyhow!("get_job_status not implemented in MockMetisClient"))
    }

    async fn get_job_context(&self, _job_id: &TaskId) -> Result<WorkerContext> {
        Err(anyhow!(
            "get_job_context not implemented in MockMetisClient"
        ))
    }

    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        self.issue_upsert_requests
            .lock()
            .unwrap()
            .push((None, request.clone()));
        self.issue_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_issue"))
    }

    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        self.issue_upsert_requests
            .lock()
            .unwrap()
            .push((Some(issue_id.clone()), request.clone()));
        self.issue_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for update_issue"))
    }

    async fn get_issue(&self, issue_id: &IssueId) -> Result<IssueRecord> {
        self.issue_get_requests
            .lock()
            .unwrap()
            .push(issue_id.clone());
        self.get_issue_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_issue"))
    }

    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        self.list_issue_queries.lock().unwrap().push(query.clone());
        self.list_issue_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_issues"))
    }

    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        self.patch_upsert_requests
            .lock()
            .unwrap()
            .push((None, request.clone()));
        self.patch_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_patch"))
    }

    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        self.patch_upsert_requests
            .lock()
            .unwrap()
            .push((Some(patch_id.clone()), request.clone()));
        self.patch_upsert_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for update_patch"))
    }

    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchRecord> {
        self.patch_get_requests
            .lock()
            .unwrap()
            .push(patch_id.clone());
        self.get_patch_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_patch"))
    }

    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        self.list_patch_queries.lock().unwrap().push(query.clone());
        self.list_patch_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_patches"))
    }
}
