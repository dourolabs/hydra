//! `AppState::resolve_status` — single resolution point from
//! `(project_id, status)` to a [`StatusDefinition`]. See
//! `/designs/per-project-issue-statuses.md` §4 for the rationale.
//!
//! PR 1/6 introduces the resolver against [`default_project`] only —
//! `Issue.project_id` does not exist yet, so every issue resolves
//! against the synthesized default project. PR 2 adds project storage;
//! PR 3 adds the `project_id` field and starts routing per-issue lookups
//! through a project read.

use crate::domain::issues::Issue;
use crate::domain::projects::default_project;
use hydra_common::api::v1::projects::{KeyError, StatusDefinition, StatusKey};
use thiserror::Error;

use super::AppState;

/// Failure modes for [`AppState::resolve_status`].
#[derive(Debug, Error)]
pub enum ResolveStatusError {
    /// The issue's status string is not a well-formed [`StatusKey`].
    /// Should be impossible while `Issue.status` is the closed
    /// `IssueStatus` enum (PR 1–3); becomes the validation gate for
    /// arbitrary user-supplied status strings in PR 3.
    #[error("invalid status key: {0}")]
    InvalidKey(KeyError),
    /// The status key does not match any declared status in the resolved
    /// project. Validation upstream should have prevented this.
    #[error("status '{0}' is not declared in the resolved project")]
    UnknownStatus(StatusKey),
}

impl AppState {
    /// Resolve an issue's `(project_id, status)` pair to a
    /// [`StatusDefinition`]. PR 1 always resolves against
    /// [`default_project`] because `Issue.project_id` doesn't exist yet.
    /// PR 3 swaps in the per-project lookup.
    pub fn resolve_status(&self, issue: &Issue) -> Result<StatusDefinition, ResolveStatusError> {
        let key =
            StatusKey::try_new(issue.status.as_str()).map_err(ResolveStatusError::InvalidKey)?;
        default_project()
            .find_status(&key)
            .cloned()
            .ok_or(ResolveStatusError::UnknownStatus(key))
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::issue_with_status;
    use crate::domain::issues::IssueStatus;
    use crate::test_utils::test_state;

    #[test]
    fn resolve_status_open_returns_open_definition() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Open, vec![]);
        let def = state.resolve_status(&issue).unwrap();
        assert_eq!(def.key.as_str(), "open");
        assert!(!def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(!def.cascades_to_children);
        assert!(def.on_enter.is_none());
    }

    #[test]
    fn resolve_status_in_progress_returns_in_progress_definition() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::InProgress, vec![]);
        let def = state.resolve_status(&issue).unwrap();
        assert_eq!(def.key.as_str(), "in-progress");
        assert!(!def.unblocks_parents);
    }

    #[test]
    fn resolve_status_closed_unblocks_parents_and_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Closed, vec![]);
        let def = state.resolve_status(&issue).unwrap();
        assert_eq!(def.key.as_str(), "closed");
        assert!(def.unblocks_parents);
        assert!(def.unblocks_dependents);
        assert!(!def.cascades_to_children);
    }

    #[test]
    fn resolve_status_dropped_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Dropped, vec![]);
        let def = state.resolve_status(&issue).unwrap();
        assert_eq!(def.key.as_str(), "dropped");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }

    #[test]
    fn resolve_status_failed_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Failed, vec![]);
        let def = state.resolve_status(&issue).unwrap();
        assert_eq!(def.key.as_str(), "failed");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }
}
