//! `AppState::resolve_status` — single resolution point from
//! `(project_id, status)` to a [`StatusDefinition`].
//!
//! Per-issue lookups go through the project store when `Issue.project_id`
//! is set, falling back to the synthesized `default_project` when it's
//! `None`. Centralizing here keeps storage, validation, and `on_enter`
//! automation aligned on one resolver instead of duplicating the
//! `project_id → statuses` walk at each call site.

use crate::domain::issues::Issue;
use crate::domain::projects::default_project;
use crate::store::{ReadOnlyStore, StoreError};
use hydra_common::ProjectId;
use hydra_common::api::v1::projects::{KeyError, StatusDefinition, StatusKey};
use thiserror::Error;

use super::AppState;

/// Failure modes for [`AppState::resolve_status`].
#[derive(Debug, Error)]
pub enum ResolveStatusError {
    /// The issue's status string is not a well-formed [`StatusKey`].
    #[error("invalid status key: {0}")]
    InvalidKey(KeyError),
    /// The status key does not match any declared status in the resolved
    /// project. Validation upstream should have prevented this.
    #[error("status '{0}' is not declared in the resolved project")]
    UnknownStatus(StatusKey),
    /// The referenced project does not exist or has been deleted.
    #[error("project '{0}' not found")]
    ProjectNotFound(ProjectId),
    /// Underlying store failure when reading the referenced project.
    #[error("project store error: {0}")]
    Store(#[from] StoreError),
}

impl AppState {
    /// Resolve an issue's `(project_id, status)` pair to a
    /// [`StatusDefinition`].
    ///
    /// When `issue.project_id` is `None`, resolution falls back to the
    /// synthesized [`default_project`] (no DB read). Otherwise the
    /// resolver fetches the project via the [`crate::store::Store`] and
    /// looks up the status by key. The result is the same
    /// [`StatusDefinition`] embedded inline as `Issue.resolved_status`
    /// on every API response.
    pub async fn resolve_status(
        &self,
        issue: &Issue,
    ) -> Result<StatusDefinition, ResolveStatusError> {
        let key =
            StatusKey::try_new(issue.status.as_str()).map_err(ResolveStatusError::InvalidKey)?;
        match &issue.project_id {
            None => default_project()
                .find_status(&key)
                .cloned()
                .ok_or(ResolveStatusError::UnknownStatus(key)),
            Some(project_id) => {
                let store: &dyn ReadOnlyStore = self.store.as_ref();
                let project = match store.get_project(project_id, false).await {
                    Ok(versioned) => versioned.item,
                    Err(StoreError::ProjectNotFound(_)) => {
                        return Err(ResolveStatusError::ProjectNotFound(project_id.clone()));
                    }
                    Err(err) => return Err(ResolveStatusError::Store(err)),
                };
                project
                    .find_status(&key)
                    .cloned()
                    .ok_or(ResolveStatusError::UnknownStatus(key))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::issue_with_status;
    use crate::domain::issues::IssueStatus;
    use crate::test_utils::test_state;

    #[tokio::test]
    async fn resolve_status_open_returns_open_definition() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Open, vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "open");
        assert!(!def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(!def.cascades_to_children);
        assert!(def.on_enter.is_none());
    }

    #[tokio::test]
    async fn resolve_status_in_progress_returns_in_progress_definition() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::InProgress, vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "in-progress");
        assert!(!def.unblocks_parents);
    }

    #[tokio::test]
    async fn resolve_status_closed_unblocks_parents_and_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Closed, vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "closed");
        assert!(def.unblocks_parents);
        assert!(def.unblocks_dependents);
        assert!(!def.cascades_to_children);
    }

    #[tokio::test]
    async fn resolve_status_dropped_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Dropped, vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "dropped");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }

    #[tokio::test]
    async fn resolve_status_failed_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", IssueStatus::Failed, vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "failed");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }
}
