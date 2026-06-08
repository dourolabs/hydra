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

/// Worker name used as the actor when the assignment-loop persists an
/// auto-assignment. The actor is wrapped as
/// `Automation { automation_name: AUTOMATION_NAME, triggered_by: Some(System { on_behalf_of: None }) }`
/// so two invariants hold at once:
/// - The self-loop early-out at the top of `execute` (matching
///   `automation_name == AUTOMATION_NAME`) blocks re-entry when the
///   `IssueUpdated` event we minted here fires the automation again.
/// - `LinkConversationToArtifactsAutomation`'s short-circuit still fires:
///   `actor.on_behalf_of()` recursively unwraps `Automation.triggered_by`
///   to the inner `System { on_behalf_of: None }`, which resolves to
///   `None`, so no spurious `refers-to` edges are minted. (Before this
///   wrapping, an unwrapped `Automation { triggered_by: ctx.actor() }`
///   carried the triggering session's principal forward and caused a
///   bulk-edge cascade across every unassigned issue.)
const ASSIGNMENT_WORKER_NAME: &str = "spawn_sessions_assignment";

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

        // Locate the assignment agent, if one is configured. Unassigned
        // issues are auto-assigned to it below so the agent queue can
        // treat every issue uniformly via the `assignee` field.
        let assignment_agent = queues
            .iter()
            .find(|q| q.agent.is_assignment_agent && !q.agent.deleted)
            .map(|q| q.agent.clone());

        // Scan all non-deleted issues for spawn readiness on every event.
        // This ensures that when a child issue transitions to a terminal state,
        // the parent issue is also evaluated for readiness.
        // Don't filter by status; agent_queue::spawn_for_issue uses
        // is_issue_ready/resolve_status to skip terminal issues, so the
        // upstream filter would just re-encode legacy enum semantics here.
        let mut target_issues: Vec<_> = {
            let query = SearchIssuesQuery::new(None, vec![], None, None, None);
            ctx.app_state
                .list_issues_with_query(&query)
                .await
                .map_err(|e| AutomationError::Other(anyhow::anyhow!("failed to list issues: {e}")))?
                .into_iter()
                .map(|(id, v)| (id, v.item))
                .collect()
        };

        // Auto-assign unassigned issues to the configured assignment agent
        // (if any) and persist the update before the per-agent spawn loop
        // runs. Issues with no assignee and no configured assignment agent
        // are left unassigned; the queue will skip them.
        if let Some(agent) = assignment_agent.as_ref() {
            // Wrap a `System { on_behalf_of: None }` inside `Automation`
            // so the self-loop early-out matches on `automation_name` and
            // blocks re-entry, while `on_behalf_of()` still resolves to
            // `None` and keeps the `LinkConversationToArtifactsAutomation`
            // short-circuit firing — see `ASSIGNMENT_WORKER_NAME`.
            let assignment_actor = ActorRef::Automation {
                automation_name: AUTOMATION_NAME.into(),
                triggered_by: Some(Box::new(ActorRef::System {
                    worker_name: ASSIGNMENT_WORKER_NAME.into(),
                    on_behalf_of: None,
                })),
            };
            let agent_name = hydra_common::api::v1::agents::AgentName::try_new(agent.name.clone())
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "assignment agent '{}' has invalid name: {e}",
                        agent.name
                    ))
                })?;
            let assignee = hydra_common::principal::Principal::Agent { name: agent_name };
            for (issue_id, issue) in target_issues.iter_mut() {
                if issue.assignee.is_some() {
                    continue;
                }
                issue.assignee = Some(assignee.clone());
                if let Err(err) = ctx
                    .app_state
                    .upsert_issue(
                        Some(issue_id.clone()),
                        hydra_common::api::v1::issues::UpsertIssueRequest::new(
                            issue.clone().into(),
                            None,
                        ),
                        assignment_actor.clone(),
                    )
                    .await
                {
                    // Roll back the in-memory mutation so the queue does not
                    // try to spawn for an assignment we failed to persist.
                    issue.assignee = None;
                    tracing::warn!(
                        automation = AUTOMATION_NAME,
                        issue_id = %issue_id,
                        agent = agent.name,
                        error = %err,
                        "failed to auto-assign unassigned issue to assignment agent"
                    );
                }
            }
        }

        for queue in &queues {
            let task_state = agent_task_state(ctx.app_state, &queue.agent.name)
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "failed to get task state for agent '{}': {e}",
                        queue.agent.name
                    ))
                })?;

            let max_simultaneous = queue.agent.max_simultaneous as usize;
            let active_tasks = task_state.running_tasks + task_state.pending_tasks;
            let mut remaining_capacity = max_simultaneous.saturating_sub(active_tasks);

            // Server-side `create_session` (AgentSpec::Named branch) now
            // resolves the agent prompt + mcp_config per call. The caller
            // no longer caches them across iterations.
            for (issue_id, issue) in &target_issues {
                if remaining_capacity == 0 {
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
                        remaining_capacity -= 1;
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
                        remaining_capacity -= 1;
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
                        max_tries,
                    }) => {
                        // Query all sessions for this issue to include in the failure message.
                        let session_ids: Vec<hydra_common::SessionId> = ctx
                            .app_state
                            .get_sessions_for_issue(&exhausted_issue_id)
                            .await
                            .unwrap_or_default();
                        let session_id_list = session_ids
                            .iter()
                            .map(|id| id.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let progress = format!(
                            "Automatic failure: the system exhausted all {max_tries} session spawn \
                             attempts for this issue. Session IDs: {session_id_list}"
                        );

                        // Update the issue to Failed with a descriptive progress message.
                        let mut failed_issue = issue.clone();
                        failed_issue.status = crate::domain::issues::IssueStatus::Failed.into();
                        failed_issue.progress = progress.clone();
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
    use crate::domain::issues::{Issue, IssueStatus, IssueType, SessionSettings};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Status;
    use crate::test_utils::{self, add_repository};
    use chrono::Utc;
    use hydra_common::RepoName;
    use std::str::FromStr;

    fn make_issue(agent_name: &str, repo_name: &RepoName) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "Run agent".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open.into(),
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
            None,
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
            status.into(),
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
            false,
            false,
            vec![],
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            deleted: false,
        };
        handles.store.add_document(doc, &ActorRef::test()).await?;
        Ok(())
    }

    async fn register_assignment_agent(
        handles: &test_utils::TestStateHandles,
        name: &str,
    ) -> anyhow::Result<()> {
        let agent = Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            None,
            DEFAULT_AGENT_MAX_TRIES,
            DEFAULT_AGENT_MAX_SIMULTANEOUS,
            true,
            false,
            vec![],
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
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
            None,
            max_tries,
            max_simultaneous,
            false,
            false,
            vec![],
        );
        handles.store.add_agent(agent).await?;

        let doc = Document {
            title: format!("{name} prompt"),
            body_markdown: format!("prompt for {name}"),
            path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
            deleted: false,
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
            IssueStatus::Failed.into(),
            "issue should be marked as failed when retries exhausted"
        );

        // Verify progress message contains session ID
        assert!(
            updated_issue.item.progress.contains("Automatic failure"),
            "progress should contain failure explanation"
        );
        assert!(
            updated_issue
                .item
                .progress
                .contains(&session_id.to_string()),
            "progress should contain the session ID"
        );

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
            IssueStatus::Open.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
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

        // Close the issue (terminal status)
        let closed_issue = make_issue_with_status(agent_name, &repo_name, IssueStatus::Closed);
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
    async fn auto_assigns_unassigned_issue_to_assignment_agent() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let agent_name = "pm";
        register_assignment_agent(&handles, agent_name).await?;

        // Create an unassigned, repo-less issue (typical assignment-agent target).
        let issue = Issue::new(
            IssueType::Task,
            "Needs assignment".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
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

        // Persisted assignee should now be the assignment agent.
        let stored = handles.store.get_issue(&issue_id, false).await?.item;
        match stored.assignee {
            Some(hydra_common::principal::Principal::Agent { ref name }) => {
                assert_eq!(name.as_str(), agent_name);
            }
            other => panic!("expected assignee Principal::Agent('{agent_name}'); got {other:?}"),
        }

        // The assignment agent's queue should also have spawned a session.
        let sessions = handles.state.list_sessions().await?;
        assert_eq!(
            sessions.len(),
            1,
            "expected one session to be spawned after auto-assignment"
        );

        Ok(())
    }

    /// The assignment-loop upsert wraps `System { on_behalf_of: None }`
    /// inside `Automation { automation_name: AUTOMATION_NAME, .. }`. The
    /// `Automation` outer layer lets the self-loop early-out at the top of
    /// `execute` short-circuit re-entry from the `IssueUpdated` event this
    /// upsert emits, while the inner `System { on_behalf_of: None }` keeps
    /// `on_behalf_of()` resolving to `None` so the resulting event does not
    /// carry the triggering session's identity downstream. If the inner
    /// actor were `ctx.actor()` instead, `LinkConversationToArtifactsAutomation`
    /// would follow the chain and mint a spurious `refers-to` edge for every
    /// auto-assigned issue. See the RCA in `i-trqznsor` for the bulk-edge
    /// cascade this prevents.
    #[tokio::test]
    async fn assignment_upsert_uses_system_actor_with_no_principal() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let agent_name = "pm";
        register_assignment_agent(&handles, agent_name).await?;

        let issue = Issue::new(
            IssueType::Task,
            "Needs assignment".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        // Fire the event as if a real session (an authenticated agent) had
        // triggered the automation — the previous bug surfaced when the
        // triggering actor was an agent whose `originating_session_id` was
        // a conversation-bearing session.
        let triggering_actor = ActorRef::Authenticated {
            actor_id: crate::domain::actors::ActorId::Agent(
                hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap(),
            ),
            session_id: Some(hydra_common::SessionId::new()),
        };
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: triggering_actor.clone(),
        });
        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let automation = SpawnSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await?;

        // The upsert that persisted the auto-assignment must be the
        // assignment-loop's wrapped actor — `Automation { automation_name:
        // AUTOMATION_NAME, triggered_by: Some(System { worker_name:
        // ASSIGNMENT_WORKER_NAME, on_behalf_of: None }) }` — not a wrapping
        // of the triggering agent.
        let versions = handles.store.get_issue_versions(&issue_id).await?;
        let upsert_version = versions
            .into_iter()
            .rev()
            .find(|v| v.item.assignee.is_some())
            .expect("expected a persisted version with an assigned principal");
        match upsert_version.actor {
            Some(ActorRef::Automation {
                ref automation_name,
                triggered_by: Some(ref triggered_by),
            }) => {
                assert_eq!(
                    automation_name, AUTOMATION_NAME,
                    "assignment upsert outer actor should name this automation \
                     so the self-loop early-out matches"
                );
                match triggered_by.as_ref() {
                    ActorRef::System {
                        worker_name,
                        on_behalf_of: None,
                    } => {
                        assert_eq!(
                            worker_name, ASSIGNMENT_WORKER_NAME,
                            "assignment upsert inner actor should use the dedicated worker name"
                        );
                    }
                    other => panic!(
                        "expected inner `System {{ worker_name: \"{ASSIGNMENT_WORKER_NAME}\", on_behalf_of: None }}` actor; got {other:?}"
                    ),
                }
            }
            other => panic!(
                "expected `Automation {{ automation_name: \"{AUTOMATION_NAME}\", triggered_by: Some(System {{ worker_name: \"{ASSIGNMENT_WORKER_NAME}\", on_behalf_of: None }}) }}` actor; got {other:?}"
            ),
        }

        // The downstream invariant the actor choice is protecting: an
        // `IssueUpdated` synthesized with this actor must NOT trip
        // `LinkConversationToArtifactsAutomation`'s `on_behalf_of()` chain.
        let assignment_actor = upsert_version.actor.unwrap();
        assert!(
            assignment_actor.on_behalf_of().is_none(),
            "assignment-loop actor must resolve to no on_behalf_of principal; got {:?}",
            assignment_actor.on_behalf_of()
        );

        Ok(())
    }

    /// The self-loop early-out at the top of `execute` must short-circuit
    /// when the triggering actor is an `Automation { automation_name:
    /// AUTOMATION_NAME, .. }` — i.e. the `IssueUpdated` event minted by
    /// the assignment-loop upsert itself. We verify the short-circuit by
    /// firing such an event against state where the next step *would*
    /// have called `list_issues` and auto-assigned an unassigned issue,
    /// then asserting nothing was persisted: no assignment, no session.
    #[tokio::test]
    async fn skips_reentry_from_wrapped_assignment_actor() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let agent_name = "pm";
        register_assignment_agent(&handles, agent_name).await?;

        let issue = Issue::new(
            IssueType::Task,
            "Needs assignment".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue.clone(), &ActorRef::test())
            .await?;

        // The same actor shape the assignment-loop upsert uses.
        let wrapped_actor = ActorRef::Automation {
            automation_name: AUTOMATION_NAME.into(),
            triggered_by: Some(Box::new(ActorRef::System {
                worker_name: ASSIGNMENT_WORKER_NAME.into(),
                on_behalf_of: None,
            })),
        };
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(issue.clone()),
            new: issue,
            actor: wrapped_actor,
        });
        let event = ServerEvent::IssueUpdated {
            seq: 2,
            issue_id: issue_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        let automation = SpawnSessionsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };
        automation.execute(&ctx).await?;

        // If `execute` had proceeded past the self-loop early-out, it would
        // have listed issues, auto-assigned this unassigned issue to `pm`,
        // and spawned a session for it. Neither should have happened.
        let stored = handles.store.get_issue(&issue_id, false).await?.item;
        assert!(
            stored.assignee.is_none(),
            "self-loop early-out must skip auto-assignment; got assignee {:?}",
            stored.assignee
        );
        let sessions = handles.state.list_sessions().await?;
        assert_eq!(
            sessions.len(),
            0,
            "self-loop early-out must skip the spawn pass; got {} sessions",
            sessions.len()
        );

        Ok(())
    }

    #[tokio::test]
    async fn unassigned_issue_is_noop_without_assignment_agent() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        // Register a non-assignment agent so the early "no agents" return
        // doesn't short-circuit. It should NOT pick up the unassigned issue.
        register_agent(&handles, "agent-a").await?;

        let issue = Issue::new(
            IssueType::Task,
            "Needs assignment".to_string(),
            "desc".to_string(),
            Username::from("worker"),
            String::new(),
            IssueStatus::Open.into(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
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

        // Issue remains unassigned; no session spawned.
        let stored = handles.store.get_issue(&issue_id, false).await?.item;
        assert!(
            stored.assignee.is_none(),
            "issue should remain unassigned without an assignment agent; got {:?}",
            stored.assignee
        );
        let sessions = handles.state.list_sessions().await?;
        assert_eq!(sessions.len(), 0, "no session should be spawned");

        Ok(())
    }

    #[tokio::test]
    async fn assigned_issue_is_not_reassigned() -> anyhow::Result<()> {
        let handles = test_utils::test_state_handles();
        let repo_name = RepoName::from_str("dourolabs/hydra")?;

        register_assignment_agent(&handles, "pm").await?;
        register_agent(&handles, "agent-a").await?;
        add_repository(&handles.state, repo_name.clone(), repository()).await?;

        // Issue explicitly assigned to agent-a — should NOT be reassigned to pm.
        let issue = make_issue("agent-a", &repo_name);
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

        let stored = handles.store.get_issue(&issue_id, false).await?.item;
        match stored.assignee {
            Some(hydra_common::principal::Principal::Agent { ref name }) => {
                assert_eq!(
                    name.as_str(),
                    "agent-a",
                    "pre-existing assignee should be preserved"
                );
            }
            other => panic!("expected assignee Principal::Agent('agent-a'); got {other:?}"),
        }

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
            statuses,
            StatusKey::try_new("backlog").unwrap(),
            hydra_common::api::v1::users::Username::try_new("worker").unwrap(),
            false,
            0.0,
        );
        let (project_id, _) = handles
            .store
            .add_project(project, &ActorRef::test())
            .await?;

        let mut issue = make_issue(agent_name, &repo_name);
        issue.project_id = Some(project_id);
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
