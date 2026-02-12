use async_trait::async_trait;

use crate::domain::issues::{IssueDependencyType, IssueStatus};
use crate::policy::context::{OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};
use metis_common::issues::IssueId;

/// Validates issue lifecycle constraints when closing:
/// - All blockers must be in a terminal state
/// - All todo items must be done
/// - All children must be closed
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
        "issue_lifecycle_validation"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        let OperationPayload::Issue { issue_id, new, .. } = ctx.payload else {
            return Ok(());
        };

        // Only validate when closing
        if new.status != IssueStatus::Closed {
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
                    policy_name: self.name().to_string(),
                    message: format!("Failed to look up blocker {}: {e}", dependency.issue_id),
                })?;

            if !matches!(
                blocker.item.status,
                IssueStatus::Closed
                    | IssueStatus::Dropped
                    | IssueStatus::Rejected
                    | IssueStatus::Failed
            ) {
                open_blockers.push(dependency.issue_id.clone());
            }
        }

        // Check todos
        let mut incomplete_todos: Vec<usize> = Vec::new();
        for (index, item) in new.todo_list.iter().enumerate() {
            if !item.is_done {
                incomplete_todos.push(index + 1);
            }
        }

        if !incomplete_todos.is_empty() {
            let numbers = incomplete_todos
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!("cannot close issue with incomplete todo items: {numbers}"),
            });
        }

        // Check children (only if we have an issue_id, i.e., this is an update)
        if let Some(issue_id) = issue_id {
            let children =
                ctx.store
                    .get_issue_children(issue_id)
                    .await
                    .map_err(|e| PolicyViolation {
                        policy_name: self.name().to_string(),
                        message: format!("Failed to look up children of {issue_id}: {e}"),
                    })?;

            let mut open_children = Vec::new();
            for child_id in children {
                let child =
                    ctx.store
                        .get_issue(&child_id, false)
                        .await
                        .map_err(|e| PolicyViolation {
                            policy_name: self.name().to_string(),
                            message: format!("Failed to look up child issue {child_id}: {e}"),
                        })?;
                if child.item.status != IssueStatus::Closed {
                    open_children.push(child_id);
                }
            }

            if !open_children.is_empty() {
                let ids = join_issue_ids(&open_children);
                return Err(PolicyViolation {
                    policy_name: self.name().to_string(),
                    message: format!("cannot close issue with open child issues: {ids}"),
                });
            }
        }

        // Check blockers (reported last as per original order)
        if !open_blockers.is_empty() {
            let ids = join_issue_ids(&open_blockers);
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!("blocked issues cannot close until blockers are closed: {ids}"),
            });
        }

        Ok(())
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
    use crate::domain::actors::UserOrWorker;
    use crate::domain::issues::{Issue, IssueDependency, IssueType, TodoItem};
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, Store};

    fn test_actor() -> UserOrWorker {
        UserOrWorker::Username(Username::from("test-user"))
    }

    fn make_issue(status: IssueStatus) -> Issue {
        Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("creator"),
            String::new(),
            status,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn allows_non_closing_status() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let issue = make_issue(IssueStatus::Open);
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_closing_with_no_deps_or_todos() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let issue = make_issue(IssueStatus::Closed);
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_closing_with_incomplete_todos() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();
        let mut issue = make_issue(IssueStatus::Closed);
        issue.todo_list = vec![
            TodoItem::new("done task".to_string(), true),
            TodoItem::new("not done".to_string(), false),
        ];
        let payload = OperationPayload::Issue {
            issue_id: None,
            new: issue,
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::UpdateIssue,
            actor: &actor,
            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "issue_lifecycle_validation");
        assert!(
            violation
                .message
                .contains("cannot close issue with incomplete todo items")
        );
    }

    #[tokio::test]
    async fn rejects_closing_with_open_children() {
        let restriction = IssueLifecycleRestriction::new();
        let store = MemoryStore::new();
        let actor = test_actor();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open);
        let (parent_id, _) = store.add_issue(parent).await.unwrap();

        let mut child = make_issue(IssueStatus::Open);
        child.dependencies = vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )];
        store.add_issue(child).await.unwrap();

        // Try to close parent
        let mut closing_parent = make_issue(IssueStatus::Closed);
        closing_parent.creator = Username::from("creator");
        let payload = OperationPayload::Issue {
            issue_id: Some(parent_id.clone()),
            new: closing_parent,
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::UpdateIssue,
            actor: &actor,
            repo: None,
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
        let actor = test_actor();

        // Create blocker that is still open
        let blocker = make_issue(IssueStatus::Open);
        let (blocker_id, _) = store.add_issue(blocker).await.unwrap();

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
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            actor: &actor,
            repo: None,
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
