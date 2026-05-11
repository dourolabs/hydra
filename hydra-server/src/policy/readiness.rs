//! Shared readiness/eligibility predicates used by multiple automations.
//!
//! `spawn_sessions` (session spawning) and `workflow_engine` (FSA state
//! transitions) both need to decide whether an issue is "ready for the
//! next step". The generic eligibility checks live here so they can be
//! reused; caller-specific concerns (agent capacity, existing sessions,
//! feedback bypass, etc.) stay in the caller.

use crate::{
    app::AppState,
    domain::issues::{Issue, IssueDependencyType},
    store::{Status, StoreError},
};
use anyhow::Context;
use hydra_common::IssueId;

/// Returns `true` if any `ChildOf` parent of the issue has a session in
/// `Pending` or `Running` status. Used to avoid acting on an issue while
/// a parent agent session is still actively processing.
pub async fn parent_has_running_task(state: &AppState, issue: &Issue) -> Result<bool, StoreError> {
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
    {
        for task_id in state.get_sessions_for_issue(&dependency.issue_id).await? {
            if matches!(
                state.get_session(&task_id).await?.status,
                Status::Pending | Status::Running
            ) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Returns `true` when an issue is generically eligible to be advanced
/// to the next step. The conditions are:
///
/// 1. The issue's status is not terminal (`Closed`/`Dropped`/`Failed`).
/// 2. All `BlockedOn` dependencies are satisfied — see
///    [`AppState::is_issue_ready`] for the full definition, which also
///    handles re-planning of `InProgress` parents.
/// 3. No `ChildOf` parent has a `Pending`/`Running` session.
///
/// Callers layer additional gates on top (agent capacity, existing
/// sessions, feedback bypass) as needed.
pub async fn is_issue_ready_for_next_step(
    state: &AppState,
    issue_id: &IssueId,
    issue: &Issue,
) -> anyhow::Result<bool> {
    if issue.status.is_terminal() {
        return Ok(false);
    }
    if !state
        .is_issue_ready(issue_id)
        .await
        .context("failed to determine if issue is ready")?
    {
        return Ok(false);
    }
    if parent_has_running_task(state, issue).await? {
        return Ok(false);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            actors::ActorRef,
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType,
                SessionSettings,
            },
            sessions::BundleSpec,
            users::Username,
        },
        store::{Session, Status},
        test_utils::test_state_handles,
    };
    use chrono::Utc;
    use std::collections::HashMap;

    fn user() -> Username {
        Username::from("tester")
    }

    fn issue_with(status: IssueStatus, dependencies: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test".to_string(),
            "desc".to_string(),
            user(),
            String::new(),
            status,
            None,
            Some(SessionSettings::default()),
            Vec::new(),
            dependencies,
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn session_for_issue(issue_id: &IssueId, status: Status) -> Session {
        let mut env_vars = HashMap::new();
        env_vars.insert("HYDRA_ISSUE_ID".to_string(), issue_id.to_string());
        Session::new(
            "prompt".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            user(),
            Some("hydra-worker:latest".to_string()),
            None,
            env_vars,
            None,
            None,
            None,
            None,
            None,
            status,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn is_ready_for_next_step_false_when_terminal() {
        let handles = test_state_handles();
        let (issue_id, _) = handles
            .store
            .add_issue(issue_with(IssueStatus::Closed, vec![]), &ActorRef::test())
            .await
            .unwrap();
        let issue = handles
            .store
            .get_issue(&issue_id, false)
            .await
            .unwrap()
            .item;
        assert!(
            !is_issue_ready_for_next_step(&handles.state, &issue_id, &issue)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn is_ready_for_next_step_false_when_blocked_on_open_dep() {
        let handles = test_state_handles();
        let (blocker_id, _) = handles
            .store
            .add_issue(issue_with(IssueStatus::Open, vec![]), &ActorRef::test())
            .await
            .unwrap();
        let blocked = issue_with(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                blocker_id.clone(),
            )],
        );
        let (blocked_id, _) = handles
            .store
            .add_issue(blocked, &ActorRef::test())
            .await
            .unwrap();
        let issue = handles
            .store
            .get_issue(&blocked_id, false)
            .await
            .unwrap()
            .item;
        assert!(
            !is_issue_ready_for_next_step(&handles.state, &blocked_id, &issue)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn is_ready_for_next_step_true_when_blocker_closed() {
        let handles = test_state_handles();
        let (blocker_id, _) = handles
            .store
            .add_issue(issue_with(IssueStatus::Closed, vec![]), &ActorRef::test())
            .await
            .unwrap();
        let blocked = issue_with(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                blocker_id.clone(),
            )],
        );
        let (blocked_id, _) = handles
            .store
            .add_issue(blocked, &ActorRef::test())
            .await
            .unwrap();
        let issue = handles
            .store
            .get_issue(&blocked_id, false)
            .await
            .unwrap()
            .item;
        assert!(
            is_issue_ready_for_next_step(&handles.state, &blocked_id, &issue)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn parent_has_running_task_detects_running_parent() {
        let handles = test_state_handles();
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue_with(IssueStatus::InProgress, vec![]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (_session_id, _) = handles
            .store
            .add_session(
                session_for_issue(&parent_id, Status::Running),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let child = issue_with(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = handles
            .store
            .add_issue(child, &ActorRef::test())
            .await
            .unwrap();
        let child_issue = handles
            .store
            .get_issue(&child_id, false)
            .await
            .unwrap()
            .item;

        assert!(
            parent_has_running_task(&handles.state, &child_issue)
                .await
                .unwrap()
        );
        assert!(
            !is_issue_ready_for_next_step(&handles.state, &child_id, &child_issue)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn parent_has_running_task_false_when_no_active_parent_session() {
        let handles = test_state_handles();
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue_with(IssueStatus::InProgress, vec![]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (_session_id, _) = handles
            .store
            .add_session(
                session_for_issue(&parent_id, Status::Complete),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let child = issue_with(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, _) = handles
            .store
            .add_issue(child, &ActorRef::test())
            .await
            .unwrap();
        let child_issue = handles
            .store
            .get_issue(&child_id, false)
            .await
            .unwrap()
            .item;

        assert!(
            !parent_has_running_task(&handles.state, &child_issue)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn parent_has_running_task_ignores_non_child_of_deps() {
        let handles = test_state_handles();
        let (parent_id, _) = handles
            .store
            .add_issue(
                issue_with(IssueStatus::InProgress, vec![]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        handles
            .store
            .add_session(
                session_for_issue(&parent_id, Status::Running),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // BlockedOn (not ChildOf) — should not be considered a "parent" here.
        let other = issue_with(
            IssueStatus::Open,
            vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                parent_id.clone(),
            )],
        );
        let (other_id, _) = handles
            .store
            .add_issue(other, &ActorRef::test())
            .await
            .unwrap();
        let issue = handles
            .store
            .get_issue(&other_id, false)
            .await
            .unwrap()
            .item;

        assert!(
            !parent_has_running_task(&handles.state, &issue)
                .await
                .unwrap()
        );
    }
}
