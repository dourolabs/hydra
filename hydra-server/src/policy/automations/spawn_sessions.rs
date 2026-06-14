use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::app::event_bus::{EventType, ServerEvent};
use crate::domain::actors::ActorRef;
use crate::policy::automations::agent_queue::{
    AgentQueue, SharedSpawnAttempts, SpawnResult, agent_task_state,
};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use hydra_common::api::v1::issues::SearchIssuesQuery;

const AUTOMATION_NAME: &str = "spawn_sessions";

/// Event-driven automation that spawns sessions for eligible issues.
///
/// Replaces the polling-based `RunSpawners` background job by reacting
/// to `IssueCreated`, `IssueUpdated`, and `SessionUpdated` events.
/// Reuses the same `AgentQueue::spawn_for_issue()` eligibility logic from `agent_queue.rs`.
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

        match ctx.event {
            ServerEvent::IssueCreated { issue_id, .. } => {
                tracing::info!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    event = "IssueCreated",
                    "automation invoked",
                );
            }
            ServerEvent::IssueUpdated { issue_id, .. } => {
                tracing::info!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    event = "IssueUpdated",
                    "automation invoked",
                );
            }
            ServerEvent::SessionUpdated { session_id, .. } => {
                tracing::info!(
                    automation = AUTOMATION_NAME,
                    session_id = %session_id,
                    event = "SessionUpdated",
                    "automation invoked",
                );
            }
            _ => {}
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

        // Scan all non-archived issues for spawn readiness on every event.
        // This ensures that when a child issue transitions to a terminal state,
        // the parent issue is also evaluated for readiness.
        // Don't filter by status; terminal-status default-project issues
        // carry `assignee = NULL` invariantly (cleared by
        // `apply_status_on_enter` when the status sets
        // `on_enter.clear_assignee = true`), so the per-agent queue
        // skips them naturally.
        let target_issues: Vec<_> = {
            let query = SearchIssuesQuery::new(None, vec![], None, None, None);
            ctx.app_state
                .list_issues_with_query(&query)
                .await
                .map_err(|e| AutomationError::Other(anyhow::anyhow!("failed to list issues: {e}")))?
                .into_iter()
                .map(|(id, v)| (id, v.item))
                .collect()
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

            let max_interactive = queue.agent.max_simultaneous_interactive as usize;
            let max_headless = queue.agent.max_simultaneous_headless as usize;
            let active_interactive =
                task_state.running_interactive + task_state.pending_interactive;
            let active_headless = task_state.running_headless + task_state.pending_headless;
            let mut remaining_interactive = max_interactive.saturating_sub(active_interactive);
            let mut remaining_headless = max_headless.saturating_sub(active_headless);

            // Server-side `create_session` (AgentSpec::Named branch) now
            // resolves the agent prompt + mcp_config per call. The caller
            // no longer caches them across iterations.
            for (issue_id, issue) in &target_issues {
                if remaining_interactive == 0 && remaining_headless == 0 {
                    break;
                }

                match queue
                    .spawn_for_issue(ctx.app_state, issue_id, issue, &task_state)
                    .await
                {
                    Ok(SpawnResult::Spawned(session_id)) => {
                        // `build_task` now persists through
                        // `AppState::create_session`, so we just decrement
                        // capacity and log the assigned id.
                        remaining_headless = remaining_headless.saturating_sub(1);
                        tracing::info!(
                            automation = AUTOMATION_NAME,
                            agent = queue.agent.name,
                            session_id = %session_id,
                            event = ?ctx.event.summary(),
                            "spawned session"
                        );
                    }
                    Ok(SpawnResult::SpawnedConversation(conversation_id)) => {
                        // Interactive branch: a Conversation is created in
                        // place of a headless session. The companion session
                        // is materialized asynchronously by
                        // `SpawnConversationSessionsAutomation`; capacity is
                        // still consumed here so a queue full of interactive
                        // issues can't run away.
                        remaining_interactive = remaining_interactive.saturating_sub(1);
                        tracing::info!(
                            automation = AUTOMATION_NAME,
                            agent = queue.agent.name,
                            conversation_id = %conversation_id,
                            event = ?ctx.event.summary(),
                            "spawned conversation"
                        );
                    }
                    Ok(SpawnResult::RetriesExhausted {
                        issue_id: exhausted_issue_id,
                    }) => {
                        let mut failed_issue = issue.clone();
                        failed_issue.status =
                            hydra_common::api::v1::projects::StatusKey::try_new("failed")
                                .expect("\"failed\" is a well-formed StatusKey");
                        if let Err(err) = ctx
                            .app_state
                            .upsert_issue(
                                Some(exhausted_issue_id.clone()),
                                hydra_common::api::v1::issues::UpsertIssueRequest::new(
                                    failed_issue.into(),
                                    None,
                                ),
                                actor.clone(),
                            )
                            .await
                        {
                            tracing::warn!(
                                automation = AUTOMATION_NAME,
                                agent = queue.agent.name,
                                issue_id = %exhausted_issue_id,
                                error = %err,
                                "failed to mark issue as failed after retries exhausted"
                            );
                        } else {
                            tracing::info!(
                                automation = AUTOMATION_NAME,
                                agent = queue.agent.name,
                                issue_id = %exhausted_issue_id,
                                "marked issue as failed: retries exhausted"
                            );
                        }
                    }
                    Ok(SpawnResult::Skipped) => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Repository;
    use crate::app::event_bus::{MutationPayload, ServerEvent};
    use crate::config::{DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES};
    use crate::domain::agents::Agent;
    use crate::domain::documents::Document;
    use crate::domain::issues::{Issue, IssueType, SessionSettings};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Status;
    use crate::test_utils::{self, add_repository};
    use chrono::Utc;
    use hydra_common::RepoName;
    use hydra_common::api::v1::projects::StatusKey;
    use hydra_common::test_utils::status::status;
    use std::str::FromStr;

    fn make_issue(agent_name: &str, repo_name: &RepoName) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "Run agent".to_string(),
            Username::from("worker"),
            status("open"),
            crate::domain::projects::default_project_id(),
            Some(hydra_common::principal::Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new(agent_name)
                    .expect("test agent name should validate"),
            }),
            Some(SessionSettings {
                repo_name: Some(repo_name.clone()),
                image: Some("agent-image".to_string()),
                ..SessionSettings::default()
            }),
            Vec::new(),
            Vec::new(),
            None,
            None,
        )
    }

    fn make_issue_with_status(agent_name: &str, repo_name: &RepoName, status: StatusKey) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "Run agent".to_string(),
            Username::from("worker"),
            status,
            crate::domain::projects::default_project_id(),
            Some(hydra_common::principal::Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new(agent_name)
                    .expect("test agent name should validate"),
            }),
            Some(SessionSettings {
                repo_name: Some(repo_name.clone()),
                image: Some("agent-image".to_string()),
                ..SessionSettings::default()
            }),
            Vec::new(),
            Vec::new(),
            None,
            None,
        )
    }

    fn repository() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("agent-image".to_string()),
        )
    }

    async fn register_agent(
        handles: &test_utils::TestStateHandles,
        name: &str,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            None,
            DEFAULT_AGENT_MAX_TRIES,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            false,
            vec![],
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            archived: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    async fn register_agent_with_capacity(
        handles: &test_utils::TestStateHandles,
        name: &str,
        max_simultaneous_headless: i32,
        max_tries: i32,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            None,
            max_tries,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            max_simultaneous_headless,
            false,
            vec![],
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            archived: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    fn issue_created_event(issue_id: hydra_common::IssueId, issue: Issue) -> ServerEvent {
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
        session_id: hydra_common::SessionId,
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
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
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
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
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
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
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
    async fn marks_issue_failed_when_attempts_exhausted() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
        let agent_name = "agent-fail";

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

        // Second event: should NOT spawn and should mark issue as Failed
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

        // Verify issue is now Failed
        let updated_issue = handles.store.get_issue(&issue_id, false).await?;
        assert_eq!(
            updated_issue.item.status,
            status("failed"),
            "issue should be marked as failed when retries exhausted"
        );
        let _ = session_id;

        Ok(())
    }

    #[tokio::test]
    async fn spawn_attempt_resets_on_status_change() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
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
        let updated_issue = make_issue_with_status(agent_name, &repo_name, status("in-progress"));
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
            status("open"),
            crate::domain::projects::default_project_id(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
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

    #[tokio::test]
    async fn session_event_does_not_spawn_for_terminal_issues() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
        let agent_name = "agent-terminal";

        register_agent(&handles, agent_name).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        // Create an open issue and spawn a session for it.
        let issue = make_issue(agent_name, &repo_name);
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        let event = issue_created_event(issue_id.clone(), issue.clone());
        let automation = SpawnSessionsAutomation::new(None).unwrap();
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
            .update_session(session_id, session.clone(), &ActorRef::test())
            .await?;

        // Close the issue (terminal status) and clear its assignee —
        // mirrors what `apply_status_on_enter.clear_assignee` does on
        // transition into a default-project terminal status. Direct
        // `store.update_issue` bypasses the automation pipeline, so the
        // test has to apply the invariant by hand.
        let mut closed_issue = make_issue_with_status(agent_name, &repo_name, status("closed"));
        closed_issue.assignee = None;
        handles
            .store
            .update_issue(&issue_id, closed_issue, &ActorRef::test())
            .await?;

        // Fire a SessionUpdated event — closed issue should NOT be spawned for
        let session_event = session_updated_event(
            session_id.clone(),
            {
                let mut s = session.clone();
                s.status = Status::Running;
                s
            },
            session,
        );
        let ctx2 = AutomationContext {
            event: &session_event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx2).await?;

        let sessions_after = handles.state.list_sessions().await?;
        assert_eq!(
            sessions_after.len(),
            1,
            "should not spawn for closed/terminal issues on session events"
        );

        Ok(())
    }

    #[tokio::test]
    async fn spawns_session_for_issue_with_custom_status() -> anyhow::Result<()> {
        use hydra_common::api::v1::projects::{
            Project, ProjectKey, StatusDefinition, StatusKey, StatusOnEnter,
        };
        use hydra_common::principal::Principal;

        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/hydra")?;
        let agent_name = "pm";

        register_agent(&handles, agent_name).await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        // Project with a custom `backlog` status whose `on_enter` assigns to pm.
        // The on_enter automation is exercised separately; this test simulates
        // the post-`apply_status_on_enter` state (assignee already = pm) and
        // verifies spawn_sessions picks the issue up despite the non-legacy
        // status string.
        let on_enter = StatusOnEnter::new(
            Some(Principal::Agent {
                name: hydra_common::api::v1::agents::AgentName::try_new(agent_name).unwrap(),
            }),
            None,
        );
        let statuses = vec![
            StatusDefinition::new(
                StatusKey::try_new("backlog").unwrap(),
                "Backlog".to_string(),
                "#abcdef".parse().unwrap(),
                false,
                false,
                false,
                Some(on_enter),
            ),
            StatusDefinition::new(
                StatusKey::try_new("done").unwrap(),
                "Done".to_string(),
                "#abcdef".parse().unwrap(),
                true,
                true,
                false,
                None,
            ),
        ];
        let project = Project::new(
            ProjectKey::try_new("engineering").unwrap(),
            "Engineering".to_string(),
            Vec::new(),
            hydra_common::api::v1::users::Username::try_new("worker").unwrap(),
            false,
            0.0,
        );
        let (project_id, _) = handles
            .store
            .add_project(project, &ActorRef::test())
            .await?;
        for def in statuses {
            handles
                .store
                .add_status(&project_id, def, &ActorRef::test())
                .await?;
        }

        let mut issue = make_issue(agent_name, &repo_name);
        issue.project_id = project_id;
        issue.status = StatusKey::try_new("backlog").unwrap();
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
        assert_eq!(
            sessions.len(),
            1,
            "expected spawn_sessions to spawn for a custom-status (backlog) issue"
        );

        Ok(())
    }
}
