use async_trait::async_trait;

use crate::app::WORKER_NAME_SESSION_LIFECYCLE;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::Status;

const AUTOMATION_NAME: &str = "start_created_sessions";

/// When a session is created or updated into `Created` status, automatically
/// start it by calling `start_pending_task`.
///
/// This replaces the polling-based `ProcessPendingSessionsWorker` with an
/// event-driven approach.
pub struct StartCreatedSessionsAutomation;

impl StartCreatedSessionsAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for StartCreatedSessionsAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::SessionCreated, EventType::SessionUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        // Fire only on a fresh `Created` session: either a `SessionCreated`
        // event, or a `SessionUpdated` event whose status TRANSITIONS into
        // `Created`. Without the transition check, any unrelated mutation on a
        // session that happens to still be in `Created` status would
        // re-trigger `start_pending_task`, racing with the original start and
        // causing the job-engine's idempotency check to surface as a Failed
        // session.
        let (session_id, new, transitioned_into_created) = match ctx.event {
            ServerEvent::SessionCreated {
                session_id,
                payload,
                ..
            } => {
                let MutationPayload::Session { new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                (session_id, new, true)
            }
            ServerEvent::SessionUpdated {
                session_id,
                payload,
                ..
            } => {
                let MutationPayload::Session { old, new, .. } = payload.as_ref() else {
                    return Ok(());
                };
                let transitioned = old.as_ref().is_some_and(|o| o.status != Status::Created);
                (session_id, new, transitioned)
            }
            _ => return Ok(()),
        };

        tracing::info!(
            automation = AUTOMATION_NAME,
            session_id = %session_id,
            "automation invoked",
        );

        if new.status != Status::Created || !transitioned_into_created {
            return Ok(());
        }

        let lifecycle_actor = ActorRef::System {
            worker_name: WORKER_NAME_SESSION_LIFECYCLE.into(),
            on_behalf_of: None,
        };

        ctx.app_state
            .start_pending_task(session_id.clone(), lifecycle_actor)
            .await;

        tracing::info!(
            session_id = %session_id,
            "start_created_sessions: started session"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Session;
    use crate::test_utils;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_session(status: Status) -> Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        use crate::routes::sessions::mount_spec_from_create_request;
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            None,
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
            status,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn session_created_triggers_start() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let session = make_session(Status::Created);
        let (session_id, _) = store
            .add_session(session.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let new_session = make_session(Status::Created);
        let payload = Arc::new(MutationPayload::Session {
            old: None,
            new: new_session,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::SessionCreated {
            seq: 1,
            session_id: session_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = StartCreatedSessionsAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_session(&session_id, false).await.unwrap();
        assert_eq!(updated.item.status, Status::Pending);
    }

    #[tokio::test]
    async fn session_in_non_created_status_is_ignored() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let session = make_session(Status::Pending);
        let (session_id, _) = store
            .add_session(session.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let new_session = make_session(Status::Pending);
        let payload = Arc::new(MutationPayload::Session {
            old: None,
            new: new_session,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::SessionCreated {
            seq: 1,
            session_id: session_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = StartCreatedSessionsAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Status should remain Pending (not changed)
        let updated = store.get_session(&session_id, false).await.unwrap();
        assert_eq!(updated.item.status, Status::Pending);
    }

    #[tokio::test]
    async fn no_op_on_created_to_created_update() {
        // An unrelated mutation on a session that is already in `Created`
        // (before it has started) must NOT re-trigger `start_pending_task`,
        // because doing so would race the original start and surface as a
        // Failed session via the job-engine's idempotency check.
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let session = make_session(Status::Created);
        let (session_id, _) = store
            .add_session(session.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let old_session = make_session(Status::Created);
        let new_session = make_session(Status::Created);
        let payload = Arc::new(MutationPayload::Session {
            old: Some(old_session),
            new: new_session,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::SessionUpdated {
            seq: 1,
            session_id: session_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = StartCreatedSessionsAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        // Status should remain Created (start_pending_task was not called).
        let updated = store.get_session(&session_id, false).await.unwrap();
        assert_eq!(updated.item.status, Status::Created);
    }

    #[tokio::test]
    async fn session_updated_to_created_triggers_start() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let session = make_session(Status::Created);
        let (session_id, _) = store
            .add_session(session.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let old_session = make_session(Status::Failed);
        let new_session = make_session(Status::Created);
        let payload = Arc::new(MutationPayload::Session {
            old: Some(old_session),
            new: new_session,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::SessionUpdated {
            seq: 1,
            session_id: session_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = StartCreatedSessionsAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let updated = store.get_session(&session_id, false).await.unwrap();
        assert_eq!(updated.item.status, Status::Pending);
    }
}
