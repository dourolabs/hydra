#![allow(clippy::too_many_arguments)]

pub mod activity_log;
pub mod actor_ref;
pub mod api;
pub mod build_cache;
pub mod constants;
pub mod document_path;
pub mod github;
pub mod ids;
pub mod models;
pub mod repo_name;
pub mod review_utils;
pub mod rgb;
pub mod util;
pub mod versioning;

pub use activity_log::{
    ActivityEvent, ActivityLogEntry, ActivityObjectKind, FieldChange,
    activity_log_for_document_versions, activity_log_for_issue_versions,
    activity_log_for_patch_versions, activity_log_for_session_versions, activity_log_from_versions,
};
pub use actor_ref::{ActorId, ActorRef, parse_actor_name};
pub use api::v1::{
    agents, documents, events, issues, labels, login, logs, merge_queues, notifications, patches,
    repositories, secrets, session_status, sessions, task_status, users, version, whoami,
};
pub use build_cache::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig};
pub use document_path::{DocumentPath, DocumentPathError};
pub use ids::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, MetisIdError, NotificationId, PatchId,
    SessionId,
};
pub use models::reviews::{ReviewCommentDraft, ReviewDraft};
pub use repo_name::{RepoName, RepoNameError};
pub use repositories::{
    CreateRepositoryRequest, DeleteRepositoryResponse, ListRepositoriesResponse, Repository,
    RepositoryRecord, SearchRepositoriesQuery, UpdateRepositoryRequest, UpsertRepositoryResponse,
};
pub use rgb::{Rgb, RgbError};
pub use util::EnvGuard;
pub use versioning::{RelativeVersionNumber, VersionNumber, Versioned};

#[cfg(test)]
pub mod test_helpers {
    use serde::Serialize;

    pub fn serialize_query_params<T: Serialize>(value: &T) -> Vec<(String, String)> {
        let encoded =
            serde_urlencoded::to_string(value).expect("failed to encode query parameters");
        serde_urlencoded::from_str(&encoded)
            .expect("failed to decode encoded query parameters into key/value pairs")
    }
}

#[cfg(test)]
#[cfg(feature = "ts")]
mod ts_export {
    use ts_rs::{Config, TS};

    /// Running this test with `TS_RS_EXPORT_DIR` set will export all TypeScript
    /// definitions to the specified directory.
    ///
    /// Usage:
    ///   TS_RS_EXPORT_DIR=metis-web/packages/api/src/generated \
    ///     cargo test -p metis-common --features ts export_bindings
    #[test]
    fn export_bindings() {
        let cfg = Config::from_env();

        // Core types
        crate::MetisId::export_all(&cfg).expect("MetisId");
        crate::IssueId::export_all(&cfg).expect("IssueId");
        crate::PatchId::export_all(&cfg).expect("PatchId");
        crate::DocumentId::export_all(&cfg).expect("DocumentId");
        crate::SessionId::export_all(&cfg).expect("SessionId");
        crate::NotificationId::export_all(&cfg).expect("NotificationId");
        crate::LabelId::export_all(&cfg).expect("LabelId");
        crate::DocumentPath::export_all(&cfg).expect("DocumentPath");
        crate::Rgb::export_all(&cfg).expect("Rgb");
        crate::RepoName::export_all(&cfg).expect("RepoName");
        crate::ActorId::export_all(&cfg).expect("ActorId");
        crate::ActorRef::export_all(&cfg).expect("ActorRef");
        crate::Versioned::<()>::export_all(&cfg).expect("Versioned");

        // Activity log
        crate::ActivityObjectKind::export_all(&cfg).expect("ActivityObjectKind");
        crate::FieldChange::export_all(&cfg).expect("FieldChange");
        crate::ActivityEvent::export_all(&cfg).expect("ActivityEvent");
        crate::ActivityLogEntry::export_all(&cfg).expect("ActivityLogEntry");

        // GitHub
        crate::github::GithubAppClientIdResponse::export_all(&cfg)
            .expect("GithubAppClientIdResponse");
        crate::github::GithubTokenResponse::export_all(&cfg).expect("GithubTokenResponse");

        // Build cache
        crate::BuildCacheSettings::export_all(&cfg).expect("BuildCacheSettings");
        crate::BuildCacheStorageConfig::export_all(&cfg).expect("BuildCacheStorageConfig");
        crate::BuildCacheContext::export_all(&cfg).expect("BuildCacheContext");

        // API v1: agents
        crate::agents::AgentRecord::export_all(&cfg).expect("AgentRecord");
        crate::agents::UpsertAgentRequest::export_all(&cfg).expect("UpsertAgentRequest");
        crate::agents::AgentResponse::export_all(&cfg).expect("AgentResponse");
        crate::agents::DeleteAgentResponse::export_all(&cfg).expect("DeleteAgentResponse");
        crate::agents::ListAgentsResponse::export_all(&cfg).expect("ListAgentsResponse");

        // API v1: documents
        crate::documents::Document::export_all(&cfg).expect("Document");
        crate::documents::DocumentVersionRecord::export_all(&cfg).expect("DocumentVersionRecord");
        crate::documents::SearchDocumentsQuery::export_all(&cfg).expect("SearchDocumentsQuery");
        crate::documents::GetDocumentQuery::export_all(&cfg).expect("GetDocumentQuery");
        crate::documents::UpsertDocumentRequest::export_all(&cfg).expect("UpsertDocumentRequest");
        crate::documents::UpsertDocumentResponse::export_all(&cfg).expect("UpsertDocumentResponse");
        crate::documents::ListDocumentsResponse::export_all(&cfg).expect("ListDocumentsResponse");
        crate::documents::ListDocumentVersionsResponse::export_all(&cfg)
            .expect("ListDocumentVersionsResponse");

        // API v1: events
        crate::events::EventsQuery::export_all(&cfg).expect("EventsQuery");
        crate::events::SseEventType::export_all(&cfg).expect("SseEventType");
        crate::events::EntityEventData::export_all(&cfg).expect("EntityEventData");
        crate::events::SnapshotEventData::export_all(&cfg).expect("SnapshotEventData");
        crate::events::ResyncEventData::export_all(&cfg).expect("ResyncEventData");
        crate::events::HeartbeatEventData::export_all(&cfg).expect("HeartbeatEventData");

        // API v1: labels
        crate::labels::Label::export_all(&cfg).expect("Label");
        crate::labels::LabelSummary::export_all(&cfg).expect("LabelSummary");
        crate::labels::LabelRecord::export_all(&cfg).expect("LabelRecord");
        crate::labels::UpsertLabelRequest::export_all(&cfg).expect("UpsertLabelRequest");
        crate::labels::UpsertLabelResponse::export_all(&cfg).expect("UpsertLabelResponse");
        crate::labels::SearchLabelsQuery::export_all(&cfg).expect("SearchLabelsQuery");
        crate::labels::ListLabelsResponse::export_all(&cfg).expect("ListLabelsResponse");

        // API v1: issues
        crate::issues::IssueStatus::export_all(&cfg).expect("IssueStatus");
        crate::issues::IssueType::export_all(&cfg).expect("IssueType");
        crate::issues::IssueDependencyType::export_all(&cfg).expect("IssueDependencyType");
        crate::issues::IssueDependency::export_all(&cfg).expect("IssueDependency");
        crate::issues::TodoItem::export_all(&cfg).expect("TodoItem");
        crate::issues::TodoListResponse::export_all(&cfg).expect("TodoListResponse");
        crate::issues::AddTodoItemRequest::export_all(&cfg).expect("AddTodoItemRequest");
        crate::issues::ReplaceTodoListRequest::export_all(&cfg).expect("ReplaceTodoListRequest");
        crate::issues::SetTodoItemStatusRequest::export_all(&cfg)
            .expect("SetTodoItemStatusRequest");
        crate::issues::Issue::export_all(&cfg).expect("Issue");
        crate::issues::SessionSettings::export_all(&cfg).expect("SessionSettings");
        crate::issues::IssueVersionRecord::export_all(&cfg).expect("IssueVersionRecord");
        crate::issues::UpsertIssueRequest::export_all(&cfg).expect("UpsertIssueRequest");
        crate::issues::UpsertIssueResponse::export_all(&cfg).expect("UpsertIssueResponse");
        crate::issues::IssueSummary::export_all(&cfg).expect("IssueSummary");
        crate::issues::IssueSummaryRecord::export_all(&cfg).expect("IssueSummaryRecord");
        crate::issues::SearchIssuesQuery::export_all(&cfg).expect("SearchIssuesQuery");
        crate::issues::ListIssuesResponse::export_all(&cfg).expect("ListIssuesResponse");
        crate::issues::ListIssueVersionsResponse::export_all(&cfg)
            .expect("ListIssueVersionsResponse");

        // API v1: session_status
        crate::session_status::SessionStatusUpdate::export_all(&cfg).expect("SessionStatusUpdate");
        crate::session_status::SetSessionStatusResponse::export_all(&cfg)
            .expect("SetSessionStatusResponse");

        // API v1: sessions
        crate::sessions::Session::export_all(&cfg).expect("Session");
        crate::sessions::CreateSessionRequest::export_all(&cfg).expect("CreateSessionRequest");
        crate::sessions::BundleSpec::export_all(&cfg).expect("BundleSpec");
        crate::sessions::Bundle::export_all(&cfg).expect("Bundle");
        crate::sessions::WorkerContext::export_all(&cfg).expect("WorkerContext");
        crate::sessions::CreateSessionResponse::export_all(&cfg).expect("CreateSessionResponse");
        crate::sessions::ListSessionsResponse::export_all(&cfg).expect("ListSessionsResponse");
        crate::sessions::SessionVersionRecord::export_all(&cfg).expect("SessionVersionRecord");
        crate::sessions::SearchSessionsQuery::export_all(&cfg).expect("SearchSessionsQuery");
        crate::sessions::ListSessionVersionsResponse::export_all(&cfg)
            .expect("ListSessionVersionsResponse");
        crate::sessions::KillSessionResponse::export_all(&cfg).expect("KillSessionResponse");

        // API v1: login
        crate::login::LoginRequest::export_all(&cfg).expect("LoginRequest");
        crate::login::LoginResponse::export_all(&cfg).expect("LoginResponse");

        // API v1: logs
        crate::logs::LogsQuery::export_all(&cfg).expect("LogsQuery");

        // API v1: merge_queues
        crate::merge_queues::MergeQueue::export_all(&cfg).expect("MergeQueue");
        crate::merge_queues::EnqueueMergePatchRequest::export_all(&cfg)
            .expect("EnqueueMergePatchRequest");

        // API v1: notifications
        crate::notifications::Notification::export_all(&cfg).expect("Notification");
        crate::notifications::NotificationResponse::export_all(&cfg).expect("NotificationResponse");
        crate::notifications::ListNotificationsQuery::export_all(&cfg)
            .expect("ListNotificationsQuery");
        crate::notifications::ListNotificationsResponse::export_all(&cfg)
            .expect("ListNotificationsResponse");
        crate::notifications::UnreadCountResponse::export_all(&cfg).expect("UnreadCountResponse");
        crate::notifications::MarkReadResponse::export_all(&cfg).expect("MarkReadResponse");

        // API v1: patches
        crate::patches::PatchStatus::export_all(&cfg).expect("PatchStatus");
        crate::patches::Review::export_all(&cfg).expect("Review");
        crate::patches::GithubPr::export_all(&cfg).expect("GithubPr");
        crate::patches::GitOid::export_all(&cfg).expect("GitOid");
        crate::patches::CommitRange::export_all(&cfg).expect("CommitRange");
        crate::patches::Patch::export_all(&cfg).expect("Patch");
        crate::patches::PatchVersionRecord::export_all(&cfg).expect("PatchVersionRecord");
        crate::patches::UpsertPatchRequest::export_all(&cfg).expect("UpsertPatchRequest");
        crate::patches::UpsertPatchResponse::export_all(&cfg).expect("UpsertPatchResponse");
        crate::patches::CreatePatchAssetQuery::export_all(&cfg).expect("CreatePatchAssetQuery");
        crate::patches::CreatePatchAssetResponse::export_all(&cfg)
            .expect("CreatePatchAssetResponse");
        crate::patches::SearchPatchesQuery::export_all(&cfg).expect("SearchPatchesQuery");
        crate::patches::GithubCiState::export_all(&cfg).expect("GithubCiState");
        crate::patches::GithubCiFailure::export_all(&cfg).expect("GithubCiFailure");
        crate::patches::GithubCiStatus::export_all(&cfg).expect("GithubCiStatus");
        crate::patches::ListPatchesResponse::export_all(&cfg).expect("ListPatchesResponse");
        crate::patches::ListPatchVersionsResponse::export_all(&cfg)
            .expect("ListPatchVersionsResponse");

        // API v1: repositories
        crate::repositories::RepoWorkflowConfig::export_all(&cfg).expect("RepoWorkflowConfig");
        crate::repositories::ReviewRequestConfig::export_all(&cfg).expect("ReviewRequestConfig");
        crate::repositories::MergeRequestConfig::export_all(&cfg).expect("MergeRequestConfig");
        crate::Repository::export_all(&cfg).expect("Repository");
        crate::RepositoryRecord::export_all(&cfg).expect("RepositoryRecord");
        crate::CreateRepositoryRequest::export_all(&cfg).expect("CreateRepositoryRequest");
        crate::UpdateRepositoryRequest::export_all(&cfg).expect("UpdateRepositoryRequest");
        crate::UpsertRepositoryResponse::export_all(&cfg).expect("UpsertRepositoryResponse");
        crate::SearchRepositoriesQuery::export_all(&cfg).expect("SearchRepositoriesQuery");
        crate::ListRepositoriesResponse::export_all(&cfg).expect("ListRepositoriesResponse");
        crate::DeleteRepositoryResponse::export_all(&cfg).expect("DeleteRepositoryResponse");

        // API v1: task_status
        crate::task_status::Status::export_all(&cfg).expect("Status");
        crate::task_status::TaskError::export_all(&cfg).expect("TaskError");

        // API v1: users
        crate::users::Username::export_all(&cfg).expect("Username");
        crate::users::User::export_all(&cfg).expect("User");
        crate::users::UserSummary::export_all(&cfg).expect("UserSummary");
        crate::users::SearchUsersQuery::export_all(&cfg).expect("SearchUsersQuery");

        // API v1: version
        crate::version::VersionResponse::export_all(&cfg).expect("VersionResponse");

        // API v1: whoami
        crate::whoami::ActorIdentity::export_all(&cfg).expect("ActorIdentity");
        crate::whoami::WhoAmIResponse::export_all(&cfg).expect("WhoAmIResponse");

        // API v1: secrets
        crate::api::v1::secrets::ListSecretsResponse::export_all(&cfg)
            .expect("ListSecretsResponse");
        crate::api::v1::secrets::SetSecretRequest::export_all(&cfg).expect("SetSecretRequest");

        // API v1: error
        crate::api::v1::error::ApiErrorBody::export_all(&cfg).expect("ApiErrorBody");
    }
}
