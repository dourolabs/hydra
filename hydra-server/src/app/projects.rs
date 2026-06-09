//! `AppState::resolve_status` — single resolution point from
//! `(project_id, status)` to a [`StatusDefinition`].
//!
//! All lookups go through the project store: every issue is guaranteed
//! to carry a real `ProjectId` (legacy NULL rows were backfilled by the
//! `seed_default_project` migration and the column is NOT NULL since
//! `20260612000000_issues_v2_project_id_not_null`). The wire request
//! DTO [`hydra_common::api::v1::issues::IssueInput`] requires
//! `project_id` to be populated.

use crate::domain::actors::ActorRef;
use crate::domain::issues::Issue;
use crate::store::{ReadOnlyStore, StoreError};
use hydra_common::api::v1::projects::{KeyError, Project, StatusDefinition, StatusKey};
use hydra_common::{ProjectId, VersionNumber};
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
    pub async fn add_project(
        &self,
        project: Project,
        actor: &ActorRef,
    ) -> Result<(ProjectId, VersionNumber), StoreError> {
        self.store.add_project(project, actor).await
    }

    pub async fn update_project(
        &self,
        id: &ProjectId,
        project: Project,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.store.update_project(id, project, actor).await
    }

    pub async fn delete_project(
        &self,
        id: &ProjectId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.store.delete_project(id, actor).await
    }

    pub async fn rename_status(
        &self,
        id: &ProjectId,
        from: &StatusKey,
        to: &StatusKey,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.store.rename_status(id, from, to, actor).await
    }

    /// Resolve an issue's `(project_id, status)` pair to a
    /// [`StatusDefinition`].
    ///
    /// The resolver fetches the project via the [`crate::store::Store`]
    /// and looks up the status by key. `Issue.project_id` is
    /// non-optional end-to-end (see the module docstring), so a missing
    /// project is a hard `ResolveStatusError::ProjectNotFound` rather
    /// than a silent fallback.
    pub async fn resolve_status(
        &self,
        issue: &Issue,
    ) -> Result<StatusDefinition, ResolveStatusError> {
        let key =
            StatusKey::try_new(issue.status.as_str()).map_err(ResolveStatusError::InvalidKey)?;
        let project_id = issue.project_id.clone();
        let store: &dyn ReadOnlyStore = self.store.as_ref();
        let project = match store.get_project(&project_id, false).await {
            Ok(versioned) => versioned.item,
            Err(StoreError::ProjectNotFound(_)) => {
                return Err(ResolveStatusError::ProjectNotFound(project_id));
            }
            Err(err) => return Err(ResolveStatusError::Store(err)),
        };
        project
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
