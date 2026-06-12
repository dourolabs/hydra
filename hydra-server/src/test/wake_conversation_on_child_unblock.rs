//! Integration test for [`WakeConversationOnChildUnblockAutomation`].
//!
//! The unit tests in the module itself drive the automation directly; this
//! file exercises the full engine path — the automation is registered in
//! `build_default_registry`, activated by `default_policy_config`, and
//! fired by the runner in response to a real `IssueUpdated` event on the
//! bus. The assertion verifies the producer side of the
//! "interactive parent wakes on child completion" invariant
//! end-to-end:
//!
//! 1. The parent's `Idle` conversation flips to `Active` (the wake signal).
//! 2. The most-recent event on the parent's current session is the
//!    `SystemEvent` (dual-write through the chat relay).
//! 3. The rendered string matches `SystemEventKind::ChildUnblocked.render()`.

use crate::app::test_helpers::{
    poll_until, register_agent, seed_linked_conversation, start_test_automation_runner,
    state_with_default_model,
};
use crate::app::{AppState, chat_relay::TO_WORKER_CAPACITY};
use crate::domain::{
    actors::ActorRef,
    conversations::ConversationStatus as DomainConversationStatus,
    issues::{Issue, IssueDependency, IssueDependencyType, IssueType, SessionSettings},
    sessions::SessionEvent as DomainSessionEvent,
    users::Username,
};
use hydra_common::api::v1::{
    projects::{Project as ApiProject, ProjectKey, StatusDefinition, StatusKey},
    sessions::SystemEventKind,
    users::Username as ApiUsername,
};
use hydra_common::{ConversationId, ProjectId, SessionId};
use std::time::Duration;

const POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Seed a project with the statuses needed for the wake invariant:
/// - `open`: the wire default; non-interactive, doesn't unblock.
/// - `chat`: interactive, doesn't unblock parents.
/// - `complete`: terminal, unblocks parents.
async fn seed_project(state: &AppState) -> (ProjectId, StatusKey, StatusKey, StatusKey) {
    let open_key = StatusKey::try_new("open").unwrap();
    let chat_key = StatusKey::try_new("chat").unwrap();
    let complete_key = StatusKey::try_new("complete-wake-it").unwrap();

    let open_def = StatusDefinition::new(
        open_key.clone(),
        "Open".to_string(),
        "#bdc3c7".parse().unwrap(),
        false,
        false,
        false,
        None,
    );

    let mut chat_def = StatusDefinition::new(
        chat_key.clone(),
        "Chat".to_string(),
        "#3498db".parse().unwrap(),
        false,
        false,
        false,
        None,
    );
    chat_def.interactive = true;

    let complete_def = StatusDefinition::new(
        complete_key.clone(),
        "Complete".to_string(),
        "#2ecc71".parse().unwrap(),
        true,
        true,
        false,
        None,
    );

    let project = ApiProject::new(
        ProjectKey::try_new("wake-it-proj").unwrap(),
        "Wake Integration Test".to_string(),
        Vec::new(),
        ApiUsername::from("alice"),
        false,
        0.0,
    );
    let (project_id, _) = state
        .store
        .add_project(project, &ActorRef::test())
        .await
        .unwrap();
    for def in [open_def, chat_def, complete_def] {
        state
            .store
            .add_status(&project_id, def, &ActorRef::test())
            .await
            .unwrap();
    }
    (project_id, chat_key, open_key, complete_key)
}

/// Build an issue in `status_key` under `project_id`, optionally with
/// `dependencies`.
fn project_issue(
    project_id: &ProjectId,
    status_key: &StatusKey,
    description: &str,
    dependencies: Vec<IssueDependency>,
) -> Issue {
    Issue::new(
        IssueType::Task,
        "Test Title".to_string(),
        description.to_string(),
        Username::from("creator"),
        String::new(),
        status_key.clone(),
        project_id.clone(),
        None,
        None,
        dependencies,
        Vec::new(),
        None,
        None,
        None,
    )
}

/// Seed a session linked to `conversation_id` so the chat-relay layer has
/// something to dual-write to when the wake fires. Production spawns
/// this via `SpawnConversationSessionsAutomation`; in this test we want
/// deterministic control over the session's lifetime, so we create it
/// directly.
async fn seed_session_for_conversation(
    state: &AppState,
    conversation_id: &ConversationId,
) -> SessionId {
    use crate::domain::sessions::{AgentConfig, Session, SessionMode};
    use crate::routes::sessions::mount_spec_from_create_request;
    use std::collections::HashMap;
    let session = Session::new(
        Username::from("creator"),
        None,
        None,
        AgentConfig::default(),
        mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
        Some("worker:latest".to_string()),
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Interactive {
            conversation_id: conversation_id.clone(),
            idle_timeout: None,
            greet_user: false,
        },
        crate::domain::task_status::Status::Running,
        None,
        None,
    );
    let (session_id, _) = state
        .store
        .add_session_with_actor(session, chrono::Utc::now(), ActorRef::test())
        .await
        .unwrap();
    session_id
}

/// Mark the chat-relay entry for `conversation_id` Active so subsequent
/// `send_event_to_conversation` calls dual-write to the session log
/// instead of queueing. Mirrors what `handle_relay_socket` does after a
/// real worker handshake; we don't need the worker side here, just the
/// receiver half pinned for the test.
async fn simulate_worker_connect(
    state: &AppState,
    conversation_id: &ConversationId,
    session_id: &SessionId,
) -> tokio::sync::mpsc::Receiver<hydra_common::api::v1::relay::ServerMessage> {
    let (tx, rx) = tokio::sync::mpsc::channel(TO_WORKER_CAPACITY);
    let _ = state
        .chat_relay_map
        .set_active(
            conversation_id.clone(),
            session_id.clone(),
            tx,
            &state.store,
        )
        .await;
    rx
}

#[tokio::test]
async fn child_unblock_wakes_parent_conversation_end_to_end() {
    // The runner spawns the full production policy engine — every
    // automation registered in `build_default_registry` and listed in
    // `default_policy_config` runs. If `wake_conversation_on_child_unblock`
    // is missing from either, this test fails.
    let state = state_with_default_model("default-model");
    let _runner = start_test_automation_runner(&state);
    register_agent(&state, "pm").await;

    let (project_id, chat_key, open_key, complete_key) = seed_project(&state).await;

    // 1) Interactive parent assigned to the pm agent.
    let mut parent_issue = project_issue(&project_id, &chat_key, "parent", vec![]);
    parent_issue.assignee = Some(hydra_common::Principal::agent(
        hydra_common::api::v1::agents::AgentName::try_new("pm").unwrap(),
    ));
    parent_issue.session_settings = SessionSettings::default();
    let (parent_id, _) = state
        .store
        .add_issue_with_actor(parent_issue, ActorRef::test())
        .await
        .unwrap();

    // 2) Parent conversation in Idle (simulating: was Active, session
    //    ended, conversation flipped Idle by
    //    `SpawnConversationSessionsAutomation`).
    let conversation_id =
        seed_linked_conversation(&state, &parent_id, DomainConversationStatus::Idle).await;

    // Seed a session linked to the conversation so the dual-write path
    // has somewhere to land the SystemEvent. In production this is the
    // worker session that just went Idle and is about to be Resumed by
    // the wake — here we keep it Running for simplicity since we only
    // care about the SystemEvent landing on the log.
    let session_id = seed_session_for_conversation(&state, &conversation_id).await;
    let mut _worker_rx = simulate_worker_connect(&state, &conversation_id, &session_id).await;

    // 3) Child issue starting at `open`.
    let child_issue = project_issue(
        &project_id,
        &open_key,
        "child",
        vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_id.clone(),
        )],
    );
    let (child_id, _) = state
        .store
        .add_issue_with_actor(child_issue.clone(), ActorRef::test())
        .await
        .unwrap();

    // 4) Transition the child to `complete` (`unblocks_parents = true`).
    //    This emits `IssueUpdated` on the bus, which the runner fans out
    //    to every registered automation — including the wake automation
    //    under test.
    let mut updated_child = child_issue;
    updated_child.status = complete_key.clone();
    state
        .store
        .update_issue_with_actor(&child_id, updated_child, ActorRef::test())
        .await
        .unwrap();

    // 5a) Parent conversation flips to Active.
    let active_conversation = poll_until(POLL_TIMEOUT, || async {
        let v = state
            .store()
            .get_conversation(&conversation_id, false)
            .await
            .ok()?;
        (v.item.status == DomainConversationStatus::Active).then_some(v.item)
    })
    .await
    .expect("expected parent conversation to flip Idle → Active after child unblock");
    assert_eq!(active_conversation.status, DomainConversationStatus::Active);

    // 5b) The most-recent session event on the parent's session is the
    //     SystemEvent with the expected ChildUnblocked kind.
    let last_event = poll_until(POLL_TIMEOUT, || async {
        let events = state.store().get_session_events(&session_id).await.ok()?;
        events
            .into_iter()
            .filter_map(|v| match v.item {
                DomainSessionEvent::SystemEvent { .. } => Some(v.item),
                _ => None,
            })
            .next_back()
    })
    .await
    .expect("expected a SystemEvent to land on the parent's session log");

    let DomainSessionEvent::SystemEvent { kind, .. } = last_event else {
        panic!("expected SystemEvent, got something else after the filter");
    };
    let expected_kind = SystemEventKind::ChildUnblocked {
        child_id: child_id.clone(),
        new_status: complete_key.clone(),
    };
    assert_eq!(
        kind, expected_kind,
        "SystemEvent kind should name the unblocked child + its new status",
    );

    // 5c) Rendered string matches the canonical projection used by the
    //     worker.
    assert_eq!(kind.render(), expected_kind.render());
    // Belt-and-suspenders: render is non-empty and mentions the child id
    // and new status verbatim. (The exact string is owned by the enum
    // impl; checking presence rather than equality lets the test survive
    // wording tweaks.)
    let rendered = kind.render();
    let child_str: &str = child_id.as_ref();
    assert!(
        rendered.contains(child_str),
        "rendered string should mention the child id; got: {rendered}",
    );
    assert!(
        rendered.contains(complete_key.as_str()),
        "rendered string should mention the new status; got: {rendered}",
    );
}
