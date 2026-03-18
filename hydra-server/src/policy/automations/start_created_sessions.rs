use async_trait::async_trait;

use crate::app::WORKER_NAME_SESSION_LIFECYCLE;
use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::Status;

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
        "start_created_sessions"
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::SessionCreated, EventType::SessionUpdated],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let (session_id, payload) = match ctx.event {
            ServerEvent::SessionCreated {
                session_id,
                payload,
                ..
            } => (session_id, payload),
            ServerEvent::SessionUpdated {
                session_id,
                payload,
                ..
            } => (session_id, payload),
            _ => return Ok(()),
        };

        let MutationPayload::Session { new, .. } = payload.as_ref() else {
            return Ok(());
        };

        if new.status != Status::Created {
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
    use crate::domain::sessions::BundleSpec;
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Session;
    use crate::test_utils;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_session(status: Status) -> Session {
        Session::new(
            "test task".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
            None,
            HashMap::new(),
            None,
            None,
            None,
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
