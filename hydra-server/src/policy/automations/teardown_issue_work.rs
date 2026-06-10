use async_trait::async_trait;

use crate::app::event_bus::{EventType, MutationPayload, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::domain::conversations::ConversationStatus;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::Status;
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::{ConversationId, IssueId};

const AUTOMATION_NAME: &str = "teardown_issue_work";

/// Tears down agent work attached to an issue when the issue either enters
/// a "teardown_work" status or is deleted (soft-delete / archive).
///
/// Trigger:
/// - `IssueUpdated` where the new status's `on_enter.teardown_work = true`
///   (the canonical "issue went terminal" path on the default project).
/// - `IssueDeleted` — always. Deleting an issue is itself the signal that
///   any attached work should stop, regardless of the status it was in.
///
/// Effects (same in both cases):
/// - Kill any `Created` / `Pending` / `Running` sessions attached to the
///   issue.
/// - Close any non-`Closed` conversations spawned from the issue (i.e.
///   conversations with a `spawned_from` relation pointing at it).
///
/// This automation should run after `cascade_issue_status` so that
/// cascaded child/dependent issues also get their sessions killed via
/// their own update events.
pub struct TeardownIssueWorkAutomation;

impl TeardownIssueWorkAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }

    async fn kill_sessions_for_issue(
        ctx: &AutomationContext<'_>,
        issue_id: &IssueId,
    ) -> Result<usize, AutomationError> {
        let store = ctx.store;
        let session_ids = store.get_sessions_for_issue(issue_id).await.map_err(|e| {
            AutomationError::Other(anyhow::anyhow!(
                "failed to get sessions for issue {issue_id}: {e}"
            ))
        })?;

        let mut killed = 0usize;
        for session_id in session_ids {
            let session = store.get_session(&session_id, false).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to fetch session {session_id}: {e}"))
            })?;

            if matches!(
                session.item.status,
                Status::Created | Status::Pending | Status::Running
            ) {
                match ctx.app_state.job_engine.kill_job(&session_id).await {
                    Ok(()) => {
                        killed += 1;
                        tracing::info!(
                            automation = AUTOMATION_NAME,
                            issue_id = %issue_id,
                            session_id = %session_id,
                            "killed session"
                        );
                    }
                    Err(crate::job_engine::JobEngineError::NotFound(_)) => {
                        tracing::info!(
                            automation = AUTOMATION_NAME,
                            issue_id = %issue_id,
                            session_id = %session_id,
                            "session already missing while killing"
                        );
                    }
                    Err(e) => {
                        return Err(AutomationError::Other(anyhow::anyhow!(
                            "failed to kill session {session_id} for issue {issue_id}: {e}"
                        )));
                    }
                }
            }
        }

        Ok(killed)
    }

    async fn close_conversations_for_issue(
        ctx: &AutomationContext<'_>,
        issue_id: &IssueId,
        actor: ActorRef,
    ) -> Result<usize, AutomationError> {
        let query = SearchConversationsQuery {
            spawned_from: Some(issue_id.clone()),
            include_deleted: Some(false),
            ..Default::default()
        };
        let conversations: Vec<(ConversationId, _)> =
            ctx.store.list_conversations(&query).await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "[{AUTOMATION_NAME}] failed to list conversations for issue {issue_id}: {e}"
                ))
            })?;

        let mut closed = 0usize;
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
                    closed += 1;
                    tracing::info!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        conversation_id = %conversation_id,
                        "closed conversation"
                    );
                }
                Err(err) => {
                    // Don't abort the whole automation on a single
                    // conversation close failure — let the other
                    // conversations attached to the same issue still
                    // get a chance.
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        conversation_id = %conversation_id,
                        error = %err,
                        "failed to close conversation"
                    );
                }
            }
        }

        Ok(closed)
    }
}

#[async_trait]
impl Automation for TeardownIssueWorkAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![EventType::IssueUpdated, EventType::IssueDeleted],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let issue_id = match ctx.event {
            ServerEvent::IssueDeleted { issue_id, .. } => issue_id,
            ServerEvent::IssueUpdated {
                issue_id, payload, ..
            } => {
                let MutationPayload::Issue {
                    old: Some(old),
                    new,
                    ..
                } = payload.as_ref()
                else {
                    return Ok(());
                };

                // Only trigger when the status actually changes.
                if old.status == new.status {
                    return Ok(());
                }
                let resolved = match ctx.app_state.resolve_status(new).await {
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
                if !resolved
                    .on_enter
                    .as_ref()
                    .is_some_and(|oe| oe.teardown_work)
                {
                    return Ok(());
                }
                issue_id
            }
            _ => return Ok(()),
        };

        tracing::info!(
            automation = AUTOMATION_NAME,
            issue_id = %issue_id,
            "automation invoked",
        );

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        let killed = Self::kill_sessions_for_issue(ctx, issue_id).await?;
        let closed = Self::close_conversations_for_issue(ctx, issue_id, actor).await?;

        if killed > 0 || closed > 0 {
            tracing::info!(
                automation = AUTOMATION_NAME,
                issue_id = %issue_id,
                killed,
                closed,
                "completed"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::app::test_helpers::{
        issue_with_status, register_agent, seed_linked_conversation, state_with_default_model,
    };
    use crate::domain::actors::ActorRef;
    use crate::domain::conversations::ConversationStatus;
    use crate::domain::issues::{Issue, IssueType};
    use crate::domain::users::Username;
    use crate::job_engine::{JobEngine, JobStatus};
    use crate::policy::context::AutomationContext;
    use crate::test_utils;
    use chrono::Utc;
    use hydra_common::api::v1::projects::StatusKey;
    use hydra_common::test_utils::status::status;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_issue(status_key: StatusKey) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test".to_string(),
            Username::from("tester"),
            String::new(),
            status_key,
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn make_task(issue_id: &hydra_common::IssueId) -> crate::domain::sessions::Session {
        use crate::domain::sessions::{AgentConfig, SessionMode};
        use crate::routes::sessions::mount_spec_from_create_request;
        crate::domain::sessions::Session::new(
            Username::from("test-creator"),
            Some(issue_id.clone()),
            None,
            AgentConfig::default(),
            mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
            crate::store::Status::Created,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn kills_tasks_when_issue_dropped() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let issue = make_issue(status("open"));
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        // Add a task for the issue
        let task = make_task(&issue_id);
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Mark task as Running
        let mut running_task = store.get_session(&task_id, false).await.unwrap().item;
        running_task.status = Status::Running;
        store
            .update_session(&task_id, running_task, &ActorRef::test())
            .await
            .unwrap();

        // Update issue to Dropped
        let old_issue = make_issue(status("open"));
        let new_issue = make_issue(status("dropped"));
        store
            .update_issue(&issue_id, new_issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        // MockJobEngine will succeed on kill_job
        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn skips_when_not_a_failure_status() {
        let handles = test_utils::test_state_handles();
        let store = handles.store.clone();

        let old_issue = make_issue(status("open"));
        let new_issue = make_issue(status("in-progress"));

        let (issue_id, _) = store
            .add_issue(new_issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: ActorRef::test(),
        });

        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn closes_linked_conversation_on_teardown_work_status_entry() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;

        let mut issue = issue_with_status("test", status("open"), vec![]);
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Active).await;

        // Flip to `dropped`, which is `teardown_work=true` in the default
        // project.
        let mut updated = issue.clone();
        updated.status = status("dropped");
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: updated.clone(),
            actor: ActorRef::test(),
        });
        issue.status = status("dropped");
        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
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
    async fn fires_on_issue_deleted() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;

        let issue = issue_with_status("test", status("open"), vec![]);
        let (issue_id, _) = state
            .store
            .add_issue_with_actor(issue.clone(), ActorRef::test())
            .await
            .unwrap();

        let conversation_id =
            seed_linked_conversation(&state, &issue_id, ConversationStatus::Active).await;

        // Build an IssueDeleted event. The payload carries the issue's
        // pre-delete state as `old` and the (still-readable, now soft-
        // deleted) state as `new`.
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue.clone(),
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueDeleted {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
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
        assert_eq!(
            after.item.status,
            ConversationStatus::Closed,
            "conversation spawned from a deleted issue should be closed"
        );
    }

    #[tokio::test]
    async fn kills_tasks_when_issue_deleted() {
        let engine = Arc::new(test_utils::MockJobEngine::new());
        let handles = test_utils::test_state_with_engine_handles(engine.clone());
        let store = handles.store.clone();

        let issue = make_issue(status("open"));
        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Seed a Created session attached to the issue and a matching
        // Pending job in the engine so we can observe the kill.
        let task = make_task(&issue_id);
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        engine.insert_job(&task_id, JobStatus::Pending).await;

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueDeleted {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: store.as_ref(),
        };
        automation.execute(&ctx).await.unwrap();

        let job = engine.find_job_by_hydra_id(&task_id).await.unwrap();
        assert_eq!(
            job.status,
            JobStatus::Failed,
            "session attached to a deleted issue should be killed",
        );
    }

    #[tokio::test]
    async fn issue_deleted_ignores_unrelated_conversations() {
        let state = state_with_default_model("default-model");
        register_agent(&state, "swe").await;

        let issue_a = issue_with_status("a", status("open"), vec![]);
        let (issue_a_id, _) = state
            .store
            .add_issue_with_actor(issue_a.clone(), ActorRef::test())
            .await
            .unwrap();

        let issue_b = issue_with_status("b", status("open"), vec![]);
        let (issue_b_id, _) = state
            .store
            .add_issue_with_actor(issue_b, ActorRef::test())
            .await
            .unwrap();

        let conv_a =
            seed_linked_conversation(&state, &issue_a_id, ConversationStatus::Active).await;
        let conv_b =
            seed_linked_conversation(&state, &issue_b_id, ConversationStatus::Active).await;

        // Only delete A.
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue_a.clone()),
            new: issue_a,
            actor: ActorRef::test(),
        });
        let event = ServerEvent::IssueDeleted {
            seq: 1,
            issue_id: issue_a_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = TeardownIssueWorkAutomation;
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
            "conversation linked to a different issue should be untouched"
        );
    }
}
