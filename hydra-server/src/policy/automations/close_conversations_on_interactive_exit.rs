//! `close_conversations_on_interactive_exit` — closes any live
//! [`Conversation`] linked to an issue when that issue's status flips out
//! of a status definition marked `interactive: true`.
//!
//! `AgentQueue` mints a `Conversation` (instead of a headless `Session`)
//! when an issue lands in an interactive status. The reverse direction —
//! the user (or PM) flipping the issue back to a non-interactive status
//! mid-flight — is captured here: each live conversation linked back to
//! the issue via `spawned_from` is closed through the existing
//! `AppState::close_conversation` path so the worker can shut down cleanly
//! instead of dangling.

use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::conversations::ConversationStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::api::v1::conversations::SearchConversationsQuery;

const AUTOMATION_NAME: &str = "close_conversations_on_interactive_exit";

pub struct CloseConversationsOnInteractiveExitAutomation;

impl CloseConversationsOnInteractiveExitAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for CloseConversationsOnInteractiveExitAutomation {
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
        // Skip events triggered by this automation to avoid loops.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

        let (issue_id, payload) = match ctx.event {
            ServerEvent::IssueUpdated {
                issue_id, payload, ..
            } => (issue_id, payload),
            _ => return Ok(()),
        };

        let MutationPayload::Issue { old, new, .. } = payload.as_ref() else {
            return Ok(());
        };

        // Only fire on status transitions. Other updates (description edits,
        // assignee changes, etc.) don't affect interactive-mode lifecycle.
        let Some(old) = old.as_ref() else {
            return Ok(());
        };
        if old.status == new.status {
            return Ok(());
        }

        // If we can't resolve the new status definition, log and treat it
        // as non-interactive — that's the safer direction (drop the
        // conversation) when configuration is ambiguous; a stuck-Active
        // conversation that nothing on the issue can flip back is worse
        // than a no-op close.
        let new_is_interactive = match ctx.app_state.resolve_status(new).await {
            Ok(def) => def.interactive,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    status = %new.status,
                    error = %err,
                    "failed to resolve new status; treating as non-interactive"
                );
                false
            }
        };
        if new_is_interactive {
            return Ok(());
        }

        // Find linked conversations and close any that are not already Closed.
        let query = SearchConversationsQuery {
            spawned_from: Some(issue_id.clone()),
            include_deleted: Some(false),
            ..Default::default()
        };
        let conversations = match ctx.store.list_conversations(&query).await {
            Ok(rows) => rows,
            Err(err) => {
                return Err(AutomationError::Other(anyhow::anyhow!(
                    "[{AUTOMATION_NAME}] failed to list conversations for issue {issue_id}: {err}"
                )));
            }
        };

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };
        for (conversation_id, versioned) in conversations {
            if versioned.item.status == ConversationStatus::Closed {
                continue;
            }
            match ctx
                .app_state
                .close_conversation(&conversation_id, actor.clone())
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        conversation_id = %conversation_id,
                        old_status = %old.status,
                        new_status = %new.status,
                        "closed conversation on issue exit from interactive status"
                    );
                }
                Err(err) => {
                    // Treat individual close failures as warnings so other
                    // conversations linked to the same issue still get a
                    // chance. The runner will surface persistent failures
                    // via the surrounding event loop.
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        conversation_id = %conversation_id,
                        error = %err,
                        "failed to close conversation on interactive exit"
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
    use crate::app::test_helpers::{issue_with_status, state_with_default_model};
    use crate::domain::actors::ActorRef;
    use crate::domain::agents::Agent;
    use crate::domain::conversations::{Conversation, ConversationStatus};
    use crate::domain::documents::Document;
    use crate::domain::issues::{IssueStatus, SessionSettings};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use chrono::Utc;
    use hydra_common::api::v1::agents::AgentName;
    use hydra_common::api::v1::projects::{
        Project as ApiProject, ProjectKey, StatusDefinition, StatusKey,
    };
    use hydra_common::api::v1::users::Username as ApiUsername;
    use std::sync::Arc;

    async fn register_agent(state: &crate::app::AppState, name: &str) {
        let prompt_path = format!("/agents/{name}/prompt.md");
        let agent = Agent::new(
            name.to_string(),
            prompt_path.clone(),
            None,
            1,
            1,
            false,
            false,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();
        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: "agent prompt body".to_string(),
            path: Some(prompt_path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();
    }

    /// Seed a project with one interactive status and one non-interactive
    /// status. Returns the project id and the two status keys.
    async fn seed_project_with_interactive(
        state: &crate::app::AppState,
    ) -> (hydra_common::ProjectId, StatusKey, StatusKey) {
        let interactive_key = StatusKey::try_new("interactive-design").unwrap();
        let backlog_key = StatusKey::try_new("backlog").unwrap();
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
        let backlog_def = StatusDefinition::new(
            backlog_key.clone(),
            "Backlog".to_string(),
            "#9b59b6".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        let project = ApiProject::new(
            ProjectKey::try_new("engineering-v2").unwrap(),
            "Engineering v2".to_string(),
            vec![interactive_def, backlog_def],
            backlog_key.clone(),
            ApiUsername::from("alice"),
            false,
            0.0,
        );
        let (project_id, _) = state
            .store
            .add_project(project, &ActorRef::test())
            .await
            .unwrap();
        (project_id, interactive_key, backlog_key)
    }

    fn make_event(
        issue_id: hydra_common::IssueId,
        old: crate::domain::issues::Issue,
        new: crate::domain::issues::Issue,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old),
            new,
            actor: ActorRef::test(),
        });
        ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    async fn seed_linked_conversation(
        state: &crate::app::AppState,
        issue_id: &hydra_common::IssueId,
        status: ConversationStatus,
    ) -> hydra_common::ConversationId {
        let conversation = Conversation {
            title: None,
            agent_name: Some(AgentName::try_new("swe").unwrap()),
            status,
            creator: Username::from("creator"),
            session_settings: SessionSettings::default(),
            spawned_from: Some(issue_id.clone()),
            deleted: false,
        };
        let (id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn closes_linked_conversation_when_status_exits_interactive() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, backlog_key) =
            seed_project_with_interactive(&state).await;

        let mut issue = issue_with_status("test", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = interactive_key.clone();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Active).await;

        // Flip to non-interactive backlog.
        let mut updated = issue.clone();
        updated.status = backlog_key.clone();
        let event = make_event(issue_id.clone(), issue, updated);

        let automation = CloseConversationsOnInteractiveExitAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Closed);
    }

    #[tokio::test]
    async fn does_not_close_when_new_status_is_interactive() {
        // Flipping interactive → interactive (e.g. between two interactive
        // statuses, or technically the same one) should not collapse the
        // live conversation.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _backlog_key) =
            seed_project_with_interactive(&state).await;

        let mut issue = issue_with_status("test", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = StatusKey::try_new("open").unwrap();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Active).await;

        // Flip into interactive.
        let mut updated = issue.clone();
        updated.status = interactive_key.clone();
        let event = make_event(issue_id.clone(), issue, updated);

        let automation = CloseConversationsOnInteractiveExitAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn no_op_when_status_unchanged() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, _) = seed_project_with_interactive(&state).await;

        let mut issue = issue_with_status("test", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = interactive_key.clone();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Active).await;

        // No status change (e.g. a title-only edit).
        let event = make_event(issue_id.clone(), issue.clone(), issue);

        let automation = CloseConversationsOnInteractiveExitAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn does_not_touch_already_closed_conversation() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, backlog_key) =
            seed_project_with_interactive(&state).await;

        let mut issue = issue_with_status("test", IssueStatus::Open, vec![]);
        issue.project_id = Some(project_id);
        issue.status = interactive_key.clone();
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Closed).await;

        let mut updated = issue.clone();
        updated.status = backlog_key.clone();
        let event = make_event(issue_id.clone(), issue, updated);

        let automation = CloseConversationsOnInteractiveExitAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        // Still Closed.
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Closed);
    }

    #[tokio::test]
    async fn ignores_unrelated_issues() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;
        let (project_id, interactive_key, backlog_key) =
            seed_project_with_interactive(&state).await;

        let mut issue_a = issue_with_status("a", IssueStatus::Open, vec![]);
        issue_a.project_id = Some(project_id.clone());
        issue_a.status = interactive_key.clone();
        let (issue_a_id, _) = state
            .store
            .add_issue_with_actor(issue_a.clone(), ActorRef::test())
            .await
            .unwrap();

        let mut issue_b = issue_with_status("b", IssueStatus::Open, vec![]);
        issue_b.project_id = Some(project_id);
        issue_b.status = interactive_key.clone();
        let (issue_b_id, _) = state
            .store
            .add_issue_with_actor(issue_b, ActorRef::test())
            .await
            .unwrap();

        let conv_a =
            seed_linked_conversation(&state, &issue_a_id, ConversationStatus::Active).await;
        let conv_b =
            seed_linked_conversation(&state, &issue_b_id, ConversationStatus::Active).await;

        // Only flip A out of interactive.
        let mut updated_a = issue_a.clone();
        updated_a.status = backlog_key.clone();
        let event = make_event(issue_a_id.clone(), issue_a, updated_a);

        let automation = CloseConversationsOnInteractiveExitAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        let after_a = state
            .store()
            .get_conversation(&conv_a, false)
            .await
            .unwrap();
        let after_b = state
            .store()
            .get_conversation(&conv_b, false)
            .await
            .unwrap();
        assert_eq!(after_a.item.status, ConversationStatus::Closed);
        assert_eq!(
            after_b.item.status,
            ConversationStatus::Active,
            "unrelated linked conversation should be untouched"
        );
    }
}
