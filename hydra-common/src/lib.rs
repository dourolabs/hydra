#![allow(clippy::too_many_arguments)]

pub mod activity_log;
pub mod actor_ref;
pub mod api;
pub mod build_cache;
pub mod constants;
pub mod document_path;
pub mod github;
pub mod graph;
pub mod ids;
pub mod models;
pub mod principal;
pub mod repo_name;
pub mod review_utils;
pub mod rgb;
pub mod time;
pub mod util;
pub mod versioning;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub use activity_log::{
    ActivityEvent, ActivityLogEntry, ActivityObjectKind, FieldChange,
    activity_log_for_document_versions, activity_log_for_patch_versions,
    activity_log_for_session_versions, activity_log_from_versions,
};
pub use actor_ref::{ActorId, ActorRef, parse_actor_name};
pub use api::v1::{
    agents, analytics, comments, conversations, documents, events, form, issues, labels, login,
    logs, merge_check, merge_queues, patches, projects, relay, repositories, secrets,
    session_status, sessions, task_status, triggers, users, version, whoami,
};
pub use build_cache::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig};
pub use document_path::{DocumentPath, DocumentPathError};
pub use ids::{
    ConversationId, DocumentId, HydraId, HydraIdError, IssueId, LabelId, PatchId, ProjectId,
    SessionId, TriggerId, random_len_for_count,
};
pub use models::reviews::{ReviewCommentDraft, ReviewDraft};
pub use principal::{
    ExternalSystem, ExternalSystemError, Principal, PrincipalParseError, principal_eq,
};
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
    ///   TS_RS_EXPORT_DIR=hydra-web/packages/api/src/generated \
    ///     cargo test -p hydra-common --features ts export_bindings
    #[test]
    fn export_bindings() {
        let cfg = Config::from_env();

        // Core types
        crate::HydraId::export_all(&cfg).expect("HydraId");
        crate::IssueId::export_all(&cfg).expect("IssueId");
        crate::PatchId::export_all(&cfg).expect("PatchId");
        crate::DocumentId::export_all(&cfg).expect("DocumentId");
        crate::SessionId::export_all(&cfg).expect("SessionId");
        crate::LabelId::export_all(&cfg).expect("LabelId");
        crate::TriggerId::export_all(&cfg).expect("TriggerId");
        crate::ProjectId::export_all(&cfg).expect("ProjectId");
        crate::DocumentPath::export_all(&cfg).expect("DocumentPath");
        crate::Rgb::export_all(&cfg).expect("Rgb");
        crate::RepoName::export_all(&cfg).expect("RepoName");
        crate::ActorId::export_all(&cfg).expect("ActorId");
        crate::ActorRef::export_all(&cfg).expect("ActorRef");
        crate::ExternalSystem::export_all(&cfg).expect("ExternalSystem");
        crate::principal::Principal::export_all(&cfg).expect("Principal");
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
        crate::agents::AgentName::export_all(&cfg).expect("AgentName");
        crate::agents::AgentRecord::export_all(&cfg).expect("AgentRecord");
        crate::agents::UpsertAgentRequest::export_all(&cfg).expect("UpsertAgentRequest");
        crate::agents::AgentResponse::export_all(&cfg).expect("AgentResponse");
        crate::agents::DeleteAgentResponse::export_all(&cfg).expect("DeleteAgentResponse");
        crate::agents::ListAgentsResponse::export_all(&cfg).expect("ListAgentsResponse");

        // API v1: analytics
        crate::analytics::BucketGranularity::export_all(&cfg).expect("BucketGranularity");
        crate::analytics::PatchesThroughputQuery::export_all(&cfg).expect("PatchesThroughputQuery");
        crate::analytics::PatchOverTimeBucket::export_all(&cfg).expect("PatchOverTimeBucket");
        crate::analytics::PatchesOverTimeResponse::export_all(&cfg)
            .expect("PatchesOverTimeResponse");
        crate::analytics::PatchesTerminalMixResponse::export_all(&cfg)
            .expect("PatchesTerminalMixResponse");
        crate::analytics::TimeToMergeBin::export_all(&cfg).expect("TimeToMergeBin");
        crate::analytics::PatchesTimeToMergeResponse::export_all(&cfg)
            .expect("PatchesTimeToMergeResponse");
        crate::analytics::PatchInFlightBucket::export_all(&cfg).expect("PatchInFlightBucket");
        crate::analytics::PatchesInFlightOverTimeResponse::export_all(&cfg)
            .expect("PatchesInFlightOverTimeResponse");
        crate::analytics::IssuesThroughputQuery::export_all(&cfg).expect("IssuesThroughputQuery");
        crate::analytics::IssuesCycleTimeResponse::export_all(&cfg)
            .expect("IssuesCycleTimeResponse");
        crate::analytics::TimeInStatusSegment::export_all(&cfg).expect("TimeInStatusSegment");
        crate::analytics::IssuesTimeInStatusBreakdownResponse::export_all(&cfg)
            .expect("IssuesTimeInStatusBreakdownResponse");
        crate::analytics::PerStatusDistribution::export_all(&cfg).expect("PerStatusDistribution");
        crate::analytics::IssuesPerStatusDistributionResponse::export_all(&cfg)
            .expect("IssuesPerStatusDistributionResponse");
        crate::analytics::IssueOverTimeBucket::export_all(&cfg).expect("IssueOverTimeBucket");
        crate::analytics::IssuesOverTimeResponse::export_all(&cfg).expect("IssuesOverTimeResponse");
        crate::analytics::TokenUsageOverTimeQuery::export_all(&cfg)
            .expect("TokenUsageOverTimeQuery");
        crate::analytics::TokenUsageQuery::export_all(&cfg).expect("TokenUsageQuery");
        crate::analytics::TokenUsageOverTimeBucket::export_all(&cfg)
            .expect("TokenUsageOverTimeBucket");
        crate::analytics::TokenUsageOverTimeResponse::export_all(&cfg)
            .expect("TokenUsageOverTimeResponse");
        crate::analytics::AgentSessionCost::export_all(&cfg).expect("AgentSessionCost");
        crate::analytics::AgentCost::export_all(&cfg).expect("AgentCost");
        crate::analytics::TokenUsageCostPerAgentResponse::export_all(&cfg)
            .expect("TokenUsageCostPerAgentResponse");
        crate::analytics::IssueCost::export_all(&cfg).expect("IssueCost");
        crate::analytics::TokenUsageTopIssuesByCostResponse::export_all(&cfg)
            .expect("TokenUsageTopIssuesByCostResponse");

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
        crate::documents::ListDocumentPathsQuery::export_all(&cfg).expect("ListDocumentPathsQuery");
        crate::documents::ListDocumentPathsResponse::export_all(&cfg)
            .expect("ListDocumentPathsResponse");
        crate::documents::PathChildEntry::export_all(&cfg).expect("PathChildEntry");
        crate::documents::PathChildDocumentRef::export_all(&cfg).expect("PathChildDocumentRef");

        // API v1: events
        crate::events::EventsQuery::export_all(&cfg).expect("EventsQuery");
        crate::events::SseEventType::export_all(&cfg).expect("SseEventType");
        crate::events::EntityEventData::export_all(&cfg).expect("EntityEventData");
        crate::events::ConnectedEventData::export_all(&cfg).expect("ConnectedEventData");
        crate::events::ResyncEventData::export_all(&cfg).expect("ResyncEventData");
        crate::events::HeartbeatEventData::export_all(&cfg).expect("HeartbeatEventData");
        crate::events::SessionLogEventData::export_all(&cfg).expect("SessionLogEventData");

        // API v1: projects
        crate::projects::ProjectKey::export_all(&cfg).expect("ProjectKey");
        crate::projects::StatusKey::export_all(&cfg).expect("StatusKey");
        crate::projects::StatusOnEnter::export_all(&cfg).expect("StatusOnEnter");
        crate::projects::StatusDefinition::export_all(&cfg).expect("StatusDefinition");
        crate::projects::Project::export_all(&cfg).expect("Project");
        crate::projects::UpsertProjectRequest::export_all(&cfg).expect("UpsertProjectRequest");
        crate::projects::UpsertProjectResponse::export_all(&cfg).expect("UpsertProjectResponse");
        crate::projects::ProjectRecord::export_all(&cfg).expect("ProjectRecord");
        crate::projects::ListProjectsResponse::export_all(&cfg).expect("ListProjectsResponse");
        crate::projects::ProjectStatusesResponse::export_all(&cfg)
            .expect("ProjectStatusesResponse");
        crate::projects::UpsertProjectStatusResponse::export_all(&cfg)
            .expect("UpsertProjectStatusResponse");

        // API v1: labels
        crate::labels::Label::export_all(&cfg).expect("Label");
        crate::labels::LabelSummary::export_all(&cfg).expect("LabelSummary");
        crate::labels::LabelRecord::export_all(&cfg).expect("LabelRecord");
        crate::labels::UpsertLabelRequest::export_all(&cfg).expect("UpsertLabelRequest");
        crate::labels::UpsertLabelResponse::export_all(&cfg).expect("UpsertLabelResponse");
        crate::labels::SearchLabelsQuery::export_all(&cfg).expect("SearchLabelsQuery");
        crate::labels::ListLabelsResponse::export_all(&cfg).expect("ListLabelsResponse");

        // API v1: issues
        crate::issues::IssueType::export_all(&cfg).expect("IssueType");
        crate::issues::IssueDependencyType::export_all(&cfg).expect("IssueDependencyType");
        crate::issues::IssueDependency::export_all(&cfg).expect("IssueDependency");
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

        // API v1: comments
        crate::comments::Comment::export_all(&cfg).expect("Comment");
        crate::comments::AddCommentRequest::export_all(&cfg).expect("AddCommentRequest");
        crate::comments::AddCommentResponse::export_all(&cfg).expect("AddCommentResponse");
        crate::comments::ListCommentsQuery::export_all(&cfg).expect("ListCommentsQuery");
        crate::comments::ListCommentsResponse::export_all(&cfg).expect("ListCommentsResponse");

        // API v1: triggers
        crate::triggers::Trigger::export_all(&cfg).expect("Trigger");
        crate::triggers::Schedule::export_all(&cfg).expect("TriggerSchedule");
        crate::triggers::Action::export_all(&cfg).expect("TriggerAction");
        crate::triggers::CreateIssueAction::export_all(&cfg).expect("CreateIssueAction");
        crate::triggers::UpsertTriggerRequest::export_all(&cfg).expect("UpsertTriggerRequest");
        crate::triggers::UpsertTriggerResponse::export_all(&cfg).expect("UpsertTriggerResponse");
        crate::triggers::TriggerVersionRecord::export_all(&cfg).expect("TriggerVersionRecord");
        crate::triggers::ListTriggersResponse::export_all(&cfg).expect("ListTriggersResponse");
        crate::triggers::ListTriggerVersionsResponse::export_all(&cfg)
            .expect("ListTriggerVersionsResponse");
        crate::triggers::SearchTriggersQuery::export_all(&cfg).expect("SearchTriggersQuery");

        // API v1: form
        crate::form::Form::export_all(&cfg).expect("Form");
        crate::form::Field::export_all(&cfg).expect("Field");
        crate::form::Input::export_all(&cfg).expect("Input");
        crate::form::SelectOption::export_all(&cfg).expect("SelectOption");
        crate::form::ActionStyle::export_all(&cfg).expect("ActionStyle");
        crate::form::Action::export_all(&cfg).expect("Action");
        crate::form::Effect::export_all(&cfg).expect("Effect");
        crate::form::FormResponse::export_all(&cfg).expect("FormResponse");

        // API v1: session_status
        crate::session_status::SessionStatusUpdate::export_all(&cfg).expect("SessionStatusUpdate");
        crate::session_status::SetSessionStatusResponse::export_all(&cfg)
            .expect("SetSessionStatusResponse");

        // API v1: sessions
        crate::sessions::Session::export_all(&cfg).expect("Session");
        crate::sessions::TokenUsage::export_all(&cfg).expect("TokenUsage");
        crate::sessions::CreateSessionRequest::export_all(&cfg).expect("CreateSessionRequest");
        crate::sessions::Bundle::export_all(&cfg).expect("Bundle");
        crate::sessions::RelativePath::export_all(&cfg).expect("RelativePath");
        crate::sessions::MountSpec::export_all(&cfg).expect("MountSpec");
        crate::sessions::MountItem::export_all(&cfg).expect("MountItem");
        crate::sessions::SessionModeKind::export_all(&cfg).expect("SessionModeKind");
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

        // API v1: merge_check
        crate::merge_check::MergeBlockedError::export_all(&cfg).expect("MergeBlockedError");
        crate::merge_check::MergeBlockedCode::export_all(&cfg).expect("MergeBlockedCode");
        crate::merge_check::BlockedAtLayer::export_all(&cfg).expect("BlockedAtLayer");
        crate::merge_check::MergeBlockedReason::export_all(&cfg).expect("MergeBlockedReason");
        crate::merge_check::EligiblePrincipal::export_all(&cfg).expect("EligiblePrincipal");
        crate::merge_check::SuggestedAction::export_all(&cfg).expect("SuggestedAction");

        // API v1: merge_queues
        crate::merge_queues::MergeQueue::export_all(&cfg).expect("MergeQueue");
        crate::merge_queues::EnqueueMergePatchRequest::export_all(&cfg)
            .expect("EnqueueMergePatchRequest");

        // API v1: patches
        crate::patches::PatchStatus::export_all(&cfg).expect("PatchStatus");
        crate::patches::Review::export_all(&cfg).expect("Review");
        crate::patches::UpsertReviewRequest::export_all(&cfg).expect("UpsertReviewRequest");
        crate::patches::GithubPr::export_all(&cfg).expect("GithubPr");
        crate::patches::GitOid::export_all(&cfg).expect("GitOid");
        crate::patches::CommitRange::export_all(&cfg).expect("CommitRange");
        crate::patches::Patch::export_all(&cfg).expect("Patch");
        crate::patches::UpsertPatch::export_all(&cfg).expect("UpsertPatch");
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
        crate::repositories::DynamicRef::export_all(&cfg).expect("DynamicRef");
        crate::repositories::AssigneeRef::export_all(&cfg).expect("AssigneeRef");
        crate::repositories::ReviewerGroup::export_all(&cfg).expect("ReviewerGroup");
        crate::repositories::MergerRule::export_all(&cfg).expect("MergerRule");
        crate::repositories::MergePolicy::export_all(&cfg).expect("MergePolicy");
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
        crate::users::ListUsersResponse::export_all(&cfg).expect("ListUsersResponse");

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

        // API v1: conversations
        crate::conversations::Conversation::export_all(&cfg).expect("Conversation");
        crate::conversations::ConversationSummary::export_all(&cfg).expect("ConversationSummary");
        crate::conversations::ConversationStatus::export_all(&cfg).expect("ConversationStatus");
        crate::conversations::CreateConversationRequest::export_all(&cfg)
            .expect("CreateConversationRequest");
        crate::conversations::ListConversationsResponse::export_all(&cfg)
            .expect("ListConversationsResponse");
        crate::conversations::SendMessageRequest::export_all(&cfg).expect("SendMessageRequest");
        crate::conversations::SearchConversationsQuery::export_all(&cfg)
            .expect("SearchConversationsQuery");

        // API v1: relay
        crate::relay::WorkerMessage::export_all(&cfg).expect("WorkerMessage");
        crate::relay::ServerMessage::export_all(&cfg).expect("ServerMessage");
        crate::relay::SessionStatePayload::export_all(&cfg).expect("SessionStatePayload");
        crate::relay::CatchUpEvent::export_all(&cfg).expect("CatchUpEvent");
    }
}
