use async_trait::async_trait;
use std::collections::HashMap;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::conversations::{Conversation, ConversationEvent, ConversationStatus};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::ConversationId;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::sessions::{AgentSpec, CreateSessionRequest, SessionMode};
use hydra_common::constants::ENV_HYDRA_CONVERSATION_ID;

const AUTOMATION_NAME: &str = "spawn_conversation_sessions";

/// Event-driven automation that keeps conversation status and the existence of
/// a companion interactive session in sync.
///
/// `Active` is treated as the invariant: a conversation in `Active` status has
/// a session running for it. To preserve that invariant the automation handles
/// two directions:
///
/// - **Spawn side.** Spawn exactly one session whenever the conversation
///   transitions *into* `Active`: either on `ConversationCreated` with
///   `new.status == Active` (fresh) or on `ConversationUpdated` with
///   `old.status != Active && new.status == Active` (resume from Idle / Closed).
///   `ConversationEventCreated` is *not* a trigger — appending a `UserMessage`
///   to an already-Active conversation does not produce a second spawn.
/// - **Idle-flip side.** When the companion session transitions to a terminal
///   status (`Complete` / `Failed`), flip the conversation `Active → Idle`.
///   `Closed` is preserved (manual user action).
///
/// The transition trigger makes duplicate-spawn races structurally impossible:
/// each `Idle → Active` flip produces exactly one `ConversationUpdated` event,
/// and that one event is the sole spawn signal.
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
                EventType::SessionUpdated,
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

        match ctx.event {
            ServerEvent::ConversationCreated {
                conversation_id,
                payload,
                ..
            } => {
                let MutationPayload::Conversation { new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                if new.status != ConversationStatus::Active {
                    return Ok(());
                }
                spawn_session(ctx, conversation_id, new, None).await?;
            }
            ServerEvent::ConversationUpdated {
                conversation_id,
                payload,
                ..
            } => {
                let MutationPayload::Conversation { old, new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                let Some(old) = old else {
                    return Ok(());
                };
                if old.status == ConversationStatus::Active
                    || new.status != ConversationStatus::Active
                {
                    return Ok(());
                }
                let resume_from = compute_resume_index(ctx, conversation_id).await;
                spawn_session(ctx, conversation_id, new, resume_from).await?;
            }
            ServerEvent::SessionUpdated { payload, .. } => {
                let MutationPayload::Session { old, new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                let Some(old) = old else {
                    return Ok(());
                };
                if !new.status.is_terminal() || old.status.is_terminal() {
                    return Ok(());
                }
                let Some(conversation_id) = new.conversation_id().cloned() else {
                    return Ok(());
                };
                flip_conversation_to_idle(ctx, conversation_id).await;
            }
            _ => {}
        }

        Ok(())
    }
}

/// Spawn a session for a conversation that has just entered `Active` status.
///
/// `resume_from` is `Some(index)` for resume spawns (the worker should replay
/// from that event index) and `None` for fresh spawns. When `resume_from` is
/// `Some`, the spawned session's `conversation_resume_from` is stamped and a
/// `Resumed` event is appended to the conversation log.
///
/// Returns `Err` for misconfiguration cases that prevent a spawn (no agent
/// available, agent prompt missing, `create_session` fails). The runner
/// surfaces these via its `automation failed` error log so they aren't lost
/// in the noise of normal `tracing::warn!` output — this is the primary
/// debugging hook for operators who list `spawn_conversation_sessions` in
/// `policies.automations` but observe no session being created.
async fn spawn_session(
    ctx: &AutomationContext<'_>,
    conversation_id: &ConversationId,
    conversation: &Conversation,
    resume_from: Option<usize>,
) -> Result<(), AutomationError> {
    if conversation.deleted {
        return Ok(());
    }

    let agent = match ctx
        .app_state
        .resolve_conversation_agent(conversation.agent_name.as_ref().map(|n| n.as_str()))
        .await
    {
        Ok(Some(agent)) => agent,
        Ok(None) => {
            return Err(AutomationError::Other(anyhow::anyhow!(
                "[{AUTOMATION_NAME}] cannot spawn session for conversation {conversation_id}: \
                 the conversation has no `agent_name` and no agent is registered with \
                 `is_default_conversation_agent: true`. Register one with \
                 `POST /v1/agents` (e.g., `is_default_conversation_agent: true`) or pass \
                 `agent_name` on `POST /v1/conversations`."
            )));
        }
        Err(err) => {
            return Err(AutomationError::Other(anyhow::anyhow!(
                "[{AUTOMATION_NAME}] failed to resolve agent for conversation \
                 {conversation_id}: {err}"
            )));
        }
    };

    // Validate the agent's stored name early so we can hand it to the
    // server as an `AgentSpec::Named { name }`. The server's
    // `AgentSpec::Named` branch will load the agent row a second time and
    // resolve its prompt / mcp_config / secrets — this automation is no
    // longer responsible for any of that.
    let agent_name = AgentName::try_new(agent.name.clone()).map_err(|source| {
        AutomationError::Other(anyhow::anyhow!(
            "[{AUTOMATION_NAME}] agent '{}' has invalid name in the store: {source}",
            agent.name
        ))
    })?;

    let conversation_settings = ctx
        .app_state
        .apply_session_settings_defaults(conversation.session_settings.clone());

    let mount_spec = match ctx
        .app_state
        .resolve_mount_spec(&conversation_settings)
        .await
    {
        Ok(spec) => spec,
        Err(err) => {
            return Err(AutomationError::Other(anyhow::anyhow!(
                "[{AUTOMATION_NAME}] failed to resolve mount_spec for conversation \
                 {conversation_id}: {err}"
            )));
        }
    };

    let actor = ActorRef::Automation {
        automation_name: AUTOMATION_NAME.into(),
        triggered_by: Some(Box::new(ctx.actor().clone())),
    };

    let mut env_vars = HashMap::new();
    env_vars.insert(
        ENV_HYDRA_CONVERSATION_ID.to_string(),
        conversation_id.to_string(),
    );

    // On the resume path, look up the most-recently-created prior session for
    // this conversation so we can carry the lineage edge directly through
    // `CreateSessionRequest` instead of a follow-up update.
    let prior_session_id = if resume_from.is_some() {
        find_prior_session_id(ctx, conversation_id).await
    } else {
        None
    };

    let request = CreateSessionRequest {
        mode: SessionMode::Interactive {
            conversation_id: conversation_id.clone(),
            idle_timeout_secs: None,
            conversation_resume_from: resume_from,
        },
        agent_config: AgentSpec::Named { name: agent_name },
        model: conversation_settings.model.clone(),
        mount_spec,
        image: conversation_settings.image.clone(),
        env_vars,
        cpu_limit: conversation_settings.cpu_limit.clone(),
        memory_limit: conversation_settings.memory_limit.clone(),
        secrets: conversation_settings.secrets.clone(),
        spawned_from: None,
        resumed_from: prior_session_id.clone(),
    };

    let session_id = match ctx
        .app_state
        .create_session(request, actor.clone(), conversation.creator.clone())
        .await
    {
        Ok((id, _session)) => id,
        Err(err) => {
            return Err(AutomationError::Other(anyhow::anyhow!(
                "[{AUTOMATION_NAME}] failed to create session for conversation \
                 {conversation_id}: {err}"
            )));
        }
    };

    if resume_from.is_some() {
        let resumed_timestamp = chrono::Utc::now();
        let resumed_event = ConversationEvent::Resumed {
            session_id: session_id.clone(),
            timestamp: resumed_timestamp,
        };
        if let Err(err) = ctx
            .app_state
            .store
            .append_conversation_event_with_actor(conversation_id, resumed_event, actor.clone())
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

        // Dual-write the SessionEvent::Resumed marker onto the newly-spawned
        // session, carrying the prior session id (Phase C step 7 of the
        // sessions-orthogonality redesign, §3.2 mapping rule).
        if let Some(from_session_id) = prior_session_id {
            let session_event = crate::domain::sessions::SessionEvent::Resumed {
                from_session_id,
                timestamp: resumed_timestamp,
            };
            let _ = crate::app::chat_relay::dual_write_session_event(
                ctx.app_state,
                &session_id,
                session_event,
                actor,
            )
            .await;
        } else {
            tracing::warn!(
                automation = AUTOMATION_NAME,
                conversation_id = %conversation_id,
                session_id = %session_id,
                "dual-write SessionEvent::Resumed skipped: no prior session found for conversation"
            );
        }
    }

    tracing::info!(
        automation = AUTOMATION_NAME,
        conversation_id = %conversation_id,
        session_id = %session_id,
        resume = resume_from.is_some(),
        "spawned conversation session"
    );

    Ok(())
}

/// Find the most-recently-created session linked to `conversation_id`.
/// Returns `None` if no such session exists (e.g. a fresh conversation that
/// has never been resumed). Callers must invoke this BEFORE creating the new
/// session — otherwise the newly-created session is the most recent, and the
/// helper returns it instead of the prior one.
async fn find_prior_session_id(
    ctx: &AutomationContext<'_>,
    conversation_id: &ConversationId,
) -> Option<hydra_common::SessionId> {
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    let mut query = SearchSessionsQuery::default();
    query.conversation_id = Some(conversation_id.clone());
    let sessions = ctx.store.list_sessions(&query).await.ok()?;
    sessions
        .into_iter()
        .max_by_key(|(_, v)| v.creation_time)
        .map(|(id, _)| id)
}

/// Compute the `conversation_resume_from` index for a resume spawn.
///
/// Scans events newest-to-oldest:
///   - First `Closed` / `Suspending` found ⇒ index just after it (the worker
///     resumes from the post-terminal boundary).
///   - First `Resumed` found ⇒ `events.len()` (the prior resume already
///     consumed the snapshot; start at the current end).
///   - No marker found ⇒ `events.len()`.
async fn compute_resume_index(
    ctx: &AutomationContext<'_>,
    conversation_id: &ConversationId,
) -> Option<usize> {
    let events = match ctx.store.get_conversation_events(conversation_id).await {
        Ok(events) => events,
        Err(err) => {
            tracing::warn!(
                automation = AUTOMATION_NAME,
                conversation_id = %conversation_id,
                error = %err,
                "failed to load conversation events; resume snapshot will use events.len()"
            );
            return None;
        }
    };
    if let Some((i, e)) = events.iter().enumerate().next_back() {
        match &e.item {
            ConversationEvent::Closed { .. } | ConversationEvent::Suspending { .. } => {
                return Some(i + 1);
            }
            ConversationEvent::Resumed { .. } => return Some(events.len()),
        }
    }
    Some(events.len())
}

/// On a session's terminal transition, flip the conversation's status from
/// `Active` to `Idle`. Other statuses (`Closed`, `Idle`) and deleted
/// conversations are left untouched.
async fn flip_conversation_to_idle(ctx: &AutomationContext<'_>, conversation_id: ConversationId) {
    let versioned = match ctx.store.get_conversation(&conversation_id, false).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                automation = AUTOMATION_NAME,
                conversation_id = %conversation_id,
                error = %err,
                "failed to load conversation for idle flip"
            );
            return;
        }
    };
    let mut conversation = versioned.item;
    if conversation.deleted || conversation.status != ConversationStatus::Active {
        return;
    }
    conversation.status = ConversationStatus::Idle;
    let actor = ActorRef::Automation {
        automation_name: AUTOMATION_NAME.into(),
        triggered_by: Some(Box::new(ctx.actor().clone())),
    };
    if let Err(err) = ctx
        .app_state
        .store
        .update_conversation_with_actor(&conversation_id, conversation, actor)
        .await
    {
        tracing::warn!(
            automation = AUTOMATION_NAME,
            conversation_id = %conversation_id,
            error = %err,
            "failed to flip conversation to Idle on session terminal"
        );
        return;
    }
    tracing::info!(
        automation = AUTOMATION_NAME,
        conversation_id = %conversation_id,
        "flipped conversation to Idle on companion session terminal"
    );
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
    use crate::domain::sessions::Session;
    use crate::domain::task_status::Status as TaskStatus;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::routes::sessions::mount_spec_from_create_request;
    use chrono::Utc;
    use hydra_common::SessionId;
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
        use hydra_common::api::v1::agents::AgentName;
        Conversation {
            title: None,
            agent_name: agent_name.map(|n| AgentName::try_new(n).expect("valid agent name")),
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

    fn make_interactive_session(
        status: TaskStatus,
        conversation_id: Option<ConversationId>,
    ) -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        let mode = match conversation_id {
            Some(cid) => SessionMode::Interactive {
                conversation_id: cid,
                idle_timeout_secs: None,
                conversation_resume_from: None,
            },
            None => SessionMode::Headless,
        };
        Session::new(
            Username::from("creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            mode,
            status,
            None,
            None,
        )
    }

    fn session_updated_event(session_id: SessionId, old: Session, new: Session) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Session {
            old: Some(old),
            new,
            actor: ActorRef::test(),
        });
        ServerEvent::SessionUpdated {
            seq: 1,
            session_id,
            version: 2,
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
            .map(|(_, s)| s.item.resolved_prompt().to_string())
    }

    #[tokio::test]
    async fn spawns_on_conversation_created_with_active_status() {
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
    async fn does_not_spawn_on_conversation_event_created() {
        // ConversationEventCreated is no longer in the EventFilter — but
        // even if it leaked through, the match arm falls through to the no-op
        // case. Verify by constructing the event explicitly.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::ConversationEvent {
            conversation_id: conversation_id.clone(),
            event: ConversationEvent::Suspending {
                reason: "idle_timeout".to_string(),
                timestamp: Utc::now(),
            },
            actor: ActorRef::test(),
        });
        let event = ServerEvent::ConversationEventCreated {
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
    async fn spawns_on_conversation_updated_transitioning_idle_to_active() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Idle);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
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
    async fn spawns_on_conversation_updated_transitioning_closed_to_active() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
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
    async fn does_not_spawn_on_conversation_updated_when_status_already_active() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // No-op update (Active → Active, e.g. a title change).
        let event =
            conversation_updated_event(conversation_id.clone(), conversation.clone(), conversation);
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
    async fn does_not_spawn_on_conversation_updated_into_non_active_status() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let active = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(active.clone(), ActorRef::test())
            .await
            .unwrap();

        // Active → Idle: not a spawn trigger.
        let event = conversation_updated_event(
            conversation_id.clone(),
            active,
            make_conversation_with_status(Some("swe"), ConversationStatus::Idle),
        );
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
    async fn does_not_spawn_when_conversation_status_is_closed_on_create() {
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
        // Reproduces the user-reported "spawn_conversation_sessions does not
        // seem to be active" scenario: the automation IS wired and IS firing,
        // but it cannot resolve an agent for the conversation (no `agent_name`
        // on the conversation, no default conversation agent registered) so
        // it bails. The bail must propagate as `Err(AutomationError)` so the
        // runner surfaces it via `tracing::error!("automation failed")` —
        // previously this was a silent `tracing::warn!` that operators missed
        // when filtering server logs.
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

        let err = automation
            .execute(&ctx)
            .await
            .expect_err("expected an error when no agent can be resolved");
        let msg = err.to_string();
        assert!(
            msg.contains("is_default_conversation_agent"),
            "error message should point operators at the registration knob; got: {msg}"
        );
        assert!(
            msg.contains("agent_name"),
            "error message should mention the request-side knob too; got: {msg}"
        );

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
    async fn skips_events_triggered_by_this_automation() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

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

    async fn spawned_session_env_vars(
        state: &AppState,
        conversation_id: &ConversationId,
    ) -> Option<HashMap<String, String>> {
        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        sessions
            .into_iter()
            .find(|(_, s)| s.item.conversation_id() == Some(conversation_id))
            .map(|(_, s)| s.item.env_vars.clone())
    }

    #[tokio::test]
    async fn fresh_spawn_sets_conversation_id_env_var() {
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

        let env_vars = spawned_session_env_vars(&state, &conversation_id)
            .await
            .expect("expected a session for the conversation");
        assert_eq!(
            env_vars.get(ENV_HYDRA_CONVERSATION_ID),
            Some(&conversation_id.to_string())
        );
    }

    #[tokio::test]
    async fn resume_spawn_sets_conversation_id_env_var() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
        );
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        let env_vars = spawned_session_env_vars(&state, &conversation_id)
            .await
            .expect("expected a session for the conversation");
        assert_eq!(
            env_vars.get(ENV_HYDRA_CONVERSATION_ID),
            Some(&conversation_id.to_string())
        );
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
    ) -> Session {
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
        assert!(session.is_interactive(), "session should be interactive");
        assert_eq!(session.mode.conversation_resume_from(), None);

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

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Simulate a closed conversation with prior history: Suspending then Closed.
        state
            .store
            .append_conversation_event_with_actor(
                &conversation_id,
                ConversationEvent::Suspending {
                    reason: "idle_timeout".into(),
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

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
        );
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        assert!(session.is_interactive(), "session should be interactive");
        assert_eq!(
            session.mode.conversation_resume_from(),
            Some(expected_resume_from)
        );

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
    async fn resume_spawn_sets_resumed_from_to_prior_session_id() {
        // Regression: the session-level `resumed_from` lineage edge must
        // point at the most-recently-created prior session for the
        // conversation. Today's other resume-flow tests cover the
        // `ConversationEvent::Resumed` side-channel but not the session
        // field itself.
        use crate::domain::sessions::{AgentConfig, SessionMode as DomainSessionMode};
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        // Seed a prior session linked to the conversation that the resume
        // spawn should pick up.
        let prior_session = Session::new(
            Username::from("creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            DomainSessionMode::Interactive {
                conversation_id: conversation_id.clone(),
                idle_timeout_secs: None,
                conversation_resume_from: None,
            },
            TaskStatus::Complete,
            None,
            None,
        );
        let prior_session_id = state
            .add_session(prior_session, Utc::now(), ActorRef::test())
            .await
            .unwrap();

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
        );
        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        // Find the newly-spawned (= newest) session and check its
        // `resumed_from` points at the seeded prior session id.
        let sessions = state
            .list_sessions_with_query(&SearchSessionsQuery::default())
            .await
            .unwrap();
        let new_session = sessions
            .into_iter()
            .filter(|(id, _)| id != &prior_session_id)
            .find(|(_, s)| s.item.conversation_id() == Some(&conversation_id))
            .map(|(_, s)| s.item)
            .expect("expected a newly-spawned session for the conversation");
        assert_eq!(new_session.resumed_from, Some(prior_session_id));
    }

    #[tokio::test]
    async fn resume_spawn_uses_position_after_most_recent_lifecycle_event() {
        // `compute_resume_index` scans the conversation events log newest-to-
        // oldest and returns the index just after the first Suspending/Closed
        // it finds. Post-Phase-E step 18 the log carries only lifecycle
        // events (Suspending/Resumed/Closed), so the previous "UserMessage
        // races in after Closed" scenario no longer applies. This regression
        // test guards the boundary-after-most-recent-lifecycle invariant
        // directly: Suspending → Closed → Suspending yields
        // `events.len() == 3`.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation.clone(), ActorRef::test())
            .await
            .unwrap();

        for ev in [
            ConversationEvent::Suspending {
                reason: "idle_timeout".into(),
                timestamp: Utc::now(),
            },
            ConversationEvent::Closed {
                timestamp: Utc::now(),
            },
            ConversationEvent::Suspending {
                reason: "sigterm".into(),
                timestamp: Utc::now(),
            },
        ] {
            state
                .store
                .append_conversation_event_with_actor(&conversation_id, ev, ActorRef::test())
                .await
                .unwrap();
        }

        let event = conversation_updated_event(
            conversation_id.clone(),
            conversation,
            make_conversation(Some("swe")),
        );
        let session = run_and_get_session(&state, &conversation_id, &event).await;

        assert!(session.is_interactive(), "session should be interactive");
        assert_eq!(session.mode.conversation_resume_from(), Some(3));
    }

    #[tokio::test]
    async fn flips_conversation_to_idle_on_session_terminal_complete() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Running, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Complete, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
        assert_eq!(after.item.status, ConversationStatus::Idle);
    }

    #[tokio::test]
    async fn flips_conversation_to_idle_on_session_terminal_failed() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Running, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Failed, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
        assert_eq!(after.item.status, ConversationStatus::Idle);
    }

    #[tokio::test]
    async fn no_flip_when_session_terminal_but_conversation_closed() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Closed);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Running, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Complete, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
    async fn no_flip_when_session_terminal_but_conversation_already_idle() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation_with_status(Some("swe"), ConversationStatus::Idle);
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Running, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Complete, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
        assert_eq!(after.item.status, ConversationStatus::Idle);
    }

    #[tokio::test]
    async fn no_flip_when_session_non_terminal_transition() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        // Created → Running is non-terminal on both sides.
        let old = make_interactive_session(TaskStatus::Created, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Running, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
    async fn no_flip_when_session_terminal_without_conversation_id() {
        // A non-interactive session terminating must not trigger any
        // conversation lookup.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Running, None);
        let new = make_interactive_session(TaskStatus::Complete, None);
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &state,
            store: state.store(),
        };

        automation.execute(&ctx).await.unwrap();

        // Unrelated conversation remains Active.
        let after = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .unwrap();
        assert_eq!(after.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn no_flip_when_terminal_to_terminal_session_update() {
        // Edge case: SessionUpdated where old.status is already terminal
        // (e.g. Complete -> Failed or a re-emission of the same terminal
        // status). The flip should fire on the *transition* to terminal,
        // not on terminal-to-terminal noise.
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe", "prompt", false).await;

        let conversation = make_conversation(Some("swe"));
        let (conversation_id, _) = state
            .store
            .add_conversation_with_actor(conversation, ActorRef::test())
            .await
            .unwrap();

        let old = make_interactive_session(TaskStatus::Complete, Some(conversation_id.clone()));
        let new = make_interactive_session(TaskStatus::Failed, Some(conversation_id.clone()));
        let event = session_updated_event(SessionId::new(), old, new);

        let automation = SpawnConversationSessionsAutomation::new(None).unwrap();
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
}
