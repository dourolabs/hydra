use crate::{
    app::{AppState, ServiceState},
    domain::{
        actors::ActorRef,
        agents::Agent,
        conversations::{Conversation, ConversationStatus},
        documents::Document,
        issues::{Issue, IssueDependency, IssueType, SessionSettings},
        sessions::{AgentConfig, Session, SessionMode},
        task_status::Status,
        users::Username,
    },
    routes::sessions::mount_spec_from_create_request,
    store::MemoryStore,
    test_utils::{MockJobEngine, test_app_config},
};
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::projects::StatusKey;
use hydra_common::{ConversationId, IssueId};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

pub fn sample_task() -> Session {
    Session::new(
        Username::from("test-creator"),
        None,
        None,
        AgentConfig::default(),
        mount_spec_from_create_request(hydra_common::api::v1::sessions::Bundle::None, None),
        Some("worker:latest".to_string()),
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        Status::Created,
        None,
        None,
    )
}

pub fn task_for_issue(issue_id: &IssueId) -> Session {
    task_for_issue_with_status(issue_id, Status::Created)
}

pub fn task_for_issue_with_status(issue_id: &IssueId, status: Status) -> Session {
    Session::new(
        Username::from("creator"),
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
        status,
        None,
        None,
    )
}

pub fn state_with_default_model(model: &str) -> AppState {
    let mut config = test_app_config();
    config.job.default_model = Some(model.to_string());
    AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        Arc::new(MemoryStore::new()),
        Arc::new(MockJobEngine::new()),
        crate::test_utils::test_secret_manager(),
    )
}

pub fn github_pull_request_response(
    number: u64,
    head_ref: &str,
    base_ref: &str,
    html_url: &str,
) -> serde_json::Value {
    json!({
        "url": format!("https://api.example.com/pulls/{number}"),
        "id": number,
        "number": number,
        "head": {
            "ref": head_ref,
            "sha": "abc123"
        },
        "base": {
            "ref": base_ref,
            "sha": "def456"
        },
        "html_url": html_url
    })
}

pub fn issue_with_status(
    description: &str,
    status: StatusKey,
    dependencies: Vec<IssueDependency>,
) -> Issue {
    Issue::new(
        IssueType::Task,
        "Test Title".to_string(),
        description.to_string(),
        Username::from("creator"),
        status,
        crate::domain::projects::default_project_id(),
        None,
        None,
        dependencies,
        Vec::new(),
        None,
        None,
    )
}

/// Register a minimal agent (`max_tries = 1`, both per-mode caps = 1) with a
/// prompt document at `/agents/<name>/prompt.md`.
pub async fn register_agent(state: &AppState, name: &str) {
    let prompt_path = format!("/agents/{name}/prompt.md");
    let agent = Agent::new(
        name.to_string(),
        prompt_path.clone(),
        None,
        1,
        1,
        1,
        false,
        vec![],
    );
    state.store.add_agent(agent).await.unwrap();
    let doc = Document {
        title: format!("{name} prompt"),
        body_markdown: "agent prompt body".to_string(),
        path: Some(prompt_path.parse().unwrap()),
        archived: false,
    };
    state
        .store
        .add_document_with_actor(doc, ActorRef::test())
        .await
        .unwrap();
}

/// Seed a conversation spawned from `issue_id` with the given status. The
/// conversation is owned by an `swe` agent.
pub async fn seed_linked_conversation(
    state: &AppState,
    issue_id: &IssueId,
    status: ConversationStatus,
) -> ConversationId {
    let conversation = Conversation {
        title: None,
        agent_name: Some(AgentName::try_new("swe").unwrap()),
        status,
        creator: Username::from("creator"),
        session_settings: SessionSettings::default(),
        spawned_from: Some(issue_id.clone()),
        archived: false,
    };
    let (id, _) = state
        .store
        .add_conversation_with_actor(conversation, ActorRef::test())
        .await
        .unwrap();
    id
}

/// Start the automation runner for a test, returning a guard that shuts
/// it down on drop.
pub fn start_test_automation_runner(state: &AppState) -> TestAutomationRunner {
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let handle = crate::policy::runner::spawn_automation_runner(state.clone(), shutdown_rx);
    TestAutomationRunner {
        shutdown_tx,
        handle: Some(handle),
    }
}

pub struct TestAutomationRunner {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TestAutomationRunner {
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

/// Poll a condition until it returns `Some(T)` or the timeout elapses.
pub async fn poll_until<T, F, Fut>(timeout: std::time::Duration, mut f: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = f().await {
            return Some(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
