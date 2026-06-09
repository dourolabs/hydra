//! Test-only [`HydraClientInterface`] wrapper that counts (and rejects) calls
//! to [`HydraClientInterface::connect_relay_websocket`].
//!
//! Used to pin the invariant in `worker_run::run` that a Codex-class model in
//! interactive mode must return `Err` **before** the relay WebSocket is opened
//! (see `hydra/src/command/sessions/worker_run.rs:213`). Every other method
//! delegates to the wrapped client.
//!
//! Returning `Err` (rather than delegating) when the relay open is attempted
//! is intentional: if the guard regresses, the test fails loudly at the call
//! site in addition to the post-hoc counter assertion.

#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use hydra::client::sse::SseEventStream;
use hydra::client::{HydraClientInterface, LogStream, RelayWebSocket};
use hydra_common::{
    agents::{AgentResponse, DeleteAgentResponse, ListAgentsResponse, UpsertAgentRequest},
    api::v1::conversations::{
        Conversation as ApiConversation, ConversationSummary as ApiConversationSummary,
        CreateConversationRequest, SearchConversationsQuery, SendMessageRequest,
        UpdateConversationRequest,
    },
    api::v1::events::EventsQuery,
    api::v1::labels::{
        ListLabelsResponse, SearchLabelsQuery, UpsertLabelRequest, UpsertLabelResponse,
    },
    api::v1::merge_check::MergeCheckResponse,
    api::v1::projects::{
        ListProjectsResponse, ProjectRecord, ProjectRef, ProjectStatusesResponse, StatusDefinition,
        StatusKey, UpsertProjectRequest, UpsertProjectResponse, UpsertProjectStatusResponse,
    },
    api::v1::relations::{
        CreateRelationRequest, ListRelationsRequest, ListRelationsResponse, RemoveRelationRequest,
    },
    api::v1::secrets::ListSecretsResponse,
    documents::{
        DocumentVersionRecord, ListDocumentVersionsResponse, ListDocumentsResponse,
        SearchDocumentsQuery, UpsertDocumentRequest, UpsertDocumentResponse,
    },
    issues::{
        IssueVersionRecord, ListIssueVersionsResponse, ListIssuesResponse, SearchIssuesQuery,
        SubmitFormRequest, SubmitFormResponse, UpsertIssueRequest, UpsertIssueResponse,
    },
    logs::LogsQuery,
    merge_queues::MergeQueue,
    patches::{
        ListPatchVersionsResponse, ListPatchesResponse, PatchVersionRecord, SearchPatchesQuery,
        UpsertPatchRequest, UpsertPatchResponse,
    },
    repositories::{
        CreateRepositoryRequest, ListRepositoriesResponse, RepositoryRecord,
        SearchRepositoriesQuery, UpdateRepositoryRequest, UpsertRepositoryResponse,
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
use reqwest::Url;

/// Wraps an [`HydraClientInterface`] and intercepts
/// [`HydraClientInterface::connect_relay_websocket`] to (1) increment a
/// counter and (2) return `Err`. All other methods are delegated unchanged.
pub struct RelayCallCountingClient {
    inner: Arc<dyn HydraClientInterface>,
    relay_calls: Arc<AtomicUsize>,
}

impl RelayCallCountingClient {
    pub fn new(inner: Arc<dyn HydraClientInterface>) -> Self {
        Self {
            inner,
            relay_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn relay_call_count(&self) -> usize {
        self.relay_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HydraClientInterface for RelayCallCountingClient {
    fn base_url(&self) -> &Url {
        self.inner.base_url()
    }

    async fn connect_relay_websocket(&self, _session_id: &SessionId) -> Result<RelayWebSocket> {
        self.relay_calls.fetch_add(1, Ordering::SeqCst);
        Err(anyhow!(
            "RelayCallCountingClient: relay websocket must not be opened in this test"
        ))
    }

    async fn create_session(
        &self,
        request: &CreateSessionRequest,
    ) -> Result<CreateSessionResponse> {
        self.inner.create_session(request).await
    }

    async fn list_sessions(&self, query: &SearchSessionsQuery) -> Result<ListSessionsResponse> {
        self.inner.list_sessions(query).await
    }

    async fn get_session(&self, job_id: &SessionId) -> Result<SessionVersionRecord> {
        self.inner.get_session(job_id).await
    }

    async fn get_session_version(
        &self,
        job_id: &SessionId,
        version: RelativeVersionNumber,
    ) -> Result<SessionVersionRecord> {
        self.inner.get_session_version(job_id, version).await
    }

    async fn kill_session(&self, job_id: &SessionId) -> Result<KillSessionResponse> {
        self.inner.kill_session(job_id).await
    }

    async fn get_session_logs(&self, job_id: &SessionId, query: &LogsQuery) -> Result<LogStream> {
        self.inner.get_session_logs(job_id, query).await
    }

    async fn set_session_status(
        &self,
        job_id: &SessionId,
        status: &SessionStatusUpdate,
    ) -> Result<SetSessionStatusResponse> {
        self.inner.set_session_status(job_id, status).await
    }

    async fn get_session_context(&self, job_id: &SessionId) -> Result<WorkerContext> {
        self.inner.get_session_context(job_id).await
    }

    async fn list_proxy_targets(&self, session_id: &SessionId) -> Result<ListProxyTargetsResponse> {
        self.inner.list_proxy_targets(session_id).await
    }

    async fn upsert_proxy_target(
        &self,
        session_id: &SessionId,
        request: &UpsertProxyTargetRequest,
    ) -> Result<()> {
        self.inner.upsert_proxy_target(session_id, request).await
    }

    async fn delete_proxy_target(&self, session_id: &SessionId, port: u16) -> Result<()> {
        self.inner.delete_proxy_target(session_id, port).await
    }

    async fn list_session_versions(
        &self,
        job_id: &SessionId,
    ) -> Result<ListSessionVersionsResponse> {
        self.inner.list_session_versions(job_id).await
    }

    async fn create_issue(&self, request: &UpsertIssueRequest) -> Result<UpsertIssueResponse> {
        self.inner.create_issue(request).await
    }

    async fn update_issue(
        &self,
        issue_id: &IssueId,
        request: &UpsertIssueRequest,
    ) -> Result<UpsertIssueResponse> {
        self.inner.update_issue(issue_id, request).await
    }

    async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<IssueVersionRecord> {
        self.inner.get_issue(issue_id, include_deleted).await
    }

    async fn get_issue_version(
        &self,
        issue_id: &IssueId,
        version: RelativeVersionNumber,
    ) -> Result<IssueVersionRecord> {
        self.inner.get_issue_version(issue_id, version).await
    }

    async fn list_issues(&self, query: &SearchIssuesQuery) -> Result<ListIssuesResponse> {
        self.inner.list_issues(query).await
    }

    async fn list_issue_versions(&self, issue_id: &IssueId) -> Result<ListIssueVersionsResponse> {
        self.inner.list_issue_versions(issue_id).await
    }

    async fn create_patch(&self, request: &UpsertPatchRequest) -> Result<UpsertPatchResponse> {
        self.inner.create_patch(request).await
    }

    async fn update_patch(
        &self,
        patch_id: &PatchId,
        request: &UpsertPatchRequest,
    ) -> Result<UpsertPatchResponse> {
        self.inner.update_patch(patch_id, request).await
    }

    async fn get_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        self.inner.get_patch(patch_id).await
    }

    async fn get_patch_version(
        &self,
        patch_id: &PatchId,
        version: RelativeVersionNumber,
    ) -> Result<PatchVersionRecord> {
        self.inner.get_patch_version(patch_id, version).await
    }

    async fn list_patches(&self, query: &SearchPatchesQuery) -> Result<ListPatchesResponse> {
        self.inner.list_patches(query).await
    }

    async fn list_patch_versions(&self, patch_id: &PatchId) -> Result<ListPatchVersionsResponse> {
        self.inner.list_patch_versions(patch_id).await
    }

    async fn merge_check(&self, patch_id: &PatchId) -> Result<MergeCheckResponse> {
        self.inner.merge_check(patch_id).await
    }

    async fn create_document(
        &self,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        self.inner.create_document(request).await
    }

    async fn update_document(
        &self,
        document_id: &DocumentId,
        request: &UpsertDocumentRequest,
    ) -> Result<UpsertDocumentResponse> {
        self.inner.update_document(document_id, request).await
    }

    async fn get_document(
        &self,
        document_id: &DocumentId,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        self.inner.get_document(document_id, include_deleted).await
    }

    async fn get_document_by_path(
        &self,
        path: &str,
        include_deleted: bool,
    ) -> Result<DocumentVersionRecord> {
        self.inner.get_document_by_path(path, include_deleted).await
    }

    async fn list_documents(&self, query: &SearchDocumentsQuery) -> Result<ListDocumentsResponse> {
        self.inner.list_documents(query).await
    }

    async fn list_document_versions(
        &self,
        document_id: &DocumentId,
    ) -> Result<ListDocumentVersionsResponse> {
        self.inner.list_document_versions(document_id).await
    }

    async fn get_document_version(
        &self,
        document_id: &DocumentId,
        version: RelativeVersionNumber,
    ) -> Result<DocumentVersionRecord> {
        self.inner.get_document_version(document_id, version).await
    }

    async fn create_patch_asset(&self, patch_id: &PatchId, file_path: &Path) -> Result<String> {
        self.inner.create_patch_asset(patch_id, file_path).await
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<ListRepositoriesResponse> {
        self.inner.list_repositories(query).await
    }

    async fn create_repository(
        &self,
        request: &CreateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        self.inner.create_repository(request).await
    }

    async fn update_repository(
        &self,
        repo_name: &RepoName,
        request: &UpdateRepositoryRequest,
    ) -> Result<UpsertRepositoryResponse> {
        self.inner.update_repository(repo_name, request).await
    }

    async fn delete_repository(&self, repo_name: &RepoName) -> Result<RepositoryRecord> {
        self.inner.delete_repository(repo_name).await
    }

    async fn list_projects(&self) -> Result<ListProjectsResponse> {
        self.inner.list_projects().await
    }

    async fn create_project(
        &self,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        self.inner.create_project(request).await
    }

    async fn get_project(&self, project_ref: &ProjectRef) -> Result<ProjectRecord> {
        self.inner.get_project(project_ref).await
    }

    async fn update_project(
        &self,
        project_ref: &ProjectRef,
        request: &UpsertProjectRequest,
    ) -> Result<UpsertProjectResponse> {
        self.inner.update_project(project_ref, request).await
    }

    async fn delete_project(&self, project_ref: &ProjectRef) -> Result<UpsertProjectResponse> {
        self.inner.delete_project(project_ref).await
    }

    async fn create_project_status(
        &self,
        project_ref: &ProjectRef,
        request: &StatusDefinition,
    ) -> Result<UpsertProjectStatusResponse> {
        self.inner.create_project_status(project_ref, request).await
    }

    async fn update_project_status(
        &self,
        project_ref: &ProjectRef,
        status_key: &StatusKey,
        request: &StatusDefinition,
    ) -> Result<UpsertProjectStatusResponse> {
        self.inner
            .update_project_status(project_ref, status_key, request)
            .await
    }

    async fn delete_project_status(
        &self,
        project_ref: &ProjectRef,
        status_key: &StatusKey,
    ) -> Result<UpsertProjectResponse> {
        self.inner
            .delete_project_status(project_ref, status_key)
            .await
    }

    async fn get_project_statuses(
        &self,
        project_ref: &ProjectRef,
    ) -> Result<ProjectStatusesResponse> {
        self.inner.get_project_statuses(project_ref).await
    }

    async fn whoami(&self) -> Result<WhoAmIResponse> {
        self.inner.whoami().await
    }

    async fn list_users(&self, query: &SearchUsersQuery) -> Result<ListUsersResponse> {
        self.inner.list_users(query).await
    }

    async fn get_user(&self, username: &str) -> Result<UserSummary> {
        self.inner.get_user(username).await
    }

    async fn list_user_secrets(&self, username: &str) -> Result<ListSecretsResponse> {
        self.inner.list_user_secrets(username).await
    }

    async fn set_user_secret(&self, username: &str, name: &str, value: &str) -> Result<()> {
        self.inner.set_user_secret(username, name, value).await
    }

    async fn delete_user_secret(&self, username: &str, name: &str) -> Result<()> {
        self.inner.delete_user_secret(username, name).await
    }

    async fn get_merge_queue(&self, repo_name: &RepoName, branch: &str) -> Result<MergeQueue> {
        self.inner.get_merge_queue(repo_name, branch).await
    }

    async fn enqueue_merge_patch(
        &self,
        repo_name: &RepoName,
        branch: &str,
        patch_id: &PatchId,
    ) -> Result<MergeQueue> {
        self.inner
            .enqueue_merge_patch(repo_name, branch, patch_id)
            .await
    }

    async fn list_agents(&self) -> Result<ListAgentsResponse> {
        self.inner.list_agents().await
    }

    async fn get_agent(&self, name: &str) -> Result<AgentResponse> {
        self.inner.get_agent(name).await
    }

    async fn create_agent(&self, request: &UpsertAgentRequest) -> Result<AgentResponse> {
        self.inner.create_agent(request).await
    }

    async fn update_agent(
        &self,
        name: &str,
        request: &UpsertAgentRequest,
    ) -> Result<AgentResponse> {
        self.inner.update_agent(name, request).await
    }

    async fn delete_agent(&self, name: &str) -> Result<DeleteAgentResponse> {
        self.inner.delete_agent(name).await
    }

    async fn delete_issue(&self, issue_id: &IssueId) -> Result<IssueVersionRecord> {
        self.inner.delete_issue(issue_id).await
    }

    async fn submit_form(
        &self,
        issue_id: &IssueId,
        request: &SubmitFormRequest,
    ) -> Result<SubmitFormResponse> {
        self.inner.submit_form(issue_id, request).await
    }

    async fn delete_patch(&self, patch_id: &PatchId) -> Result<PatchVersionRecord> {
        self.inner.delete_patch(patch_id).await
    }

    async fn delete_document(&self, document_id: &DocumentId) -> Result<DocumentVersionRecord> {
        self.inner.delete_document(document_id).await
    }

    async fn create_trigger(
        &self,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        self.inner.create_trigger(request).await
    }

    async fn update_trigger(
        &self,
        trigger_id: &TriggerId,
        request: &UpsertTriggerRequest,
    ) -> Result<UpsertTriggerResponse> {
        self.inner.update_trigger(trigger_id, request).await
    }

    async fn get_trigger(
        &self,
        trigger_id: &TriggerId,
        include_deleted: bool,
    ) -> Result<TriggerVersionRecord> {
        self.inner.get_trigger(trigger_id, include_deleted).await
    }

    async fn get_trigger_version(
        &self,
        trigger_id: &TriggerId,
        version: RelativeVersionNumber,
    ) -> Result<TriggerVersionRecord> {
        self.inner.get_trigger_version(trigger_id, version).await
    }

    async fn list_triggers(&self, query: &SearchTriggersQuery) -> Result<ListTriggersResponse> {
        self.inner.list_triggers(query).await
    }

    async fn list_trigger_versions(
        &self,
        trigger_id: &TriggerId,
    ) -> Result<ListTriggerVersionsResponse> {
        self.inner.list_trigger_versions(trigger_id).await
    }

    async fn delete_trigger(&self, trigger_id: &TriggerId) -> Result<TriggerVersionRecord> {
        self.inner.delete_trigger(trigger_id).await
    }

    async fn subscribe_events(
        &self,
        query: &EventsQuery,
        last_event_id: Option<u64>,
    ) -> Result<SseEventStream> {
        self.inner.subscribe_events(query, last_event_id).await
    }

    async fn list_relations(&self, query: &ListRelationsRequest) -> Result<ListRelationsResponse> {
        self.inner.list_relations(query).await
    }

    async fn list_labels(&self, query: &SearchLabelsQuery) -> Result<ListLabelsResponse> {
        self.inner.list_labels(query).await
    }

    async fn create_label(&self, request: &UpsertLabelRequest) -> Result<UpsertLabelResponse> {
        self.inner.create_label(request).await
    }

    async fn add_label_association(&self, label_id: &LabelId, object_id: &HydraId) -> Result<()> {
        self.inner.add_label_association(label_id, object_id).await
    }

    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &HydraId,
    ) -> Result<()> {
        self.inner
            .remove_label_association(label_id, object_id)
            .await
    }

    async fn create_relation(&self, request: &CreateRelationRequest) -> Result<bool> {
        self.inner.create_relation(request).await
    }

    async fn remove_relation(&self, request: &RemoveRelationRequest) -> Result<bool> {
        self.inner.remove_relation(request).await
    }

    async fn create_conversation(
        &self,
        request: &CreateConversationRequest,
    ) -> Result<ApiConversation> {
        self.inner.create_conversation(request).await
    }

    async fn send_message(
        &self,
        conversation_id: &ConversationId,
        request: &SendMessageRequest,
    ) -> Result<hydra_common::api::v1::sessions::SessionEvent> {
        self.inner.send_message(conversation_id, request).await
    }

    async fn get_conversation_versions(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<hydra_common::Versioned<ApiConversation>>> {
        self.inner.get_conversation_versions(conversation_id).await
    }

    async fn get_conversation_version(
        &self,
        conversation_id: &ConversationId,
        version: hydra_common::RelativeVersionNumber,
    ) -> Result<hydra_common::Versioned<ApiConversation>> {
        self.inner
            .get_conversation_version(conversation_id, version)
            .await
    }

    async fn close_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        self.inner.close_conversation(conversation_id).await
    }

    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<ApiConversationSummary>> {
        self.inner.list_conversations(query).await
    }

    async fn get_conversation(&self, conversation_id: &ConversationId) -> Result<ApiConversation> {
        self.inner.get_conversation(conversation_id).await
    }

    async fn update_conversation(
        &self,
        conversation_id: &ConversationId,
        request: &UpdateConversationRequest,
    ) -> Result<ApiConversation> {
        self.inner
            .update_conversation(conversation_id, request)
            .await
    }

    async fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<ApiConversation> {
        self.inner.delete_conversation(conversation_id).await
    }

    async fn current_actor_id(&self) -> Result<ActorId> {
        self.inner.current_actor_id().await
    }
}
