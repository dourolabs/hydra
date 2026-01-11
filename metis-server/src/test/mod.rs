use crate::{
    AppState,
    config::{AppConfig, KubernetesSection, MetisSection, ServiceSection},
    job_engine::{JobEngine, MockJobEngine},
    run_with_state,
    state::ServiceState,
    store::MemoryStore,
};
use reqwest::Client;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::RwLock, task::JoinHandle, time::sleep};

pub(crate) struct TestServer {
    pub(crate) address: String,
    handle: JoinHandle<anyhow::Result<()>>,
}

impl TestServer {
    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(crate) fn test_app_config() -> AppConfig {
    AppConfig {
        metis: MetisSection::default(),
        kubernetes: KubernetesSection::default(),
        service: ServiceSection::default(),
    }
}

pub(crate) fn test_state_with_engine(job_engine: Arc<dyn JobEngine>) -> AppState {
    AppState {
        config: Arc::new(test_app_config()),
        service_state: Arc::new(ServiceState::default()),
        store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
        job_engine,
        spawners: Vec::new(),
    }
}

pub(crate) fn test_state() -> AppState {
    test_state_with_engine(Arc::new(MockJobEngine::new()))
}

pub(crate) fn test_client() -> Client {
    Client::new()
}

pub(crate) async fn spawn_test_server() -> anyhow::Result<TestServer> {
    spawn_test_server_with_state(test_state()).await
}

pub(crate) async fn spawn_test_server_with_state(state: AppState) -> anyhow::Result<TestServer> {
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
