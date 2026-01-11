use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures::{stream, Stream, StreamExt};
use metis_common::{
    artifact_status::{ArtifactStatusUpdate, GetArtifactStatusResponse, SetArtifactStatusResponse},
    artifacts::{
        Artifact, ArtifactKind, ArtifactRecord, ListArtifactsResponse, SearchArtifactsQuery,
        UpsertArtifactRequest, UpsertArtifactResponse,
    },
    logs::LogsQuery,
    sessions::{
        CreateSessionRequest, CreateSessionResponse, KillSessionResponse, ListSessionsResponse,
        SessionSummary,
    },
    task_status::{TaskError, TaskStatusLog},
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
    async fn create_session(&self, request: &CreateSessionRequest)
        -> Result<CreateSessionResponse>;
    async fn list_sessions(&self) -> Result<ListSessionsResponse>;
    async fn kill_session(&self, session_id: &MetisId) -> Result<KillSessionResponse>;
    async fn get_session_logs(&self, session_id: &MetisId, query: &LogsQuery) -> Result<LogStream>;
    async fn set_artifact_status(
        &self,
        artifact_id: &MetisId,
        status: &ArtifactStatusUpdate,
    ) -> Result<SetArtifactStatusResponse>;
    #[allow(dead_code)]
    async fn get_artifact_status(&self, artifact_id: &MetisId)
        -> Result<GetArtifactStatusResponse>;
    async fn create_artifact(
        &self,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse>;
    #[allow(dead_code)]
    async fn update_artifact(
        &self,
        artifact_id: &MetisId,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse>;
    async fn get_artifact(&self, artifact_id: &MetisId) -> Result<ArtifactRecord>;
    async fn list_artifacts(&self, query: &SearchArtifactsQuery) -> Result<ListArtifactsResponse>;
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

    /// Call `POST /v1/sessions` to create a new session.
    pub async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        let url = self.endpoint("/v1/sessions")?;
        let response = self
            .http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit create session request")?
            .error_for_status()
            .context("metis-server rejected create session request")?;

        response
            .json::<CreateSessionResponse>()
            .await
            .context("failed to decode create session response")
    }

    /// Build session summaries from session artifacts and their status logs.
    pub async fn list_sessions(&self) -> Result<ListSessionsResponse> {
        let artifacts = self
            .list_artifacts(&SearchArtifactsQuery {
                artifact_type: Some(ArtifactKind::Session),
                issue_type: None,
                status: None,
                q: None,
            })
            .await
            .context("failed to list session artifacts")?;

        let mut summaries_with_times = Vec::new();

        for record in artifacts.artifacts {
            match self.session_summary_from_artifact(record).await {
                Ok(Some(summary_with_time)) => summaries_with_times.push(summary_with_time),
                Ok(None) => {}
                Err(_) => {}
            }
        }

        summaries_with_times.sort_by(|a, b| b.1.cmp(&a.1));
        let sessions = summaries_with_times
            .into_iter()
            .map(|(summary, _)| summary)
            .collect();

        Ok(ListSessionsResponse { sessions })
    }

    async fn session_summary_from_artifact(
        &self,
        record: ArtifactRecord,
    ) -> Result<Option<(SessionSummary, Option<DateTime<Utc>>)>> {
        let ArtifactRecord { id, artifact } = record;
        let Artifact::Session {
            program, params, ..
        } = artifact
        else {
            return Ok(None);
        };

        let status_log = match self.get_artifact_status(&id).await {
            Ok(response) => response.status_log,
            Err(_) => TaskStatusLog::default(),
        };

        let notes = Self::derive_session_notes(&status_log);
        let reference_time = status_log.start_time().or(status_log.creation_time());

        Ok(Some((
            SessionSummary {
                id,
                notes,
                program,
                params,
                status_log,
            },
            reference_time,
        )))
    }

    fn derive_session_notes(status_log: &TaskStatusLog) -> Option<String> {
        if let Some(Err(error)) = status_log.result() {
            return format_error_note(&error);
        }

        None
    }

    /// Call `DELETE /v1/sessions/:session_id` to terminate a running session.
    pub async fn kill_session(&self, session_id: &MetisId) -> Result<KillSessionResponse> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(anyhow!("session_id must not be empty"));
        }

        let path = format!("/v1/sessions/{session_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .delete(url)
            .send()
            .await
            .context("failed to submit kill session request")?
            .error_for_status()
            .context("metis-server returned an error while killing session")?;

        response
            .json::<KillSessionResponse>()
            .await
            .context("failed to decode kill session response")
    }

    /// Call `GET /v1/sessions/:session_id/logs` to fetch or stream session logs.
    ///
    /// When `query.watch` is `Some(true)` the returned stream yields log lines
    /// as new SSE events arrive.
    pub async fn get_session_logs(
        &self,
        session_id: &MetisId,
        query: &LogsQuery,
    ) -> Result<LogStream> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(anyhow!("session_id must not be empty"));
        }

        let path = format!("/v1/sessions/{session_id}/logs");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .context("failed to request session logs")?
            .error_for_status()
            .context("metis-server returned an error while fetching session logs")?;

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

    /// Call `POST /v1/artifacts/:artifact_id/status` to update the recorded agent status.
    pub async fn set_artifact_status(
        &self,
        artifact_id: &MetisId,
        status: &ArtifactStatusUpdate,
    ) -> Result<SetArtifactStatusResponse> {
        let artifact_id = artifact_id.trim();
        if artifact_id.is_empty() {
            return Err(anyhow!("artifact_id must not be empty"));
        }

        let path = format!("/v1/artifacts/{artifact_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .post(url)
            .json(status)
            .send()
            .await
            .context("failed to submit set artifact status request")?
            .error_for_status()
            .context("metis-server returned an error while setting artifact status")?;

        response
            .json::<SetArtifactStatusResponse>()
            .await
            .context("failed to decode set artifact status response")
    }

    /// Call `GET /v1/artifacts/:artifact_id/status` to retrieve the status log for an artifact.
    pub async fn get_artifact_status(
        &self,
        artifact_id: &MetisId,
    ) -> Result<GetArtifactStatusResponse> {
        let artifact_id = artifact_id.trim();
        if artifact_id.is_empty() {
            return Err(anyhow!("artifact_id must not be empty"));
        }

        let path = format!("/v1/artifacts/{artifact_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to request artifact status")?
            .error_for_status()
            .context("metis-server returned an error while fetching artifact status")?;

        response
            .json::<GetArtifactStatusResponse>()
            .await
            .context("failed to decode artifact status response")
    }

    /// Call `POST /v1/artifacts` to create a new artifact.
    pub async fn create_artifact(
        &self,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        let url = self.endpoint("/v1/artifacts")?;
        let response = self
            .http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit create artifact request")?
            .error_for_status()
            .context("metis-server rejected create artifact request")?;

        response
            .json::<UpsertArtifactResponse>()
            .await
            .context("failed to decode create artifact response")
    }

    /// Call `PUT /v1/artifacts/:artifact_id` to update an existing artifact.
    pub async fn update_artifact(
        &self,
        artifact_id: &MetisId,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        let artifact_id = artifact_id.trim();
        if artifact_id.is_empty() {
            return Err(anyhow!("artifact_id must not be empty"));
        }

        let path = format!("/v1/artifacts/{artifact_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .put(url)
            .json(request)
            .send()
            .await
            .context("failed to submit update artifact request")?
            .error_for_status()
            .context("metis-server returned an error while updating artifact")?;

        response
            .json::<UpsertArtifactResponse>()
            .await
            .context("failed to decode update artifact response")
    }

    /// Call `GET /v1/artifacts/:artifact_id` to fetch an artifact.
    pub async fn get_artifact(&self, artifact_id: &MetisId) -> Result<ArtifactRecord> {
        let artifact_id = artifact_id.trim();
        if artifact_id.is_empty() {
            return Err(anyhow!("artifact_id must not be empty"));
        }

        let path = format!("/v1/artifacts/{artifact_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("failed to fetch artifact")?
            .error_for_status()
            .context("metis-server returned an error while fetching artifact")?;

        response
            .json::<ArtifactRecord>()
            .await
            .context("failed to decode get artifact response")
    }

    /// Call `GET /v1/artifacts` to list artifacts with optional filters.
    pub async fn list_artifacts(
        &self,
        query: &SearchArtifactsQuery,
    ) -> Result<ListArtifactsResponse> {
        let url = self.endpoint("/v1/artifacts")?;
        let response = self
            .http
            .get(url)
            .query(query)
            .send()
            .await
            .context("failed to fetch artifacts list")?
            .error_for_status()
            .context("metis-server returned an error while listing artifacts")?;

        response
            .json::<ListArtifactsResponse>()
            .await
            .context("failed to decode list artifacts response")
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
        let byte_stream: BytesStream = Box::pin(response.bytes_stream());
        Box::pin(stream::unfold(
            (byte_stream, String::new(), false),
            |(mut byte_stream, mut buffer, finished)| async move {
                if finished {
                    return None;
                }

                loop {
                    if let Some(idx) = buffer.find("\n\n") {
                        let event_block = buffer[..idx].to_string();
                        buffer.drain(..idx + 2);
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
                            let normalized = String::from_utf8_lossy(&chunk).replace('\r', "");
                            buffer.push_str(&normalized);
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

fn sanitize_note(note: &str) -> Option<String> {
    let collapsed = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

fn format_error_note(error: &TaskError) -> Option<String> {
    match error {
        TaskError::JobEngineError { reason } => {
            sanitize_note(reason).map(|msg| format!("error: {msg}"))
        }
    }
}

fn parse_sse_event(block: &str) -> Option<(Option<String>, String)> {
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
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
    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        MetisClient::create_session(self, request).await
    }

    async fn list_sessions(&self) -> Result<ListSessionsResponse> {
        MetisClient::list_sessions(self).await
    }

    async fn kill_session(&self, session_id: &MetisId) -> Result<KillSessionResponse> {
        MetisClient::kill_session(self, session_id).await
    }

    async fn get_session_logs(&self, session_id: &MetisId, query: &LogsQuery) -> Result<LogStream> {
        MetisClient::get_session_logs(self, session_id, query).await
    }

    async fn set_artifact_status(
        &self,
        artifact_id: &MetisId,
        status: &ArtifactStatusUpdate,
    ) -> Result<SetArtifactStatusResponse> {
        MetisClient::set_artifact_status(self, artifact_id, status).await
    }

    async fn get_artifact_status(
        &self,
        artifact_id: &MetisId,
    ) -> Result<GetArtifactStatusResponse> {
        MetisClient::get_artifact_status(self, artifact_id).await
    }

    async fn create_artifact(
        &self,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        MetisClient::create_artifact(self, request).await
    }

    async fn update_artifact(
        &self,
        artifact_id: &MetisId,
        request: &UpsertArtifactRequest,
    ) -> Result<UpsertArtifactResponse> {
        MetisClient::update_artifact(self, artifact_id, request).await
    }

    async fn get_artifact(&self, artifact_id: &MetisId) -> Result<ArtifactRecord> {
        MetisClient::get_artifact(self, artifact_id).await
    }

    async fn list_artifacts(&self, query: &SearchArtifactsQuery) -> Result<ListArtifactsResponse> {
        MetisClient::list_artifacts(self, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_note_collapses_whitespace_and_rejects_empty() {
        assert_eq!(
            sanitize_note("  hello   world\tfrom  metis "),
            Some("hello world from metis".to_string())
        );
        assert_eq!(sanitize_note("   "), None);
    }

    #[test]
    fn format_error_note_prefixes_message() {
        let error = TaskError::JobEngineError {
            reason: "boom happens".into(),
        };
        assert_eq!(
            format_error_note(&error),
            Some("error: boom happens".to_string())
        );
    }
}

#[cfg(test)]
mod mock;

#[cfg(test)]
pub use mock::MockMetisClient;
