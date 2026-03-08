use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::{ActorId, ActorRef};
use crate::domain::issues::IssueStatus;
use crate::domain::users::Username;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "inbox_label";
pub const INBOX_LABEL_NAME: &str = "inbox";

/// Automatically applies the 'inbox' label to issues when:
/// - A human user creates an issue (IssueCreated with Authenticated Username actor)
/// - An issue is created/updated to (open || in-progress) with a human assignee
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
            ServerEvent::IssueCreated { payload, .. } => {
                if is_human_actor(payload.actor()) {
                    return self.apply_label(ctx).await;
                }
                if let MutationPayload::Issue { new, .. } = payload.as_ref() {
                    is_active_status(&new.status)
                        && is_human_assignee(ctx, new.assignee.as_deref()).await
                } else {
                    false
                }
            }
            ServerEvent::IssueUpdated { payload, .. } => {
                if let MutationPayload::Issue { new, .. } = payload.as_ref() {
                    is_active_status(&new.status)
                        && is_human_assignee(ctx, new.assignee.as_deref()).await
                } else {
                    false
                }
            }
            _ => false,
        };

        if !should_apply {
            return Ok(());
        }

        self.apply_label(ctx).await
    }
}

impl InboxLabelAutomation {
    async fn apply_label(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
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

fn is_active_status(status: &IssueStatus) -> bool {
    matches!(status, IssueStatus::Open | IssueStatus::InProgress)
}

async fn is_human_assignee(ctx: &AutomationContext<'_>, assignee: Option<&str>) -> bool {
    let Some(assignee) = assignee else {
        return false;
    };
    let username = Username::from(assignee);
    ctx.app_state.get_user(&username).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueStatus, IssueType};
    use crate::domain::users::{User, Username};
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

    async fn add_human_user(handles: &test_utils::TestStateHandles, name: &str) {
        let user = User::new(Username::from(name), Some(12345), false);
        handles.store.add_user(user, &human_actor()).await.unwrap();
    }

    async fn setup_inbox_label(handles: &test_utils::TestStateHandles) -> metis_common::LabelId {
        handles
            .state
            .create_label(
                INBOX_LABEL_NAME.to_string(),
                None,
                false,
                true,
                ActorRef::test(),
            )
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
        add_human_user(&handles, "assignee").await;

        let old_issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &human_actor())
            .await
            .unwrap();

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

    #[tokio::test]
    async fn applies_inbox_label_on_update_with_human_assignee_and_open_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;
        add_human_user(&handles, "human_assignee").await;

        let old_issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &system_actor())
            .await
            .unwrap();

        let new_issue = make_issue(IssueStatus::Open, Some("human_assignee".to_string()));

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: system_actor(),
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
    async fn does_not_apply_on_update_with_non_human_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let _inbox_label_id = setup_inbox_label(&handles).await;

        let old_issue = make_issue(IssueStatus::Open, None);
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &system_actor())
            .await
            .unwrap();

        // "bot_user" is not in the user store
        let new_issue = make_issue(IssueStatus::Open, Some("bot_user".to_string()));

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: system_actor(),
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
    async fn does_not_apply_on_update_with_closed_status_and_human_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let _inbox_label_id = setup_inbox_label(&handles).await;
        add_human_user(&handles, "human_assignee").await;

        let old_issue = make_issue(IssueStatus::Open, Some("human_assignee".to_string()));
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &system_actor())
            .await
            .unwrap();

        let new_issue = make_issue(IssueStatus::Closed, Some("human_assignee".to_string()));

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: system_actor(),
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
    async fn applies_on_status_change_to_open_with_existing_human_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;
        add_human_user(&handles, "human_assignee").await;

        let old_issue = make_issue(IssueStatus::Closed, Some("human_assignee".to_string()));
        let (issue_id, _) = store
            .add_issue(old_issue.clone(), &system_actor())
            .await
            .unwrap();

        // Status changes from Closed -> Open, assignee stays the same
        let new_issue = make_issue(IssueStatus::Open, Some("human_assignee".to_string()));

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: system_actor(),
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
    async fn applies_inbox_label_on_create_with_human_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let inbox_label_id = setup_inbox_label(&handles).await;
        add_human_user(&handles, "human_assignee").await;

        let issue = make_issue(IssueStatus::Open, Some("human_assignee".to_string()));
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
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label_id, inbox_label_id);
    }

    #[tokio::test]
    async fn does_not_apply_on_create_with_non_human_assignee() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let _inbox_label_id = setup_inbox_label(&handles).await;

        // "bot_user" is not in the user store
        let issue = make_issue(IssueStatus::Open, Some("bot_user".to_string()));
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
}
