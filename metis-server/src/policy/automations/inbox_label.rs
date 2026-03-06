use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::{ActorId, ActorRef};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "inbox_label";
pub const INBOX_LABEL_NAME: &str = "inbox";

/// Automatically applies the 'inbox' label to issues when:
/// - A human user creates an issue (IssueCreated with Authenticated Username actor)
/// - An issue's assignee changes (IssueUpdated with assignee diff)
pub struct InboxLabelAutomation;

impl InboxLabelAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for InboxLabelAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueCreated, EventType::IssueUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let should_apply = match ctx.event {
            ServerEvent::IssueCreated { payload, .. } => is_human_actor(payload.actor()),
            ServerEvent::IssueUpdated { payload, .. } => {
                if let MutationPayload::Issue {
                    old: Some(old),
                    new,
                    ..
                } = payload.as_ref()
                {
                    old.assignee != new.assignee && new.assignee.is_some()
                } else {
                    false
                }
            }
            _ => false,
        };

        if !should_apply {
            return Ok(());
        }

        let issue_id = match ctx.event {
            ServerEvent::IssueCreated { issue_id, .. }
            | ServerEvent::IssueUpdated { issue_id, .. } => issue_id,
            _ => return Ok(()),
        };

        let label = ctx
            .store
            .get_label_by_name(INBOX_LABEL_NAME)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to look up inbox label: {e}"))
            })?;

        let (label_id, _) = match label {
            Some(pair) => pair,
            None => {
                tracing::warn!("inbox label not found; skipping inbox_label automation");
                return Ok(());
            }
        };

        let object_id: metis_common::MetisId = issue_id.clone().into();
        ctx.app_state
            .add_label_association(&label_id, &object_id)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to add inbox label to issue {issue_id}: {e}"
                ))
            })?;

        tracing::info!(
            issue_id = %issue_id,
            "inbox_label automation applied inbox label"
        );

        Ok(())
    }
}

fn is_human_actor(actor: &ActorRef) -> bool {
    matches!(
        actor,
        ActorRef::Authenticated {
            actor_id: ActorId::Username(_),
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_issue(status: IssueStatus, assignee: Option<String>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test".to_string(),
            Username::from("tester"),
            String::new(),
            status,
            assignee,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn human_actor() -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("tester").into()),
        }
    }

    fn system_actor() -> ActorRef {
        ActorRef::Automation {
            automation_name: "some_automation".into(),
            triggered_by: None,
        }
    }

    async fn setup_inbox_label(handles: &test_utils::TestStateHandles) -> metis_common::LabelId {
        handles
            .state
            .create_label(INBOX_LABEL_NAME.to_string(), None, false, true)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn applies_inbox_label_on_human_issue_creation() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;

        let issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(issue.clone(), &human_actor())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: human_actor(),
        });

        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InboxLabelAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let labels = store
            .get_labels_for_object(&issue_id.clone().into())
            .await
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, inbox_label_id);
    }

    #[tokio::test]
    async fn does_not_apply_inbox_label_on_automation_issue_creation() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let _inbox_label_id = setup_inbox_label(&handles).await;

        let issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(issue.clone(), &system_actor())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: system_actor(),
        });

        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InboxLabelAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let labels = store
            .get_labels_for_object(&issue_id.clone().into())
            .await
            .unwrap();
        assert!(labels.is_empty());
    }

    #[tokio::test]
    async fn applies_inbox_label_on_assignee_change() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;

        let old_issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &human_actor())
            .await
            .unwrap();

        // Remove inbox label added during creation test setup
        // (In real usage the automation would have fired, but here we're testing
        // the update path separately.)

        let new_issue = make_issue(IssueStatus::Open, Some("assignee".to_string()));

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: human_actor(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InboxLabelAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let labels = store
            .get_labels_for_object(&issue_id.clone().into())
            .await
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, inbox_label_id);
    }

    #[tokio::test]
    async fn does_not_apply_on_assignee_cleared() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let _inbox_label_id = setup_inbox_label(&handles).await;

        let old_issue = make_issue(IssueStatus::Open, Some("old_assignee".to_string()));
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &human_actor())
            .await
            .unwrap();

        let new_issue = make_issue(IssueStatus::Open, None);

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: human_actor(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InboxLabelAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let labels = store
            .get_labels_for_object(&issue_id.clone().into())
            .await
            .unwrap();
        assert!(labels.is_empty());
    }

    #[tokio::test]
    async fn idempotent_double_application() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;

        let issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(issue.clone(), &human_actor())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: human_actor(),
        });

        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = InboxLabelAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Apply twice
        automation.execute(&ctx).await.unwrap();
        automation.execute(&ctx).await.unwrap();

        let labels = store
            .get_labels_for_object(&issue_id.clone().into())
            .await
            .unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, inbox_label_id);
    }
}
