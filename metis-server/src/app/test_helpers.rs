use crate::{
    app::{AppState, ServiceState},
    domain::{
        issues::{Issue, IssueDependency, IssueStatus, IssueType},
        jobs::{BundleSpec, Task},
        users::Username,
    },
    store::MemoryStore,
    test_utils::{MockJobEngine, test_app_config},
};
use metis_common::IssueId;
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub fn sample_task() -> Task {
    Task::new(
        "Spawn me".to_string(),
        BundleSpec::None,
        None,
        None,
        Some("worker:latest".to_string()),
        None,
        HashMap::new(),
        None,
        None,
        None,
    )
}

pub fn task_for_issue(issue_id: &IssueId) -> Task {
    Task::new(
        "Spawn me".to_string(),
        BundleSpec::None,
        Some(issue_id.clone()),
        None,
        Some("worker:latest".to_string()),
        None,
        HashMap::new(),
        None,
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
        Arc::new(RwLock::new(Vec::new())),
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
    status: IssueStatus,
    dependencies: Vec<IssueDependency>,
) -> Issue {
    Issue::new(
        IssueType::Task,
        description.to_string(),
        Username::from("creator"),
        String::new(),
        status,
        None,
        None,
        Vec::new(),
        dependencies,
        Vec::new(),
    )
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
