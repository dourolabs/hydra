use async_trait::async_trait;

use crate::app::projects::resolve_status_via_store;
use crate::domain::issues::{Issue, IssueDependencyType};
use crate::policy::context::{OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};
use crate::store::ReadOnlyStore;
use hydra_common::issues::IssueId;

const RESTRICTION_NAME: &str = "issue_lifecycle_validation";

/// Validates issue lifecycle constraints when closing:
/// - All blockers must be in a terminal state
/// - All children must be in a terminal state
#[derive(Default)]
pub struct IssueLifecycleRestriction;

impl IssueLifecycleRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for IssueLifecycleRestriction {
    fn name(&self) -> &str {
        RESTRICTION_NAME
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        let OperationPayload::Issue { issue_id, new, .. } = ctx.payload else {
            return Ok(());
        };

        // Only validate when the proposed status unblocks dependents — i.e.
        // the success-close lane. Dropped/Failed leave `unblocks_dependents`
        // false, so this restriction lets those through (matching the
        // legacy `IssueStatus::Closed`-only gate).
        let resolved_new = match resolve_status_via_store(ctx.store, new).await {
            Ok(def) => def,
            Err(err) => {
                tracing::warn!(
                    restriction = RESTRICTION_NAME,
                    status = %new.status,
                    error = %err,
                    "issue_lifecycle_validation: failed to resolve new status; skipping validation"
                );
                return Ok(());
            }
        };
        if !resolved_new.unblocks_dependents {
            return Ok(());
        }

        // Check blockers
        let mut open_blockers = Vec::new();
        for dependency in new
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == IssueDependencyType::BlockedOn)
        {
            let blocker = ctx
                .store
                .get_issue(&dependency.issue_id, false)
                .await
                .map_err(|e| PolicyViolation {
                    policy_name: RESTRICTION_NAME.to_string(),
                    message: format!("Failed to look up blocker {}: {e}", dependency.issue_id),
                })?;

            if !is_terminal(ctx.store, &blocker.item, "blocker", &dependency.issue_id).await {
                open_blockers.push(dependency.issue_id.clone());
            }
        }

        // Check children (only if we have an issue_id, i.e., this is an update)
        if let Some(issue_id) = issue_id {
            let children =
                ctx.store
                    .get_issue_children(issue_id)
                    .await
                    .map_err(|e| PolicyViolation {
                        policy_name: RESTRICTION_NAME.to_string(),
                        message: format!("Failed to look up children of {issue_id}: {e}"),
                    })?;

            let mut open_children = Vec::new();
            for child_id in children {
                let child =
                    ctx.store
                        .get_issue(&child_id, false)
                        .await
                        .map_err(|e| PolicyViolation {
                            policy_name: RESTRICTION_NAME.to_string(),
                            message: format!("Failed to look up child issue {child_id}: {e}"),
                        })?;
                if !is_terminal(ctx.store, &child.item, "child", &child_id).await {
                    open_children.push(child_id);
                }
            }

            if !open_children.is_empty() {
                let ids = join_issue_ids(&open_children);
                return Err(PolicyViolation {
                    policy_name: RESTRICTION_NAME.to_string(),
                    message: format!("cannot close issue with open child issues: {ids}"),
                });
            }
        }

        // Check blockers (reported last as per original order)
        if !open_blockers.is_empty() {
            let ids = join_issue_ids(&open_blockers);
            return Err(PolicyViolation {
                policy_name: RESTRICTION_NAME.to_string(),
                message: format!("blocked issues cannot close until blockers are closed: {ids}"),
            });
        }

        Ok(())
    }
}

/// Resolve the status of a related issue (blocker or child) and report
/// whether it counts as terminal for lifecycle gating. Unresolvable
/// statuses are conservatively treated as non-terminal (still blocking),
/// with a warn so operators can spot misconfigurations.
async fn is_terminal(
    store: &dyn ReadOnlyStore,
    issue: &Issue,
    kind: &'static str,
    issue_id: &IssueId,
) -> bool {
    match resolve_status_via_store(store, issue).await {
        Ok(def) => def.unblocks_parents,
        Err(err) => {
            tracing::warn!(
                restriction = RESTRICTION_NAME,
                kind,
                issue_id = %issue_id,
                status = %issue.status,
                error = %err,
                "issue_lifecycle_validation: failed to resolve related issue status; treating as still-blocking"
            );
            false
        }
    }
}

fn join_issue_ids(ids: &[IssueId]) -> String {
    let mut values: Vec<String> = ids.iter().map(ToString::to_string).collect();
    values.sort();
    values.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueDependency, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, Store};

    fn make_issue(status: IssueStatus) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            status.into(),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn allows_non_closing_status() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();

        let issue = make_issue(IssueStatus::Open);
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_closing_with_no_deps() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();

        let issue = make_issue(IssueStatus::Closed);
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_closing_with_open_children() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open);
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mut child = make_issue(IssueStatus::Open);
        child.dependencies = vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )];
        store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Try to close parent
        let mut closing_parent = make_issue(IssueStatus::Closed);
        closing_parent.creator = Username::from("creator");
        let payload = OperationPayload::Issue {
            issue_id: Some(parent_id.clone()),
            new: closing_parent,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::UpdateIssue,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(
            violation
                .message
                .contains("cannot close issue with open child issues")
        );
    }

    #[tokio::test]
    async fn allows_closing_with_terminal_children() {
        let restriction = IssueLifecycleRestriction::new();

        for status in [
            IssueStatus::Closed,
            IssueStatus::Failed,
            IssueStatus::Dropped,
        ] {
            let store = MemoryStore::new();

            let parent = make_issue(IssueStatus::Open);
            let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

            let mut child = make_issue(status);
            child.dependencies = vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )];
            store.add_issue(child, &ActorRef::test()).await.unwrap();

            let closing_parent = make_issue(IssueStatus::Closed);
            let payload = OperationPayload::Issue {
                issue_id: Some(parent_id.clone()),
                new: closing_parent,
                old: None,
            };
            let actor = ActorRef::test();
            let ctx = RestrictionContext {
                operation: Operation::UpdateIssue,
                actor: &actor,
                payload: &payload,
                store: &store,
            };
            assert!(
                restriction.evaluate(&ctx).await.is_ok(),
                "closing parent should succeed when child is {status:?}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_closing_with_in_progress_children() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();

        let parent = make_issue(IssueStatus::Open);
        let (parent_id, _) = store.add_issue(parent, &ActorRef::test()).await.unwrap();

        let mut child = make_issue(IssueStatus::InProgress);
        child.dependencies = vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )];
        store.add_issue(child, &ActorRef::test()).await.unwrap();

        let closing_parent = make_issue(IssueStatus::Closed);
        let payload = OperationPayload::Issue {
            issue_id: Some(parent_id.clone()),
            new: closing_parent,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::UpdateIssue,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(
            violation
                .message
                .contains("cannot close issue with open child issues")
        );
    }

    #[tokio::test]
    async fn rejects_closing_with_open_blockers() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();

        // Create blocker that is still open
        let blocker = make_issue(IssueStatus::Open);
        let (blocker_id, _) = store.add_issue(blocker, &ActorRef::test()).await.unwrap();

        let mut issue = make_issue(IssueStatus::Closed);
        issue.dependencies = vec![IssueDependency::new(
            IssueDependencyType::BlockedOn,
            blocker_id.clone(),
        )];
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert!(
            violation
                .message
                .contains("blocked issues cannot close until blockers are closed")
        );
    }
}
