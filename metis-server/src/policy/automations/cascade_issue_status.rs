use async_trait::async_trait;
use std::collections::HashSet;

use crate::app::AppState;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::issues::IssueStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use metis_common::IssueId;

const AUTOMATION_NAME: &str = "cascade_issue_status";

/// When an issue's status changes to a terminal/failure status, recursively
/// drop all child issues.
///
/// Configurable via `trigger_statuses` param (defaults to Dropped, Rejected, Failed).
pub struct CascadeIssueStatusAutomation {
    trigger_statuses: Vec<IssueStatus>,
}

impl CascadeIssueStatusAutomation {
    pub fn new(params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        let trigger_statuses = if let Some(params) = params {
            let table = params
                .as_mapping()
                .ok_or("cascade_issue_status params must be a mapping")?;
            if let Some(statuses) = table.get("trigger_statuses") {
                let arr = statuses
                    .as_sequence()
                    .ok_or("trigger_statuses must be a sequence")?;
                let mut result = Vec::new();
                for v in arr {
                    let s = v
                        .as_str()
                        .ok_or("trigger_statuses entries must be strings")?;
                    let status = parse_issue_status(s)?;
                    result.push(status);
                }
                result
            } else {
                default_trigger_statuses()
            }
        } else {
            default_trigger_statuses()
        };
        Ok(Self { trigger_statuses })
    }
}

fn default_trigger_statuses() -> Vec<IssueStatus> {
    vec![
        IssueStatus::Dropped,
        IssueStatus::Rejected,
        IssueStatus::Failed,
    ]
}

fn parse_issue_status(s: &str) -> Result<IssueStatus, String> {
    match s.to_lowercase().as_str() {
        "dropped" => Ok(IssueStatus::Dropped),
        "rejected" => Ok(IssueStatus::Rejected),
        "failed" => Ok(IssueStatus::Failed),
        "closed" => Ok(IssueStatus::Closed),
        "open" => Ok(IssueStatus::Open),
        "in-progress" | "in_progress" => Ok(IssueStatus::InProgress),
        other => Err(format!("unknown issue status: '{other}'")),
    }
}

#[async_trait]
impl Automation for CascadeIssueStatusAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueUpdated],
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let ServerEvent::IssueUpdated {
            issue_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Issue {
            old: Some(old),
            new,
            ..
        } = payload.as_ref()
        else {
            return Ok(());
        };

        // Only trigger when the status changed to one of the trigger statuses
        if old.status == new.status {
            return Ok(());
        }
        if !self.trigger_statuses.contains(&new.status) {
            return Ok(());
        }

        let store = ctx.store;
        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        // Drop all children recursively
        drop_children_recursively(ctx.app_state, store, issue_id, actor).await?;

        tracing::info!(
            issue_id = %issue_id,
            new_status = ?new.status,
            "cascade_issue_status completed"
        );

        Ok(())
    }
}

/// Helper to update an issue via `AppState::upsert_issue`.
async fn upsert_issue(
    app_state: &AppState,
    issue_id: &IssueId,
    issue: crate::domain::issues::Issue,
    actor: ActorRef,
) -> Result<(), AutomationError> {
    app_state
        .upsert_issue(
            Some(issue_id.clone()),
            metis_common::api::v1::issues::UpsertIssueRequest::new(issue.into(), None),
            actor,
        )
        .await
        .map_err(|e| {
            AutomationError::Other(anyhow::anyhow!("failed to update issue {issue_id}: {e}"))
        })?;
    Ok(())
}

/// Recursively drop all child issues of the given issue via BFS.
async fn drop_children_recursively(
    app_state: &AppState,
    store: &dyn crate::store::ReadOnlyStore,
    issue_id: &IssueId,
    actor: ActorRef,
) -> Result<(), AutomationError> {
    let mut to_visit = store.get_issue_children(issue_id).await.map_err(|e| {
        AutomationError::Other(anyhow::anyhow!("failed to get children of {issue_id}: {e}"))
    })?;

    let mut visited = HashSet::new();

    while let Some(child_id) = to_visit.pop() {
        if !visited.insert(child_id.clone()) {
            continue;
        }

        let child = store.get_issue(&child_id, false).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to fetch child issue {child_id}: {e}"
            ))
        })?;

        if !child.item.status.is_terminal() {
            let mut child_issue = child.item;
            child_issue.status = IssueStatus::Dropped;
            upsert_issue(app_state, &child_id, child_issue, actor.clone()).await?;
        }

        let grandchildren = store.get_issue_children(&child_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!("failed to get children of {child_id}: {e}"))
        })?;
        to_visit.extend(grandchildren);
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
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use std::sync::Arc;

    fn make_issue(status: IssueStatus, deps: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "test".to_string(),
            Username::from("tester"),
            String::new(),
            status,
            None,
            None,
            Vec::new(),
            deps,
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn drops_children_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Update parent to Dropped
        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped;
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn does_not_cascade_to_blocked_on_dependents_when_failed() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create issue A (will fail)
        let issue_a = make_issue(IssueStatus::Open, Vec::new());
        let (id_a, _) = store
            .add_issue(issue_a.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Create issue B that is blocked on A
        let issue_b = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                id_a.clone(),
            )],
        );
        let (id_b, _) = store.add_issue(issue_b, &ActorRef::test()).await.unwrap();

        // Fail issue A — Failed is no longer a trigger status,
        // so the automation should not fire and B should stay Open.
        let mut failed_a = issue_a;
        failed_a.status = IssueStatus::Failed;
        store
            .update_issue(&id_a, failed_a.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: failed_a,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: id_a.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // B should remain Open (not dropped)
        let b_result = store.get_issue(&id_b, false).await.unwrap();
        assert_eq!(b_result.item.status, IssueStatus::Open);
    }

    #[tokio::test]
    async fn drops_children_when_parent_failed() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Fail the parent — children should be dropped.
        let mut failed_parent = parent;
        failed_parent.status = IssueStatus::Failed;
        store
            .update_issue(&parent_id, failed_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: failed_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn drops_children_when_parent_rejected() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent and child
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        // Reject the parent — children should be dropped.
        let mut rejected_parent = parent;
        rejected_parent.status = IssueStatus::Rejected;
        store
            .update_issue(&parent_id, rejected_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: rejected_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn no_cascade_when_status_unchanged() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let issue = make_issue(IssueStatus::Failed, Vec::new());
        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Event where old and new status are both Failed (no change)
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // Should return Ok without doing anything
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_closed_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Closed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped;
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Closed);
    }

    #[tokio::test]
    async fn skips_failed_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Failed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped;
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Failed);
    }

    #[tokio::test]
    async fn skips_rejected_child_when_parent_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Rejected,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped;
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Rejected);
    }

    #[tokio::test]
    async fn drops_grandchild_of_terminal_child() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        // Create parent -> closed child -> open grandchild
        let parent = make_issue(IssueStatus::Open, Vec::new());
        let (parent_id, _) = store
            .add_issue(parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let child = make_issue(
            IssueStatus::Closed,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = store.add_issue(child, &ActorRef::test()).await.unwrap();

        let grandchild = make_issue(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                child_id.clone(),
            )],
        );
        let (grandchild_id, _) = store
            .add_issue(grandchild, &ActorRef::test())
            .await
            .unwrap();

        let mut dropped_parent = parent;
        dropped_parent.status = IssueStatus::Dropped;
        store
            .update_issue(&parent_id, dropped_parent.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue(IssueStatus::Open, Vec::new())),
            new: dropped_parent,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: parent_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = CascadeIssueStatusAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Closed child should stay closed
        let child_result = store.get_issue(&child_id, false).await.unwrap();
        assert_eq!(child_result.item.status, IssueStatus::Closed);

        // Open grandchild should be dropped
        let grandchild_result = store.get_issue(&grandchild_id, false).await.unwrap();
        assert_eq!(grandchild_result.item.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn custom_trigger_statuses_from_config() {
        let mut map = serde_yaml_ng::Mapping::new();
        map.insert(
            serde_yaml_ng::Value::String("trigger_statuses".to_string()),
            serde_yaml_ng::Value::Sequence(vec![serde_yaml_ng::Value::String(
                "closed".to_string(),
            )]),
        );
        let params = serde_yaml_ng::Value::Mapping(map);

        let automation = CascadeIssueStatusAutomation::new(Some(&params)).unwrap();
        assert_eq!(automation.trigger_statuses, vec![IssueStatus::Closed]);
    }
}
