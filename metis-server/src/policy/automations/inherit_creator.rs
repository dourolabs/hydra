use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::issues::IssueDependencyType;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

/// When a new issue is created with an empty creator and has a `ChildOf`
/// dependency, copy the parent issue's creator to the child.
pub struct InheritCreatorAutomation;

impl InheritCreatorAutomation {
    pub fn new(_params: Option<&toml::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for InheritCreatorAutomation {
    fn name(&self) -> &str {
        "inherit_creator_from_parent"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueCreated],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let ServerEvent::IssueCreated {
            issue_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Issue { new, .. } = payload.as_ref() else {
            return Ok(());
        };

        if !new.creator.as_ref().trim().is_empty() {
            return Ok(());
        }

        let parent_dep = new
            .dependencies
            .iter()
            .find(|d| d.dependency_type == IssueDependencyType::ChildOf);

        let Some(parent_dep) = parent_dep else {
            return Ok(());
        };

        let parent_issue = ctx
            .store
            .get_issue(&parent_dep.issue_id, false)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to fetch parent issue {}: {e}",
                    parent_dep.issue_id
                ))
            })?;

        let mut issue = ctx
            .store
            .get_issue(issue_id, false)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch issue {issue_id}: {e}"))
            })?
            .item;

        issue.creator = parent_issue.item.creator;
        ctx.app_state
            .upsert_issue(
                Some(issue_id.clone()),
                metis_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
                Some(ctx.actor().to_string()),
            )
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to update creator on issue {issue_id}: {e}"
                ))
            })?;

        tracing::info!(
            issue_id = %issue_id,
            parent_id = %parent_dep.issue_id,
            "inherited creator from parent issue"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType,
    };
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use std::sync::Arc;

    #[tokio::test]
    async fn inherits_creator_from_parent() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = Issue::new(
            IssueType::Task,
            "parent".to_string(),
            Username::from("alice"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (parent_id, _) = store.add_issue(parent).await.unwrap();

        let child = Issue::new(
            IssueType::Task,
            "child".to_string(),
            Username::from(""),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
            Vec::new(),
        );
        let (child_id, _) = store.add_issue(child.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: child,
            actor: "test-actor".to_string(),
        });

        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: child_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InheritCreatorAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(updated.item.creator, Username::from("alice"));
    }

    #[tokio::test]
    async fn skips_when_creator_already_set() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let issue = Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("bob"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = store.add_issue(issue.clone()).await.unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: "test-actor".to_string(),
        });

        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InheritCreatorAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let result = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(result.item.creator, Username::from("bob"));
    }
}
