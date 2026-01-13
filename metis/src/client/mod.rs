use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use metis_common::{
    artifacts::{
        IssueRecord, ListIssuesResponse, ListPatchesResponse, PatchRecord, SearchIssuesQuery,
        SearchPatchesQuery, UpsertIssueRequest, UpsertIssueResponse, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    job_status::{GetJobStatusResponse, JobStatusUpdate, SetJobStatusResponse},
    jobs::{
        CreateJobRequest, CreateJobResponse, JobSummary, KillJobResponse, ListJobsResponse,
        WorkerContext,
    },
    logs::LogsQuery,
    MetisId,
};
use reqwest::{header, Client as HttpClient, Response, Url};
use serde::Deserialize;
use std::pin::Pin;

use crate::config::AppConfig;

/// HTTP client for interacting with the metis-server REST API.
#[derive(Clone)]
pub struct MetisClient {
    base_url: Url,
    http: HttpClient,
}

pub type LogStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;
type BytesStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

#[async_trait]
pub trait MetisClientInterface: Send + Sync {
    async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse>;
    async fn list_jobs(&self) -> Result<ListJobsResponse>;
    #[allow(dead_code)]
    async fn get_job(&self, job_id: &MetisId) -> Result<JobSummary>;
    async fn kill_job(&self, job_id: &MetisId) -> Result<KillJobResponse>;
    async fn get_job_logs(&self, job_id: &MetisId, query: &LogsQuery) -> Result<LogStream>;
    async fn set_job_status(
        &self,
        job_id: &MetisId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse>;
    #[allow(dead_code)]
    async fn get_job_status(&self, job_id: &MetisId) -> Result<GetJobStatusResponse>;

    async fn get_job_context(&self, job_id: &MetisId) -> Result<WorkerContext>;
    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse>;
    #[allow(dead_code)]
    async fn update_issue(
        &self,
        issue_id: &MetisId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse>;
    async fn get_issue(&self, issue_id: &MetisId) -> Result<IssueRecord>;
    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse>;
    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse>;
    #[allow(dead_code)]
    async fn update_patch(
        &self,
        patch_id: &MetisId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse>;
    async fn get_patch(&self, patch_id: &MetisId) -> Result<PatchRecord>;
    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse>;
}

impl MetisClient {
    /// Construct a new client using the server URL from the CLI configuration.
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        Self::new(&config.server.url)
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
    #[allow(dead_code)]
    pub fn base_url(&self) -> &Url {
        &self.base_url
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
            .http
            .get(url)
            .send()
            .await
            .context("failed to contact metis-server health endpoint")?
            .error_for_status()
            .context("metis-server health endpoint returned an error status")?;

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
            .http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit create job request")?
            .error_for_status()
            .context("metis-server rejected create job request")?;

        response
            .json::<CreateJobResponse>()
            .await
            .context("failed to decode create job response")
    }

    /// Call `GET /v1/jobs/` to list existing jobs.
    pub async fn list_jobs(&self) -> Result<ListJobsResponse> {
        let url = self.endpoint("/v1/jobs/")?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch jobs list")?
            .error_for_status()
            .context("metis-server returned an error while listing jobs")?;

        response
            .json::<ListJobsResponse>()
            .await
            .context("failed to decode list jobs response")
    }

    /// Call `GET /v1/jobs/:job_id` to fetch an individual job summary.
    pub async fn get_job(&self, job_id: &MetisId) -> Result<JobSummary> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }

        let path = format!("/v1/jobs/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch job")?
            .error_for_status()
            .context("metis-server returned an error while fetching job")?;

        response
            .json::<JobSummary>()
            .await
            .context("failed to decode job response")
    }

    /// Call `DELETE /v1/jobs/:job_id` to terminate a running job.
    pub async fn kill_job(&self, job_id: &MetisId) -> Result<KillJobResponse> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }

        let path = format!("/v1/jobs/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .delete(url)
            .send()
            .await
            .context("failed to submit kill job request")?
            .error_for_status()
            .context("metis-server returned an error while killing job")?;

        response
            .json::<KillJobResponse>()
            .await
            .context("failed to decode kill job response")
    }

    /// Call `GET /v1/jobs/:job_id/logs` to fetch or stream job logs.
    ///
    /// When `query.watch` is `Some(true)` the returned stream yields log lines
    /// as new SSE events arrive.
    pub async fn get_job_logs(&self, job_id: &MetisId, query: &LogsQuery) -> Result<LogStream> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }

        let path = format!("/v1/jobs/{job_id}/logs");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .context("failed to request job logs")?
            .error_for_status()
            .context("metis-server returned an error while fetching job logs")?;

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
        job_id: &MetisId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }

        let path = format!("/v1/jobs/{job_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .post(url)
            .json(status)
            .send()
            .await
            .context("failed to submit set job status request")?
            .error_for_status()
            .context("metis-server returned an error while setting job status")?;

        response
            .json::<SetJobStatusResponse>()
            .await
            .context("failed to decode set job status response")
    }

    /// Call `GET /v1/jobs/:job_id/status` to retrieve the status log for a job.
    pub async fn get_job_status(&self, job_id: &MetisId) -> Result<GetJobStatusResponse> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }

        let path = format!("/v1/jobs/{job_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to request job status")?
            .error_for_status()
            .context("metis-server returned an error while fetching job status")?;

        response
            .json::<GetJobStatusResponse>()
            .await
            .context("failed to decode job status response")
    }

    /// Call `GET /v1/jobs/:job_id/context` to retrieve the stored job context.
    pub async fn get_job_context(&self, job_id: &MetisId) -> Result<WorkerContext> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(anyhow!("job_id must not be empty"));
        }
        let path = format!("/v1/jobs/{job_id}/context");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to request job context")?
            .error_for_status()
            .context("metis-server returned an error while fetching job context")?;
        response
            .json::<WorkerContext>()
            .await
            .context("failed to decode job context response")
    }

    /// Call `POST /v1/issues` to create a new issue.
    pub async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        let url = self.endpoint("/v1/issues")?;
        let response = self
            .http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit create issue request")?
            .error_for_status()
            .context("metis-server rejected create issue request")?;

        response
            .json::<UpsertIssueResponse>()
            .await
            .context("failed to decode create issue response")
    }

    /// Call `PUT /v1/issues/:issue_id` to update an existing issue.
    pub async fn update_issue(
        &self,
        issue_id: &MetisId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        let issue_id = issue_id.trim();
        if issue_id.is_empty() {
            return Err(anyhow!("issue_id must not be empty"));
        }

        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .put(url)
            .json(request)
            .send()
            .await
            .context("failed to submit update issue request")?
            .error_for_status()
            .context("metis-server returned an error while updating issue")?;

        response
            .json::<UpsertIssueResponse>()
            .await
            .context("failed to decode update issue response")
    }

    /// Call `GET /v1/issues/:issue_id` to fetch an issue.
    pub async fn get_issue(&self, issue_id: &MetisId) -> Result<IssueRecord> {
        let issue_id = issue_id.trim();
        if issue_id.is_empty() {
            return Err(anyhow!("issue_id must not be empty"));
        }

        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch issue")?
            .error_for_status()
            .context("metis-server returned an error while fetching issue")?;

        response
            .json::<IssueRecord>()
            .await
            .context("failed to decode get issue response")
    }

    /// Call `GET /v1/issues` to list issues with optional filters.
    pub async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        let url = self.endpoint("/v1/issues")?;
        let response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .context("failed to fetch issues list")?
            .error_for_status()
            .context("metis-server returned an error while listing issues")?;

        response
            .json::<ListIssuesResponse>()
            .await
            .context("failed to decode list issues response")
    }

    /// Call `POST /v1/patches` to create a new patch.
    pub async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        let url = self.endpoint("/v1/patches")?;
        let response = self
            .http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit create patch request")?
            .error_for_status()
            .context("metis-server rejected create patch request")?;

        response
            .json::<UpsertPatchResponse>()
            .await
            .context("failed to decode create patch response")
    }

    /// Call `PUT /v1/patches/:patch_id` to update an existing patch.
    pub async fn update_patch(
        &self,
        patch_id: &MetisId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        let patch_id = patch_id.trim();
        if patch_id.is_empty() {
            return Err(anyhow!("patch_id must not be empty"));
        }

        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .put(url)
            .json(request)
            .send()
            .await
            .context("failed to submit update patch request")?
            .error_for_status()
            .context("metis-server returned an error while updating patch")?;

        response
            .json::<UpsertPatchResponse>()
            .await
            .context("failed to decode update patch response")
    }

    /// Call `GET /v1/patches/:patch_id` to fetch a patch.
    pub async fn get_patch(&self, patch_id: &MetisId) -> Result<PatchRecord> {
        let patch_id = patch_id.trim();
        if patch_id.is_empty() {
            return Err(anyhow!("patch_id must not be empty"));
        }

        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch patch")?
            .error_for_status()
            .context("metis-server returned an error while fetching patch")?;

        response
            .json::<PatchRecord>()
            .await
            .context("failed to decode get patch response")
    }

    /// Call `GET /v1/patches` to list patches with optional filters.
    pub async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        let url = self.endpoint("/v1/patches")?;
        let response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .context("failed to fetch patches list")?
            .error_for_status()
            .context("metis-server returned an error while listing patches")?;

        response
            .json::<ListPatchesResponse>()
            .await
            .context("failed to decode list patches response")
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
    async fn create_job(&self, request: &CreateJobRequest) -> Result<CreateJobResponse> {
        MetisClient::create_job(self, request).await
    }

    async fn list_jobs(&self) -> Result<ListJobsResponse> {
        MetisClient::list_jobs(self).await
    }

    async fn get_job(&self, job_id: &MetisId) -> Result<JobSummary> {
        MetisClient::get_job(self, job_id).await
    }

    async fn kill_job(&self, job_id: &MetisId) -> Result<KillJobResponse> {
        MetisClient::kill_job(self, job_id).await
    }

    async fn get_job_logs(&self, job_id: &MetisId, query: &LogsQuery) -> Result<LogStream> {
        MetisClient::get_job_logs(self, job_id, query).await
    }

    async fn set_job_status(
        &self,
        job_id: &MetisId,
        status: &JobStatusUpdate,
    ) -> Result<SetJobStatusResponse> {
        MetisClient::set_job_status(self, job_id, status).await
    }

    async fn get_job_status(&self, job_id: &MetisId) -> Result<GetJobStatusResponse> {
        MetisClient::get_job_status(self, job_id).await
    }

    async fn get_job_context(&self, job_id: &MetisId) -> Result<WorkerContext> {
        MetisClient::get_job_context(self, job_id).await
    }

    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        MetisClient::create_issue(self, request).await
    }

    async fn update_issue(
        &self,
        issue_id: &MetisId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        MetisClient::update_issue(self, issue_id, request).await
    }

    async fn get_issue(&self, issue_id: &MetisId) -> Result<IssueRecord> {
        MetisClient::get_issue(self, issue_id).await
    }

    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        MetisClient::list_issues(self, query).await
    }

    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        MetisClient::create_patch(self, request).await
    }

    async fn update_patch(
        &self,
        patch_id: &MetisId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        MetisClient::update_patch(self, patch_id, request).await
    }

    async fn get_patch(&self, patch_id: &MetisId) -> Result<PatchRecord> {
        MetisClient::get_patch(self, patch_id).await
    }

    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        MetisClient::list_patches(self, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[cfg(test)]
mod mock;

#[cfg(test)]
pub use mock::MockMetisClient;
