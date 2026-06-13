//! `AppState::resolve_status` — single resolution point from
//! `(project_id, status)` to a [`StatusDefinition`].
//!
//! All lookups go through the project store: every issue is guaranteed
//! to carry a real `ProjectId` (legacy NULL rows were backfilled by the
//! `seed_default_project` migration and the column is NOT NULL since
//! `20260612000000_issues_v2_project_id_not_null`). The wire request
//! DTO [`hydra_common::api::v1::issues::IssueInput`] requires
//! `project_id` to be populated.

use std::collections::HashMap;

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

    pub async fn add_status(
        &self,
        id: &ProjectId,
        status: StatusDefinition,
        actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        self.store.add_status(id, status, actor).await
    }

    pub async fn update_status(
        &self,
        id: &ProjectId,
        status_key: &StatusKey,
        status: StatusDefinition,
        actor: &ActorRef,
    ) -> Result<(StatusDefinition, VersionNumber), StoreError> {
        self.store
            .update_status(id, status_key, status, actor)
            .await
    }

    pub async fn delete_status(
        &self,
        id: &ProjectId,
        status_key: &StatusKey,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        self.store.delete_status(id, status_key, actor).await
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
        resolve_status_via_store(self.store.as_ref(), issue).await
    }
}

/// Free-function variant of [`AppState::resolve_status`] for callers that
/// hold a [`ReadOnlyStore`] but not a full [`AppState`] (e.g. restrictions
/// evaluated through [`crate::policy::context::RestrictionContext`]).
pub async fn resolve_status_via_store(
    store: &dyn ReadOnlyStore,
    issue: &Issue,
) -> Result<StatusDefinition, ResolveStatusError> {
    let mut cache = HashMap::new();
    resolve_status_with_cache(&mut cache, store, issue).await
}

/// Cached variant of [`resolve_status_via_store`]. Callers driving loops
/// over many issues that share a [`ProjectId`] can pass a single
/// `HashMap<ProjectId, Project>` across iterations to collapse N
/// `get_project` round-trips down to one per distinct project. Cache
/// lifetime should be scoped to the surrounding request — never global.
pub async fn resolve_status_with_cache(
    cache: &mut HashMap<ProjectId, Project>,
    store: &dyn ReadOnlyStore,
    issue: &Issue,
) -> Result<StatusDefinition, ResolveStatusError> {
    let key = StatusKey::try_new(issue.status.as_str()).map_err(ResolveStatusError::InvalidKey)?;
    let project = project_cached(cache, store, &issue.project_id).await?;
    project
        .find_status(&key)
        .cloned()
        .ok_or(ResolveStatusError::UnknownStatus(key))
}

/// Look up a [`Project`] through `cache`, fetching from `store` on miss.
/// Companion to [`resolve_status_with_cache`] for callers that also need
/// direct project access (e.g. checking whether a target status key is
/// declared) over the same cache.
///
/// `include_deleted=true` is intentional: a soft-deleted project's
/// `Project.statuses` is still authoritative for resolving the status
/// definitions of issues that still reference it. Filtering tombstoned
/// projects out here causes the issue-list / get-issue routes to 500
/// on every orphan row (see `routes/issue_response.rs`).
pub async fn project_cached<'a>(
    cache: &'a mut HashMap<ProjectId, Project>,
    store: &dyn ReadOnlyStore,
    project_id: &ProjectId,
) -> Result<&'a Project, ResolveStatusError> {
    if !cache.contains_key(project_id) {
        let project = match store.get_project(project_id, true).await {
            Ok(versioned) => versioned.item,
            Err(StoreError::ProjectNotFound(_)) => {
                return Err(ResolveStatusError::ProjectNotFound(project_id.clone()));
            }
            Err(err) => return Err(ResolveStatusError::Store(err)),
        };
        cache.insert(project_id.clone(), project);
    }
    Ok(cache.get(project_id).expect("just inserted"))
}

#[cfg(test)]
mod tests {
    use crate::app::test_helpers::issue_with_status;
    use crate::test_utils::test_state;
    use hydra_common::test_utils::status::status;

    #[tokio::test]
    async fn resolve_status_open_returns_open_definition() {
        let state = test_state();
        let issue = issue_with_status("test", status("open"), vec![]);
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
        let issue = issue_with_status("test", status("in-progress"), vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "in-progress");
        assert!(!def.unblocks_parents);
    }

    #[tokio::test]
    async fn resolve_status_closed_unblocks_parents_and_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", status("closed"), vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "closed");
        assert!(def.unblocks_parents);
        assert!(def.unblocks_dependents);
        assert!(!def.cascades_to_children);
    }

    #[tokio::test]
    async fn resolve_status_dropped_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", status("dropped"), vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "dropped");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }

    #[tokio::test]
    async fn resolve_status_failed_cascades_but_does_not_unblock_dependents() {
        let state = test_state();
        let issue = issue_with_status("test", status("failed"), vec![]);
        let def = state.resolve_status(&issue).await.unwrap();
        assert_eq!(def.key.as_str(), "failed");
        assert!(def.unblocks_parents);
        assert!(!def.unblocks_dependents);
        assert!(def.cascades_to_children);
    }
}
