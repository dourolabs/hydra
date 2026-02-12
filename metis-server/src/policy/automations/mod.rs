pub mod cascade_issue_status;
pub mod close_merge_request_issues;
pub mod create_merge_request_issue;
pub mod inherit_creator;
pub mod kill_tasks_on_failure;

pub use cascade_issue_status::CascadeIssueStatusAutomation;
pub use close_merge_request_issues::CloseMergeRequestIssuesAutomation;
pub use create_merge_request_issue::CreateMergeRequestIssueAutomation;
pub use inherit_creator::InheritCreatorAutomation;
pub use kill_tasks_on_failure::KillTasksOnFailureAutomation;

use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::domain::issues::{Issue, IssueStatus, IssueType};
use crate::domain::users::Username;
use std::mem;
use std::sync::Arc;

/// Helper to create a discriminant for `IssueCreated` events.
pub(crate) fn issue_created_discriminant() -> mem::Discriminant<ServerEvent> {
    let sentinel = ServerEvent::IssueCreated {
        seq: 0,
        issue_id: metis_common::IssueId::new(),
        version: 0,
        timestamp: chrono::Utc::now(),
        payload: dummy_issue_payload(),
    };
    mem::discriminant(&sentinel)
}

/// Helper to create a discriminant for `IssueUpdated` events.
pub(crate) fn issue_updated_discriminant() -> mem::Discriminant<ServerEvent> {
    let sentinel = ServerEvent::IssueUpdated {
        seq: 0,
        issue_id: metis_common::IssueId::new(),
        version: 0,
        timestamp: chrono::Utc::now(),
        payload: dummy_issue_payload(),
    };
    mem::discriminant(&sentinel)
}

/// Helper to create a discriminant for `PatchUpdated` events.
pub(crate) fn patch_updated_discriminant() -> mem::Discriminant<ServerEvent> {
    let sentinel = ServerEvent::PatchUpdated {
        seq: 0,
        patch_id: metis_common::PatchId::new(),
        version: 0,
        timestamp: chrono::Utc::now(),
        payload: dummy_patch_payload(),
    };
    mem::discriminant(&sentinel)
}

fn dummy_issue_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Issue {
        old: None,
        new: Issue::new(
            IssueType::Task,
            String::new(),
            Username::from(""),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    })
}

fn dummy_patch_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Patch {
        old: None,
        new: crate::domain::patches::Patch::new(
            String::new(),
            String::new(),
            String::new(),
            crate::domain::patches::PatchStatus::Open,
            false,
            None,
            Vec::new(),
            metis_common::RepoName::new("x", "x").unwrap(),
            None,
        ),
    })
}
