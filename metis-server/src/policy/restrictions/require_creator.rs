use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Requires that issues have a non-empty creator field.
#[derive(Default)]
pub struct RequireCreatorRestriction;

impl RequireCreatorRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for RequireCreatorRestriction {
    fn name(&self) -> &str {
        "require_creator"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        let OperationPayload::Issue { new, .. } = ctx.payload else {
            return Ok(());
        };

        if !matches!(
            ctx.operation,
            Operation::CreateIssue | Operation::UpdateIssue
        ) {
            return Ok(());
        }

        if new.creator.as_ref().trim().is_empty() {
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: "Issue creator is required. Set the creator field when creating an issue."
                    .to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::MemoryStore;

    fn make_issue(creator: &str) -> Issue {
        Issue::new(
            IssueType::Task,
            "test issue".to_string(),
            Username::from(creator),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn allows_issue_with_creator() {
        let restriction = RequireCreatorRestriction::new();
        let store = MemoryStore::new();

        let payload = OperationPayload::Issue {
            issue_id: None,
            new: make_issue("jayantk"),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,

            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_empty_creator() {
        let restriction = RequireCreatorRestriction::new();
        let store = MemoryStore::new();

        let payload = OperationPayload::Issue {
            issue_id: None,
            new: make_issue(""),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,

            repo: None,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "require_creator");
        assert!(violation.message.contains("creator is required"));
    }

    #[tokio::test]
    async fn rejects_whitespace_only_creator() {
        let restriction = RequireCreatorRestriction::new();
        let store = MemoryStore::new();

        let payload = OperationPayload::Issue {
            issue_id: None,
            new: make_issue("   "),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,

            repo: None,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_err());
    }
}
