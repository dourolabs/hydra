pub mod github_org_check;
pub mod github_pr_poller;
pub mod github_pr_sync;

pub use github_org_check::GithubOrgCheckRestriction;
pub use github_pr_poller::GithubPollerWorker;
pub use github_pr_sync::GithubPrSyncAutomation;

use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::domain::patches::PatchStatus;
use std::mem;
use std::sync::Arc;

/// Helper to create a discriminant for `PatchCreated` events.
pub(crate) fn patch_created_discriminant() -> mem::Discriminant<ServerEvent> {
    let sentinel = ServerEvent::PatchCreated {
        seq: 0,
        patch_id: metis_common::PatchId::new(),
        version: 0,
        timestamp: chrono::Utc::now(),
        payload: dummy_patch_payload(),
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

fn dummy_patch_payload() -> Arc<MutationPayload> {
    Arc::new(MutationPayload::Patch {
        old: None,
        new: crate::domain::patches::Patch::new(
            String::new(),
            String::new(),
            String::new(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            metis_common::RepoName::new("x", "x").unwrap(),
            None,
        ),
    })
}
