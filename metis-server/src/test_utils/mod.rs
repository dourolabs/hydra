use crate::{
    app::{AppState, Repository, ServiceState},
    config::{
        AppConfig, BackgroundSection, DatabaseSection, GithubAppSection, JobSection,
        KubernetesSection, MetisSection,
    },
    domain::actors::Actor,
    job_engine::JobEngine,
    run_with_state,
    store::{MemoryStore, StoreError},
};
use anyhow::Context;
use metis_common::{RepoName, TaskId};
use reqwest::{Client, header};
use std::{
    sync::Arc,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::{sync::RwLock, task::JoinHandle, time::sleep};

pub mod github_test_utils;
pub mod job_engine;
pub mod store;

pub use github_test_utils::{
    github_user_response, test_state_with_github_api_base_url, test_state_with_github_urls,
};
pub use job_engine::MockJobEngine;
pub use store::FailingStore;

pub struct TestServer {
    pub address: String,
    handle: JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub fn test_app_config() -> AppConfig {
    AppConfig {
        metis: MetisSection::default(),
        kubernetes: KubernetesSection::default(),
        job: JobSection {
            default_image: "metis-worker:latest".to_string(),
            cpu_limit: "500m".to_string(),
            memory_limit: "1Gi".to_string(),
            cpu_request: "500m".to_string(),
            memory_request: "1Gi".to_string(),
        },
        database: DatabaseSection::default(),
        github_app: GithubAppSection {
            app_id: 1,
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            private_key: "private-key".to_string(),
            api_base_url: "https://api.github.com".to_string(),
            oauth_base_url: "https://github.com".to_string(),
        },
        background: BackgroundSection::default(),
    }
}

pub fn test_state_with_engine(job_engine: Arc<dyn JobEngine>) -> AppState {
    AppState::new(
        Arc::new(test_app_config()),
        None,
        Arc::new(ServiceState::default()),
        Arc::new(MemoryStore::new()),
        job_engine,
        Arc::new(RwLock::new(Vec::new())),
    )
}

pub fn test_state() -> AppState {
    test_state_with_engine(Arc::new(MockJobEngine::new()))
}

pub async fn add_repository(
    state: &AppState,
    name: RepoName,
    config: Repository,
) -> anyhow::Result<()> {
    state
        .create_repository(name, config)
        .await
        .context("failed to add repository to test state")?;
    Ok(())
}

pub async fn test_state_with_repo(name: RepoName, config: Repository) -> anyhow::Result<AppState> {
    let state = test_state();
    add_repository(&state, name, config).await?;
    Ok(state)
}

fn test_auth() -> (Actor, String) {
    static TEST_AUTH: OnceLock<(Actor, String)> = OnceLock::new();
    TEST_AUTH
        .get_or_init(|| Actor::new_for_task(TaskId::new()))
        .clone()
}

pub fn test_auth_token() -> String {
    let (_, token) = test_auth();
    token
}

pub fn test_client() -> Client {
    let mut headers = header::HeaderMap::new();
    let auth_value = format!("Bearer {}", test_auth_token());
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&auth_value).expect("valid test auth header"),
    );

    Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build test client")
}

pub fn test_client_without_auth() -> Client {
    Client::new()
}

pub async fn spawn_test_server() -> anyhow::Result<TestServer> {
    spawn_test_server_with_state(test_state()).await
}

pub async fn spawn_test_server_with_state(state: AppState) -> anyhow::Result<TestServer> {
    seed_test_actor(&state).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let server_state = state.clone();
    let handle = tokio::spawn(async move { run_with_state(server_state, listener).await });
    let server = TestServer {
        address: addr.to_string(),
        handle,
    };

    wait_for_server_ready(&server.base_url()).await?;
    Ok(server)
}

async fn seed_test_actor(state: &AppState) -> anyhow::Result<()> {
    let (actor, _) = test_auth();
    match state.add_actor(actor).await {
        Ok(_) => Ok(()),
        Err(StoreError::ActorAlreadyExists(_)) => Ok(()),
        Err(err) => Err(anyhow::anyhow!("failed to seed test actor: {err}")),
    }
}

async fn wait_for_server_ready(base_url: &str) -> anyhow::Result<()> {
    let client = Client::new();
    let health_url = format!("{base_url}/health");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut delay = Duration::from_millis(25);
    while Instant::now() < deadline {
        if client.get(&health_url).send().await.is_ok() {
            return Ok(());
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(200));
    }

    Err(anyhow::anyhow!(
        "test server did not respond at {health_url}"
    ))
}
