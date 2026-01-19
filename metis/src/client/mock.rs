use super::{LogStream, MetisClientInterface};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::stream;
use metis_common::{
    agents::ListAgentsResponse,
    issues::{
        AddTodoItemRequest, IssueRecord, ListIssuesResponse, ReplaceTodoListRequest,
        SearchIssuesQuery, SetTodoItemStatusRequest, TodoListResponse, UpsertIssueRequest,
        UpsertIssueResponse,
    },
    job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobRecord, KillJobResponse, ListJobsResponse,
        SearchJobsQuery, WorkerContext,
    },
    logs::LogsQuery,
    merge_queues::MergeQueue,
    patches::{
        ListPatchesResponse, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    repositories::{
        CreateRepositoryRequest, ListRepositoriesResponse, UpdateRepositoryRequest,
        UpsertRepositoryResponse,
    },
    IssueId, PatchId, RepoName, TaskId,
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
    pub add_todo_responses: Mutex<VecDeque<TodoListResponse>>,
    pub replace_todo_responses: Mutex<VecDeque<TodoListResponse>>,
    pub set_todo_status_responses: Mutex<VecDeque<TodoListResponse>>,
    pub patch_upsert_responses: Mutex<VecDeque<UpsertPatchResponse>>,
    pub get_issue_responses: Mutex<VecDeque<IssueRecord>>,
    pub get_patch_responses: Mutex<VecDeque<PatchRecord>>,
    pub get_job_responses: Mutex<VecDeque<JobRecord>>,
    pub list_issue_responses: Mutex<VecDeque<ListIssuesResponse>>,
    pub list_patch_responses: Mutex<VecDeque<ListPatchesResponse>>,
    pub list_repository_responses: Mutex<VecDeque<ListRepositoriesResponse>>,
    pub create_repository_responses: Mutex<VecDeque<UpsertRepositoryResponse>>,
    pub update_repository_responses: Mutex<VecDeque<UpsertRepositoryResponse>>,
    pub list_agents_responses: Mutex<VecDeque<ListAgentsResponse>>,
    pub merge_queue_responses: Mutex<VecDeque<MergeQueue>>,
    pub enqueue_merge_queue_responses: Mutex<VecDeque<MergeQueue>>,
    pub issue_upsert_requests: Mutex<Vec<(Option<IssueId>, UpsertIssueRequest)>>,
    pub add_todo_requests: Mutex<Vec<(IssueId, AddTodoItemRequest)>>,
    pub replace_todo_requests: Mutex<Vec<(IssueId, ReplaceTodoListRequest)>>,
    pub set_todo_status_requests: Mutex<Vec<(IssueId, usize, SetTodoItemStatusRequest)>>,
    pub patch_upsert_requests: Mutex<Vec<(Option<PatchId>, UpsertPatchRequest)>>,
    pub create_repository_requests: Mutex<Vec<CreateRepositoryRequest>>,
    pub update_repository_requests: Mutex<Vec<(RepoName, UpdateRepositoryRequest)>>,
    pub issue_get_requests: Mutex<Vec<IssueId>>,
    pub patch_get_requests: Mutex<Vec<PatchId>>,
    pub job_get_requests: Mutex<Vec<TaskId>>,
    pub list_job_queries: Mutex<Vec<SearchJobsQuery>>,
    pub list_issue_queries: Mutex<Vec<SearchIssuesQuery>>,
    pub list_patch_queries: Mutex<Vec<SearchPatchesQuery>>,
    pub list_agents_calls: Mutex<usize>,
    pub merge_queue_requests: Mutex<Vec<(RepoName, String)>>,
    pub enqueue_merge_queue_requests: Mutex<Vec<(RepoName, String, PatchId)>>,
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

    pub fn push_add_todo_response(&self, response: TodoListResponse) {
        self.add_todo_responses.lock().unwrap().push_back(response);
    }

    pub fn push_replace_todo_response(&self, response: TodoListResponse) {
        self.replace_todo_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_set_todo_status_response(&self, response: TodoListResponse) {
        self.set_todo_status_responses
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

    pub fn push_get_job_response(&self, response: JobRecord) {
        self.get_job_responses.lock().unwrap().push_back(response);
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

    pub fn push_list_repositories_response(&self, response: ListRepositoriesResponse) {
        self.list_repository_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_create_repository_response(&self, response: UpsertRepositoryResponse) {
        self.create_repository_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_update_repository_response(&self, response: UpsertRepositoryResponse) {
        self.update_repository_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_list_agents_response(&self, response: ListAgentsResponse) {
        self.list_agents_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_merge_queue_response(&self, response: MergeQueue) {
        self.merge_queue_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn push_enqueue_merge_queue_response(&self, response: MergeQueue) {
        self.enqueue_merge_queue_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn recorded_issue_upserts(&self) -> Vec<(Option<IssueId>, UpsertIssueRequest)> {
        self.issue_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_add_todo_requests(&self) -> Vec<(IssueId, AddTodoItemRequest)> {
        self.add_todo_requests.lock().unwrap().clone()
    }

    pub fn recorded_replace_todo_requests(&self) -> Vec<(IssueId, ReplaceTodoListRequest)> {
        self.replace_todo_requests.lock().unwrap().clone()
    }

    pub fn recorded_set_todo_status_requests(
        &self,
    ) -> Vec<(IssueId, usize, SetTodoItemStatusRequest)> {
        self.set_todo_status_requests.lock().unwrap().clone()
    }

    pub fn recorded_patch_upserts(&self) -> Vec<(Option<PatchId>, UpsertPatchRequest)> {
        self.patch_upsert_requests.lock().unwrap().clone()
    }

    pub fn recorded_create_repository_requests(&self) -> Vec<CreateRepositoryRequest> {
        self.create_repository_requests.lock().unwrap().clone()
    }

    pub fn recorded_update_repository_requests(&self) -> Vec<(RepoName, UpdateRepositoryRequest)> {
        self.update_repository_requests.lock().unwrap().clone()
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

    pub fn recorded_list_agents_calls(&self) -> usize {
        *self.list_agents_calls.lock().unwrap()
    }

    pub fn recorded_merge_queue_requests(&self) -> Vec<(RepoName, String)> {
        self.merge_queue_requests.lock().unwrap().clone()
    }

    pub fn recorded_enqueue_merge_queue_requests(&self) -> Vec<(RepoName, String, PatchId)> {
        self.enqueue_merge_queue_requests.lock().unwrap().clone()
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

    async fn get_job(&self, job_id: &TaskId) -> Result<JobRecord> {
        self.job_get_requests.lock().unwrap().push(job_id.clone());
        self.get_job_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_job"))
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

    async fn add_todo_item(
        &self,
        issue_id: &IssueId,
        request: &AddTodoItemRequest,
    ) -> Result<TodoListResponse> {
        self.add_todo_requests
            .lock()
            .unwrap()
            .push((issue_id.clone(), request.clone()));
        self.add_todo_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for add_todo_item"))
    }

    async fn replace_todo_list(
        &self,
        issue_id: &IssueId,
        request: &ReplaceTodoListRequest,
    ) -> Result<TodoListResponse> {
        self.replace_todo_requests
            .lock()
            .unwrap()
            .push((issue_id.clone(), request.clone()));
        self.replace_todo_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for replace_todo_list"))
    }

    async fn set_todo_item_status(
        &self,
        issue_id: &IssueId,
        item_number: usize,
        request: &SetTodoItemStatusRequest,
    ) -> Result<TodoListResponse> {
        self.set_todo_status_requests.lock().unwrap().push((
            issue_id.clone(),
            item_number,
            request.clone(),
        ));
        self.set_todo_status_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for set_todo_item_status"))
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

    async fn list_repositories(&self) -> Result<ListRepositoriesResponse> {
        self.list_repository_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_repositories"))
    }

    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        self.create_repository_requests
            .lock()
            .unwrap()
            .push(request.clone());
        self.create_repository_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for create_repository"))
    }

    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        self.update_repository_requests
            .lock()
            .unwrap()
            .push((repo_name.clone(), request.clone()));
        self.update_repository_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for update_repository"))
    }

    async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue> {
        self.merge_queue_requests
            .lock()
            .unwrap()
            .push((repo_name.clone(), branch.to_string()));
        self.merge_queue_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for get_merge_queue"))
    }

    async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue> {
        self.enqueue_merge_queue_requests.lock().unwrap().push((
            repo_name.clone(),
            branch.to_string(),
            patch_id.clone(),
        ));
        self.enqueue_merge_queue_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for enqueue_merge_patch"))
    }

    async fn list_agents(&self) -> Result<ListAgentsResponse> {
        let mut calls = self.list_agents_calls.lock().unwrap();
        *calls = calls.saturating_add(1);
        self.list_agents_responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow!("no mock response configured for list_agents"))
    }
}
