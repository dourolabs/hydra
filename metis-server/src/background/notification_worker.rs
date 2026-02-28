use crate::app::AppState;
use crate::app::event_bus::ServerEvent;
use crate::domain::actors::ActorId;
use crate::domain::notifications::{
    Notification, NotificationPolicy, WalkUpPolicy, actor_ref_to_actor_id, event_object_id,
    event_object_kind, event_source_issue_id, event_type_str, event_version, generate_summary,
};
use tokio::sync::broadcast;
use tokio::sync::watch;

/// Spawn the notification worker as a background tokio task.
///
/// The worker subscribes to the event bus and generates notification rows for each
/// event by running all registered `NotificationPolicy` implementations.
///
/// Returns a `JoinHandle` that can be awaited for graceful shutdown.
pub fn spawn_notification_worker(
    state: AppState,
    shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let rx = state.subscribe();
    let policies: Vec<Box<dyn NotificationPolicy>> = vec![Box::new(WalkUpPolicy)];
    tokio::spawn(run_notification_loop(state, rx, shutdown_rx, policies))
}

async fn run_notification_loop(
    state: AppState,
    mut rx: broadcast::Receiver<ServerEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
    policies: Vec<Box<dyn NotificationPolicy>>,
) {
    tracing::info!("notification worker started");

    loop {
        let event = tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                tracing::info!("notification worker shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            skipped = n,
                            "notification worker lagged behind event bus; skipped events"
                        );
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("event bus closed; notification worker exiting");
                        break;
                    }
                }
            }
        };

        if let Err(err) = process_event(&state, &event, &policies).await {
            tracing::error!(error = %err, "notification worker failed to process event");
        }
    }
}

async fn process_event(
    state: &AppState,
    event: &ServerEvent,
    policies: &[Box<dyn NotificationPolicy>],
) -> Result<(), anyhow::Error> {
    let payload = event.payload();
    let source_actor = actor_ref_to_actor_id(payload.actor());

    // Collect recipients from all policies, deduplicating and tracking which
    // policy produced each recipient.
    let mut policy_recipients: Vec<(ActorId, String)> = Vec::new();

    for policy in policies {
        let recipients = policy.resolve_recipients(event, state.store()).await?;
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

    for (recipient, policy_name) in policy_recipients {
        let notification = Notification::new(
            recipient,
            source_actor.clone(),
            object_kind.clone(),
            object_id.clone(),
            object_version,
            event_type.clone(),
            summary.clone(),
            source_issue_id.clone(),
            policy_name,
        );

        if let Err(err) = state.store.insert_notification(notification).await {
            tracing::error!(error = %err, "failed to insert notification");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType,
    };
    use crate::domain::users::Username;
    use crate::store::ReadOnlyStore;
    use crate::test_utils::test_state;
    use chrono::Utc;
    use metis_common::api::v1::notifications::ListNotificationsQuery;
    use std::sync::Arc;

    fn make_issue(creator: &str, assignee: Option<&str>, deps: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
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
    async fn notification_worker_processes_event_and_creates_notifications() {
        let state = test_state();

        // Create a parent issue
        let parent = make_issue("alice", Some("bob"), vec![]);
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent.clone(), test_actor())
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
        let (child_id, child_version) = state
            .store
            .add_issue_with_actor(child.clone(), test_actor())
            .await
            .unwrap();

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

        let policies: Vec<Box<dyn NotificationPolicy>> = vec![Box::new(WalkUpPolicy)];
        process_event(&state, &event, &policies).await.unwrap();

        // Check that notifications were created
        let query = ListNotificationsQuery::default();
        let notifications = state.store.list_notifications(&query).await.unwrap();

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
