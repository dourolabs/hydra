pub mod sse;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use hydra_common::{
    agents::{AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest},
    api::v1::conversations::{
        Conversation as ApiConversation, ConversationSummary as ApiConversationSummary,
        CreateConversationRequest, SearchConversationsQuery, SendMessageRequest,
        UpdateConversationRequest,
    },
    api::v1::error::ApiErrorBody,
    api::v1::events::EventsQuery,
    api::v1::labels::{
        ListLabelsResponse, SearchLabelsQuery, UpsertLabelRequest, UpsertLabelResponse,
    },
    api::v1::login::{
        DevicePollRequest, DevicePollResponse, DeviceStartResponse, LoginRequest, LoginResponse,
    },
    api::v1::merge_check::{MergeBlockedError, MergeCheckOk, MergeCheckResponse},
    api::v1::projects::{
        ListProjectsResponse, ProjectRecord, ProjectRef, ProjectStatusesResponse,
        RenameStatusRequest, UpsertProjectRequest, UpsertProjectResponse,
    },
    api::v1::relations::{
        CreateRelationRequest, ListRelationsRequest, ListRelationsResponse, RemoveRelationRequest,
        RemoveRelationResponse,
    },
    api::v1::secrets::{ListSecretsResponse, SetSecretRequest},
    documents::{
        DocumentVersionRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
        SearchDocumentsQuery, UpsertDocumentRequest, UpsertDocumentResponse,
    },
    github::{GithubAppClientIdResponse, GithubTokenResponse},
    issues::{
        IssueVersionRecord, ListIssueVersionsResponse, ListIssuesResponse, SearchIssuesQuery,
        SubmitFormRequest, SubmitFormResponse, UpsertIssueRequest, UpsertIssueResponse,
    },
    logs::LogsQuery,
    merge_queues::{EnqueueMergePatchRequest, MergeQueue},
    patches::{
        CreatePatchAssetQuery, CreatePatchAssetResponse, ListPatchVersionsResponse,
        ListPatchesResponse, PatchVersionRecord, SearchPatchesQuery, UpsertPatchRequest,
        UpsertPatchResponse,
    },
    repositories::{
        CreateRepositoryRequest, DeleteRepositoryResponse, ListRepositoriesResponse,
        RepositoryRecord, SearchRepositoriesQuery, UpdateRepositoryRequest,
        UpsertRepositoryResponse,
    },
    session_status::{SessionStatusUpdate, SetSessionStatusResponse},
    sessions::{
        CreateSessionRequest, CreateSessionResponse, KillSessionResponse, ListProxyTargetsResponse,
        ListSessionVersionsResponse, ListSessionsResponse, SearchSessionsQuery,
        SessionVersionRecord, UpsertProxyTargetRequest, WorkerContext,
    },
    triggers::{
        ListTriggerVersionsResponse, ListTriggersResponse, SearchTriggersQuery,
        TriggerVersionRecord, UpsertTriggerRequest, UpsertTriggerResponse,
    },
    users::{ListUsersResponse, SearchUsersQuery, UserSummary},
    whoami::WhoAmIResponse,
    ActorId, ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, RelativeVersionNumber,
    RepoName, SessionId, TriggerId,
};
use reqwest::{header, Client as HttpClient, RequestBuilder, Response, StatusCode, Url};
use sse::SseEventStream;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;
use tokio_tungstenite::{tungstenite, MaybeTlsStream};

use crate::config::AppConfig;

/// Configurable HTTP timeouts applied to the `HydraClient`.
///
/// These guard against indefinite hangs on stalled requests. The streaming
/// endpoints (SSE log/event streams, relay WebSocket) deliberately bypass the
/// overall request timeout while still honouring the connect timeout.
#[derive(Debug, Clone, Copy)]
pub struct HydraClientTimeouts {
    /// Maximum duration for a single non-streaming request (header + body).
    pub request_timeout: Duration,
    /// Maximum duration to establish a TCP connection.
    pub connect_timeout: Duration,
    /// How long an idle connection in the pool is kept alive.
    pub pool_idle_timeout: Duration,
}

impl HydraClientTimeouts {
    /// Default request timeout (60s).
    pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
    /// Default connect timeout (10s).
    pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    /// Default pool idle timeout (60s).
    pub const DEFAULT_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
}

impl Default for HydraClientTimeouts {
    fn default() -> Self {
        Self {
            request_timeout: Self::DEFAULT_REQUEST_TIMEOUT,
            connect_timeout: Self::DEFAULT_CONNECT_TIMEOUT,
            pool_idle_timeout: Self::DEFAULT_POOL_IDLE_TIMEOUT,
        }
    }
}

/// Build the standard `reqwest::Client` used by `HydraClient` for one-shot
/// HTTP requests. All endpoints inherit the overall request timeout.
fn build_http_client(timeouts: &HydraClientTimeouts) -> Result<HttpClient> {
    HttpClient::builder()
        .timeout(timeouts.request_timeout)
        .connect_timeout(timeouts.connect_timeout)
        .pool_idle_timeout(timeouts.pool_idle_timeout)
        .build()
        .context("failed to build HydraClient HTTP client")
}

/// Build the `reqwest::Client` used for streaming/long-lived endpoints (SSE
/// log + event streams). The request timeout is intentionally omitted so a
/// long-running stream is not torn down mid-flight; the connect timeout still
/// applies so we fail fast on an unreachable server.
fn build_streaming_http_client(timeouts: &HydraClientTimeouts) -> Result<HttpClient> {
    HttpClient::builder()
        .connect_timeout(timeouts.connect_timeout)
        .pool_idle_timeout(timeouts.pool_idle_timeout)
        .build()
        .context("failed to build HydraClient streaming HTTP client")
}

/// Type alias for a connected WebSocket stream to the relay endpoint.
pub type RelayWebSocket = tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// HTTP client for interacting with the hydra-server REST API.
#[derive(Clone)]
pub struct HydraClient {
    base_url: Url,
    http: HttpClient,
    streaming_http: HttpClient,
    auth_token: String,
}

/// HTTP client for interacting with unauthenticated hydra-server endpoints.
#[derive(Clone)]
pub struct HydraClientUnauthenticated {
    base_url: Url,
    http: HttpClient,
}

pub type LogStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;
type BytesStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Error returned when the server responds with 409 Conflict.
#[derive(Debug, thiserror::Error)]
#[error("conflict: {message}")]
pub struct ConflictError {
    pub message: String,
}

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
pub trait HydraClientInterface: Send + Sync {
    fn base_url(&self) -> &Url;

    /// Connect to the relay WebSocket for an interactive session.
    async fn connect_relay_websocket(&self, session_id: &SessionId) -> Result<RelayWebSocket>;

    async fn create_session(&self, request: &CreateSessionRequest)
        -> Result<CreateSessionResponse>;
    async fn list_sessions(&self, query: &SearchSessionsQuery) -> Result<ListSessionsResponse>;
    async fn get_session(&self, job_id: &SessionId) -> Result<SessionVersionRecord>;
    async fn get_session_version(
        &self,
        job_id: &SessionId,
        version: RelativeVersionNumber,
    ) -> Result<SessionVersionRecord>;
    async fn kill_session(&self, job_id: &SessionId) -> Result<KillSessionResponse>;
    async fn get_session_logs(&self, job_id: &SessionId, query: &LogsQuery) -> Result<LogStream>;
    async fn set_session_status(
        &self,
        job_id: &SessionId,
        status: &SessionStatusUpdate,
    ) -> Result<SetSessionStatusResponse>;

    async fn get_session_context(&self, job_id: &SessionId) -> Result<WorkerContext>;
    async fn list_proxy_targets(&self, session_id: &SessionId) -> Result<ListProxyTargetsResponse>;
    async fn upsert_proxy_target(
        &self,
        session_id: &SessionId,
        request: &UpsertProxyTargetRequest,
    ) -> Result<()>;
    async fn delete_proxy_target(&self, session_id: &SessionId, port: u16) -> Result<()>;
    async fn list_session_versions(
        &self,
        job_id: &SessionId,
    ) -> Result<ListSessionVersionsResponse>;
    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse>;
    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse>;
    async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<IssueVersionRecord>;
    async fn get_issue_version(
        &self,
        issue_id: &IssueId,
        version: RelativeVersionNumber,
    ) -> Result<IssueVersionRecord>;
    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse>;
    async fn list_issue_versions(&self, issue_id: &IssueId) -> Result<ListIssueVersionsResponse>;
    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse>;
    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse>;
    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord>;
    async fn get_patch_version(
        &self,
        patch_id: &PatchId,
        version: RelativeVersionNumber,
    ) -> Result<PatchVersionRecord>;
    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse>;
    async fn list_patch_versions(&self, patch_id: &PatchId) -> Result<ListPatchVersionsResponse>;
    async fn merge_check(&self, patch_id: &PatchId) -> Result<MergeCheckResponse>;
    async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse>;
    async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse>;
    async fn get_document(
        &self,
        document_id: &DocumentId,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord>;
    async fn get_document_by_path(
        &self,
        path: &str,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord>;
    async fn list_documents(&self, query: &SearchDocumentsQuery) -> Result<ListDocumentsResponse>;
    async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse>;
    async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: RelativeVersionNumber,
    ) -> Result<DocumentVersionRecord>;
    async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String>;
    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<ListRepositoriesResponse>;
    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse>;
    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse>;
    async fn delete_repository(&self, repo_name: &RepoName) -> Result<RepositoryRecord>;
    async fn list_projects(&self) -> Result<ListProjectsResponse>;
    async fn create_project(&self, request: &UpsertProjectRequest)
        -> Result<UpsertProjectResponse>;
    async fn get_project(&self, project_ref: &ProjectRef) -> Result<ProjectRecord>;
    async fn update_project(
        &self,
        project_ref: &ProjectRef,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse>;
    async fn delete_project(&self, project_ref: &ProjectRef) -> Result<UpsertProjectResponse>;
    async fn rename_project_status(
        &self,
        project_ref: &ProjectRef,
        request: &RenameStatusRequest,
    ) -> Result<UpsertProjectResponse>;
    async fn get_project_statuses(
        &self,
        project_ref: &ProjectRef,
    ) -> Result<ProjectStatusesResponse>;
    async fn whoami(&self) -> Result<WhoAmIResponse>;
    async fn list_users(&self, query: &SearchUsersQuery) -> Result<ListUsersResponse>;
    async fn get_user(&self, username: &str) -> Result<UserSummary>;
    async fn list_user_secrets(&self, username: &str) -> Result<ListSecretsResponse>;
    async fn set_user_secret(&self, username: &str, name: &str, value: &str) -> Result<()>;
    async fn delete_user_secret(&self, username: &str, name: &str) -> Result<()>;
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
    async fn delete_issue(&self, issue_id: &IssueId) -> Result<IssueVersionRecord>;
    async fn submit_form(
        &self,
        issue_id: &IssueId,
        request: &SubmitFormRequest,
    ) -> Result<SubmitFormResponse>;
    async fn delete_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord>;
    async fn delete_document(&self, document_id: &DocumentId) -> Result<DocumentVersionRecord>;

    async fn create_trigger(&self, request: &UpsertTriggerRequest)
        -> Result<UpsertTriggerResponse>;
    async fn update_trigger(
        &self,
        trigger_id: &TriggerId,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse>;
    async fn get_trigger(
        &self,
        trigger_id: &TriggerId,
        include_deleted: bool,
    ) -> Result<TriggerVersionRecord>;
    async fn get_trigger_version(
        &self,
        trigger_id: &TriggerId,
        version: RelativeVersionNumber,
    ) -> Result<TriggerVersionRecord>;
    async fn list_triggers(&self, query: &SearchTriggersQuery) -> Result<ListTriggersResponse>;
    async fn list_trigger_versions(
        &self,
        trigger_id: &TriggerId,
    ) -> Result<ListTriggerVersionsResponse>;
    async fn delete_trigger(&self, trigger_id: &TriggerId) -> Result<TriggerVersionRecord>;

    /// Open an SSE connection to GET /v1/events and return a stream of parsed events.
    async fn subscribe_events(
        &self,
        query: &EventsQuery,
        last_event_id: Option<u64>,
    ) -> Result<SseEventStream>;

    async fn list_relations(&self, query: &ListRelationsRequest) -> Result<ListRelationsResponse>;

    async fn list_labels(&self, query: &SearchLabelsQuery) -> Result<ListLabelsResponse>;
    async fn create_label(&self, request: &UpsertLabelRequest) -> Result<UpsertLabelResponse>;
    async fn add_label_association(&self, label_id: &LabelId, object_id: &HydraId) -> Result<()>;
    async fn remove_label_association(&self, label_id: &LabelId, object_id: &HydraId)
        -> Result<()>;

    async fn create_relation(&self, request: &CreateRelationRequest) -> Result<bool>;

    async fn remove_relation(&self, request: &RemoveRelationRequest) -> Result<bool>;

    async fn create_conversation(
        &self,
        request: &CreateConversationRequest,
    ) -> Result<ApiConversation>;
    async fn send_message(
        &self,
        conversation_id: &ConversationId,
        request: &SendMessageRequest,
    ) -> Result<hydra_common::api::v1::sessions::SessionEvent>;
    async fn get_conversation_versions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<hydra_common::Versioned<ApiConversation>>>;
    async fn get_conversation_version(
        &self,
        conversation_id: &ConversationId,
        version: hydra_common::RelativeVersionNumber,
    ) -> Result<hydra_common::Versioned<ApiConversation>>;
    async fn close_conversation(&self, conversation_id: &ConversationId)
        -> Result<ApiConversation>;
    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<ApiConversationSummary>>;
    async fn get_conversation(&self, conversation_id: &ConversationId) -> Result<ApiConversation>;
    async fn update_conversation(
        &self,
        conversation_id: &ConversationId,
        request: &UpdateConversationRequest,
    ) -> Result<ApiConversation>;
    async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation>;

    /// Resolve the current actor's ID from the auth context.
    async fn current_actor_id(&self) -> Result<ActorId> {
        let whoami = self
            .whoami()
            .await
            .context("failed to fetch current actor")?;
        ActorId::try_from(whoami.actor).map_err(|e| anyhow!(e))
    }
}

impl HydraClientUnauthenticated {
    /// Construct a new client using the server URL from the CLI configuration.
    pub fn from_config(config: &AppConfig, timeouts: &HydraClientTimeouts) -> Result<Self> {
        let server = config.default_server()?;
        Self::new(&server.url, timeouts)
    }

    /// Construct a new client with the supplied request timeouts.
    pub fn new(base_url: impl AsRef<str>, timeouts: &HydraClientTimeouts) -> Result<Self> {
        Self::with_http_client(base_url, build_http_client(timeouts)?)
    }

    /// Construct a new client with a custom `reqwest::Client`. The caller is
    /// responsible for configuring any desired timeouts on the client.
    pub fn with_http_client(base_url: impl AsRef<str>, http: HttpClient) -> Result<Self> {
        let url = Url::parse(base_url.as_ref())
            .with_context(|| format!("invalid Hydra server URL '{}'", base_url.as_ref()))?;

        Ok(Self {
            base_url: url,
            http,
        })
    }

    /// Expose the resolved base URL used for requests.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Call `POST /v1/login` to exchange a GitHub token for a Hydra login token.
    pub async fn login(&self, request: &LoginRequest) -> Result<(String, HydraClient)> {
        self.login_with_http_client(self.http.clone(), request)
            .await
    }

    /// Call `POST /v1/login` using a custom `reqwest::Client`.
    pub async fn login_with_http_client(
        &self,
        http: HttpClient,
        request: &LoginRequest,
    ) -> Result<(String, HydraClient)> {
        let url = self
            .endpoint("/v1/login")
            .with_context(|| "failed to construct login endpoint URL")?;
        let response = http
            .post(url)
            .json(request)
            .send()
            .await
            .context("failed to submit login request")?
            .error_for_status_with_body("hydra-server rejected login request")
            .await?;

        let login_response = response
            .json::<LoginResponse>()
            .await
            .context("failed to decode login response")?;
        let auth_token = login_response.login_token.clone();
        let client =
            HydraClient::with_http_client(self.base_url.as_str(), auth_token.clone(), http)?;

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
                "hydra-server returned an error while fetching GitHub app client id",
            )
            .await?;

        response
            .json::<GithubAppClientIdResponse>()
            .await
            .context("failed to decode GitHub app client id response")
    }

    /// Call `POST /v1/login/device/start` to initiate a server-side GitHub device flow.
    pub async fn device_start(&self) -> Result<DeviceStartResponse> {
        let url = self.endpoint("/v1/login/device/start")?;
        let response = self
            .http
            .post(url)
            .send()
            .await
            .context("failed to start device flow")?
            .error_for_status_with_body("hydra-server rejected device start request")
            .await?;

        response
            .json::<DeviceStartResponse>()
            .await
            .context("failed to decode device start response")
    }

    /// Call `POST /v1/login/device/poll` to check device flow authorization status.
    pub async fn device_poll(&self, device_session_id: &str) -> Result<DevicePollResponse> {
        let url = self.endpoint("/v1/login/device/poll")?;
        let request = DevicePollRequest::new(device_session_id.to_string());
        let response = self
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .context("failed to poll device flow")?
            .error_for_status_with_body("hydra-server rejected device poll request")
            .await?;

        response
            .json::<DevicePollResponse>()
            .await
            .context("failed to decode device poll response")
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .with_context(|| format!("failed to construct endpoint URL for '{path}'"))
    }
}

impl HydraClient {
    /// Construct a new client using the server URL from the CLI configuration.
    pub fn from_config(
        config: &AppConfig,
        auth_token: impl Into<String>,
        timeouts: &HydraClientTimeouts,
    ) -> Result<Self> {
        let server = config.default_server()?;
        Self::new(&server.url, auth_token, timeouts)
    }

    /// Construct a new client with the supplied request timeouts. Builds
    /// separate request and streaming HTTP clients so SSE endpoints are not
    /// subject to the per-request timeout.
    pub fn new(
        base_url: impl AsRef<str>,
        auth_token: impl Into<String>,
        timeouts: &HydraClientTimeouts,
    ) -> Result<Self> {
        let http = build_http_client(timeouts)?;
        let streaming_http = build_streaming_http_client(timeouts)?;
        let url = Url::parse(base_url.as_ref())
            .with_context(|| format!("invalid Hydra server URL '{}'", base_url.as_ref()))?;

        Ok(Self {
            base_url: url,
            http,
            streaming_http,
            auth_token: auth_token.into(),
        })
    }

    /// Construct a new client with a custom `reqwest::Client`. The same client
    /// is used for both regular requests and SSE/streaming endpoints; callers
    /// are responsible for any timeout configuration.
    pub fn with_http_client(
        base_url: impl AsRef<str>,
        auth_token: impl Into<String>,
        http: HttpClient,
    ) -> Result<Self> {
        let streaming_http = http.clone();
        let url = Url::parse(base_url.as_ref())
            .with_context(|| format!("invalid Hydra server URL '{}'", base_url.as_ref()))?;

        Ok(Self {
            base_url: url,
            http,
            streaming_http,
            auth_token: auth_token.into(),
        })
    }

    /// Expose the resolved base URL used for requests.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    fn authed(&self, builder: RequestBuilder) -> RequestBuilder {
        builder.bearer_auth(&self.auth_token)
    }

    /// Connect to the relay WebSocket for a session.
    pub async fn connect_relay_websocket(&self, session_id: &SessionId) -> Result<RelayWebSocket> {
        let mut ws_url = self.base_url.clone();
        match ws_url.scheme() {
            "https" => ws_url
                .set_scheme("wss")
                .map_err(|()| anyhow!("failed to set wss scheme"))?,
            "http" => ws_url
                .set_scheme("ws")
                .map_err(|()| anyhow!("failed to set ws scheme"))?,
            scheme => return Err(anyhow!("unsupported server URL scheme: {scheme}")),
        }
        ws_url.set_path(&format!("/v1/sessions/{session_id}/events"));

        let host_header = match ws_url.port() {
            Some(port) => format!("{}:{}", ws_url.host_str().unwrap_or_default(), port),
            None => ws_url.host_str().unwrap_or_default().to_string(),
        };
        let auth_value = format!("Bearer {}", self.auth_token);
        let request = tungstenite::http::Request::builder()
            .uri(ws_url.as_str())
            .header("Host", &host_header)
            .header("Authorization", &auth_value)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .context("failed to build WebSocket request")?;

        let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .context("failed to connect to relay WebSocket")?;

        Ok(ws_stream)
    }

    /// Call `POST /v1/sessions` to create a new session.
    pub async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        let url = self.endpoint("/v1/sessions")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create session request")?
            .error_for_status_with_body("hydra-server rejected create session request")
            .await?;

        response
            .json::<CreateSessionResponse>()
            .await
            .context("failed to decode create session response")
    }

    /// Call `GET /v1/sessions` to list existing sessions.
    pub async fn list_sessions(&self, query: &SearchSessionsQuery) -> Result<ListSessionsResponse> {
        let url = self.endpoint("/v1/sessions")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch sessions list")?
            .error_for_status_with_body("hydra-server returned an error while listing sessions")
            .await?;

        response
            .json::<ListSessionsResponse>()
            .await
            .context("failed to decode list sessions response")
    }

    /// Call `GET /v1/sessions/:session_id/versions` to list session history.
    pub async fn list_session_versions(
        &self,
        job_id: &SessionId,
    ) -> Result<ListSessionVersionsResponse> {
        let path = format!("/v1/sessions/{job_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch session versions")?
            .error_for_status_with_body(
                "hydra-server returned an error while listing session versions",
            )
            .await?;

        response
            .json::<ListSessionVersionsResponse>()
            .await
            .context("failed to decode list session versions response")
    }

    /// Call `GET /v1/sessions/:session_id` to fetch an individual session summary.
    pub async fn get_session(&self, job_id: &SessionId) -> Result<SessionVersionRecord> {
        let path = format!("/v1/sessions/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch session")?
            .error_for_status_with_body("hydra-server returned an error while fetching session")
            .await?;

        response
            .json::<SessionVersionRecord>()
            .await
            .context("failed to decode session response")
    }

    /// Call `GET /v1/sessions/:session_id/versions/:version` to fetch a specific session version.
    pub async fn get_session_version(
        &self,
        job_id: &SessionId,
        version: RelativeVersionNumber,
    ) -> Result<SessionVersionRecord> {
        let path = format!("/v1/sessions/{job_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch session version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching session version",
            )
            .await?;

        response
            .json::<SessionVersionRecord>()
            .await
            .context("failed to decode session version response")
    }

    /// Call `DELETE /v1/sessions/:session_id` to terminate a running session.
    pub async fn kill_session(&self, job_id: &SessionId) -> Result<KillSessionResponse> {
        let path = format!("/v1/sessions/{job_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit kill session request")?
            .error_for_status_with_body("hydra-server returned an error while killing session")
            .await?;

        response
            .json::<KillSessionResponse>()
            .await
            .context("failed to decode kill session response")
    }

    /// Call `GET /v1/sessions/:session_id/logs` to fetch or stream session logs.
    ///
    /// When `query.watch` is `Some(true)` the returned stream yields log lines
    /// as new SSE events arrive. Uses the streaming HTTP client so the
    /// per-request timeout does not terminate long-lived watches.
    pub async fn get_session_logs(
        &self,
        job_id: &SessionId,
        query: &LogsQuery,
    ) -> Result<LogStream> {
        let path = format!("/v1/sessions/{job_id}/logs");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.streaming_http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to request session logs")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching session logs",
            )
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

    /// Call `POST /v1/sessions/:session_id/status` to update the recorded agent status.
    ///
    /// Returns [`ConflictError`] when the server responds with 409 Conflict
    /// (i.e., the session status was already submitted by a prior worker invocation).
    pub async fn set_session_status(
        &self,
        job_id: &SessionId,
        status: &SessionStatusUpdate,
    ) -> Result<SetSessionStatusResponse> {
        let path = format!("/v1/sessions/{job_id}/status");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(status)
            .send()
            .await
            .context("failed to submit set session status request")?;

        if response.status() == StatusCode::CONFLICT {
            let body_text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiErrorBody>(&body_text)
                .ok()
                .map(|body| body.error)
                .unwrap_or(body_text);
            return Err(ConflictError { message }.into());
        }

        let response = response
            .error_for_status_with_body(
                "hydra-server returned an error while setting session status",
            )
            .await?;

        response
            .json::<SetSessionStatusResponse>()
            .await
            .context("failed to decode set session status response")
    }

    /// Call `GET /v1/sessions/:session_id/context` to retrieve the stored session context.
    pub async fn get_session_context(&self, job_id: &SessionId) -> Result<WorkerContext> {
        let path = format!("/v1/sessions/{job_id}/context");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to request session context")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching session context",
            )
            .await?;
        response
            .json::<WorkerContext>()
            .await
            .context("failed to decode session context response")
    }

    /// Call `GET /v1/sessions/:session_id/proxy-targets` to list the proxy
    /// targets the worker has advertised on a session.
    pub async fn list_proxy_targets(
        &self,
        session_id: &SessionId,
    ) -> Result<ListProxyTargetsResponse> {
        let path = format!("/v1/sessions/{session_id}/proxy-targets");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to request proxy targets")?
            .error_for_status_with_body(
                "hydra-server returned an error while listing proxy targets",
            )
            .await?;
        response
            .json::<ListProxyTargetsResponse>()
            .await
            .context("failed to decode proxy targets response")
    }

    /// Call `POST /v1/sessions/:session_id/proxy-targets` to add (or replace
    /// when `port` already exists) a proxy target on the session. Idempotent.
    pub async fn upsert_proxy_target(
        &self,
        session_id: &SessionId,
        request: &UpsertProxyTargetRequest,
    ) -> Result<()> {
        let path = format!("/v1/sessions/{session_id}/proxy-targets");
        let url = self.endpoint(&path)?;
        self.authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit upsert proxy target request")?
            .error_for_status_with_body(
                "hydra-server returned an error while adding a proxy target",
            )
            .await?;
        Ok(())
    }

    /// Call `DELETE /v1/sessions/:session_id/proxy-targets/:port` to remove a
    /// proxy target from the session. Idempotent — removing an absent port
    /// is a no-op.
    pub async fn delete_proxy_target(&self, session_id: &SessionId, port: u16) -> Result<()> {
        let path = format!("/v1/sessions/{session_id}/proxy-targets/{port}");
        let url = self.endpoint(&path)?;
        self.authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete proxy target request")?
            .error_for_status_with_body(
                "hydra-server returned an error while removing a proxy target",
            )
            .await?;
        Ok(())
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
            .error_for_status_with_body("hydra-server rejected create issue request")
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
            .error_for_status_with_body("hydra-server returned an error while updating issue")
            .await?;

        response
            .json::<UpsertIssueResponse>()
            .await
            .context("failed to decode update issue response")
    }

    /// Call `GET /v1/issues/:issue_id` to fetch an issue.
    pub async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<IssueVersionRecord> {
        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let mut builder = self.authed(self.http.get(url));
        if include_deleted {
            builder = builder.query(&[("include_deleted", "true")]);
        }
        let response = builder
            .send()
            .await
            .context("failed to fetch issue")?
            .error_for_status_with_body("hydra-server returned an error while fetching issue")
            .await?;

        response
            .json::<IssueVersionRecord>()
            .await
            .context("failed to decode get issue response")
    }

    /// Call `GET /v1/issues/:issue_id/versions/:version` to fetch a specific issue version.
    pub async fn get_issue_version(
        &self,
        issue_id: &IssueId,
        version: RelativeVersionNumber,
    ) -> Result<IssueVersionRecord> {
        let path = format!("/v1/issues/{issue_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch issue version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching issue version",
            )
            .await?;

        response
            .json::<IssueVersionRecord>()
            .await
            .context("failed to decode issue version response")
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
            .error_for_status_with_body("hydra-server returned an error while listing issues")
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
                "hydra-server returned an error while listing issue versions",
            )
            .await?;

        response
            .json::<ListIssueVersionsResponse>()
            .await
            .context("failed to decode list issue versions response")
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
            .error_for_status_with_body("hydra-server rejected create patch request")
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
            .error_for_status_with_body("hydra-server returned an error while updating patch")
            .await?;

        response
            .json::<UpsertPatchResponse>()
            .await
            .context("failed to decode update patch response")
    }

    /// Call `GET /v1/patches/:patch_id` to fetch a patch.
    pub async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch patch")?
            .error_for_status_with_body("hydra-server returned an error while fetching patch")
            .await?;

        response
            .json::<PatchVersionRecord>()
            .await
            .context("failed to decode get patch response")
    }

    /// Call `GET /v1/patches/:patch_id/versions/:version` to fetch a specific patch version.
    pub async fn get_patch_version(
        &self,
        patch_id: &PatchId,
        version: RelativeVersionNumber,
    ) -> Result<PatchVersionRecord> {
        let path = format!("/v1/patches/{patch_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch patch version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching patch version",
            )
            .await?;

        response
            .json::<PatchVersionRecord>()
            .await
            .context("failed to decode patch version response")
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
            .error_for_status_with_body("hydra-server returned an error while listing patches")
            .await?;

        response
            .json::<ListPatchesResponse>()
            .await
            .context("failed to decode list patches response")
    }

    /// Call `POST /v1/patches/:patch_id/merge_check` to ask the server whether
    /// `hydra patches merge` would succeed *right now* for the calling actor.
    ///
    /// Returns `Ok(MergeCheckResponse::Ok)` on HTTP 200 and
    /// `Ok(MergeCheckResponse::Blocked(_))` on HTTP 422 — both are normal,
    /// in-band outcomes from the preflight endpoint. Other statuses
    /// (network errors, 404, 5xx) surface as `Err` and the caller MUST NOT
    /// proceed with the merge.
    pub async fn merge_check(&self, patch_id: &PatchId) -> Result<MergeCheckResponse> {
        let path = format!("/v1/patches/{patch_id}/merge_check");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .send()
            .await
            .context("failed to submit merge_check request")?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let body = response
                    .json::<MergeCheckOk>()
                    .await
                    .context("failed to decode merge_check success body")?;
                Ok(MergeCheckResponse::Ok(body))
            }
            StatusCode::UNPROCESSABLE_ENTITY => {
                let body = response
                    .json::<MergeBlockedError>()
                    .await
                    .context("failed to decode merge_check blocked body")?;
                Ok(MergeCheckResponse::Blocked(body))
            }
            _ => {
                let _ = response
                    .error_for_status_with_body(
                        "hydra-server returned an error while running merge_check",
                    )
                    .await?;
                Err(anyhow!(
                    "merge_check returned unexpected status {status} — refusing to proceed"
                ))
            }
        }
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
                "hydra-server returned an error while listing patch versions",
            )
            .await?;

        response
            .json::<ListPatchVersionsResponse>()
            .await
            .context("failed to decode list patch versions response")
    }

    /// Call `POST /v1/documents` to create a document.
    ///
    /// Returns [`ConflictError`] when the server responds with 409 Conflict
    /// (i.e., a document already exists at the requested path).
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
            .context("failed to submit create document request")?;

        if response.status() == StatusCode::CONFLICT {
            let body_text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ApiErrorBody>(&body_text)
                .ok()
                .map(|body| body.error)
                .unwrap_or(body_text);
            return Err(ConflictError { message }.into());
        }

        let response = response
            .error_for_status_with_body("hydra-server rejected create document request")
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
            .error_for_status_with_body("hydra-server returned an error while updating document")
            .await?;

        response
            .json::<UpsertDocumentResponse>()
            .await
            .context("failed to decode update document response")
    }

    /// Call `GET /v1/documents/:document_id` to fetch a document.
    pub async fn get_document(
        &self,
        document_id: &DocumentId,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        let path = format!("/v1/documents/{document_id}");
        let url = self.endpoint(&path)?;
        let mut builder = self.authed(self.http.get(url));
        if include_deleted {
            builder = builder.query(&[("include_deleted", "true")]);
        }
        let response = builder
            .send()
            .await
            .context("failed to fetch document")?
            .error_for_status_with_body("hydra-server returned an error while fetching document")
            .await?;

        response
            .json::<DocumentVersionRecord>()
            .await
            .context("failed to decode document response")
    }

    /// Fetch a document by its exact path.
    ///
    /// Uses the list documents endpoint with a path prefix filter and
    /// path_is_exact=true to find a document matching the path, then
    /// fetches the full record via the detail endpoint.
    pub async fn get_document_by_path(
        &self,
        path: &str,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        let include_deleted_opt = if include_deleted { Some(true) } else { None };
        let query = SearchDocumentsQuery::new(
            None,
            Some(path.to_string()),
            Some(true),
            include_deleted_opt,
        );
        let response = self.list_documents(&query).await?;

        let summary = response
            .documents
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("document with path '{path}' not found"))?;

        self.get_document(&summary.document_id, include_deleted)
            .await
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
            .error_for_status_with_body("hydra-server returned an error while listing documents")
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
                "hydra-server returned an error while listing document versions",
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
        version: RelativeVersionNumber,
    ) -> Result<DocumentVersionRecord> {
        let path = format!("/v1/documents/{document_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch document version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching document version",
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
                "hydra-server returned an error while uploading patch asset",
            )
            .await?;

        let response = response
            .json::<CreatePatchAssetResponse>()
            .await
            .context("failed to decode patch asset upload response")?;

        Ok(response.asset_url)
    }

    /// Call `GET /v1/repositories` to list configured repositories.
    pub async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<ListRepositoriesResponse> {
        let url = self.endpoint("/v1/repositories")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch repositories list")?
            .error_for_status_with_body("hydra-server returned an error while listing repositories")
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
            .error_for_status_with_body("hydra-server rejected create repository request")
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
            .error_for_status_with_body("hydra-server returned an error while updating repository")
            .await?;

        response
            .json::<UpsertRepositoryResponse>()
            .await
            .context("failed to decode update repository response")
    }

    /// Call `DELETE /v1/repositories/:organization/:repo` to soft-delete a repository.
    pub async fn delete_repository(&self, repo_name: &RepoName) -> Result<RepositoryRecord> {
        let path = format!(
            "/v1/repositories/{}/{}",
            repo_name.organization, repo_name.repo
        );
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete repository request")?
            .error_for_status_with_body("hydra-server returned an error while deleting repository")
            .await?;

        let delete_response = response
            .json::<DeleteRepositoryResponse>()
            .await
            .context("failed to decode delete repository response")?;

        Ok(delete_response.repository)
    }

    /// Call `GET /v1/projects` to list non-deleted projects.
    pub async fn list_projects(&self) -> Result<ListProjectsResponse> {
        let url = self.endpoint("/v1/projects")?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch projects list")?
            .error_for_status_with_body("hydra-server returned an error while listing projects")
            .await?;

        response
            .json::<ListProjectsResponse>()
            .await
            .context("failed to decode list projects response")
    }

    /// Call `POST /v1/projects` to create a new project.
    pub async fn create_project(
        &self,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        let url = self.endpoint("/v1/projects")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create project request")?
            .error_for_status_with_body("hydra-server rejected create project request")
            .await?;

        response
            .json::<UpsertProjectResponse>()
            .await
            .context("failed to decode create project response")
    }

    /// Call `GET /v1/projects/:project_ref` to fetch a single project.
    /// Accepts either a [`ProjectId`](hydra_common::ProjectId) (`j-…`) or
    /// a [`ProjectKey`](hydra_common::api::v1::projects::ProjectKey)
    /// (slug) via [`ProjectRef`].
    pub async fn get_project(&self, project_ref: &ProjectRef) -> Result<ProjectRecord> {
        let url = self.endpoint(&format!("/v1/projects/{project_ref}"))?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch project")?
            .error_for_status_with_body("hydra-server returned an error while fetching project")
            .await?;

        response
            .json::<ProjectRecord>()
            .await
            .context("failed to decode project response")
    }

    /// Call `PUT /v1/projects/:project_ref` to replace a project (full update).
    pub async fn update_project(
        &self,
        project_ref: &ProjectRef,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        let url = self.endpoint(&format!("/v1/projects/{project_ref}"))?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update project request")?
            .error_for_status_with_body("hydra-server rejected update project request")
            .await?;

        response
            .json::<UpsertProjectResponse>()
            .await
            .context("failed to decode update project response")
    }

    /// Call `DELETE /v1/projects/:project_ref` to soft-delete a project.
    pub async fn delete_project(&self, project_ref: &ProjectRef) -> Result<UpsertProjectResponse> {
        let url = self.endpoint(&format!("/v1/projects/{project_ref}"))?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete project request")?
            .error_for_status_with_body("hydra-server returned an error while deleting project")
            .await?;

        response
            .json::<UpsertProjectResponse>()
            .await
            .context("failed to decode delete project response")
    }

    /// Call `POST /v1/projects/:project_ref/statuses/rename` to rename a
    /// status key in place. Preserves the status's storage identity, so
    /// existing issues continue to resolve through the same sequence.
    pub async fn rename_project_status(
        &self,
        project_ref: &ProjectRef,
        request: &RenameStatusRequest,
    ) -> Result<UpsertProjectResponse> {
        let url = self.endpoint(&format!("/v1/projects/{project_ref}/statuses/rename"))?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit rename status request")?
            .error_for_status_with_body("hydra-server rejected rename status request")
            .await?;

        response
            .json::<UpsertProjectResponse>()
            .await
            .context("failed to decode rename status response")
    }

    /// Call `GET /v1/projects/:project_ref/statuses` to list the
    /// project's status definitions. Pass the literal `"default"` key to
    /// get the seeded default project's statuses.
    pub async fn get_project_statuses(
        &self,
        project_ref: &ProjectRef,
    ) -> Result<ProjectStatusesResponse> {
        let url = self.endpoint(&format!("/v1/projects/{project_ref}/statuses"))?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch project statuses")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching project statuses",
            )
            .await?;

        response
            .json::<ProjectStatusesResponse>()
            .await
            .context("failed to decode project statuses response")
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
                "hydra-server returned an error while fetching GitHub token",
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
            .error_for_status_with_body("hydra-server returned an error while fetching whoami")
            .await?;

        response
            .json::<WhoAmIResponse>()
            .await
            .context("failed to decode whoami response")
    }

    /// Call `GET /v1/users` to list users with optional filters.
    pub async fn list_users(&self, query: &SearchUsersQuery) -> Result<ListUsersResponse> {
        let url = self.endpoint("/v1/users")?;
        let mut builder = self.authed(self.http.get(url));
        if let Some(ref q) = query.q {
            builder = builder.query(&[("q", q.as_str())]);
        }
        if let Some(include_deleted) = query.include_deleted {
            builder = builder.query(&[("include_deleted", include_deleted.to_string())]);
        }
        let response = builder
            .send()
            .await
            .context("failed to fetch users list")?
            .error_for_status_with_body("hydra-server returned an error while listing users")
            .await?;

        response
            .json::<ListUsersResponse>()
            .await
            .context("failed to decode list users response")
    }

    /// Call `GET /v1/users/:username` to fetch public user info.
    pub async fn get_user(&self, username: &str) -> Result<UserSummary> {
        let path = format!("/v1/users/{username}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch user info")?
            .error_for_status_with_body("hydra-server returned an error while fetching user info")
            .await?;

        response
            .json::<UserSummary>()
            .await
            .context("failed to decode user info response")
    }

    pub async fn list_user_secrets(&self, username: &str) -> Result<ListSecretsResponse> {
        let path = format!("/v1/users/{username}/secrets");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to list user secrets")?
            .error_for_status_with_body("hydra-server returned an error while listing secrets")
            .await?;

        response
            .json::<ListSecretsResponse>()
            .await
            .context("failed to decode list secrets response")
    }

    pub async fn set_user_secret(&self, username: &str, name: &str, value: &str) -> Result<()> {
        let path = format!("/v1/users/{username}/secrets/{name}");
        let url = self.endpoint(&path)?;
        let body = SetSecretRequest {
            value: value.to_string(),
        };
        self.authed(self.http.put(url))
            .json(&body)
            .send()
            .await
            .context("failed to set user secret")?
            .error_for_status_with_body("hydra-server returned an error while setting secret")
            .await?;
        Ok(())
    }

    pub async fn delete_user_secret(&self, username: &str, name: &str) -> Result<()> {
        let path = format!("/v1/users/{username}/secrets/{name}");
        let url = self.endpoint(&path)?;
        self.authed(self.http.delete(url))
            .send()
            .await
            .context("failed to delete user secret")?
            .error_for_status_with_body("hydra-server returned an error while deleting secret")
            .await?;
        Ok(())
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
            .error_for_status_with_body("hydra-server returned an error while fetching merge queue")
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
                "hydra-server returned an error while enqueuing merge patch",
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
            .error_for_status_with_body("hydra-server returned an error while listing agents")
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
            .error_for_status_with_body("hydra-server returned an error while fetching agent")
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
            .error_for_status_with_body("hydra-server returned an error while creating agent")
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
            .error_for_status_with_body("hydra-server returned an error while updating agent")
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
            .error_for_status_with_body("hydra-server returned an error while deleting agent")
            .await?;

        response
            .json::<DeleteAgentResponse>()
            .await
            .context("failed to decode delete agent response")
    }

    /// Call `DELETE /v1/issues/:issue_id` to soft-delete an issue.
    pub async fn delete_issue(&self, issue_id: &IssueId) -> Result<IssueVersionRecord> {
        let path = format!("/v1/issues/{issue_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete issue request")?
            .error_for_status_with_body("hydra-server returned an error while deleting issue")
            .await?;

        response
            .json::<IssueVersionRecord>()
            .await
            .context("failed to decode delete issue response")
    }

    /// Call `POST /v1/issues/:issue_id/actions` to submit a form response.
    pub async fn submit_form(
        &self,
        issue_id: &IssueId,
        request: &SubmitFormRequest,
    ) -> Result<SubmitFormResponse> {
        let path = format!("/v1/issues/{issue_id}/actions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit form action request")?
            .error_for_status_with_body(
                "hydra-server returned an error while submitting form action",
            )
            .await?;

        response
            .json::<SubmitFormResponse>()
            .await
            .context("failed to decode submit form response")
    }

    /// Call `DELETE /v1/patches/:patch_id` to soft-delete a patch.
    pub async fn delete_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        let path = format!("/v1/patches/{patch_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete patch request")?
            .error_for_status_with_body("hydra-server returned an error while deleting patch")
            .await?;

        response
            .json::<PatchVersionRecord>()
            .await
            .context("failed to decode delete patch response")
    }

    /// Call `DELETE /v1/documents/:document_id` to soft-delete a document.
    pub async fn delete_document(&self, document_id: &DocumentId) -> Result<DocumentVersionRecord> {
        let path = format!("/v1/documents/{document_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete document request")?
            .error_for_status_with_body("hydra-server returned an error while deleting document")
            .await?;

        response
            .json::<DocumentVersionRecord>()
            .await
            .context("failed to decode delete document response")
    }

    /// Call `POST /v1/triggers` to create a new trigger.
    pub async fn create_trigger(
        &self,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        let url = self.endpoint("/v1/triggers")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create trigger request")?
            .error_for_status_with_body("hydra-server rejected create trigger request")
            .await?;

        response
            .json::<UpsertTriggerResponse>()
            .await
            .context("failed to decode create trigger response")
    }

    /// Call `PUT /v1/triggers/:trigger_id` to update an existing trigger.
    pub async fn update_trigger(
        &self,
        trigger_id: &TriggerId,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        let path = format!("/v1/triggers/{trigger_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.put(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update trigger request")?
            .error_for_status_with_body("hydra-server returned an error while updating trigger")
            .await?;

        response
            .json::<UpsertTriggerResponse>()
            .await
            .context("failed to decode update trigger response")
    }

    /// Call `GET /v1/triggers/:trigger_id` to fetch a trigger.
    pub async fn get_trigger(
        &self,
        trigger_id: &TriggerId,
        include_deleted: bool,
    ) -> Result<TriggerVersionRecord> {
        let path = format!("/v1/triggers/{trigger_id}");
        let url = self.endpoint(&path)?;
        let mut builder = self.authed(self.http.get(url));
        if include_deleted {
            builder = builder.query(&[("include_deleted", "true")]);
        }
        let response = builder
            .send()
            .await
            .context("failed to fetch trigger")?
            .error_for_status_with_body("hydra-server returned an error while fetching trigger")
            .await?;

        response
            .json::<TriggerVersionRecord>()
            .await
            .context("failed to decode get trigger response")
    }

    /// Call `GET /v1/triggers/:trigger_id/versions/:version` to fetch a
    /// specific trigger version.
    pub async fn get_trigger_version(
        &self,
        trigger_id: &TriggerId,
        version: RelativeVersionNumber,
    ) -> Result<TriggerVersionRecord> {
        let path = format!("/v1/triggers/{trigger_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch trigger version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching trigger version",
            )
            .await?;

        response
            .json::<TriggerVersionRecord>()
            .await
            .context("failed to decode trigger version response")
    }

    /// Call `GET /v1/triggers` to list triggers.
    pub async fn list_triggers(&self, query: &SearchTriggersQuery) -> Result<ListTriggersResponse> {
        let url = self.endpoint("/v1/triggers")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch triggers list")?
            .error_for_status_with_body("hydra-server returned an error while listing triggers")
            .await?;

        response
            .json::<ListTriggersResponse>()
            .await
            .context("failed to decode list triggers response")
    }

    /// Call `GET /v1/triggers/:trigger_id/versions` to list trigger history.
    pub async fn list_trigger_versions(
        &self,
        trigger_id: &TriggerId,
    ) -> Result<ListTriggerVersionsResponse> {
        let path = format!("/v1/triggers/{trigger_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch trigger versions")?
            .error_for_status_with_body(
                "hydra-server returned an error while listing trigger versions",
            )
            .await?;

        response
            .json::<ListTriggerVersionsResponse>()
            .await
            .context("failed to decode list trigger versions response")
    }

    /// Call `DELETE /v1/triggers/:trigger_id` to soft-delete a trigger.
    pub async fn delete_trigger(&self, trigger_id: &TriggerId) -> Result<TriggerVersionRecord> {
        let path = format!("/v1/triggers/{trigger_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete trigger request")?
            .error_for_status_with_body("hydra-server returned an error while deleting trigger")
            .await?;

        response
            .json::<TriggerVersionRecord>()
            .await
            .context("failed to decode delete trigger response")
    }

    /// Open an SSE connection to GET /v1/events. Uses the streaming HTTP
    /// client so the per-request timeout does not terminate the subscription.
    pub async fn subscribe_events(
        &self,
        query: &EventsQuery,
        last_event_id: Option<u64>,
    ) -> Result<SseEventStream> {
        use hydra_common::api::v1::events::LAST_EVENT_ID_HEADER;

        let url = self.endpoint("/v1/events")?;
        let mut builder = self
            .authed(self.streaming_http.get(url))
            .query(&query.query_pairs())
            .header(header::ACCEPT, "text/event-stream");

        if let Some(id) = last_event_id {
            builder = builder.header(LAST_EVENT_ID_HEADER, id.to_string());
        }

        let response = builder
            .send()
            .await
            .context("failed to connect to events endpoint")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("events endpoint returned {status}: {body}"));
        }

        let is_sse = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("text/event-stream"))
            .unwrap_or(false);

        if !is_sse {
            return Err(anyhow!("events endpoint returned non-SSE content type"));
        }

        Ok(sse::parse_sse_event_stream(Box::pin(
            response.bytes_stream(),
        )))
    }

    /// Call `GET /v1/labels` to list labels with optional filters.
    pub async fn list_labels(&self, query: &SearchLabelsQuery) -> Result<ListLabelsResponse> {
        let url = self.endpoint("/v1/labels")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch labels list")?
            .error_for_status_with_body("hydra-server returned an error while listing labels")
            .await?;

        response
            .json::<ListLabelsResponse>()
            .await
            .context("failed to decode list labels response")
    }

    /// Call `POST /v1/labels` to create a new label.
    pub async fn create_label(&self, request: &UpsertLabelRequest) -> Result<UpsertLabelResponse> {
        let url = self.endpoint("/v1/labels")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create label request")?
            .error_for_status_with_body("hydra-server rejected create label request")
            .await?;

        response
            .json::<UpsertLabelResponse>()
            .await
            .context("failed to decode create label response")
    }

    /// Call `PUT /v1/labels/:label_id/objects/:object_id` to add a label association.
    pub async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<()> {
        let path = format!("/v1/labels/{label_id}/objects/{object_id}");
        let url = self.endpoint(&path)?;
        self.authed(self.http.put(url))
            .send()
            .await
            .context("failed to add label association")?
            .error_for_status_with_body(
                "hydra-server returned an error while adding label association",
            )
            .await?;
        Ok(())
    }

    /// Call `DELETE /v1/labels/:label_id/objects/:object_id` to remove a label association.
    pub async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<()> {
        let path = format!("/v1/labels/{label_id}/objects/{object_id}");
        let url = self.endpoint(&path)?;
        self.authed(self.http.delete(url))
            .send()
            .await
            .context("failed to remove label association")?
            .error_for_status_with_body(
                "hydra-server returned an error while removing label association",
            )
            .await?;
        Ok(())
    }

    /// Call `POST /v1/relations` to create a relation. Returns `true` when
    /// the server reports the relation was newly created (HTTP 201), or
    /// `false` when it already existed (HTTP 200).
    pub async fn create_relation(&self, request: &CreateRelationRequest) -> Result<bool> {
        let url = self.endpoint("/v1/relations")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create relation request")?
            .error_for_status_with_body("hydra-server rejected create relation request")
            .await?;
        Ok(response.status() == StatusCode::CREATED)
    }

    /// Call `DELETE /v1/relations` to remove a relation. Returns the
    /// `removed` flag from the server (`true` when an existing relation was
    /// deleted, `false` when no such relation existed).
    pub async fn remove_relation(&self, request: &RemoveRelationRequest) -> Result<bool> {
        let url = self.endpoint("/v1/relations")?;
        let response = self
            .authed(self.http.delete(url))
            .json(request)
            .send()
            .await
            .context("failed to submit remove relation request")?
            .error_for_status_with_body("hydra-server rejected remove relation request")
            .await?;
        let body: RemoveRelationResponse = response
            .json()
            .await
            .context("failed to decode remove relation response")?;
        Ok(body.removed)
    }

    /// Call `GET /v1/relations` to list relations matching the given filters.
    pub async fn list_relations(
        &self,
        request: &ListRelationsRequest,
    ) -> Result<ListRelationsResponse> {
        let url = self.endpoint("/v1/relations")?;
        let response = self
            .authed(self.http.get(url))
            .query(request)
            .send()
            .await
            .context("failed to fetch relations list")?
            .error_for_status_with_body("hydra-server returned an error while listing relations")
            .await?;

        response
            .json::<ListRelationsResponse>()
            .await
            .context("failed to decode list relations response")
    }

    /// Call `POST /v1/conversations` to create a new conversation.
    pub async fn create_conversation(
        &self,
        request: &CreateConversationRequest,
    ) -> Result<ApiConversation> {
        let url = self.endpoint("/v1/conversations")?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit create conversation request")?
            .error_for_status_with_body("hydra-server rejected create conversation request")
            .await?;

        response
            .json::<ApiConversation>()
            .await
            .context("failed to decode create conversation response")
    }

    /// Call `POST /v1/conversations/:id/messages` to send a message.
    pub async fn send_message(
        &self,
        conversation_id: &ConversationId,
        request: &SendMessageRequest,
    ) -> Result<hydra_common::api::v1::sessions::SessionEvent> {
        let path = format!("/v1/conversations/{conversation_id}/messages");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .json(request)
            .send()
            .await
            .context("failed to submit send message request")?
            .error_for_status_with_body("hydra-server rejected send message request")
            .await?;

        response
            .json::<hydra_common::api::v1::sessions::SessionEvent>()
            .await
            .context("failed to decode send message response")
    }

    /// Call `GET /v1/conversations/:id/versions` to list the full version
    /// history of a conversation (one snapshot per status transition).
    pub async fn get_conversation_versions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<hydra_common::Versioned<ApiConversation>>> {
        let path = format!("/v1/conversations/{conversation_id}/versions");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch conversation versions")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching conversation versions",
            )
            .await?;

        response
            .json::<Vec<hydra_common::Versioned<ApiConversation>>>()
            .await
            .context("failed to decode conversation versions response")
    }

    /// Call `GET /v1/conversations/:id/versions/:version` to fetch a single
    /// versioned conversation snapshot.
    pub async fn get_conversation_version(
        &self,
        conversation_id: &ConversationId,
        version: hydra_common::RelativeVersionNumber,
    ) -> Result<hydra_common::Versioned<ApiConversation>> {
        let path = format!("/v1/conversations/{conversation_id}/versions/{version}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch conversation version")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching conversation version",
            )
            .await?;

        response
            .json::<hydra_common::Versioned<ApiConversation>>()
            .await
            .context("failed to decode conversation version response")
    }

    /// Call `POST /v1/conversations/:id/close` to close a conversation.
    pub async fn close_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        let path = format!("/v1/conversations/{conversation_id}/close");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.post(url))
            .send()
            .await
            .context("failed to submit close conversation request")?
            .error_for_status_with_body("hydra-server rejected close conversation request")
            .await?;

        response
            .json::<ApiConversation>()
            .await
            .context("failed to decode close conversation response")
    }

    /// Call `GET /v1/conversations` to list conversations.
    pub async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<ApiConversationSummary>> {
        let url = self.endpoint("/v1/conversations")?;
        let response = self
            .authed(self.http.get(url))
            .query(query)
            .send()
            .await
            .context("failed to fetch conversations")?
            .error_for_status_with_body(
                "hydra-server returned an error while listing conversations",
            )
            .await?;

        response
            .json::<Vec<ApiConversationSummary>>()
            .await
            .context("failed to decode list conversations response")
    }

    /// Call `GET /v1/conversations/:id` to get a conversation.
    pub async fn get_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        let path = format!("/v1/conversations/{conversation_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.get(url))
            .send()
            .await
            .context("failed to fetch conversation")?
            .error_for_status_with_body(
                "hydra-server returned an error while fetching conversation",
            )
            .await?;

        response
            .json::<ApiConversation>()
            .await
            .context("failed to decode conversation response")
    }

    /// Call `PATCH /v1/conversations/:id` to update a conversation.
    pub async fn update_conversation(
        &self,
        conversation_id: &ConversationId,
        request: &UpdateConversationRequest,
    ) -> Result<ApiConversation> {
        let path = format!("/v1/conversations/{conversation_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.patch(url))
            .json(request)
            .send()
            .await
            .context("failed to submit update conversation request")?
            .error_for_status_with_body("hydra-server rejected update conversation request")
            .await?;

        response
            .json::<ApiConversation>()
            .await
            .context("failed to decode update conversation response")
    }

    /// Call `DELETE /v1/conversations/:id` to soft-delete a conversation.
    pub async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        let path = format!("/v1/conversations/{conversation_id}");
        let url = self.endpoint(&path)?;
        let response = self
            .authed(self.http.delete(url))
            .send()
            .await
            .context("failed to submit delete conversation request")?
            .error_for_status_with_body("hydra-server rejected delete conversation request")
            .await?;

        response
            .json::<ApiConversation>()
            .await
            .context("failed to decode delete conversation response")
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
        Box::pin(
            stream::unfold(
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
            )
            .fuse(),
        )
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
impl HydraClientInterface for HydraClient {
    fn base_url(&self) -> &Url {
        self.base_url()
    }

    async fn connect_relay_websocket(&self, session_id: &SessionId) -> Result<RelayWebSocket> {
        HydraClient::connect_relay_websocket(self, session_id).await
    }

    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        HydraClient::create_session(self, request).await
    }

    async fn list_sessions(&self, query: &SearchSessionsQuery) -> Result<ListSessionsResponse> {
        HydraClient::list_sessions(self, query).await
    }

    async fn get_session(&self, job_id: &SessionId) -> Result<SessionVersionRecord> {
        HydraClient::get_session(self, job_id).await
    }

    async fn get_session_version(
        &self,
        job_id: &SessionId,
        version: RelativeVersionNumber,
    ) -> Result<SessionVersionRecord> {
        HydraClient::get_session_version(self, job_id, version).await
    }

    async fn kill_session(&self, job_id: &SessionId) -> Result<KillSessionResponse> {
        HydraClient::kill_session(self, job_id).await
    }

    async fn get_session_logs(&self, job_id: &SessionId, query: &LogsQuery) -> Result<LogStream> {
        HydraClient::get_session_logs(self, job_id, query).await
    }

    async fn set_session_status(
        &self,
        job_id: &SessionId,
        status: &SessionStatusUpdate,
    ) -> Result<SetSessionStatusResponse> {
        HydraClient::set_session_status(self, job_id, status).await
    }

    async fn get_session_context(&self, job_id: &SessionId) -> Result<WorkerContext> {
        HydraClient::get_session_context(self, job_id).await
    }

    async fn list_proxy_targets(&self, session_id: &SessionId) -> Result<ListProxyTargetsResponse> {
        HydraClient::list_proxy_targets(self, session_id).await
    }

    async fn upsert_proxy_target(
        &self,
        session_id: &SessionId,
        request: &UpsertProxyTargetRequest,
    ) -> Result<()> {
        HydraClient::upsert_proxy_target(self, session_id, request).await
    }

    async fn delete_proxy_target(&self, session_id: &SessionId, port: u16) -> Result<()> {
        HydraClient::delete_proxy_target(self, session_id, port).await
    }

    async fn list_session_versions(
        &self,
        job_id: &SessionId,
    ) -> Result<ListSessionVersionsResponse> {
        HydraClient::list_session_versions(self, job_id).await
    }

    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        HydraClient::create_issue(self, request).await
    }

    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        HydraClient::update_issue(self, issue_id, request).await
    }

    async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<IssueVersionRecord> {
        HydraClient::get_issue(self, issue_id, include_deleted).await
    }

    async fn get_issue_version(
        &self,
        issue_id: &IssueId,
        version: RelativeVersionNumber,
    ) -> Result<IssueVersionRecord> {
        HydraClient::get_issue_version(self, issue_id, version).await
    }

    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        HydraClient::list_issues(self, query).await
    }

    async fn list_issue_versions(&self, issue_id: &IssueId) -> Result<ListIssueVersionsResponse> {
        HydraClient::list_issue_versions(self, issue_id).await
    }

    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        HydraClient::create_patch(self, request).await
    }

    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        HydraClient::update_patch(self, patch_id, request).await
    }

    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        HydraClient::get_patch(self, patch_id).await
    }

    async fn get_patch_version(
        &self,
        patch_id: &PatchId,
        version: RelativeVersionNumber,
    ) -> Result<PatchVersionRecord> {
        HydraClient::get_patch_version(self, patch_id, version).await
    }

    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        HydraClient::list_patches(self, query).await
    }

    async fn list_patch_versions(&self, patch_id: &PatchId) -> Result<ListPatchVersionsResponse> {
        HydraClient::list_patch_versions(self, patch_id).await
    }

    async fn merge_check(&self, patch_id: &PatchId) -> Result<MergeCheckResponse> {
        HydraClient::merge_check(self, patch_id).await
    }

    async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        HydraClient::create_document(self, request).await
    }

    async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        HydraClient::update_document(self, document_id, request).await
    }

    async fn get_document(
        &self,
        document_id: &DocumentId,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        HydraClient::get_document(self, document_id, include_deleted).await
    }

    async fn get_document_by_path(
        &self,
        path: &str,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        HydraClient::get_document_by_path(self, path, include_deleted).await
    }

    async fn list_documents(&self, query: &SearchDocumentsQuery) -> Result<ListDocumentsResponse> {
        HydraClient::list_documents(self, query).await
    }

    async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse> {
        HydraClient::list_document_versions(self, document_id).await
    }

    async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: RelativeVersionNumber,
    ) -> Result<DocumentVersionRecord> {
        HydraClient::get_document_version(self, document_id, version).await
    }

    async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String> {
        HydraClient::create_patch_asset(self, patch_id, file_path).await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<ListRepositoriesResponse> {
        HydraClient::list_repositories(self, query).await
    }

    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        HydraClient::create_repository(self, request).await
    }

    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        HydraClient::update_repository(self, repo_name, request).await
    }

    async fn delete_repository(&self, repo_name: &RepoName) -> Result<RepositoryRecord> {
        HydraClient::delete_repository(self, repo_name).await
    }

    async fn list_projects(&self) -> Result<ListProjectsResponse> {
        HydraClient::list_projects(self).await
    }

    async fn create_project(
        &self,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        HydraClient::create_project(self, request).await
    }

    async fn get_project(&self, project_ref: &ProjectRef) -> Result<ProjectRecord> {
        HydraClient::get_project(self, project_ref).await
    }

    async fn update_project(
        &self,
        project_ref: &ProjectRef,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        HydraClient::update_project(self, project_ref, request).await
    }

    async fn delete_project(&self, project_ref: &ProjectRef) -> Result<UpsertProjectResponse> {
        HydraClient::delete_project(self, project_ref).await
    }

    async fn rename_project_status(
        &self,
        project_ref: &ProjectRef,
        request: &RenameStatusRequest,
    ) -> Result<UpsertProjectResponse> {
        HydraClient::rename_project_status(self, project_ref, request).await
    }

    async fn get_project_statuses(
        &self,
        project_ref: &ProjectRef,
    ) -> Result<ProjectStatusesResponse> {
        HydraClient::get_project_statuses(self, project_ref).await
    }

    async fn whoami(&self) -> Result<WhoAmIResponse> {
        HydraClient::whoami(self).await
    }

    async fn list_users(&self, query: &SearchUsersQuery) -> Result<ListUsersResponse> {
        HydraClient::list_users(self, query).await
    }

    async fn get_user(&self, username: &str) -> Result<UserSummary> {
        HydraClient::get_user(self, username).await
    }

    async fn list_user_secrets(&self, username: &str) -> Result<ListSecretsResponse> {
        HydraClient::list_user_secrets(self, username).await
    }

    async fn set_user_secret(&self, username: &str, name: &str, value: &str) -> Result<()> {
        HydraClient::set_user_secret(self, username, name, value).await
    }

    async fn delete_user_secret(&self, username: &str, name: &str) -> Result<()> {
        HydraClient::delete_user_secret(self, username, name).await
    }

    async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue> {
        HydraClient::get_merge_queue(self, repo_name, branch).await
    }

    async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue> {
        HydraClient::enqueue_merge_patch(self, repo_name, branch, patch_id).await
    }

    async fn list_agents(&self) -> Result<ListAgentsResponse> {
        HydraClient::list_agents(self).await
    }

    async fn get_agent(&self, name: &str) -> Result<AgentResponse> {
        HydraClient::get_agent(self, name).await
    }

    async fn create_agent(&self, request: &UpsertAgentRequest) -> Result<AgentResponse> {
        HydraClient::create_agent(self, request).await
    }

    async fn update_agent(
        &self,
        name: &str,
        request: &UpsertAgentRequest,
    ) -> Result<AgentResponse> {
        HydraClient::update_agent(self, name, request).await
    }

    async fn delete_agent(&self, name: &str) -> Result<DeleteAgentResponse> {
        HydraClient::delete_agent(self, name).await
    }

    async fn delete_issue(&self, issue_id: &IssueId) -> Result<IssueVersionRecord> {
        HydraClient::delete_issue(self, issue_id).await
    }

    async fn submit_form(
        &self,
        issue_id: &IssueId,
        request: &SubmitFormRequest,
    ) -> Result<SubmitFormResponse> {
        HydraClient::submit_form(self, issue_id, request).await
    }

    async fn delete_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        HydraClient::delete_patch(self, patch_id).await
    }

    async fn delete_document(&self, document_id: &DocumentId) -> Result<DocumentVersionRecord> {
        HydraClient::delete_document(self, document_id).await
    }

    async fn create_trigger(
        &self,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        HydraClient::create_trigger(self, request).await
    }

    async fn update_trigger(
        &self,
        trigger_id: &TriggerId,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        HydraClient::update_trigger(self, trigger_id, request).await
    }

    async fn get_trigger(
        &self,
        trigger_id: &TriggerId,
        include_deleted: bool,
    ) -> Result<TriggerVersionRecord> {
        HydraClient::get_trigger(self, trigger_id, include_deleted).await
    }

    async fn get_trigger_version(
        &self,
        trigger_id: &TriggerId,
        version: RelativeVersionNumber,
    ) -> Result<TriggerVersionRecord> {
        HydraClient::get_trigger_version(self, trigger_id, version).await
    }

    async fn list_triggers(&self, query: &SearchTriggersQuery) -> Result<ListTriggersResponse> {
        HydraClient::list_triggers(self, query).await
    }

    async fn list_trigger_versions(
        &self,
        trigger_id: &TriggerId,
    ) -> Result<ListTriggerVersionsResponse> {
        HydraClient::list_trigger_versions(self, trigger_id).await
    }

    async fn delete_trigger(&self, trigger_id: &TriggerId) -> Result<TriggerVersionRecord> {
        HydraClient::delete_trigger(self, trigger_id).await
    }

    async fn subscribe_events(
        &self,
        query: &EventsQuery,
        last_event_id: Option<u64>,
    ) -> Result<SseEventStream> {
        HydraClient::subscribe_events(self, query, last_event_id).await
    }

    async fn list_relations(&self, query: &ListRelationsRequest) -> Result<ListRelationsResponse> {
        HydraClient::list_relations(self, query).await
    }

    async fn list_labels(&self, query: &SearchLabelsQuery) -> Result<ListLabelsResponse> {
        HydraClient::list_labels(self, query).await
    }

    async fn create_label(&self, request: &UpsertLabelRequest) -> Result<UpsertLabelResponse> {
        HydraClient::create_label(self, request).await
    }

    async fn add_label_association(&self, label_id: &LabelId, object_id: &HydraId) -> Result<()> {
        HydraClient::add_label_association(self, label_id, object_id).await
    }

    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<()> {
        HydraClient::remove_label_association(self, label_id, object_id).await
    }

    async fn create_relation(&self, request: &CreateRelationRequest) -> Result<bool> {
        HydraClient::create_relation(self, request).await
    }

    async fn remove_relation(&self, request: &RemoveRelationRequest) -> Result<bool> {
        HydraClient::remove_relation(self, request).await
    }

    async fn create_conversation(
        &self,
        request: &CreateConversationRequest,
    ) -> Result<ApiConversation> {
        HydraClient::create_conversation(self, request).await
    }

    async fn send_message(
        &self,
        conversation_id: &ConversationId,
        request: &SendMessageRequest,
    ) -> Result<hydra_common::api::v1::sessions::SessionEvent> {
        HydraClient::send_message(self, conversation_id, request).await
    }

    async fn get_conversation_versions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<hydra_common::Versioned<ApiConversation>>> {
        HydraClient::get_conversation_versions(self, conversation_id).await
    }

    async fn get_conversation_version(
        &self,
        conversation_id: &ConversationId,
        version: hydra_common::RelativeVersionNumber,
    ) -> Result<hydra_common::Versioned<ApiConversation>> {
        HydraClient::get_conversation_version(self, conversation_id, version).await
    }

    async fn close_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        HydraClient::close_conversation(self, conversation_id).await
    }

    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<ApiConversationSummary>> {
        HydraClient::list_conversations(self, query).await
    }

    async fn get_conversation(&self, conversation_id: &ConversationId) -> Result<ApiConversation> {
        HydraClient::get_conversation(self, conversation_id).await
    }

    async fn update_conversation(
        &self,
        conversation_id: &ConversationId,
        request: &UpdateConversationRequest,
    ) -> Result<ApiConversation> {
        HydraClient::update_conversation(self, conversation_id, request).await
    }

    async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        HydraClient::delete_conversation(self, conversation_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use hydra_common::{
        repositories::{
            CreateRepositoryRequest, Repository, RepositoryRecord, UpdateRepositoryRequest,
        },
        users::Username,
        PatchId,
    };
    use serde_json::json;
    use std::io::Write;
    use std::str::FromStr;
    use tempfile::tempdir;

    const TEST_HYDRA_TOKEN: &str = "users/test:test-hydra-token";

    #[tokio::test]
    async fn list_repositories_fetches_config() -> Result<()> {
        let server = MockServer::start();
        let repositories = vec![RepositoryRecord::new(
            RepoName::from_str("dourolabs/hydra")?,
            Repository::new(
                "https://example.com/repo.git".to_string(),
                Some("main".to_string()),
                Some("ghcr.io/example/repo:main".to_string()),
            ),
        )];
        let payload = ListRepositoriesResponse::new(repositories);
        let payload_for_mock = payload.clone();
        let expected_auth_header = format!("Bearer {TEST_HYDRA_TOKEN}");

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/repositories")
                .header("authorization", expected_auth_header.as_str());
            then.status(200).json_body_obj(&payload_for_mock);
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let response = client
            .list_repositories(&SearchRepositoriesQuery::default())
            .await?;

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
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

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
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let error = client
            .update_repository(&repo_name, &request)
            .await
            .unwrap_err();

        mock.assert();
        let message = format!("{error:#}");
        assert!(
            message.contains("hydra-server returned an error while updating repository"),
            "{message}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_patch_asset_uploads_file_and_returns_url() -> Result<()> {
        let server = MockServer::start();
        let patch_id = PatchId::new();
        let expected_auth_header = format!("Bearer {TEST_HYDRA_TOKEN}");
        let asset_url = "https://github.com/dourolabs/hydra/assets/123";
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
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let response = client.create_patch_asset(&patch_id, &file_path).await?;

        mock.assert();
        assert_eq!(response, asset_url);

        Ok(())
    }

    #[tokio::test]
    async fn stream_sse_logs_preserves_carriage_returns() {
        let events = b"data: Downloading 10%\rprogress\n\n";
        let byte_stream: BytesStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(events))]));

        let mut stream = HydraClient::stream_sse_bytes(byte_stream);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, "Downloading 10%\rprogress");
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn stream_sse_logs_handles_crlf_separators() {
        let events = b"data: first line\r\n\r\ndata: second\r\n\r\n";
        let byte_stream: BytesStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(events))]));

        let mut stream = HydraClient::stream_sse_bytes(byte_stream);

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, "first line");

        let second = stream.next().await.unwrap().unwrap();
        assert_eq!(second, "second");

        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn get_user_returns_user_summary() -> Result<()> {
        let server = MockServer::start();
        let username = "testuser";
        let expected_auth_header = format!("Bearer {TEST_HYDRA_TOKEN}");
        let user_summary = UserSummary::new(Username::from(username), Some(12345));
        let user_summary_clone = user_summary.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/users/testuser")
                .header("authorization", expected_auth_header.as_str());
            then.status(200).json_body_obj(&user_summary_clone);
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let response = client.get_user(username).await?;

        mock.assert();
        assert_eq!(response, user_summary);
        assert_eq!(response.username.as_str(), username);
        assert_eq!(response.github_user_id, Some(12345));

        Ok(())
    }

    #[tokio::test]
    async fn get_user_returns_error_for_not_found() -> Result<()> {
        let server = MockServer::start();
        let expected_auth_header = format!("Bearer {TEST_HYDRA_TOKEN}");

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/users/nonexistent")
                .header("authorization", expected_auth_header.as_str());
            then.status(404)
                .json_body(json!({ "error": "user not found" }));
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let error = client.get_user("nonexistent").await.unwrap_err();

        mock.assert();
        let message = format!("{error:#}");
        assert!(
            message.contains("hydra-server returned an error while fetching user info"),
            "{message}"
        );
        assert!(message.contains("404"), "{message}");

        Ok(())
    }

    #[tokio::test]
    async fn list_users_returns_user_summaries() -> Result<()> {
        let server = MockServer::start();
        let expected_auth_header = format!("Bearer {TEST_HYDRA_TOKEN}");
        let response = ListUsersResponse::new(vec![
            UserSummary::new(Username::from("alice"), Some(1)),
            UserSummary::new(Username::from("bob"), None),
        ]);
        let response_clone = response.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/users")
                .header("authorization", expected_auth_header.as_str());
            then.status(200).json_body_obj(&response_clone);
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let actual = client.list_users(&SearchUsersQuery::default()).await?;

        mock.assert();
        assert_eq!(actual, response);

        Ok(())
    }

    #[tokio::test]
    async fn list_users_passes_query_filters() -> Result<()> {
        let server = MockServer::start();
        let response =
            ListUsersResponse::new(vec![UserSummary::new(Username::from("alice"), Some(1))]);
        let response_clone = response.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET)
                .path("/v1/users")
                .query_param("q", "alice")
                .query_param("include_deleted", "true");
            then.status(200).json_body_obj(&response_clone);
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let query = SearchUsersQuery::new(Some("alice".to_string()), Some(true));
        let actual = client.list_users(&query).await?;

        mock.assert();
        assert_eq!(actual, response);

        Ok(())
    }

    #[tokio::test]
    async fn create_relation_returns_true_on_201_created() -> Result<()> {
        let server = MockServer::start();
        let request = CreateRelationRequest {
            source_id: "i-aaaaaa".parse()?,
            target_id: "i-bbbbbb".parse()?,
            rel_type: "child-of".to_string(),
        };

        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/relations").json_body(json!({
                "source_id": "i-aaaaaa",
                "target_id": "i-bbbbbb",
                "rel_type": "child-of",
            }));
            then.status(201).json_body(json!({
                "source_id": "i-aaaaaa",
                "target_id": "i-bbbbbb",
                "rel_type": "child-of",
            }));
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let created = client.create_relation(&request).await?;

        mock.assert();
        assert!(created, "201 Created should be reported as created=true");
        Ok(())
    }

    #[tokio::test]
    async fn create_relation_returns_false_on_200_already_existed() -> Result<()> {
        let server = MockServer::start();
        let request = CreateRelationRequest {
            source_id: "i-aaaaaa".parse()?,
            target_id: "i-bbbbbb".parse()?,
            rel_type: "child-of".to_string(),
        };

        let mock = server.mock(|when, then| {
            when.method(POST).path("/v1/relations");
            then.status(200).json_body(json!({
                "source_id": "i-aaaaaa",
                "target_id": "i-bbbbbb",
                "rel_type": "child-of",
            }));
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let created = client.create_relation(&request).await?;

        mock.assert();
        assert!(
            !created,
            "200 OK should be reported as created=false (already existed)"
        );
        Ok(())
    }

    #[tokio::test]
    async fn remove_relation_returns_removed_flag_from_body() -> Result<()> {
        let server = MockServer::start();
        let request = RemoveRelationRequest {
            source_id: "i-aaaaaa".parse()?,
            target_id: "i-bbbbbb".parse()?,
            rel_type: "child-of".to_string(),
        };

        let mock_true = server.mock(|when, then| {
            when.method(DELETE).path("/v1/relations").json_body(json!({
                "source_id": "i-aaaaaa",
                "target_id": "i-bbbbbb",
                "rel_type": "child-of",
            }));
            then.status(200).json_body(json!({ "removed": true }));
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let removed = client.remove_relation(&request).await?;

        mock_true.assert();
        assert!(removed, "server's removed=true should propagate");
        Ok(())
    }

    #[tokio::test]
    async fn remove_relation_returns_false_when_not_found() -> Result<()> {
        let server = MockServer::start();
        let request = RemoveRelationRequest {
            source_id: "i-aaaaaa".parse()?,
            target_id: "i-bbbbbb".parse()?,
            rel_type: "child-of".to_string(),
        };

        let mock_false = server.mock(|when, then| {
            when.method(DELETE).path("/v1/relations");
            then.status(200).json_body(json!({ "removed": false }));
        });

        let client =
            HydraClient::with_http_client(server.base_url(), TEST_HYDRA_TOKEN, HttpClient::new())?;

        let removed = client.remove_relation(&request).await?;

        mock_false.assert();
        assert!(!removed, "server's removed=false should propagate");
        Ok(())
    }

    #[tokio::test]
    async fn request_times_out_when_server_is_slow() -> Result<()> {
        let server = MockServer::start();
        let username = "slow-user";
        let user_summary = UserSummary::new(Username::from(username), Some(99));
        let user_summary_clone = user_summary.clone();

        let mock = server.mock(move |when, then| {
            when.method(GET).path(format!("/v1/users/{username}"));
            then.status(200)
                .delay(Duration::from_secs(5))
                .json_body_obj(&user_summary_clone);
        });

        let timeouts = HydraClientTimeouts {
            request_timeout: Duration::from_millis(100),
            connect_timeout: Duration::from_secs(2),
            pool_idle_timeout: Duration::from_secs(60),
        };
        let client = HydraClient::new(server.base_url(), TEST_HYDRA_TOKEN, &timeouts)?;

        let error = client.get_user(username).await.unwrap_err();

        let timed_out = error.chain().any(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .map(|err| err.is_timeout())
                .unwrap_or(false)
        });
        assert!(
            timed_out,
            "expected a reqwest timeout error, got: {error:#}"
        );

        mock.assert();
        Ok(())
    }
}
