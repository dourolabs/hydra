use crate::{
    app::{AppState, ServiceState},
    config::{
        AppConfig, BackgroundSection, DatabaseSection, GithubAppSection, JobSection,
        KubernetesSection, MetisSection,
    },
    job_engine::JobEngine,
    run_with_state,
    store::MemoryStore,
};
use reqwest::Client;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::RwLock, task::JoinHandle, time::sleep};

pub mod job_engine;
pub mod store;

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
        },
        database: DatabaseSection::default(),
        github_app: GithubAppSection {
            app_id: 1,
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            private_key: "private-key".to_string(),
        },
        background: BackgroundSection::default(),
    }
}

pub fn test_state_with_engine(job_engine: Arc<dyn JobEngine>) -> AppState {
    AppState {
        config: Arc::new(test_app_config()),
        github_app: None,
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine,
        spawners: Vec::new(),
    }
}

pub fn test_state() -> AppState {
    test_state_with_engine(Arc::new(MockJobEngine::new()))
}

pub fn test_client() -> Client {
    Client::new()
}

pub async fn spawn_test_server() -> anyhow::Result<TestServer> {
    spawn_test_server_with_state(test_state()).await
}

pub async fn spawn_test_server_with_state(state: AppState) -> anyhow::Result<TestServer> {
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
