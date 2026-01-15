use crate::{background::Spawner, config::AppConfig, job_engine::JobEngine, store::Store};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::ServiceState;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub service_state: Arc<ServiceState>,
    pub store: Arc<RwLock<Box<dyn Store>>>,
    pub job_engine: Arc<dyn JobEngine>,
    pub spawners: Vec<Arc<dyn Spawner>>,
}
