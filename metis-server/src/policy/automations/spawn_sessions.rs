use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::app::event_bus::{EventType, ServerEvent};
use crate::background::spawner::SharedSpawnAttempts;
use crate::background::{AgentQueue, agent_task_state};
use crate::domain::actors::ActorRef;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};

const AUTOMATION_NAME: &str = "spawn_sessions";

/// Event-driven automation that spawns sessions for eligible issues.
///
/// Replaces the polling-based `RunSpawners` background job by reacting
/// to `IssueCreated`, `IssueUpdated`, and `SessionUpdated` events.
/// Reuses the same `AgentQueue::spawn()` eligibility logic from `spawner.rs`.
///
/// Spawn attempt tracking is maintained in-memory per agent to prevent
/// infinite session spawns, matching the behavior of the old polling job.
pub struct SpawnSessionsAutomation {
    spawn_attempts_by_agent: Arc<RwLock<HashMap<String, SharedSpawnAttempts>>>,
}

impl SpawnSessionsAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self {
            spawn_attempts_by_agent: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

#[async_trait]
impl Automation for SpawnSessionsAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![
                EventType::IssueCreated,
                EventType::IssueUpdated,
                EventType::SessionUpdated,
            ],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        // Skip events triggered by this automation to avoid infinite loops.
        if let ActorRef::Automation {
            automation_name, ..
        } = ctx.actor()
        {
            if automation_name == AUTOMATION_NAME {
                return Ok(());
            }
        }

        let agents =
            ctx.app_state.list_agents().await.map_err(|e| {
                AutomationError::Other(anyhow::anyhow!("failed to list agents: {e}"))
            })?;

        if agents.is_empty() {
            return Ok(());
        }

        let mut queues = Vec::with_capacity(agents.len());
        {
            let mut attempts_map = self.spawn_attempts_by_agent.write().await;
            for agent in agents {
                let shared = attempts_map
                    .entry(agent.name.clone())
                    .or_insert_with(|| Arc::new(RwLock::new(HashMap::new())))
                    .clone();
                queues.push(AgentQueue::new(agent, shared));
            }
        }

        let actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ctx.actor().clone())),
        };

        // Collect the issues to evaluate based on the event type.
        let target_issues: Vec<_> = match ctx.event {
            ServerEvent::IssueCreated { issue_id, .. }
            | ServerEvent::IssueUpdated { issue_id, .. } => {
                // For issue events, evaluate only the specific issue.
                match ctx.app_state.get_issue(issue_id, false).await {
                    Ok(versioned) => vec![(issue_id.clone(), versioned.item)],
                    Err(_) => vec![],
                }
            }
            _ => {
                // For session events (and any other), capacity may have changed
                // so we need to scan all issues.
                ctx.app_state
                    .list_issues()
                    .await
                    .map_err(|e| {
                        AutomationError::Other(anyhow::anyhow!("failed to list issues: {e}"))
                    })?
                    .into_iter()
                    .map(|(id, v)| (id, v.item))
                    .collect()
            }
        };

        for queue in &queues {
            let task_state = agent_task_state(ctx.app_state, &queue.agent.name)
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "failed to get task state for agent '{}': {e}",
                        queue.agent.name
                    ))
                })?;

            for (issue_id, issue) in &target_issues {
                match queue
                    .spawn_for_issue(ctx.app_state, issue_id, issue, &task_state)
                    .await
                {
                    Ok(Some(task)) => {
                        match ctx
                            .app_state
                            .add_session(task, chrono::Utc::now(), actor.clone())
                            .await
                        {
                            Ok(session_id) => {
                                tracing::info!(
                                    automation = AUTOMATION_NAME,
                                    agent = queue.agent.name,
                                    session_id = %session_id,
                                    event = ?event_summary(ctx.event),
                                    "spawned session"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    automation = AUTOMATION_NAME,
                                    agent = queue.agent.name,
                                    error = %err,
                                    "failed to add spawned session"
                                );
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            automation = AUTOMATION_NAME,
                            agent = queue.agent.name,
                            issue_id = %issue_id,
                            error = %err,
                            "spawn_for_issue check failed"
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Returns a short summary of the triggering event for logging.
fn event_summary(event: &ServerEvent) -> String {
    match event {
        ServerEvent::IssueCreated { issue_id, .. } => format!("IssueCreated({issue_id})"),
        ServerEvent::IssueUpdated { issue_id, .. } => format!("IssueUpdated({issue_id})"),
        ServerEvent::SessionUpdated { session_id, .. } => {
            format!("SessionUpdated({session_id})")
        }
        other => format!("{:?}", other.event_type()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Repository;
    use crate::app::event_bus::MutationPayload;
    use crate::config::{DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES};
    use crate::domain::agents::Agent;
    use crate::domain::documents::Document;
    use crate::domain::issues::{Issue, IssueStatus, IssueType, SessionSettings};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Status;
    use crate::test_utils::{self, add_repository};
    use chrono::Utc;
    use metis_common::RepoName;
    use std::str::FromStr;

    fn make_issue(agent_name: &str, repo_name: &RepoName) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "Run agent".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open,
            Some(agent_name.to_string()),
            Some(SessionSettings {
                repo_name: Some(repo_name.clone()),
                image: Some("agent-image".to_string()),
                ..SessionSettings::default()
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn make_issue_with_status(
        agent_name: &str,
        repo_name: &RepoName,
        status: IssueStatus,
    ) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "Run agent".to_string(),
            Username::from("worker"),
            String::new(),
            status,
            Some(agent_name.to_string()),
            Some(SessionSettings {
                repo_name: Some(repo_name.clone()),
                image: Some("agent-image".to_string()),
                ..SessionSettings::default()
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn repository() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("agent-image".to_string()),
            None,
        )
    }

    async fn register_agent(
        handles: &test_utils::TestStateHandles,
        name: &str,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            DEFAULT_AGENT_MAX_TRIES,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            false,
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    async fn register_agent_with_capacity(
        handles: &test_utils::TestStateHandles,
        name: &str,
        max_simultaneous: i32,
        max_tries: i32,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            max_tries,
            max_simultaneous,
            false,
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            created_by: None,
            deleted: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    fn issue_created_event(issue_id: metis_common::IssueId, issue: Issue) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: ActorRef::test(),
        });
        ServerEvent::IssueCreated {
            seq: 1,
            issue_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn session_updated_event(
        session_id: metis_common::SessionId,
        old_session: crate::domain::sessions::Session,
        new_session: crate::domain::sessions::Session,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Session {
            old: Some(old_session),
            new: new_session,
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

    #[tokio::test]
    async fn spawns_session_on_issue_created() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/metis")?;
        let agent_name = "agent-a";

        register_agent(&handles, agent_name).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        let issue = make_issue(agent_name, &repo_name);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        let event = issue_created_event(issue_id.clone(), issue);
        let automation = SpawnSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await?;

        let sessions = handles.state.list_sessions().await?;
        assert_eq!(
            sessions.len(),
            1,
            "expected exactly one session to be spawned"
        );

        Ok(())
    }

    #[tokio::test]
    async fn no_spawn_when_at_capacity() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/metis")?;
        let agent_name = "agent-cap";

        register_agent_with_capacity(&handles, agent_name, 1, DEFAULT_AGENT_MAX_TRIES).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        // Create first issue and spawn a session for it
        let issue1 = make_issue(agent_name, &repo_name);
        let (issue_id1, _) = handles
            .store
            .add_issue(issue1.clone(), &ActorRef::test())
            .await?;

        let event1 = issue_created_event(issue_id1.clone(), issue1);
        let automation = SpawnSessionsAutomation::new(None).unwrap();
        let ctx1 = AutomationContext {
            event: &event1,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx1).await?;

        let sessions_after_first = handles.state.list_sessions().await?;
        assert_eq!(sessions_after_first.len(), 1);

        // Create second issue - should NOT spawn because max_simultaneous=1
        let issue2 = make_issue(agent_name, &repo_name);
        let (issue_id2, _) = handles
            .store
            .add_issue(issue2.clone(), &ActorRef::test())
            .await?;

        let event2 = issue_created_event(issue_id2.clone(), issue2);
        let ctx2 = AutomationContext {
            event: &event2,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx2).await?;

        let sessions_after_second = handles.state.list_sessions().await?;
        assert_eq!(
            sessions_after_second.len(),
            1,
            "should not spawn when at capacity"
        );

        Ok(())
    }

    #[tokio::test]
    async fn no_spawn_when_attempts_exhausted() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/metis")?;
        let agent_name = "agent-retry";

        // max_tries=1, so only one attempt is allowed
        register_agent_with_capacity(&handles, agent_name, 5, 1).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        let issue = make_issue(agent_name, &repo_name);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        let automation = SpawnSessionsAutomation::new(None).unwrap();

        // First event: should spawn
        let event = issue_created_event(issue_id.clone(), issue.clone());
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await?;

        let sessions = handles.state.list_sessions().await?;
        assert_eq!(sessions.len(), 1, "first attempt should spawn");

        // Complete the session so the issue no longer has an active session
        let session_id = &sessions[0];
        let mut session = handles.state.get_session(session_id).await?;
        session.status = Status::Complete;
        handles
            .store
            .update_session(session_id, session.clone(), &ActorRef::test())
            .await?;

        // Second event: should NOT spawn because max_tries=1 and attempts exhausted
        let session_updated = session_updated_event(
            session_id.clone(),
            {
                let mut s = session.clone();
                s.status = Status::Running;
                s
            },
            session,
        );
        let ctx2 = AutomationContext {
            event: &session_updated,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx2).await?;

        let sessions_after = handles.state.list_sessions().await?;
        // Only the original completed session should exist, no new ones
        assert_eq!(
            sessions_after.len(),
            1,
            "should not spawn when attempts exhausted"
        );

        Ok(())
    }

    #[tokio::test]
    async fn spawn_attempt_resets_on_status_change() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/metis")?;
        let agent_name = "agent-reset";

        // max_tries=1
        register_agent_with_capacity(&handles, agent_name, 5, 1).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        let issue = make_issue(agent_name, &repo_name);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        let automation = SpawnSessionsAutomation::new(None).unwrap();

        // First attempt: spawn succeeds
        let event = issue_created_event(issue_id.clone(), issue.clone());
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await?;

        let sessions = handles.state.list_sessions().await?;
        assert_eq!(sessions.len(), 1);

        // Complete the session
        let session_id = &sessions[0];
        let mut session = handles.state.get_session(session_id).await?;
        session.status = Status::Complete;
        handles
            .store
            .update_session(session_id, session, &ActorRef::test())
            .await?;

        // Update the issue's status to InProgress (status change should reset attempts)
        let updated_issue = make_issue_with_status(agent_name, &repo_name, IssueStatus::InProgress);
        handles
            .store
            .update_issue(&issue_id, updated_issue.clone(), &ActorRef::test())
            .await?;

        // Trigger on issue updated - should spawn because status changed
        let update_payload = Arc::new(MutationPayload::Issue {
            old: Some(issue),
            new: updated_issue.clone(),
            actor: ActorRef::test(),
        });
        let update_event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload: update_payload,
        };
        let ctx2 = AutomationContext {
            event: &update_event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx2).await?;

        let sessions_after = handles.state.list_sessions().await?;
        assert_eq!(
            sessions_after.len(),
            2,
            "should spawn again after status change resets attempts"
        );

        Ok(())
    }

    #[tokio::test]
    async fn no_agents_configured_is_noop() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();

        let issue = Issue::new(
            IssueType::Task,
            "Test".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        let event = issue_created_event(issue_id, issue);
        let automation = SpawnSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await?;

        let sessions = handles.state.list_sessions().await?;
        assert_eq!(sessions.len(), 0);

        Ok(())
    }
}
