use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use metis_common::{
    agents::{AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest},
    api::v1::error::ApiErrorBody,
    api::v1::login::{LoginRequest, LoginResponse},
    documents::{
        DocumentRecord, DocumentVersionRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
        SearchDocumentsQuery, UpsertDocumentRequest, UpsertDocumentResponse,
    },
    github::{GithubAppClientIdResponse, GithubTokenResponse},
    issues::{
        AddTodoItemRequest, IssueRecord, ListIssueVersionsResponse, ListIssuesResponse,
        ReplaceTodoListRequest, SearchIssuesQuery, SetTodoItemStatusRequest, TodoListResponse,
        UpsertIssueRequest, UpsertIssueResponse,
    },
    job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobRecord, KillJobResponse, ListJobVersionsResponse,
        ListJobsResponse, SearchJobsQuery, WorkerContext,
    },
    logs::LogsQuery,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
    patches::{
        CreatePatchAssetQuery, CreatePatchAssetResponse, ListPatchVersionsResponse,
        ListPatchesResponse, PatchRecord, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    repositories::{
        CreateRepositoryRequest, ListRepositoriesResponse, UpdateRepositoryRequest,
        UpsertRepositoryResponse,
    },
    whoami::WhoAmIResponse,
    DocumentId, IssueId, PatchId, RepoName, TaskId, VersionNumber,
};
use reqwest::{header, Client as HttpClient, RequestBuilder, Response, Url};
use serde::Deserialize;
use std::path::Path;
use std::pin::Pin;

use crate::config::AppConfig;

/// HTTP client for interacting with the metis-server REST API.
#[derive(Clone)]
pub struct MetisClient {
    base_url: Url,
    http: HttpClient,
    auth_token: String,
}

/// HTTP client for interacting with unauthenticated metis-server endpoints.
#[derive(Clone)]
pub struct MetisClientUnauthenticated {
    base_url: Url,
    http: HttpClient,
}

pub type LogStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;
type BytesStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

trait ResponseExt {
    async fn error_for_status_with_body(self, context: &str) -> Result<Response>;
}

impl ResponseExt for Response {
    async fn error_for_status_with_body(self, context: &str) -> Result<Response> {
        let status = self.status();
        if status.is_success() {
            return Ok(self);
        }

        let is_json = self
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("application/json"))
            .unwrap_or(false);

        let body_text = self.text().await.unwrap_or_default();

        let server_message = if is_json {
            serde_json::from_str::<ApiErrorBody>(&body_text)
                .ok()
                .map(|body| body.error)
        } else {
            None
        }
        .or_else(|| {
            let trimmed = body_text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let mut message = format!("{context}: {status}");
        if let Some(details) = server_message {
            message.push_str(": ");
            message.push_str(&details);
        }

        Err(anyhow!(message))
    }
}

#[async_trait]
pub trait MetisClientInterface: Send + Sync {
    fn base_url(&self) -> &Url;

    async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse>;
    async fn list_jobs(&self, query: &SearchJobsQuery) -> Result<ListJobsResponse>;
    #[allow(dead_code)]
    async fn get_job(&self, job_id: &TaskId) -> Result<JobRecord>;
    async fn kill_job(&self, job_id: &TaskId) -> Result<KillJobResponse>;
    async fn get_job_logs(&self, job_id: &TaskId, query: &LogsQuery) -> Result<LogStream>;
    async fn set_job_status(
        &self,
        job_id: &TaskId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse>;
    #[allow(dead_code)]
    async fn get_job_status(&self, job_id: &TaskId) -> Result<GetJobStatusResponse>;

    async fn get_job_context(&self, job_id: &TaskId) -> Result<WorkerContext>;
    async fn list_job_versions(&self, job_id: &TaskId) -> Result<ListJobVersionsResponse>;
    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse>;
    #[allow(dead_code)]
    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse>;
    async fn add_todo_item(
        &self,
        issue_id: &IssueId,
        request: &AddTodoItemRequest,
    ) -> Result<TodoListResponse>;
    async fn replace_todo_list(
        &self,
        issue_id: &IssueId,
        request: &ReplaceTodoListRequest,
    ) -> Result<TodoListResponse>;
    async fn set_todo_item_status(
        &self,
        issue_id: &IssueId,
        item_number: usize,
        request: &SetTodoItemStatusRequest,
    ) -> Result<TodoListResponse>;
    async fn get_issue(&self, issue_id: &IssueId) -> Result<IssueRecord>;
    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse>;
    async fn list_issue_versions(&self, issue_id: &IssueId) -> Result<ListIssueVersionsResponse>;
    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse>;
    #[allow(dead_code)]
    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse>;
    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchRecord>;
    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse>;
    async fn list_patch_versions(&self, patch_id: &PatchId) -> Result<ListPatchVersionsResponse>;
    async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse>;
    async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse>;
    async fn get_document(&self, document_id: &DocumentId) -> Result<DocumentRecord>;
    async fn get_document_by_path(&self, path: &str) -> Result<DocumentRecord>;
    async fn list_documents(&self, query: &SearchDocumentsQuery) -> Result<ListDocumentsResponse>;
    async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse>;
    async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: &VersionNumber,
    ) -> Result<DocumentVersionRecord>;
    async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String>;
    async fn list_repositories(&self) -> Result<ListRepositoriesResponse>;
    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse>;
    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse>;
    async fn get_github_token(&self) -> Result<String>;
    async fn whoami(&self) -> Result<WhoAmIResponse>;
    async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue>;
    async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue>;
    async fn list_agents(&self) -> Result<ListAgentsResponse>;
    async fn get_agent(&self, name: &str) -> Result<AgentResponse>;
    async fn create_agent(&self, request: &UpsertAgentRequest) -> Result<AgentResponse>;
    async fn update_agent(&self, name: &str, request: &UpsertAgentRequest)
        -> Result<AgentResponse>;
    async fn delete_agent(&self, name: &str) -> Result<DeleteAgentResponse>;
}

impl MetisClientUnauthenticated {
    /// Construct a new client using the server URL from the CLI configuration.
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let server = config.default_server()?;
        Self::new(&server.url)
    }

    /// Construct a new client with the default reqwest HTTP client.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self> {
        Self::with_http_client(base_url, HttpClient::new())
    }

    /// Construct a new client with a custom `reqwest::Client`.
    pub fn with_http_client(base_url: impl AsRef<str>, http: HttpClient) -> Result<Self> {
        let url = Url::parse(base_url.as_ref())
            .with_context(|| format!("invalid Metis server URL '{}'", base_url.as_ref()))?;

        Ok(Self {
            base_url: url,
            http,
        })
    }

    /// Expose the underlying HTTP client for advanced operations.
    #[allow(dead_code)]
    pub fn http_client(&self) -> &HttpClient {
        &self.http
    }

    /// Expose the resolved base URL used for requests.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Call `POST /v1/login` to exchange a GitHub token for a Metis login token.
    pub async fn login(&self, request: &LoginRequest) -> Result<(String, MetisClient)> {
        self.login_with_http_client(self.http.clone(), request)
            .await
    }

    /// Call `POST /v1/login` using a custom `reqwest::Client`.
    pub async fn login_with_http_client(
        &self,
        http: HttpClient,
        request: &LoginRequest,
    ) -> Result<(String, MetisClient)> {
        let url = self
            .endpoint("/v1/login")
            .with_context(|| "failed to construct login endpoint URL")?;
        let response = http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit login request")?
            .error_for_status_with_body("metis-server rejected login request")
            .await?;

        let login_response = response
            .json::<LoginResponse>()
            .await
            .context("failed to decode login response")?;
        let auth_token = login_response.login_token.clone();
        let client =
            MetisClient::with_http_client(self.base_url.as_str(), auth_token.clone(), http)?;

        Ok((auth_token, client))
    }

    /// Call `GET /v1/github/app/client-id` to fetch the GitHub OAuth client id.
    pub async fn get_github_app_client_id(&self) -> Result<GithubAppClientIdResponse> {
        let url = self.endpoint("/v1/github/app/client-id")?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch GitHub app client id")?
            .error_for_status_with_body(
                "metis-server returned an error while fetching GitHub app client id",
            )
            .await?;

        response
            .json::<GithubAppClientIdResponse>()
            .await
            .context("failed to decode GitHub app client id response")
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .with_context(|| format!("failed to construct endpoint URL for '{path}'"))
    }
}

impl MetisClient {
    /// Construct a new client using the server URL from the CLI configuration.
    pub fn from_config(config: &AppConfig, auth_token: impl Into<String>) -> Result<Self> {
        let server = config.default_server()?;
        Self::new(&server.url, auth_token)
    }

    /// Construct a new client with the default reqwest HTTP client.
    pub fn new(base_url: impl AsRef<str>, auth_token: impl Into<String>) -> Result<Self> {
        Self::with_http_client(base_url, auth_token, HttpClient::new())
    }

    /// Construct a new client with a custom `reqwest::Client`.
    pub fn with_http_client(
        base_url: impl AsRef<str>,
        auth_token: impl Into<String>,
        http: HttpClient,
    ) -> Result<Self> {
        let url = Url::parse(base_url.as_ref())
            .with_context(|| format!("invalid Metis server URL '{}'", base_url.as_ref()))?;

        Ok(Self {
            base_url: url,
            http,
            auth_token: auth_token.into(),
        })
    }

    /// Expose the underlying HTTP client for advanced operations.
    #[allow(dead_code)]
    pub fn http_client(&self) -> &HttpClient {
        &self.http
    }

    /// Expose the resolved base URL used for requests.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Expose the auth token used for requests.
    #[allow(dead_code)]
    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    fn authed(&self, builder: RequestBuilder) -> RequestBuilder {
        builder.bearer_auth(&self.auth_token)
    }

    /// Call the `/health` endpoint and return the reported status string.
    #[allow(dead_code)]
    pub async fn health(&self) -> Result<String> {
        #[allow(dead_code)]
        #[derive(Deserialize)]
        struct HealthResponse {
            #[allow(dead_code)]
            status: String,
        }

        let url = self.endpoint("/health")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to contact metis-server health endpoint")?
            .error_for_status_with_body("metis-server health endpoint returned an error status")
            .await?;

        let health = response
            .json::<HealthResponse>()
            .await
            .context("failed to decode metis-server health response")?;

        Ok(health.status)
    }

    /// Call `POST /v1/jobs` to create a new job.
    pub async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse> {
        let url = self.endpoint("/v1/jobs")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create job request")?
            .error_for_status_with_body("metis-server rejected create job request")
            .await?;

        response
            .json::<CreateJobResponse>()
            .await
            .context("failed to decode create job response")
    }

    /// Call `GET /v1/jobs/` to list existing jobs.
    pub async fn list_jobs(&self, query: &SearchJobsQuery) -> Result<ListJobsResponse> {
        let url = self.endpoint("/v1/jobs/")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch jobs list")?
            .error_for_status_with_body("metis-server returned an error while listing jobs")
            .await?;

        response
            .json::<ListJobsResponse>()
            .await
            .context("failed to decode list jobs response")
    }

    /// Call `GET /v1/jobs/:job_id/versions` to list job history.
    pub async fn list_job_versions(&self, job_id: &TaskId) -> Result<ListJobVersionsResponse> {
        let path = format!("/v1/jobs/{job_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch job versions")?
            .error_for_status_with_body("metis-server returned an error while listing job versions")
            .await?;

        response
            .json::<ListJobVersionsResponse>()
            .await
            .context("failed to decode list job versions response")
    }

    /// Call `GET /v1/jobs/:job_id` to fetch an individual job summary.
    pub async fn get_job(&self, job_id: &TaskId) -> Result<JobRecord> {
        let path = format!("/v1/jobs/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch job")?
            .error_for_status_with_body("metis-server returned an error while fetching job")
            .await?;

        response
            .json::<JobRecord>()
            .await
            .context("failed to decode job response")
    }

    /// Call `DELETE /v1/jobs/:job_id` to terminate a running job.
    pub async fn kill_job(&self, job_id: &TaskId) -> Result<KillJobResponse> {
        let path = format!("/v1/jobs/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit kill job request")?
            .error_for_status_with_body("metis-server returned an error while killing job")
            .await?;

        response
            .json::<KillJobResponse>()
            .await
            .context("failed to decode kill job response")
    }

    /// Call `GET /v1/jobs/:job_id/logs` to fetch or stream job logs.
    ///
    /// When `query.watch` is `Some(true)` the returned stream yields log lines
    /// as new SSE events arrive.
    pub async fn get_job_logs(&self, job_id: &TaskId, query: &LogsQuery) -> Result<LogStream> {
        let path = format!("/v1/jobs/{job_id}/logs");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to request job logs")?
            .error_for_status_with_body("metis-server returned an error while fetching job logs")
            .await?;

        let is_sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("text/event-stream"))
            .unwrap_or(false);

        if is_sse {
            Ok(Self::stream_sse_logs(response))
        } else {
            let body = response.text().await?;
            Ok(Self::stream_text_logs(body))
        }
    }

    /// Call `POST /v1/jobs/:job_id/status` to update the recorded agent status.
    pub async fn set_job_status(
        &self,
        job_id: &TaskId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse> {
        let path = format!("/v1/jobs/{job_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(status)
            .send()
            .await
            .context("failed to submit set job status request")?
            .error_for_status_with_body("metis-server returned an error while setting job status")
            .await?;

        response
            .json::<SetJobStatusResponse>()
            .await
            .context("failed to decode set job status response")
    }

    /// Call `GET /v1/jobs/:job_id/status` to retrieve the status log for a job.
    pub async fn get_job_status(&self, job_id: &TaskId) -> Result<GetJobStatusResponse> {
        let path = format!("/v1/jobs/{job_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to request job status")?
            .error_for_status_with_body("metis-server returned an error while fetching job status")
            .await?;

        response
            .json::<GetJobStatusResponse>()
            .await
            .context("failed to decode job status response")
    }

    /// Call `GET /v1/jobs/:job_id/context` to retrieve the stored job context.
    pub async fn get_job_context(&self, job_id: &TaskId) -> Result<WorkerContext> {
        let path = format!("/v1/jobs/{job_id}/context");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to request job context")?
            .error_for_status_with_body("metis-server returned an error while fetching job context")
            .await?;
        response
            .json::<WorkerContext>()
            .await
            .context("failed to decode job context response")
    }

    /// Call `POST /v1/issues` to create a new issue.
    pub async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        let url = self.endpoint("/v1/issues")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create issue request")?
            .error_for_status_with_body("metis-server rejected create issue request")
            .await?;

        response
            .json::<UpsertIssueResponse>()
            .await
            .context("failed to decode create issue response")
    }

    /// Call `PUT /v1/issues/:issue_id` to update an existing issue.
    pub async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update issue request")?
            .error_for_status_with_body("metis-server returned an error while updating issue")
            .await?;

        response
            .json::<UpsertIssueResponse>()
            .await
            .context("failed to decode update issue response")
    }

    /// Call `GET /v1/issues/:issue_id` to fetch an issue.
    pub async fn get_issue(&self, issue_id: &IssueId) -> Result<IssueRecord> {
        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch issue")?
            .error_for_status_with_body("metis-server returned an error while fetching issue")
            .await?;

        response
            .json::<IssueRecord>()
            .await
            .context("failed to decode get issue response")
    }

    /// Call `GET /v1/issues` to list issues with optional filters.
    pub async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        let url = self.endpoint("/v1/issues")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch issues list")?
            .error_for_status_with_body("metis-server returned an error while listing issues")
            .await?;

        response
            .json::<ListIssuesResponse>()
            .await
            .context("failed to decode list issues response")
    }

    /// Call `GET /v1/issues/:issue_id/versions` to list issue history.
    pub async fn list_issue_versions(
        &self,
        issue_id: &IssueId,
    ) -> Result<ListIssueVersionsResponse> {
        let path = format!("/v1/issues/{issue_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch issue versions")?
            .error_for_status_with_body(
                "metis-server returned an error while listing issue versions",
            )
            .await?;

        response
            .json::<ListIssueVersionsResponse>()
            .await
            .context("failed to decode list issue versions response")
    }

    /// Call `POST /v1/issues/:issue_id/todo-items` to append a todo item.
    pub async fn add_todo_item(
        &self,
        issue_id: &IssueId,
        request: &AddTodoItemRequest,
    ) -> Result<TodoListResponse> {
        let path = format!("/v1/issues/{issue_id}/todo-items");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit add todo item request")?
            .error_for_status_with_body("metis-server rejected add todo item request")
            .await?;

        response
            .json::<TodoListResponse>()
            .await
            .context("failed to decode add todo item response")
    }

    /// Call `PUT /v1/issues/:issue_id/todo-items` to replace the todo list.
    pub async fn replace_todo_list(
        &self,
        issue_id: &IssueId,
        request: &ReplaceTodoListRequest,
    ) -> Result<TodoListResponse> {
        let path = format!("/v1/issues/{issue_id}/todo-items");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit replace todo list request")?
            .error_for_status_with_body("metis-server returned an error while replacing todo list")
            .await?;

        response
            .json::<TodoListResponse>()
            .await
            .context("failed to decode replace todo list response")
    }

    /// Call `POST /v1/issues/:issue_id/todo-items/:item_number` to update an item's status.
    pub async fn set_todo_item_status(
        &self,
        issue_id: &IssueId,
        item_number: usize,
        request: &SetTodoItemStatusRequest,
    ) -> Result<TodoListResponse> {
        let path = format!("/v1/issues/{issue_id}/todo-items/{item_number}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit todo status update request")?
            .error_for_status_with_body(
                "metis-server returned an error while updating todo item status",
            )
            .await?;

        response
            .json::<TodoListResponse>()
            .await
            .context("failed to decode todo status update response")
    }

    /// Call `POST /v1/patches` to create a new patch.
    pub async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        let url = self.endpoint("/v1/patches")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create patch request")?
            .error_for_status_with_body("metis-server rejected create patch request")
            .await?;

        response
            .json::<UpsertPatchResponse>()
            .await
            .context("failed to decode create patch response")
    }

    /// Call `PUT /v1/patches/:patch_id` to update an existing patch.
    pub async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update patch request")?
            .error_for_status_with_body("metis-server returned an error while updating patch")
            .await?;

        response
            .json::<UpsertPatchResponse>()
            .await
            .context("failed to decode update patch response")
    }

    /// Call `GET /v1/patches/:patch_id` to fetch a patch.
    pub async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchRecord> {
        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch patch")?
            .error_for_status_with_body("metis-server returned an error while fetching patch")
            .await?;

        response
            .json::<PatchRecord>()
            .await
            .context("failed to decode get patch response")
    }

    /// Call `GET /v1/patches` to list patches with optional filters.
    pub async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        let url = self.endpoint("/v1/patches")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch patches list")?
            .error_for_status_with_body("metis-server returned an error while listing patches")
            .await?;

        response
            .json::<ListPatchesResponse>()
            .await
            .context("failed to decode list patches response")
    }

    /// Call `GET /v1/patches/:patch_id/versions` to list patch history.
    pub async fn list_patch_versions(
        &self,
        patch_id: &PatchId,
    ) -> Result<ListPatchVersionsResponse> {
        let path = format!("/v1/patches/{patch_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch patch versions")?
            .error_for_status_with_body(
                "metis-server returned an error while listing patch versions",
            )
            .await?;

        response
            .json::<ListPatchVersionsResponse>()
            .await
            .context("failed to decode list patch versions response")
    }

    /// Call `POST /v1/documents` to create a document.
    pub async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        let url = self.endpoint("/v1/documents")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create document request")?
            .error_for_status_with_body("metis-server rejected create document request")
            .await?;

        response
            .json::<UpsertDocumentResponse>()
            .await
            .context("failed to decode create document response")
    }

    /// Call `PUT /v1/documents/:document_id` to update a document.
    pub async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        let path = format!("/v1/documents/{document_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update document request")?
            .error_for_status_with_body("metis-server returned an error while updating document")
            .await?;

        response
            .json::<UpsertDocumentResponse>()
            .await
            .context("failed to decode update document response")
    }

    /// Call `GET /v1/documents/:document_id` to fetch a document.
    pub async fn get_document(&self, document_id: &DocumentId) -> Result<DocumentRecord> {
        let path = format!("/v1/documents/{document_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch document")?
            .error_for_status_with_body("metis-server returned an error while fetching document")
            .await?;

        response
            .json::<DocumentRecord>()
            .await
            .context("failed to decode document response")
    }

    /// Fetch a document by its exact path.
    ///
    /// Uses the list documents endpoint with a path prefix filter and
    /// path_is_exact=true to find a document with an exact path match.
    pub async fn get_document_by_path(&self, path: &str) -> Result<DocumentRecord> {
        let query = SearchDocumentsQuery::new(None, Some(path.to_string()), Some(true), None, None);
        let response = self.list_documents(&query).await?;

        response
            .documents
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("document with path '{path}' not found"))
    }

    /// Call `GET /v1/documents` to list documents.
    pub async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<ListDocumentsResponse> {
        let url = self.endpoint("/v1/documents")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch documents list")?
            .error_for_status_with_body("metis-server returned an error while listing documents")
            .await?;

        response
            .json::<ListDocumentsResponse>()
            .await
            .context("failed to decode list documents response")
    }

    /// Call `GET /v1/documents/:document_id/versions` to list document versions.
    pub async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse> {
        let path = format!("/v1/documents/{document_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch document versions")?
            .error_for_status_with_body(
                "metis-server returned an error while listing document versions",
            )
            .await?;

        response
            .json::<ListDocumentVersionsResponse>()
            .await
            .context("failed to decode list document versions response")
    }

    /// Call `GET /v1/documents/:document_id/versions/:version` to fetch a document version.
    pub async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: &VersionNumber,
    ) -> Result<DocumentVersionRecord> {
        let path = format!("/v1/documents/{document_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch document version")?
            .error_for_status_with_body(
                "metis-server returned an error while fetching document version",
            )
            .await?;

        response
            .json::<DocumentVersionRecord>()
            .await
            .context("failed to decode document version response")
    }

    /// Call `POST /v1/patches/:patch_id/assets` to upload a patch asset.
    pub async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String> {
        let file_name = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string());
        let query = CreatePatchAssetQuery::new(file_name);
        let body = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("failed to read asset file '{}'", file_path.display()))?;
        let path = format!("/v1/patches/{patch_id}/assets");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .query(&query)
            .body(body)
            .send()
            .await
            .context("failed to submit patch asset upload")?
            .error_for_status_with_body(
                "metis-server returned an error while uploading patch asset",
            )
            .await?;

        let response = response
            .json::<CreatePatchAssetResponse>()
            .await
            .context("failed to decode patch asset upload response")?;

        Ok(response.asset_url)
    }

    /// Call `GET /v1/repositories` to list configured repositories.
    pub async fn list_repositories(&self) -> Result<ListRepositoriesResponse> {
        let url = self.endpoint("/v1/repositories")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch repositories list")?
            .error_for_status_with_body("metis-server returned an error while listing repositories")
            .await?;

        response
            .json::<ListRepositoriesResponse>()
            .await
            .context("failed to decode list repositories response")
    }

    /// Call `POST /v1/repositories` to create a new repository.
    pub async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        let url = self.endpoint("/v1/repositories")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create repository request")?
            .error_for_status_with_body("metis-server rejected create repository request")
            .await?;

        response
            .json::<UpsertRepositoryResponse>()
            .await
            .context("failed to decode create repository response")
    }

    /// Call `PUT /v1/repositories/:organization/:repo` to update a repository config.
    pub async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        let path = format!(
            "/v1/repositories/{}/{}",
            repo_name.organization, repo_name.repo
        );
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update repository request")?
            .error_for_status_with_body("metis-server returned an error while updating repository")
            .await?;

        response
            .json::<UpsertRepositoryResponse>()
            .await
            .context("failed to decode update repository response")
    }

    /// Call `GET /v1/github/token` to fetch the authenticated user's GitHub token.
    pub async fn get_github_token(&self) -> Result<String> {
        let url = self.endpoint("/v1/github/token")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch GitHub token")?
            .error_for_status_with_body(
                "metis-server returned an error while fetching GitHub token",
            )
            .await?;

        let token = response
            .json::<GithubTokenResponse>()
            .await
            .context("failed to decode GitHub token response")?;

        Ok(token.github_token)
    }

    /// Call `GET /v1/whoami` to identify the authenticated actor.
    pub async fn whoami(&self) -> Result<WhoAmIResponse> {
        let url = self.endpoint("/v1/whoami")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch whoami response")?
            .error_for_status_with_body("metis-server returned an error while fetching whoami")
            .await?;

        response
            .json::<WhoAmIResponse>()
            .await
            .context("failed to decode whoami response")
    }

    /// Call `GET /v1/merge-queues/:organization/:repo/:branch/patches` to fetch the merge queue.
    pub async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue> {
        let path = format!(
            "/v1/merge-queues/{}/{}/{}/patches",
            repo_name.organization, repo_name.repo, branch
        );
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch merge queue")?
            .error_for_status_with_body("metis-server returned an error while fetching merge queue")
            .await?;

        response
            .json::<MergeQueue>()
            .await
            .context("failed to decode merge queue response")
    }

    /// Call `POST /v1/merge-queues/:organization/:repo/:branch/patches` to enqueue a patch.
    pub async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue> {
        let path = format!(
            "/v1/merge-queues/{}/{}/{}/patches",
            repo_name.organization, repo_name.repo, branch
        );
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(&EnqueueMergePatchRequest::new(patch_id.clone()))
            .send()
            .await
            .context("failed to submit enqueue merge patch request")?
            .error_for_status_with_body(
                "metis-server returned an error while enqueuing merge patch",
            )
            .await?;

        response
            .json::<MergeQueue>()
            .await
            .context("failed to decode enqueue merge patch response")
    }

    /// Call `GET /v1/agents` to list available assignee agents.
    pub async fn list_agents(&self) -> Result<ListAgentsResponse> {
        let url = self.endpoint("/v1/agents")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch agents list")?
            .error_for_status_with_body("metis-server returned an error while listing agents")
            .await?;

        response
            .json::<ListAgentsResponse>()
            .await
            .context("failed to decode list agents response")
    }

    /// Call `GET /v1/agents/:name` to fetch a specific agent.
    pub async fn get_agent(&self, name: &str) -> Result<AgentResponse> {
        let url = self.endpoint(&format!("/v1/agents/{name}"))?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch agent")?
            .error_for_status_with_body("metis-server returned an error while fetching agent")
            .await?;

        response
            .json::<AgentResponse>()
            .await
            .context("failed to decode agent response")
    }

    /// Call `POST /v1/agents` to create an agent.
    pub async fn create_agent(&self, request: &UpsertAgentRequest) -> Result<AgentResponse> {
        let url = self.endpoint("/v1/agents")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create agent request")?
            .error_for_status_with_body("metis-server returned an error while creating agent")
            .await?;

        response
            .json::<AgentResponse>()
            .await
            .context("failed to decode create agent response")
    }

    /// Call `PUT /v1/agents/:name` to update an agent.
    pub async fn update_agent(
        &self,
        name: &str,
        request: &UpsertAgentRequest,
    ) -> Result<AgentResponse> {
        let url = self.endpoint(&format!("/v1/agents/{name}"))?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update agent request")?
            .error_for_status_with_body("metis-server returned an error while updating agent")
            .await?;

        response
            .json::<AgentResponse>()
            .await
            .context("failed to decode update agent response")
    }

    /// Call `DELETE /v1/agents/:name` to delete an agent.
    pub async fn delete_agent(&self, name: &str) -> Result<DeleteAgentResponse> {
        let url = self.endpoint(&format!("/v1/agents/{name}"))?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete agent request")?
            .error_for_status_with_body("metis-server returned an error while deleting agent")
            .await?;

        response
            .json::<DeleteAgentResponse>()
            .await
            .context("failed to decode delete agent response")
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .with_context(|| format!("failed to construct endpoint URL for '{path}'"))
    }

    fn stream_text_logs(body: String) -> LogStream {
        if body.is_empty() {
            Box::pin(stream::iter(Vec::<Result<String>>::new()))
        } else {
            Box::pin(stream::iter(vec![Ok(body)]))
        }
    }

    fn stream_sse_logs(response: Response) -> LogStream {
        Self::stream_sse_bytes(Box::pin(response.bytes_stream()))
    }

    fn stream_sse_bytes(byte_stream: BytesStream) -> LogStream {
        Box::pin(stream::unfold(
            (byte_stream, String::new(), false),
            |(mut byte_stream, mut buffer, finished)| async move {
                if finished {
                    return None;
                }

                loop {
                    if let Some((idx, separator_len)) = buffer
                        .find("\n\n")
                        .map(|idx| (idx, 2))
                        .or_else(|| buffer.find("\r\n\r\n").map(|idx| (idx, "\r\n\r\n".len())))
                    {
                        let event_block = buffer[..idx].to_string();
                        buffer.drain(..idx + separator_len);
                        if event_block.trim().is_empty() {
                            continue;
                        }

                        if let Some((event_name, data)) = parse_sse_event(&event_block) {
                            if event_name.as_deref() == Some("error") {
                                return Some((
                                    Err(anyhow!("error streaming logs: {data}")),
                                    (byte_stream, buffer, true),
                                ));
                            }

                            return Some((Ok(data), (byte_stream, buffer, false)));
                        }
                    }

                    match byte_stream.next().await {
                        Some(Ok(chunk)) => {
                            if chunk.is_empty() {
                                continue;
                            }
                            let chunk_text = String::from_utf8_lossy(&chunk);
                            buffer.push_str(&chunk_text);
                        }
                        Some(Err(err)) => {
                            return Some((Err(err.into()), (byte_stream, buffer, true)));
                        }
                        None => {
                            if buffer.trim().is_empty() {
                                return None;
                            }

                            if let Some((event_name, data)) = parse_sse_event(&buffer) {
                                let new_state = (byte_stream, String::new(), true);
                                if event_name.as_deref() == Some("error") {
                                    return Some((
                                        Err(anyhow!("error streaming logs: {data}")),
                                        new_state,
                                    ));
                                }

                                return Some((Ok(data), new_state));
                            } else {
                                return None;
                            }
                        }
                    }
                }
            },
        ))
    }
}

fn parse_sse_event(block: &str) -> Option<(Option<String>, String)> {
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            let trimmed = value.strip_prefix(' ').unwrap_or(value);
            data_lines.push(trimmed);
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some((event_name, data_lines.join("\n")))
    }
}

#[async_trait]
impl MetisClientInterface for MetisClient {
    fn base_url(&self) -> &Url {
        self.base_url()
    }

    async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse> {
        MetisClient::create_job(self, request).await
    }

    async fn list_jobs(&self, query: &SearchJobsQuery) -> Result<ListJobsResponse> {
        MetisClient::list_jobs(self, query).await
    }

    async fn get_job(&self, job_id: &TaskId) -> Result<JobRecord> {
        MetisClient::get_job(self, job_id).await
    }

    async fn kill_job(&self, job_id: &TaskId) -> Result<KillJobResponse> {
        MetisClient::kill_job(self, job_id).await
    }

    async fn get_job_logs(&self, job_id: &TaskId, query: &LogsQuery) -> Result<LogStream> {
        MetisClient::get_job_logs(self, job_id, query).await
    }

    async fn set_job_status(
        &self,
        job_id: &TaskId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse> {
        MetisClient::set_job_status(self, job_id, status).await
    }

    async fn get_job_status(&self, job_id: &TaskId) -> Result<GetJobStatusResponse> {
        MetisClient::get_job_status(self, job_id).await
    }

    async fn get_job_context(&self, job_id: &TaskId) -> Result<WorkerContext> {
        MetisClient::get_job_context(self, job_id).await
    }

    async fn list_job_versions(&self, job_id: &TaskId) -> Result<ListJobVersionsResponse> {
        MetisClient::list_job_versions(self, job_id).await
    }

    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        MetisClient::create_issue(self, request).await
    }

    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        MetisClient::update_issue(self, issue_id, request).await
    }

    async fn add_todo_item(
        &self,
        issue_id: &IssueId,
        request: &AddTodoItemRequest,
    ) -> Result<TodoListResponse> {
        MetisClient::add_todo_item(self, issue_id, request).await
    }

    async fn replace_todo_list(
        &self,
        issue_id: &IssueId,
        request: &ReplaceTodoListRequest,
    ) -> Result<TodoListResponse> {
        MetisClient::replace_todo_list(self, issue_id, request).await
    }

    async fn set_todo_item_status(
        &self,
        issue_id: &IssueId,
        item_number: usize,
        request: &SetTodoItemStatusRequest,
    ) -> Result<TodoListResponse> {
        MetisClient::set_todo_item_status(self, issue_id, item_number, request).await
    }

    async fn get_issue(&self, issue_id: &IssueId) -> Result<IssueRecord> {
        MetisClient::get_issue(self, issue_id).await
    }

    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        MetisClient::list_issues(self, query).await
    }

    async fn list_issue_versions(&self, issue_id: &IssueId) -> Result<ListIssueVersionsResponse> {
        MetisClient::list_issue_versions(self, issue_id).await
    }

    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        MetisClient::create_patch(self, request).await
    }

    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        MetisClient::update_patch(self, patch_id, request).await
    }

    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchRecord> {
        MetisClient::get_patch(self, patch_id).await
    }

    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        MetisClient::list_patches(self, query).await
    }

    async fn list_patch_versions(&self, patch_id: &PatchId) -> Result<ListPatchVersionsResponse> {
        MetisClient::list_patch_versions(self, patch_id).await
    }

    async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        MetisClient::create_document(self, request).await
    }

    async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        MetisClient::update_document(self, document_id, request).await
    }

    async fn get_document(&self, document_id: &DocumentId) -> Result<DocumentRecord> {
        MetisClient::get_document(self, document_id).await
    }

    async fn get_document_by_path(&self, path: &str) -> Result<DocumentRecord> {
        MetisClient::get_document_by_path(self, path).await
    }

    async fn list_documents(&self, query: &SearchDocumentsQuery) -> Result<ListDocumentsResponse> {
        MetisClient::list_documents(self, query).await
    }

    async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse> {
        MetisClient::list_document_versions(self, document_id).await
    }

    async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: &VersionNumber,
    ) -> Result<DocumentVersionRecord> {
        MetisClient::get_document_version(self, document_id, version).await
    }

    async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String> {
        MetisClient::create_patch_asset(self, patch_id, file_path).await
    }

    async fn list_repositories(&self) -> Result<ListRepositoriesResponse> {
        MetisClient::list_repositories(self).await
    }

    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        MetisClient::create_repository(self, request).await
    }

    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        MetisClient::update_repository(self, repo_name, request).await
    }

    async fn get_github_token(&self) -> Result<String> {
        MetisClient::get_github_token(self).await
    }

    async fn whoami(&self) -> Result<WhoAmIResponse> {
        MetisClient::whoami(self).await
    }

    async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue> {
        MetisClient::get_merge_queue(self, repo_name, branch).await
    }

    async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue> {
        MetisClient::enqueue_merge_patch(self, repo_name, branch, patch_id).await
    }

    async fn list_agents(&self) -> Result<ListAgentsResponse> {
        MetisClient::list_agents(self).await
    }

    async fn get_agent(&self, name: &str) -> Result<AgentResponse> {
        MetisClient::get_agent(self, name).await
    }

    async fn create_agent(&self, request: &UpsertAgentRequest) -> Result<AgentResponse> {
        MetisClient::create_agent(self, request).await
    }

    async fn update_agent(
        &self,
        name: &str,
        request: &UpsertAgentRequest,
    ) -> Result<AgentResponse> {
        MetisClient::update_agent(self, name, request).await
    }

    async fn delete_agent(&self, name: &str) -> Result<DeleteAgentResponse> {
        MetisClient::delete_agent(self, name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use metis_common::{
        repositories::{
            CreateRepositoryRequest, Repository, RepositoryRecord, UpdateRepositoryRequest,
        },
        PatchId,
    };
    use serde_json::json;
    use std::io::Write;
    use std::str::FromStr;
    use tempfile::tempdir;

    const TEST_METIS_TOKEN: &str = "u-test:test-metis-token";

    #[tokio::test]
    async fn list_repositories_fetches_config() -> Result<()> {
        let server = MockServer::start();
        let repositories = vec![RepositoryRecord::new(
            RepoName::from_str("dourolabs/metis")?,
            Repository::new(
                "https://example.com/repo.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/example/repo:main".to_string()),
            ),
        )];
        let payload = ListRepositoriesResponse::new(repositories);
        let payload_for_mock = payload.clone();
        let expected_auth_header = format!("Bearer {TEST_METIS_TOKEN}");

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/repositories")
                .header("authorization", expected_auth_header.as_str());
            then.status(200).json_body_obj(&payload_for_mock);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let response = client.list_repositories().await?;

        mock.assert();
        assert_eq!(response, payload);

        Ok(())
    }

    #[tokio::test]
    async fn create_repository_sends_payload_and_parses_response() -> Result<()> {
        let server = MockServer::start();
        let repo_name = RepoName::from_str("dourolabs/new-repo")?;
        let request = CreateRepositoryRequest::new(
            repo_name.clone(),
            Repository::new(
                "https://example.com/new-repo.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/example/new-repo:main".to_string()),
            ),
        );
        let response_body = UpsertRepositoryResponse::new(RepositoryRecord::new(
            repo_name.clone(),
            request.repository.clone(),
        ));

        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/repositories").json_body(json!({
                "name": "dourolabs/new-repo",
                "remote_url": "https://example.com/new-repo.git",
                "default_branch": "main",
                "default_image": "ghcr.io/example/new-repo:main"
            }));
            then.status(200).json_body_obj(&response_body);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let response = client.create_repository(&request).await?;

        mock.assert();
        assert_eq!(response.repository.name, repo_name);
        assert_eq!(
            response.repository.repository.default_image.as_deref(),
            Some("ghcr.io/example/new-repo:main")
        );

        Ok(())
    }

    #[tokio::test]
    async fn update_repository_includes_repo_and_propagates_errors() -> Result<()> {
        let server = MockServer::start();
        let repo_name = RepoName::from_str("dourolabs/missing")?;
        let request = UpdateRepositoryRequest::new(Repository::new(
            "https://example.com/updated.git".to_string(),
            None,
            None,
        ));

        let mock = server.mock(|when, then| {
            when.method(PUT)
                .path("/v1/repositories/dourolabs/missing")
                .json_body(json!({
                    "remote_url": "https://example.com/updated.git",
                    "default_branch": null,
                    "default_image": null
                }));
            then.status(404);
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let error = client
            .update_repository(&repo_name, &request)
            .await
            .unwrap_err();

        mock.assert();
        let message = format!("{error:#}");
        assert!(
            message.contains("metis-server returned an error while updating repository"),
            "{message}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_asset_uploads_file_and_returns_url() -> Result<()> {
        let server = MockServer::start();
        let patch_id = PatchId::new();
        let expected_auth_header = format!("Bearer {TEST_METIS_TOKEN}");
        let asset_url = "https://github.com/dourolabs/metis/assets/123";
        let path = format!("/v1/patches/{patch_id}/assets");

        let mock = server.mock(|when, then| {
            when.method(POST)
                .path(path.as_str())
                .query_param("name", "asset.txt")
                .header("authorization", expected_auth_header.as_str())
                .body("asset-bytes");
            then.status(200)
                .json_body(json!({ "asset_url": asset_url }));
        });

        let tempdir = tempdir()?;
        let file_path = tempdir.path().join("asset.txt");
        let mut file = std::fs::File::create(&file_path)?;
        file.write_all(b"asset-bytes")?;

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;

        let response = client.create_patch_asset(&patch_id, &file_path).await?;

        mock.assert();
        assert_eq!(response, asset_url);

        Ok(())
    }

    #[tokio::test]
    async fn stream_sse_logs_preserves_carriage_returns() {
        let events = b"data: Downloading 10%\rprogress\n\n";
        let byte_stream: BytesStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(events))]));

        let mut stream = MetisClient::stream_sse_bytes(byte_stream);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, "Downloading 10%\rprogress");
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn stream_sse_logs_handles_crlf_separators() {
        let events = b"data: first line\r\n\r\ndata: second\r\n\r\n";
        let byte_stream: BytesStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(events))]));

        let mut stream = MetisClient::stream_sse_bytes(byte_stream);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, "first line");

        let second = stream.next().await.unwrap().unwrap();
        assert_eq!(second, "second");

        assert!(stream.next().await.is_none());
    }
}
