use anyhow::{anyhow, Context, Result};
use metis_common::{
    jobs::{CreateJobRequest, CreateJobResponse, ListJobsResponse},
    logs::LogsQuery,
};
use reqwest::{Client as HttpClient, Response, Url};
use serde::Deserialize;

use crate::config::AppConfig;

/// HTTP client for interacting with the metis-server REST API.
#[derive(Clone)]
pub struct MetisClient {
    base_url: Url,
    http: HttpClient,
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

        Ok(Self { base_url: url, http })
    }

    /// Expose the underlying HTTP client for advanced operations.
    pub fn http_client(&self) -> &HttpClient {
        &self.http
    }

    /// Expose the resolved base URL used for requests.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Call the `/health` endpoint and return the reported status string.
    pub async fn health(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct HealthResponse {
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

    /// Call `GET /v1/jobs/:job_id/logs` to fetch or stream job logs.
    ///
    /// When `query.watch` is `Some(true)` the returned `Response` will stream
    /// SSE events that the caller must consume.
    pub async fn get_job_logs(&self, job_id: &str, query: &LogsQuery) -> Result<Response> {
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

        Ok(response)
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .with_context(|| format!("failed to construct endpoint URL for '{}'", path))
    }
}

