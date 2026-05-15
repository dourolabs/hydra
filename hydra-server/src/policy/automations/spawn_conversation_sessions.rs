use async_trait::async_trait;
use std::collections::HashMap;

use crate::app::event_bus::{EventType, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::conversations::{ConversationEvent, ConversationStatus};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::Status;
use hydra_common::ConversationId;
use hydra_common::api::v1::sessions::{BundleSpec, CreateSessionRequest, SearchSessionsQuery};

const AUTOMATION_NAME: &str = "spawn_conversation_sessions";

/// Event-driven automation that spawns interactive sessions for conversations.
///
/// Mirrors `SpawnSessionsAutomation` but for conversations: when a conversation
/// is created, updated, or receives an event, ensure that a session linked to
/// the conversation exists. The automation is idempotent — if an active session
/// already exists for the conversation it is a no-op.
///
/// The automation resolves the conversation's agent (the one named on the
/// conversation, or the registered default conversation agent) and uses the
/// agent's prompt as the session's prompt. If no agent can be resolved, a
/// warning is logged and the session is not spawned.
///
/// **Resume vs. fresh detection.** When spawning, the automation scans the
/// conversation's existing events from newest to oldest looking for the most
/// recent `Closed` / `Resumed` marker. If `Closed` appears first (i.e. the
/// conversation has been closed and not yet resumed), the spawn is a resume:
/// the automation records `conversation_resume_from = index_just_after_closed`
/// on the new session and appends a `Resumed` event. The "index just after
/// the most recent `Closed`" rule (rather than `events.len()`) is robust
/// against races where, for example, `send_message` appends a `UserMessage`
/// between the status flip and the automation firing — the resume snapshot
/// still points to the post-close boundary as it does in the legacy
/// synchronous path.
pub struct SpawnConversationSessionsAutomation;

impl SpawnConversationSessionsAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for SpawnConversationSessionsAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![
                EventType::ConversationCreated,
                EventType::ConversationUpdated,
                EventType::ConversationEventCreated,
            ],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        // Skip events triggered by this automation itself to avoid infinite loops.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

        let conversation_id = match conversation_id_for_event(ctx.event) {
            Some(id) => id,
            None => return Ok(()),
        };

        let conversation = match ctx.store.get_conversation(&conversation_id, false).await {
            Ok(v) => v.item,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    error = %err,
                    "failed to load conversation"
                );
                return Ok(());
            }
        };

        if conversation.deleted {
            return Ok(());
        }
        if conversation.status != ConversationStatus::Active {
            return Ok(());
        }

        // Resolve the agent (either the one named on the conversation or the
        // registered default). The agent's prompt is what we'll seed the
        // session with. Doing the lookup here means the automation owns the
        // prompt and we can emit a clear warning when no agent is configured.
        let agent = match ctx
            .app_state
            .resolve_conversation_agent(conversation.agent_name.as_deref())
            .await
        {
            Ok(Some(agent)) => agent,
            Ok(None) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    "no agent configured for conversation and no default conversation agent registered; not spawning session"
                );
                return Ok(());
            }
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    error = %err,
                    "failed to resolve conversation agent; not spawning session"
                );
                return Ok(());
            }
        };

        let agent_prompt = match ctx.app_state.resolve_agent_prompt(&agent.prompt_path).await {
            Ok(prompt) => prompt,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    agent = %agent.name,
                    error = %err,
                    "failed to resolve agent prompt; not spawning session"
                );
                return Ok(());
            }
        };

        // Determine whether this spawn is a resume (the conversation has been
        // closed previously) or a fresh start. Done before the idempotency
        // check so that a stale "active" session left over from before a
        // Closed event doesn't suppress the resume spawn.
        let events = match ctx.store.get_conversation_events(&conversation_id).await {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    error = %err,
                    "failed to load conversation events; not spawning session"
                );
                return Ok(());
            }
        };
        let resume_state = detect_resume_state(&events);

        // Idempotency: avoid double-spawning on a fresh conversation if an
        // active session already exists. We only enforce this for fresh
        // spawns — resume spawns must always proceed because any prior
        // "active" session for the conversation is stale (its worker died
        // when the conversation was closed; its status in the store hasn't
        // necessarily been transitioned to a terminal value yet). The
        // Resumed event we append at the end of the resume spawn serves as
        // the idempotency marker for subsequent triggers: once it's
        // present, `detect_resume_state` returns `Fresh`, so a follow-up
        // event won't double-spawn either.
        if matches!(resume_state, ResumeState::Fresh) {
            let query = SearchSessionsQuery::new(
                None,
                None,
                None,
                vec![
                    Status::Created.into(),
                    Status::Pending.into(),
                    Status::Running.into(),
                ],
            );
            let active_sessions = match ctx.app_state.list_sessions_with_query(&query).await {
                Ok(sessions) => sessions,
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        conversation_id = %conversation_id,
                        error = %err,
                        "failed to list sessions"
                    );
                    return Ok(());
                }
            };
            if active_sessions
                .iter()
                .any(|(_, s)| s.item.conversation_id() == Some(&conversation_id))
            {
                return Ok(());
            }
        }

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        let request = CreateSessionRequest::new(
            agent_prompt,
            None,
            BundleSpec::None,
            HashMap::new(),
            None,
            Some(conversation_id.clone()),
            true,
        );

        let session_id = match ctx
            .app_state
            .create_session(request, actor.clone(), conversation.creator.clone())
            .await
        {
            Ok(id) => id,
            Err(err) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    error = %err,
                    "failed to spawn conversation session"
                );
                return Ok(());
            }
        };

        if let ResumeState::Resume { pre_resume_count } = resume_state {
            // Stamp the resume snapshot on the new session so the worker
            // knows where to start replaying events from.
            match ctx.store.get_session(&session_id, false).await {
                Ok(versioned) => {
                    let mut session = versioned.item;
                    if let Some(opts) = session.interactive.as_mut() {
                        opts.conversation_resume_from = Some(pre_resume_count);
                    }
                    if let Err(err) = ctx
                        .app_state
                        .store
                        .update_session_with_actor(&session_id, session, actor.clone())
                        .await
                    {
                        tracing::warn!(
                            automation = AUTOMATION_NAME,
                            conversation_id = %conversation_id,
                            session_id = %session_id,
                            error = %err,
                            "failed to set conversation_resume_from on resumed session"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        conversation_id = %conversation_id,
                        session_id = %session_id,
                        error = %err,
                        "failed to load newly-spawned session to set conversation_resume_from"
                    );
                }
            }

            let resumed_event = ConversationEvent::Resumed {
                session_id: session_id.clone(),
                timestamp: chrono::Utc::now(),
            };
            if let Err(err) = ctx
                .app_state
                .store
                .append_conversation_event_with_actor(&conversation_id, resumed_event, actor)
                .await
            {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    conversation_id = %conversation_id,
                    session_id = %session_id,
                    error = %err,
                    "failed to append Resumed event"
                );
            }
        }

        tracing::info!(
            automation = AUTOMATION_NAME,
            conversation_id = %conversation_id,
            session_id = %session_id,
            resume = matches!(resume_state, ResumeState::Resume { .. }),
            "spawned conversation session"
        );

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResumeState {
    /// The conversation has been closed previously and is being resumed.
    /// `pre_resume_count` is the event index just after the most recent
    /// `Closed` event — i.e. the position the worker should start replaying
    /// from on the new session.
    Resume { pre_resume_count: usize },
    /// Fresh spawn — either the conversation has no events or the most
    /// recent terminal marker is already a `Resumed`.
    Fresh,
}

/// Decide whether a spawn is a resume or a fresh start by scanning events
/// from newest to oldest. The first `Closed`, `Suspending` (idle-suspend),
/// or `Resumed` we see wins:
///
/// - `Closed` or `Suspending` means the conversation has been terminated /
///   put to sleep and not yet resumed (this spawn is the resume).
/// - `Resumed` means the conversation has already been resumed by a prior
///   path (so this spawn is "fresh-like" — we leave
///   `conversation_resume_from` unset and append no further `Resumed`).
/// - No terminal marker at all means a fresh conversation (no prior worker
///   to "resume from").
fn detect_resume_state(events: &[hydra_common::Versioned<ConversationEvent>]) -> ResumeState {
    for (i, e) in events.iter().enumerate().rev() {
        match &e.item {
            ConversationEvent::Closed { .. } | ConversationEvent::Suspending { .. } => {
                return ResumeState::Resume {
                    pre_resume_count: i + 1,
                };
            }
            ConversationEvent::Resumed { .. } => return ResumeState::Fresh,
            _ => continue,
        }
    }
    ResumeState::Fresh
}

fn conversation_id_for_event(event: &ServerEvent) -> Option<ConversationId> {
    match event {
        ServerEvent::ConversationCreated {
            conversation_id, ..
        }
        | ServerEvent::ConversationUpdated {
            conversation_id, ..
        }
        | ServerEvent::ConversationEventCreated {
            conversation_id, ..
        } => Some(conversation_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::app::event_bus::MutationPayload;
    use crate::app::test_helpers::state_with_default_model;
    use crate::domain::actors::ActorRef;
    use crate::domain::agents::Agent;
    use crate::domain::conversations::{Conversation, ConversationEvent, ConversationStatus};
    use crate::domain::documents::Document;
    use crate::domain::issues::SessionSettings;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use chrono::Utc;
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    use std::sync::Arc;

    async fn register_default_conversation_agent(state: &AppState) {
        register_agent(state, "swe", "you are an SWE", true).await;
    }

    async fn register_agent(state: &AppState, name: &str, prompt_body: &str, is_default: bool) {
        let prompt_path = format!("/agents/{name}/prompt.md");
        let agent = Agent::new(
            name.to_string(),
            prompt_path.clone(),
            None,
            1,
            1,
            false,
            is_default,
            vec![],
        );
        state.store.add_agent(agent).await.unwrap();

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: prompt_body.to_string(),
            path: Some(prompt_path.parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();
    }

    fn make_conversation_with_status(
        agent_name: Option<&str>,
        status: ConversationStatus,
    ) -> Conversation {
        Conversation {
            title: None,
            agent_name: agent_name.map(String::from),
            status,
            creator: Username::from("creator"),
            session_settings: SessionSettings::default(),
            deleted: false,
        }
    }

    fn make_conversation(agent_name: Option<&str>) -> Conversation {
        make_conversation_with_status(agent_name, ConversationStatus::Active)
    }

    fn conversation_created_event(
        conversation_id: ConversationId,
        conversation: Conversation,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Conversation {
            old: None,
            new: conversation,
            actor: ActorRef::test(),
        });
        ServerEvent::ConversationCreated {
            seq: 1,
            conversation_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn conversation_updated_event(
        conversation_id: ConversationId,
        old: Conversation,
        new: Conversation,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Conversation {
            old: Some(old),
            new,
            actor: ActorRef::test(),
        });
        ServerEvent::ConversationUpdated {
            seq: 1,
            conversation_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn conversation_event_created_event(
        conversation_id: ConversationId,
        event: ConversationEvent,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::ConversationEvent {
            conversation_id: conversation_id.clone(),
            event,
            actor: ActorRef::test(),
        });
        ServerEvent::ConversationEventCreated {
            seq: 1,
            conversation_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    async fn sessions_for_conversation(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> usize {
        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        sessions
            .into_iter()
            .filter(|(_, s)| s.item.conversation_id() == Some(conversation_id))
            .count()
    }

    async fn spawned_session_prompt(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> Option<String> {
        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        sessions
            .into_iter()
            .find(|(_, s)| s.item.conversation_id() == Some(conversation_id))
            .map(|(_, s)| s.item.prompt.clone())
    }

    #[tokio::test]
    async fn spawns_on_conversation_created_when_no_session_exists() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);
    }

    #[tokio::test]
    async fn does_not_spawn_when_active_session_already_exists() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Drive a first spawn so an active session exists.
        let event = conversation_created_event(conversation_id.clone(), conversation.clone());
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();
        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);

        // A second invocation must not spawn another session.
        let event2 =
            conversation_updated_event(conversation_id.clone(), conversation.clone(), conversation);
        let ctx2 = AutomationContext {
            event: &event2,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx2).await.unwrap();
        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);
    }

    #[tokio::test]
    async fn does_not_spawn_when_conversation_status_is_closed() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 0);
    }

    #[tokio::test]
    async fn does_not_spawn_when_conversation_is_deleted() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let mut conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Mark the persisted conversation as deleted.
        conversation.deleted = true;
        state
            .store
            .update_conversation_with_actor(
                &conversation_id,
                conversation.clone(),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 0);
    }

    #[tokio::test]
    async fn does_not_spawn_when_no_agent_and_no_default() {
        let state = state_with_default_model("default-model");
        // No agents registered at all.

        let conversation = make_conversation(None);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 0);
    }

    #[tokio::test]
    async fn spawns_when_no_agent_but_default_conversation_agent_exists() {
        let state = state_with_default_model("default-model");
        register_default_conversation_agent(&state).await;

        let conversation = make_conversation(None);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);
    }

    #[tokio::test]
    async fn spawns_on_conversation_event_created() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_event_created_event(
            conversation_id.clone(),
            ConversationEvent::UserMessage {
                content: "hi".to_string(),
                timestamp: Utc::now(),
            },
        );
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);
    }

    #[tokio::test]
    async fn does_not_infinite_loop_on_follow_up_conversation_updated() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();

        // First invocation: spawns a session.
        let event = conversation_created_event(conversation_id.clone(), conversation.clone());
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();
        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);

        // Simulate a follow-up ConversationUpdated event. The active session
        // check must short-circuit and not spawn a second session.
        let event2 =
            conversation_updated_event(conversation_id.clone(), conversation.clone(), conversation);
        let ctx2 = AutomationContext {
            event: &event2,
            app_state: &state,
            store: state.store(),
        };
        automation.execute(&ctx2).await.unwrap();
        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 1);
    }

    #[tokio::test]
    async fn skips_events_triggered_by_this_automation() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Build an event whose payload actor is this automation. The
        // automation should bail before doing anything.
        let payload = Arc::new(MutationPayload::Conversation {
            old: None,
            new: conversation,
            actor: ActorRef::Automation {
                automation_name: AUTOMATION_NAME.to_string(),
                triggered_by: None,
            },
        });
        let event = ServerEvent::ConversationCreated {
            seq: 1,
            conversation_id: conversation_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();
        assert_eq!(sessions_for_conversation(&state, &conversation_id).await, 0);
    }

    #[tokio::test]
    async fn spawned_session_uses_agent_prompt() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "you are an SWE", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        assert_eq!(
            spawned_session_prompt(&state, &conversation_id).await,
            Some("you are an SWE".to_string())
        );
    }

    /// Helper that runs the automation once and returns the (single) session
    /// it spawned for the conversation.
    async fn run_and_get_session(
        state: &AppState,
        conversation_id: &ConversationId,
        event: &ServerEvent,
    ) -> crate::domain::sessions::Session {
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event,
            app_state: state,
            store: state.store(),
        };
        automation.execute(&ctx).await.unwrap();

        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        sessions
            .into_iter()
            .find(|(_, s)| s.item.conversation_id() == Some(conversation_id))
            .map(|(_, s)| s.item)
            .expect("expected a session for the conversation")
    }

    #[tokio::test]
    async fn fresh_spawn_does_not_set_conversation_resume_from_or_append_resumed() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_created_event(conversation_id.clone(), conversation);
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        // No conversation_resume_from on a fresh spawn.
        let opts = session
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(opts.conversation_resume_from, None);

        // No Resumed event on a fresh conversation.
        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e.item, ConversationEvent::Resumed { .. })),
            "fresh conversation should not have a Resumed event"
        );
    }

    #[tokio::test]
    async fn resume_spawn_sets_conversation_resume_from_and_appends_resumed_event() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Simulate a closed conversation with prior history: UserMessage then Closed.
        state
            .store
            .append_conversation_event_with_actor(
                &conversation_id,
                ConversationEvent::UserMessage {
                    content: "hello".into(),
                    timestamp: Utc::now(),
                },
                ActorRef::test(),
            )
            .await
            .unwrap();
        state
            .store
            .append_conversation_event_with_actor(
                &conversation_id,
                ConversationEvent::Closed {
                    timestamp: Utc::now(),
                },
                ActorRef::test(),
            )
            .await
            .unwrap();

        // The events count just before the automation fires is the expected
        // conversation_resume_from value (index just after Closed).
        let expected_resume_from = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap()
            .len();

        // Drive the automation via a synthetic ConversationUpdated event (a
        // Closed→Active flip is what callers will produce in practice).
        let event = conversation_updated_event(
            conversation_id.clone(),
            make_conversation_with_status(Some("swe"), ConversationStatus::Closed),
            make_conversation(Some("swe")),
        );
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        let opts = session
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(opts.conversation_resume_from, Some(expected_resume_from));

        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let resumed = events
            .iter()
            .find_map(|e| match &e.item {
                ConversationEvent::Resumed { session_id, .. } => Some(session_id.clone()),
                _ => None,
            })
            .expect("expected a Resumed event after resume spawn");
        // The Resumed event's session_id must match the newly-spawned session.
        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        let session_id = sessions
            .into_iter()
            .find(|(_, s)| s.item.conversation_id() == Some(&conversation_id))
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(resumed, session_id);
    }

    #[tokio::test]
    async fn resume_spawn_uses_position_after_last_closed_when_user_message_races_in() {
        // Simulates the race where, between status flip (Closed → Active) and
        // the automation firing, the caller has already appended a follow-up
        // UserMessage. The resume snapshot must still point to the post-close
        // boundary, not to events.len() at automation time.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        for ev in [
            ConversationEvent::UserMessage {
                content: "first".into(),
                timestamp: Utc::now(),
            },
            ConversationEvent::Closed {
                timestamp: Utc::now(),
            },
            ConversationEvent::UserMessage {
                content: "follow-up".into(),
                timestamp: Utc::now(),
            },
        ] {
            state
                .store
                .append_conversation_event_with_actor(&conversation_id, ev, ActorRef::test())
                .await
                .unwrap();
        }

        // The expected resume index is the position just after the Closed
        // event (= 2), NOT events.len() (= 3).
        let event = conversation_updated_event(
            conversation_id.clone(),
            make_conversation_with_status(Some("swe"), ConversationStatus::Closed),
            make_conversation(Some("swe")),
        );
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        let opts = session
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(opts.conversation_resume_from, Some(2));
    }

    #[tokio::test]
    async fn does_not_append_second_resumed_when_one_already_exists() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Conversation history ending in a Resumed marker (e.g. another path
        // already appended one).
        let fake_session_id = hydra_common::SessionId::new();
        for ev in [
            ConversationEvent::Closed {
                timestamp: Utc::now(),
            },
            ConversationEvent::Resumed {
                session_id: fake_session_id,
                timestamp: Utc::now(),
            },
        ] {
            state
                .store
                .append_conversation_event_with_actor(&conversation_id, ev, ActorRef::test())
                .await
                .unwrap();
        }

        let event =
            conversation_updated_event(conversation_id.clone(), conversation.clone(), conversation);
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        // Treated as fresh-like — no resume snapshot stamped.
        let opts = session
            .interactive
            .as_ref()
            .expect("session should be interactive");
        assert_eq!(opts.conversation_resume_from, None);

        // And no additional Resumed event appended.
        let events = state
            .store()
            .get_conversation_events(&conversation_id)
            .await
            .unwrap();
        let resumed_count = events
            .iter()
            .filter(|e| matches!(e.item, ConversationEvent::Resumed { .. }))
            .count();
        assert_eq!(resumed_count, 1);
    }
}
