//! FSA execution engine for workflows.
//!
//! Reacts to `IssueUpdated` events: when the issue is a workflow's currently
//! active child issue and has reached a terminal status, the engine picks
//! the first matching `on_child_status` transition out of the workflow's
//! current state and asks `AppState` to advance the workflow. Subsequent
//! `Auto` transitions out of `Noop` states are chased by the AppState
//! helper, so the engine itself does not have to recurse here.

use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "workflow_engine";

/// Drives running workflows forward when their active child issue reaches
/// a terminal status. Registered in `build_default_registry` and activated
/// via `default_policy_config()`.
pub struct WorkflowEngineAutomation;

impl WorkflowEngineAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for WorkflowEngineAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let ServerEvent::IssueUpdated {
            issue_id, payload, ..
        } = ctx.event
        else {
            return Ok(());
        };

        let MutationPayload::Issue { old, new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Only react when the status actually transitioned to a terminal
        // value. Updates that only mutate progress / assignee / etc. should
        // not trigger another transition attempt.
        if !new.status.is_terminal() {
            return Ok(());
        }
        if let Some(old) = old {
            if old.status == new.status {
                return Ok(());
            }
        }

        let store = ctx.store;
        let Some(versioned) = store
            .find_workflow_by_issue_id(issue_id)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to look up workflow for issue {issue_id}: {e}"
                ))
            })?
        else {
            return Ok(());
        };
        let workflow = versioned.item;

        // Stale event: the workflow has already moved past this child
        // issue (e.g., a later state created another child and we are
        // seeing a delayed update for the previous one).
        if workflow.active_issue_id.as_ref() != Some(issue_id) {
            tracing::debug!(
                workflow_id = %workflow.workflow_id,
                active_issue_id = ?workflow.active_issue_id,
                stale_issue_id = %issue_id,
                "workflow_engine: skipping stale child-issue event"
            );
            return Ok(());
        }

        // Already terminal — nothing to drive forward.
        if workflow.status.is_terminal() {
            return Ok(());
        }

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };
        let advance_result = ctx
            .app_state
            .advance_workflow_from_child_status(&workflow.workflow_id, new.status, actor)
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to advance workflow {}: {e}",
                    workflow.workflow_id
                ))
            })?;

        match advance_result {
            Some(advanced) => {
                tracing::info!(
                    workflow_id = %advanced.workflow_id,
                    issue_id = %issue_id,
                    issue_status = ?new.status,
                    current_state = %advanced.current_state,
                    workflow_status = ?advanced.status,
                    "workflow_engine advanced workflow"
                );
            }
            None => {
                tracing::info!(
                    workflow_id = %workflow.workflow_id,
                    issue_id = %issue_id,
                    issue_status = ?new.status,
                    current_state = %workflow.current_state,
                    "workflow_engine: no transition matches child status; workflow unchanged"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::domain::issues::IssueStatus;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::test_utils::test_state_handles;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Arc;

    const TEMPLATE_PATH: &str = "/workflows/engine-test.yaml";

    /// A multi-state template that exercises both `OnChildStatus` and a
    /// terminal noop state at the end.
    fn patch_review_yaml() -> &'static str {
        r#"
name: "Engine Test"
description: "FSA exercise"
initial_state: develop

states:
  - id: develop
    name: "Development"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Develop"
      description_template: "do it"
      assignee: "swe"
  - id: review
    name: "Review"
    on_enter:
      type: create_issue
      issue_type: review-request
      title_template: "Review"
      description_template: "review"
      assignee: "reviewer"
  - id: fix
    name: "Fix"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Fix"
      description_template: "fix it"
      assignee: "swe"
  - id: merge
    name: "Merge"
    on_enter:
      type: create_issue
      issue_type: merge-request
      title_template: "Merge"
      description_template: "merge"
      assignee: "swe"
  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop
  - id: abandoned
    name: "Abandoned"
    terminal: true
    terminal_status: dropped
    on_enter:
      type: noop

transitions:
  - from: develop
    to: review
    label: "Ready for Review"
    trigger:
      type: on_child_status
      status: closed
  - from: develop
    to: abandoned
    label: "Abandoned"
    trigger:
      type: on_child_status
      status: failed
  - from: review
    to: merge
    label: "Approved"
    trigger:
      type: on_child_status
      status: closed
  - from: review
    to: fix
    label: "Changes Requested"
    trigger:
      type: on_child_status
      status: failed
  - from: fix
    to: review
    label: "Ready for Re-review"
    trigger:
      type: on_child_status
      status: closed
  - from: merge
    to: merged
    label: "Merge Complete"
    trigger:
      type: on_child_status
      status: closed
"#
    }

    async fn upload_template(state: &crate::app::AppState, yaml: &str) {
        let document = Document {
            title: "Template".to_string(),
            body_markdown: yaml.to_string(),
            path: Some(TEMPLATE_PATH.parse().expect("valid path")),
            created_by: None,
            deleted: false,
        };
        state
            .upsert_document(None, document, ActorRef::test())
            .await
            .expect("upload template");
    }

    async fn drive_active_child(
        handles: &crate::test_utils::TestStateHandles,
        workflow_id: &hydra_common::WorkflowId,
        new_status: IssueStatus,
    ) {
        let workflow = handles
            .state
            .get_workflow(workflow_id)
            .await
            .expect("get workflow")
            .item;
        let issue_id = workflow
            .active_issue_id
            .clone()
            .expect("workflow has an active child");
        let old = handles
            .state
            .get_issue(&issue_id, false)
            .await
            .expect("get issue")
            .item;
        let mut updated = old.clone();
        updated.status = new_status;
        handles
            .store
            .update_issue(&issue_id, updated.clone(), &ActorRef::test())
            .await
            .expect("update issue");

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old),
            new: updated,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };
        let automation = WorkflowEngineAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.expect("automation execute");
    }

    fn empty_context() -> HashMap<String, String> {
        HashMap::new()
    }

    #[tokio::test]
    async fn drives_workflow_through_full_happy_path() {
        let handles = test_state_handles();
        upload_template(&handles.state, patch_review_yaml()).await;

        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        // develop -> review
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after_develop = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_develop.current_state, "review");
        assert_eq!(after_develop.history.len(), 2);

        // review -> merge (closed = approved)
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after_review = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_review.current_state, "merge");
        assert_eq!(after_review.history.len(), 3);

        // merge -> merged (terminal)
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after_merge = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_merge.current_state, "merged");
        assert_eq!(
            after_merge.status,
            crate::domain::workflows::WorkflowStatus::Completed
        );
        assert!(after_merge.history.len() >= 4);

        // History entries should be in order: develop, review, merge, merged.
        let states: Vec<&str> = after_merge
            .history
            .iter()
            .map(|h| h.to_state.as_str())
            .collect();
        assert_eq!(states, vec!["develop", "review", "merge", "merged"]);

        // Tracking issue flips to terminal_status (closed).
        let tracking = handles
            .state
            .get_issue(&after_merge.tracking_issue_id, false)
            .await
            .expect("tracking exists");
        assert_eq!(tracking.item.status, IssueStatus::Closed);
    }

    #[tokio::test]
    async fn failed_transitions_loop_through_fix_back_to_review() {
        let handles = test_state_handles();
        upload_template(&handles.state, patch_review_yaml()).await;

        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        // develop -> review
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        // review fails (changes requested) -> fix
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Failed).await;
        let after_fail = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_fail.current_state, "fix");
        // fix closed -> review again
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after_fix = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_fix.current_state, "review");

        // Workflow should still be active.
        assert_eq!(
            after_fix.status,
            crate::domain::workflows::WorkflowStatus::Active
        );
    }

    #[tokio::test]
    async fn failure_in_develop_drives_to_failed_terminal() {
        let handles = test_state_handles();
        upload_template(&handles.state, patch_review_yaml()).await;

        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Failed).await;
        let after = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after.current_state, "abandoned");
        assert_eq!(
            after.status,
            crate::domain::workflows::WorkflowStatus::Failed
        );

        let tracking = handles
            .state
            .get_issue(&after.tracking_issue_id, false)
            .await
            .unwrap()
            .item;
        assert_eq!(tracking.status, IssueStatus::Dropped);
    }

    #[tokio::test]
    async fn stale_child_issue_event_is_ignored() {
        let handles = test_state_handles();
        upload_template(&handles.state, patch_review_yaml()).await;

        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        // The develop child closes and we move on to review.
        let develop_child_id = workflow.active_issue_id.clone().unwrap();
        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after_first = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after_first.current_state, "review");
        assert_ne!(
            after_first.active_issue_id.as_ref(),
            Some(&develop_child_id)
        );

        // Now fire a stale IssueUpdated for the old develop child (e.g. a
        // late progress update flipping it back). The workflow must not
        // advance from review.
        let stale_old = handles
            .state
            .get_issue(&develop_child_id, false)
            .await
            .unwrap()
            .item;
        let mut stale_new = stale_old.clone();
        stale_new.status = IssueStatus::Failed;
        handles
            .store
            .update_issue(&develop_child_id, stale_new.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(stale_old),
            new: stale_new,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 99,
            issue_id: develop_child_id.clone(),
            version: 3,
            timestamp: Utc::now(),
            payload,
        };
        let automation = WorkflowEngineAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await.expect("execute");

        let after_stale = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        // Still in review — the stale event did not transition us to
        // `abandoned` (which would be the develop→failed path).
        assert_eq!(after_stale.current_state, "review");
    }

    #[tokio::test]
    async fn auto_transition_out_of_noop_state_chains() {
        let handles = test_state_handles();
        let yaml = r#"
name: "Auto Chain"
description: "exercise auto out of noop"
initial_state: develop

states:
  - id: develop
    name: "Develop"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "develop"
      description_template: "do"
      assignee: "swe"
  - id: bridge
    name: "Bridge"
    on_enter:
      type: noop
  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop

transitions:
  - from: develop
    to: bridge
    trigger:
      type: on_child_status
      status: closed
  - from: bridge
    to: merged
    trigger:
      type: auto
"#;
        upload_template(&handles.state, yaml).await;
        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Closed).await;
        let after = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        // The auto transition out of `bridge` should have run synchronously
        // and landed us in `merged`.
        assert_eq!(after.current_state, "merged");
        assert_eq!(
            after.status,
            crate::domain::workflows::WorkflowStatus::Completed
        );
    }

    #[tokio::test]
    async fn unmatched_terminal_status_leaves_workflow_in_place() {
        let handles = test_state_handles();
        let yaml = r#"
name: "Only Closed"
description: "no failed transition"
initial_state: develop

states:
  - id: develop
    name: "Develop"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "develop"
      description_template: "do"
      assignee: "swe"
  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop

transitions:
  - from: develop
    to: merged
    trigger:
      type: on_child_status
      status: closed
"#;
        upload_template(&handles.state, yaml).await;
        let workflow = handles
            .state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                empty_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        drive_active_child(&handles, &workflow.workflow_id, IssueStatus::Failed).await;
        let after = handles
            .state
            .get_workflow(&workflow.workflow_id)
            .await
            .unwrap()
            .item;
        assert_eq!(after.current_state, "develop");
        assert_eq!(
            after.status,
            crate::domain::workflows::WorkflowStatus::Active
        );
    }

    #[tokio::test]
    async fn event_filter_only_matches_issue_updated() {
        let automation = WorkflowEngineAutomation::new(None).unwrap();
        let filter = automation.event_filter();
        assert_eq!(filter.event_types, vec![EventType::IssueUpdated]);
    }
}
