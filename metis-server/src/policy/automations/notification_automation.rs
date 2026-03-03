use async_trait::async_trait;

use crate::app::event_bus::EventType;
use crate::domain::actors::ActorId;
use crate::domain::notifications::{
    Notification, NotificationPolicy, WalkUpPolicy, actor_ref_to_actor_id, event_object_id,
    event_object_kind, event_source_issue_id, event_type_str, event_version, generate_summary,
};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "notification_generation";

/// Automation that generates notification rows for each event.
///
/// Runs all registered `NotificationPolicy` implementations to resolve recipients,
/// deduplicates them, excludes the source actor, generates a human-readable summary,
/// and inserts notification rows via the event-bypass method on `StoreWithEvents`.
pub struct NotificationAutomation {
    policies: Vec<Box<dyn NotificationPolicy>>,
}

impl NotificationAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self {
            policies: vec![Box::new(WalkUpPolicy)],
        })
    }
}

#[async_trait]
impl Automation for NotificationAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        // Match all events except NotificationCreated to avoid infinite loops
        // (this automation emits NotificationCreated events).
        EventFilter {
            exclude_event_types: vec![EventType::NotificationCreated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let event = ctx.event;
        let payload = event.payload();
        let source_actor = actor_ref_to_actor_id(payload.actor());

        // Collect recipients from all policies, deduplicating and tracking which
        // policy produced each recipient.
        let mut policy_recipients: Vec<(ActorId, String)> = Vec::new();

        for policy in &self.policies {
            let recipients = policy
                .resolve_recipients(event, ctx.store)
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "policy '{}' failed to resolve recipients: {e}",
                        policy.name()
                    ))
                })?;
            for recipient in recipients {
                // Exclude the source actor (don't notify yourself)
                if let Some(ref actor) = source_actor {
                    if recipient == *actor {
                        continue;
                    }
                }
                // Deduplicate by checking if this recipient is already present
                if !policy_recipients.iter().any(|(r, _)| r == &recipient) {
                    policy_recipients.push((recipient, policy.name().to_string()));
                }
            }
        }

        if policy_recipients.is_empty() {
            return Ok(());
        }

        let summary = generate_summary(event);
        let object_kind = event_object_kind(event).to_string();
        let object_id = event_object_id(event);
        let object_version = event_version(event);
        let event_type = event_type_str(event).to_string();
        let source_issue_id = event_source_issue_id(event);

        let actor = crate::domain::actors::ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        for (recipient, policy_name) in policy_recipients {
            let notification = Notification::new(
                recipient.clone(),
                source_actor.clone(),
                object_kind.clone(),
                object_id.clone(),
                object_version,
                event_type.clone(),
                summary.clone(),
                source_issue_id.clone(),
                policy_name,
            );

            // Use insert_notification_with_actor so StoreWithEvents emits the
            // NotificationCreated event to the EventBus automatically.
            if let Err(err) = ctx
                .app_state
                .store
                .insert_notification_with_actor(notification, actor.clone())
                .await
            {
                tracing::error!(error = %err, "failed to insert notification");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::{MutationPayload, ServerEvent};
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType,
    };
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use metis_common::api::v1::notifications::ListNotificationsQuery;
    use std::sync::Arc;

    fn make_issue(creator: &str, assignee: Option<&str>, deps: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test issue".to_string(),
            Username::from(creator),
            String::new(),
            IssueStatus::Open,
            assignee.map(|s| s.to_string()),
            None,
            vec![],
            deps,
            vec![],
        )
    }

    fn test_actor() -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Username(metis_common::api::v1::users::Username::from("test-actor")),
        }
    }

    #[tokio::test]
    async fn notification_automation_processes_event_and_creates_notifications() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create a parent issue
        let parent = make_issue("alice", Some("bob"), vec![]);
        let (parent_id, _) = store
            .add_issue(parent.clone(), &test_actor())
            .await
            .unwrap();

        // Create a child issue
        let child = make_issue(
            "charlie",
            None,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, child_version) =
            store.add_issue(child.clone(), &test_actor()).await.unwrap();

        // Simulate an IssueUpdated event on the child
        let mut updated_child = child.clone();
        updated_child.status = IssueStatus::InProgress;
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(child),
            new: updated_child,
            actor: ActorRef::Authenticated {
                actor_id: ActorId::Issue(child_id.clone()),
            },
        });
        let event = ServerEvent::IssueUpdated {
            seq: 100,
            issue_id: child_id.clone(),
            version: child_version + 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = NotificationAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Check that notifications were created
        let query = ListNotificationsQuery::default();
        let notifications = store.list_notifications(&query).await.unwrap();

        // The source actor is ActorId::Issue(child_id) which should be excluded.
        // Recipients should be: charlie (child creator), parent issue, alice (parent creator), bob (parent assignee)
        assert!(
            !notifications.is_empty(),
            "should have created notifications"
        );

        // Verify the source actor (child issue) was excluded
        for (_, notif) in &notifications {
            assert_ne!(
                notif.recipient,
                ActorId::Issue(child_id.clone()),
                "source actor should be excluded from recipients"
            );
        }

        // Verify the parent issue's creator got a notification
        let alice_notifs: Vec<_> = notifications
            .iter()
            .filter(|(_, n)| {
                n.recipient
                    == ActorId::Username(metis_common::api::v1::users::Username::from("alice"))
            })
            .collect();
        assert!(
            !alice_notifs.is_empty(),
            "parent creator alice should get a notification"
        );

        // Check summary content
        let (_, first) = &notifications[0];
        assert!(
            first.summary.contains("status changed"),
            "summary should describe status change, got: {}",
            first.summary
        );
        assert_eq!(first.event_type, "updated");
        assert_eq!(first.object_kind, "issue");
    }
}
