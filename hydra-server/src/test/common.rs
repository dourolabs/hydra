use crate::app::{AppState, Repository};
use crate::domain::{actors::ActorRef, task_status::Status as TaskStatus};
use hydra_common::{RepoName, SessionId};
use std::str::FromStr;

pub(crate) fn default_image() -> String {
    "hydra-worker:latest".to_string()
}

pub(crate) fn task_id(value: &str) -> SessionId {
    value.parse().expect("task id should be valid")
}

pub(crate) fn service_repo_name() -> RepoName {
    RepoName::from_str("dourolabs/private-repo").expect("service repo name should parse")
}

pub(crate) fn patch_diff() -> String {
    "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
}

pub(crate) fn service_repository() -> (RepoName, Repository) {
    let name = service_repo_name();
    let repository = Repository::new(
        format!("https://example.com/{}.git", name.as_str()),
        Some("develop".to_string()),
        Some("ghcr.io/example/repo:main".to_string()),
    );

    (name, repository)
}

/// Drive a session to a terminal status (`Complete` / `Failed`) via the
/// event-emitting store path so that
/// `SpawnConversationSessionsAutomation` picks up the `SessionUpdated`
/// transition and flips the conversation to `Idle`.
///
/// Tests previously relied on the relay's WS-close / `Suspending` branch to
/// synchronously flip the conversation status; under the trigger-on-transition
/// design that flip is owned by the automation, driven off the session's
/// terminal transition.
pub(crate) async fn mark_session_terminal(
    state: &AppState,
    session_id: &SessionId,
    status: TaskStatus,
) {
    let mut session = state
        .store()
        .get_session(session_id, false)
        .await
        .expect("session must exist")
        .item;
    session.status = status;
    state
        .store
        .update_session_with_actor(session_id, session, ActorRef::test())
        .await
        .expect("update_session_with_actor must succeed");
}
