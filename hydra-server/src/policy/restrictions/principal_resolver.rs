//! Resolves [`Principal`]s in a [`MergePolicy`] to concrete usernames at
//! merge-attempt time.
//!
//! `Principal::User(u)` is the identity case. `Principal::Dynamic(d)` is
//! resolved against the current state of the patch and (when relevant) its
//! parent issue — never snapshotted. See
//! `/designs/merge-time-constraints.md` §4.4 for why resolution is live.
//!
//! The parent-issue lookup is duplicated verbatim from
//! [`crate::policy::automations::patch_workflow`] (see the issue spec
//! "Parent-issue lookup helper") to keep the legacy automation's tests
//! unchanged through Phase 2. Phase 3 deletes that automation and this copy
//! becomes the only one.

use hydra_common::api::v1::repositories::{DynamicRef, Principal};

use crate::domain::issues::{Issue, IssueType};
use crate::domain::patches::Patch;
use crate::store::ReadOnlyStore;

/// Inputs available to [`resolve_principal`] / [`resolve_any_of`].
///
/// The restriction pre-resolves the parent issue once per evaluation and
/// passes it through so each principal resolution does not re-hit the store.
pub struct ResolutionContext<'a> {
    pub patch: &'a Patch,
    pub patch_id: &'a hydra_common::PatchId,
    pub parent_issue: Option<&'a Issue>,
    pub store: &'a dyn ReadOnlyStore,
}

/// A principal alongside the username it currently resolves to (if any).
///
/// `resolved_to` is `None` for `Dynamic` refs that cannot be resolved against
/// the current state (e.g. `ParentIssueCreator` with no parent issue, or
/// `ParentIssueAssignee` when the parent issue is unassigned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrincipal {
    pub source: Principal,
    pub resolved_to: Option<String>,
}

/// Resolve a single [`Principal`] to a username string, or `None` if a
/// dynamic reference cannot be resolved.
pub fn resolve_principal(principal: &Principal, ctx: &ResolutionContext<'_>) -> Option<String> {
    match principal {
        Principal::User(username) => Some(username.as_str().to_string()),
        Principal::Dynamic(dref) => resolve_dynamic_ref(*dref, ctx),
    }
}

/// Resolve every principal in `any_of`, preserving order and the original
/// [`Principal`] alongside each result.
pub fn resolve_any_of(
    principals: &[Principal],
    ctx: &ResolutionContext<'_>,
) -> Vec<ResolvedPrincipal> {
    principals
        .iter()
        .map(|p| ResolvedPrincipal {
            source: p.clone(),
            resolved_to: resolve_principal(p, ctx),
        })
        .collect()
}

fn resolve_dynamic_ref(dref: DynamicRef, ctx: &ResolutionContext<'_>) -> Option<String> {
    match dref {
        DynamicRef::PatchAuthor => Some(ctx.patch.creator.as_str().to_string()),
        DynamicRef::ParentIssueCreator => ctx
            .parent_issue
            .map(|issue| issue.creator.as_str().to_string()),
        DynamicRef::ParentIssueAssignee => ctx
            .parent_issue
            .and_then(|issue| issue.assignee.as_ref().map(|s| s.to_string())),
    }
}

/// Resolves the parent issue for a patch by tracing its lineage.
///
/// First tries: `patch.created_by` (SessionId) -> `session.spawned_from`
/// (IssueId). Fallback: finds a non-MergeRequest, non-ReviewRequest issue
/// that references this patch via `get_issues_for_patch`.
///
/// Returns `Ok(None)` if no parent issue can be found through either path —
/// dynamic-ref resolution will then return `None` for `parent_issue.*` refs
/// and the error payload will surface that to the caller.
///
/// This is a verbatim copy of `PatchWorkflowAutomation::resolve_parent_issue`
/// (`hydra-server/src/policy/automations/patch_workflow.rs`) adapted for
/// `&dyn ReadOnlyStore`. Behavioural drift between the two would be a bug;
/// Phase 3 deletes the automation copy.
pub async fn resolve_parent_issue(
    store: &dyn ReadOnlyStore,
    patch_id: &hydra_common::PatchId,
    patch: &Patch,
) -> Result<Option<Issue>, crate::store::StoreError> {
    // Try tracing via created_by -> task.spawned_from
    if let Some(task_id) = &patch.created_by {
        match store.get_session(task_id, false).await {
            Ok(task) => {
                if let Some(issue_id) = &task.item.spawned_from {
                    match store.get_issue(issue_id, false).await {
                        Ok(issue) => return Ok(Some(issue.item)),
                        Err(e) => {
                            tracing::warn!(
                                patch_id = %patch_id,
                                issue_id = %issue_id,
                                error = %e,
                                "merge_authorization: failed to fetch parent issue from \
                                 task.spawned_from"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    patch_id = %patch_id,
                    task_id = %task_id,
                    error = %e,
                    "merge_authorization: failed to fetch task for patch.created_by"
                );
            }
        }
    }

    // Fallback: find a non-MergeRequest, non-ReviewRequest issue that
    // references this patch. `PatchNotFound` is treated as "no parent" —
    // restrictions evaluate against a proposed mutation, and a brand-new
    // patch (or a patch deleted between events) has no parent issue to
    // resolve.
    let issue_ids = match store.get_issues_for_patch(patch_id).await {
        Ok(ids) => ids,
        Err(crate::store::StoreError::PatchNotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    for issue_id in issue_ids {
        let issue = store.get_issue(&issue_id, false).await?;
        if issue.item.issue_type != IssueType::MergeRequest
            && issue.item.issue_type != IssueType::ReviewRequest
        {
            return Ok(Some(issue.item));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{Issue, IssueStatus};
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::domain::users::Username;
    use crate::store::MemoryStore;
    use hydra_common::RepoName;
    use std::str::FromStr;

    fn make_patch(creator: &str) -> Patch {
        Patch {
            title: "t".to_string(),
            description: String::new(),
            diff: String::new(),
            status: PatchStatus::Open,
            is_automatic_backup: false,
            branch_name: None,
            service_repo_name: RepoName::from_str("test/repo").unwrap(),
            created_by: None,
            creator: Username::from(creator),
            github: None,
            commit_range: None,
            base_branch: None,
            reviews: Vec::new(),
            deleted: false,
        }
    }

    fn make_parent_issue(creator: &str, assignee: Option<&str>) -> Issue {
        Issue::new(
            IssueType::Task,
            "parent".to_string(),
            String::new(),
            Username::from(creator),
            String::new(),
            IssueStatus::Open,
            assignee.map(String::from),
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn ctx<'a>(
        patch: &'a Patch,
        patch_id: &'a hydra_common::PatchId,
        parent: Option<&'a Issue>,
        store: &'a dyn ReadOnlyStore,
    ) -> ResolutionContext<'a> {
        ResolutionContext {
            patch,
            patch_id,
            parent_issue: parent,
            store,
        }
    }

    #[tokio::test]
    async fn resolves_user_principal_to_its_username() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, None, &store);

        let p = Principal::User(hydra_common::api::v1::users::Username::from("alice"));
        assert_eq!(resolve_principal(&p, &c), Some("alice".to_string()));
    }

    #[tokio::test]
    async fn patch_author_dynamic_ref_resolves_to_patch_creator() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, None, &store);

        let p = Principal::Dynamic(DynamicRef::PatchAuthor);
        assert_eq!(resolve_principal(&p, &c), Some("author".to_string()));
    }

    #[tokio::test]
    async fn parent_issue_creator_resolves_when_parent_is_present() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let parent = make_parent_issue("jayantk", None);
        let c = ctx(&patch, &patch_id, Some(&parent), &store);

        let p = Principal::Dynamic(DynamicRef::ParentIssueCreator);
        assert_eq!(resolve_principal(&p, &c), Some("jayantk".to_string()));
    }

    #[tokio::test]
    async fn parent_issue_creator_resolves_to_none_without_parent() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let c = ctx(&patch, &patch_id, None, &store);

        let p = Principal::Dynamic(DynamicRef::ParentIssueCreator);
        assert_eq!(resolve_principal(&p, &c), None);
    }

    #[tokio::test]
    async fn parent_issue_assignee_resolves_when_assigned() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let parent = make_parent_issue("jayantk", Some("bob"));
        let c = ctx(&patch, &patch_id, Some(&parent), &store);

        let p = Principal::Dynamic(DynamicRef::ParentIssueAssignee);
        assert_eq!(resolve_principal(&p, &c), Some("bob".to_string()));
    }

    #[tokio::test]
    async fn parent_issue_assignee_resolves_to_none_when_unassigned() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let parent = make_parent_issue("jayantk", None);
        let c = ctx(&patch, &patch_id, Some(&parent), &store);

        let p = Principal::Dynamic(DynamicRef::ParentIssueAssignee);
        assert_eq!(resolve_principal(&p, &c), None);
    }

    #[tokio::test]
    async fn resolve_any_of_preserves_order_and_source() {
        let store = MemoryStore::new();
        let patch = make_patch("author");
        let patch_id = hydra_common::PatchId::new();
        let parent = make_parent_issue("jayantk", None);
        let c = ctx(&patch, &patch_id, Some(&parent), &store);

        let principals = vec![
            Principal::User(hydra_common::api::v1::users::Username::from("alice")),
            Principal::Dynamic(DynamicRef::ParentIssueCreator),
            Principal::Dynamic(DynamicRef::ParentIssueAssignee),
        ];

        let resolved = resolve_any_of(&principals, &c);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].source, principals[0]);
        assert_eq!(resolved[0].resolved_to, Some("alice".to_string()));
        assert_eq!(resolved[1].source, principals[1]);
        assert_eq!(resolved[1].resolved_to, Some("jayantk".to_string()));
        assert_eq!(resolved[2].source, principals[2]);
        assert_eq!(resolved[2].resolved_to, None);
    }
}
