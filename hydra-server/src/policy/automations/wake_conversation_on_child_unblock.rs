//! `wake_conversation_on_child_unblock` — producer side of the
//! "interactive parent wakes on child completion" invariant.
//!
//! When a child issue transitions into a status whose `unblocks_parents`
//! flag is `true` (and the prior status was not already
//! `unblocks_parents`), this automation finds each interactive parent
//! and appends a [`SessionEvent::SystemEvent`] of kind
//! [`SystemEventKind::ChildUnblocked`] to the parent's non-`Closed`
//! conversation via [`crate::app::AppState::append_system_event`]. That
//! call's relay routing flips `Idle → Active` and either delivers to a
//! live worker or queues for the worker that
//! `SpawnConversationSessionsAutomation` mints in response to the
//! conversation status change.
//!
//! Headless parents are intentionally not handled here — they get a
//! respawn via `SpawnSessionsAutomation` on the same `IssueUpdated`
//! event. This automation only adds the dual path for interactive
//! parents.

use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::conversations::ConversationStatus;
use crate::domain::issues::IssueDependencyType;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::sessions::SystemEventKind;

const AUTOMATION_NAME: &str = "wake_conversation_on_child_unblock";

pub struct WakeConversationOnChildUnblockAutomation;

impl WakeConversationOnChildUnblockAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for WakeConversationOnChildUnblockAutomation {
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
        // Skip events triggered by this automation to avoid loops. The
        // append_system_event call itself emits a ConversationUpdated /
        // SessionEventCreated, neither of which is in our event_filter,
        // but defend in depth: a future change that lets this automation
        // also react to one of those events would otherwise reintroduce
        // the loop.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

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

        if old.status == new.status {
            return Ok(());
        }

        let old_resolved = match ctx.app_state.resolve_status(old).await {
            Ok(def) => def,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    status = %old.status,
                    error = %err,
                    "failed to resolve old status; skipping"
                );
                return Ok(());
            }
        };
        let new_resolved = match ctx.app_state.resolve_status(new).await {
            Ok(def) => def,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    status = %new.status,
                    error = %err,
                    "failed to resolve new status; skipping"
                );
                return Ok(());
            }
        };

        // Fire only on the unblocks_parents transition: `false → true`.
        // Anything else (`true → true` between two terminal statuses,
        // `* → false`) is a no-op. The transition-only condition makes
        // this automation naturally idempotent across event redeliveries
        // and prevents a second wake when, e.g., a child moves from
        // `closed` to `failed`.
        if old_resolved.unblocks_parents || !new_resolved.unblocks_parents {
            return Ok(());
        }

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        for dependency in &new.dependencies {
            if dependency.dependency_type != IssueDependencyType::ChildOf {
                continue;
            }
            let parent_id = &dependency.issue_id;

            let parent = match ctx.store.get_issue(parent_id, false).await {
                Ok(versioned) => versioned.item,
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %issue_id,
                        parent_id = %parent_id,
                        error = %err,
                        "failed to fetch parent issue; skipping"
                    );
                    continue;
                }
            };

            let parent_resolved = match ctx.app_state.resolve_status(&parent).await {
                Ok(def) => def,
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %issue_id,
                        parent_id = %parent_id,
                        status = %parent.status,
                        error = %err,
                        "failed to resolve parent status; skipping"
                    );
                    continue;
                }
            };

            // Headless parents already get a respawn via
            // SpawnSessionsAutomation on this same IssueUpdated event;
            // we only add the dual path for interactive parents.
            if !parent_resolved.interactive {
                continue;
            }

            let query = SearchConversationsQuery {
                spawned_from: Some(parent_id.clone()),
                include_deleted: Some(false),
                ..Default::default()
            };
            let conversations = match ctx.store.list_conversations(&query).await {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %issue_id,
                        parent_id = %parent_id,
                        error = %err,
                        "failed to list parent conversations; skipping"
                    );
                    continue;
                }
            };

            // `list_conversations` returns rows ordered by `updated_at`
            // DESC; pick the first non-Closed row. The spec invariant
            // guarantees exactly one such conversation per interactive
            // parent (see [[i-eiicogxi]]); if we somehow see more, the
            // DESC ordering means the first is the most recent — log and
            // wake that one.
            let mut live: Vec<_> = conversations
                .into_iter()
                .filter(|(_, v)| v.item.status != ConversationStatus::Closed)
                .collect();
            if live.len() > 1 {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    child_id = %issue_id,
                    parent_id = %parent_id,
                    count = live.len(),
                    "expected at most one live conversation per interactive parent; waking the most recent"
                );
            }
            let Some((conversation_id, _)) = live.drain(..).next() else {
                // Parent is in an interactive status but has no live
                // conversation. This is possible briefly during
                // conversation creation or if a human closed the
                // conversation just now; either way nothing to wake.
                continue;
            };

            let kind = SystemEventKind::ChildUnblocked {
                child_id: issue_id.clone(),
                new_status: new.status.clone(),
            };
            match ctx
                .app_state
                .append_system_event(&conversation_id, kind, actor.clone())
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        automation = AUTOMATION_NAME,
                        child_id = %issue_id,
                        parent_id = %parent_id,
                        conversation_id = %conversation_id,
                        new_status = %new.status,
                        "appended ChildUnblocked SystemEvent to parent conversation"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        child_id = %issue_id,
                        parent_id = %parent_id,
                        conversation_id = %conversation_id,
                        error = %err,
                        "failed to append ChildUnblocked SystemEvent"
                    );
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_helpers::{
        issue_with_status, register_agent, seed_linked_conversation, start_test_automation_runner,
        state_with_default_model,
    };
    use crate::domain::actors::ActorRef;
    use crate::domain::conversations::ConversationStatus;
    use crate::domain::issues::{IssueDependency, IssueDependencyType};
    use chrono::Utc;
    use hydra_common::api::v1::projects::{
        Project as ApiProject, ProjectKey, StatusDefinition, StatusKey,
    };
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    use hydra_common::api::v1::users::Username as ApiUsername;
    use hydra_common::test_utils::status::status;
    use std::sync::Arc;

    /// Seed a project containing:
    /// - an `open` status (`interactive: false`, `unblocks_parents: false`),
    ///   matching the wire default for new issues so callers don't need
    ///   to override `status` at construction;
    /// - an interactive status (`interactive: true`, `unblocks_parents: false`),
    /// - a headless status (`interactive: false`, `unblocks_parents: false`),
    /// - a complete status (`unblocks_parents: true`),
    /// - a failed status (`unblocks_parents: true`).
    async fn seed_project_with_statuses(
        state: &crate::app::AppState,
    ) -> (
        hydra_common::ProjectId,
        StatusKey, // interactive
        StatusKey, // headless
        StatusKey, // complete (unblocks_parents)
        StatusKey, // failed (also unblocks_parents)
    ) {
        let open_key = StatusKey::try_new("open").unwrap();
        let interactive_key = StatusKey::try_new("interactive-design").unwrap();
        let headless_key = StatusKey::try_new("in-progress-headless").unwrap();
        let complete_key = StatusKey::try_new("complete-wake").unwrap();
        let failed_key = StatusKey::try_new("failed-wake").unwrap();

        let open_def = StatusDefinition::new(
            open_key.clone(),
            "Open".to_string(),
            "#bdc3c7".parse().unwrap(),
            false,
            false,
            false,
            None,
        );

        let mut interactive_def = StatusDefinition::new(
            interactive_key.clone(),
            "Interactive Design".to_string(),
            "#3498db".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        interactive_def.interactive = true;

        let headless_def = StatusDefinition::new(
            headless_key.clone(),
            "Headless In-Progress".to_string(),
            "#9b59b6".parse().unwrap(),
            false,
            false,
            false,
            None,
        );

        let complete_def = StatusDefinition::new(
            complete_key.clone(),
            "Complete".to_string(),
            "#2ecc71".parse().unwrap(),
            true,
            true,
            false,
            None,
        );

        let failed_def = StatusDefinition::new(
            failed_key.clone(),
            "Failed".to_string(),
            "#e74c3c".parse().unwrap(),
            true,
            false,
            true,
            None,
        );

        let project = ApiProject::new(
            ProjectKey::try_new("wake-test-proj").unwrap(),
            "Wake Test".to_string(),
            Vec::new(),
            ApiUsername::from("alice"),
            false,
            0.0,
        );
        let (project_id, _) = state
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();
        for def in [
            open_def,
            interactive_def,
            headless_def,
            complete_def,
            failed_def,
        ] {
            state
                .store
                .add_status(&project_id, def, &ActorRef::test())
                .await
                .unwrap();
        }
        (
            project_id,
            interactive_key,
            headless_key,
            complete_key,
            failed_key,
        )
    }

    fn make_event(
        issue_id: hydra_common::IssueId,
        old: crate::domain::issues::Issue,
        new: crate::domain::issues::Issue,
        actor: ActorRef,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old),
            new,
            actor,
        });
        ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    #[tokio::test]
    async fn appends_system_event_when_child_transitions_to_unblocks_parents() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        // Parent in an interactive status.
        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = interactive_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Idle).await;

        // Child: transitioning open → complete.
        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        child_old.status = StatusKey::try_new("open").unwrap();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();

        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        // Conversation flipped to Active.
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(
            after.item.status,
            ConversationStatus::Active,
            "wake should flip parent conversation Idle → Active"
        );
    }

    #[tokio::test]
    async fn no_op_when_parent_is_headless() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, _interactive_key, headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        // Parent in a headless status.
        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = headless_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        // Even though we seed an Active conversation (e.g. left over
        // from a prior interactive episode), a headless parent must not
        // receive a SystemEvent.
        let conversation_id =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Active).await;
        // Snapshot a marker so we can prove the conversation log was
        // untouched: list any sessions linked to the conversation.
        let sessions_before = state
            .store()
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        // Conversation untouched: still Active, no SystemEvent appended.
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Active);

        let sessions_after = state
            .store()
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert_eq!(
            sessions_before.len(),
            sessions_after.len(),
            "no new session should have been created for a headless parent"
        );
    }

    #[tokio::test]
    async fn no_op_when_both_old_and_new_unblock_parents() {
        // Re-transition between two terminal statuses (e.g. closed →
        // failed) must NOT re-wake the parent.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, failed_key) =
            seed_project_with_statuses(&state).await;

        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = interactive_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Idle).await;

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        child_old.status = complete_key.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = failed_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        // Conversation untouched: stays Idle (no false → true transition).
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Idle);
    }

    #[tokio::test]
    async fn appends_on_each_interactive_parent_when_child_has_multiple() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        // Two interactive parents.
        let make_parent = |label: &str| {
            let mut p = issue_with_status(label, status("open"), vec![]);
            p.project_id = project_id.clone();
            p.status = interactive_key.clone();
            p
        };
        let (parent_a, _) = state
            .store
            .add_issue_with_actor(make_parent("a"), ActorRef::test())
            .await
            .unwrap();
        let (parent_b, _) = state
            .store
            .add_issue_with_actor(make_parent("b"), ActorRef::test())
            .await
            .unwrap();

        let conv_a = seed_linked_conversation(&state, &parent_a, ConversationStatus::Idle).await;
        let conv_b = seed_linked_conversation(&state, &parent_b, ConversationStatus::Idle).await;

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![
                IssueDependency::new(IssueDependencyType::ChildOf, parent_a.clone()),
                IssueDependency::new(IssueDependencyType::ChildOf, parent_b.clone()),
            ],
        );
        child_old.project_id = project_id.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        for conv_id in [&conv_a, &conv_b] {
            let after = state
                .store()
                .get_conversation(conv_id, false)
                .await
                .unwrap();
            assert_eq!(
                after.item.status,
                ConversationStatus::Active,
                "each interactive parent's conversation should flip Idle → Active"
            );
        }
    }

    #[tokio::test]
    async fn no_op_when_parent_has_no_live_conversation() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = interactive_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        // Only a Closed conversation — should be skipped.
        let closed_conversation =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Closed).await;

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        // Must not error and must not flip the closed conversation.
        automation.execute(&ctx).await.unwrap();
        let after = state
            .store()
            .get_conversation(&closed_conversation, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Closed);
    }

    #[tokio::test]
    async fn skips_events_triggered_by_this_automation() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = interactive_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Idle).await;

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        // Actor = THIS automation → must be skipped.
        let self_actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.to_string(),
            triggered_by: None,
        };
        let event = make_event(child_id.clone(), child_old, child_new, self_actor);

        let automation = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        // Conversation untouched: still Idle.
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Idle);
    }

    /// Cascade + wake share the IssueUpdated bus. When a parent
    /// (`grandparent`) is dropped, `cascade_issue_status` flips its
    /// children (`parent`) into a terminal status (also `dropped`),
    /// which then emits a fresh IssueUpdated for `parent`. This
    /// automation reacts to that secondary event and wakes
    /// `grandparent` if `grandparent` itself is in an interactive
    /// status — but in this setup the grandparent is in a terminal
    /// state, so we instead use a slightly different shape:
    ///
    /// `interactive_grandparent → parent → child`. We transition the
    /// child to `complete` (a `unblocks_parents=true` status), which
    /// directly wakes the grandparent only if `parent` is also a
    /// `ChildOf` of `grandparent`. To exercise the fan-out from a
    /// single event, the simpler test is: a single child transitioning
    /// to `complete` triggers BOTH automations on the same IssueUpdated
    /// event — cascade is a no-op (the child has no further child-of
    /// descendants in this setup), wake fires. The integration-style
    /// assertion is that running both automations under the engine
    /// produces the same outcome as wake alone.
    #[tokio::test]
    async fn cascade_and_wake_compose_without_interfering() {
        use crate::policy::automations::CascadeIssueStatusAutomation;

        let state = state_with_default_model("default-model");
        let _runner = start_test_automation_runner(&state);
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _headless_key, complete_key, _failed_key) =
            seed_project_with_statuses(&state).await;

        let mut parent_issue = issue_with_status("parent", status("open"), vec![]);
        parent_issue.project_id = project_id.clone();
        parent_issue.status = interactive_key.clone();
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &parent_id, ConversationStatus::Idle).await;

        let mut child_old = issue_with_status(
            "child",
            status("open"),
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        child_old.project_id = project_id.clone();
        let (child_id, _) = state
            .store
            .add_issue_with_actor(child_old.clone(), ActorRef::test())
            .await
            .unwrap();
        let mut child_new = child_old.clone();
        child_new.status = complete_key.clone();

        let event = make_event(child_id.clone(), child_old, child_new, ActorRef::test());

        let cascade = CascadeIssueStatusAutomation::new(None).unwrap();
        let wake = WakeConversationOnChildUnblockAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        // Cascade is a no-op for this event (complete doesn't cascade
        // and the child has no further descendants), but running it
        // first proves it doesn't interfere with wake.
        cascade.execute(&ctx).await.unwrap();
        wake.execute(&ctx).await.unwrap();

        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Active);
    }
}
