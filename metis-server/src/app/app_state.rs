use crate::{
    background::Spawner,
    config::AppConfig,
    job_engine::JobEngine,
    store::{Store, StoreError, Task, TaskExt, TaskResolutionError},
};
use chrono::Utc;
use metis_common::{TaskId, constants::ENV_METIS_ID, jobs::CreateJobRequest};
use std::sync::Arc;
use thiserror::Error;
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

#[derive(Debug, Error)]
pub enum CreateJobError {
    #[error(transparent)]
    TaskResolution(#[from] TaskResolutionError),
    #[error("failed to store job {job_id}")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

impl AppState {
    pub async fn create_job(&self, request: CreateJobRequest) -> Result<TaskId, CreateJobError> {
        let job_id = TaskId::new();
        let fallback_image = self.config.metis.worker_image.clone();

        let mut env_vars = request.variables;
        env_vars.insert(ENV_METIS_ID.to_string(), job_id.to_string());

        let task = Task {
            prompt: request.prompt,
            context: request.context,
            spawned_from: None,
            image: request.image,
            env_vars,
        };

        task.resolve(self.service_state.as_ref(), &fallback_image)?;

        let mut store = self.store.write().await;
        store
            .add_task_with_id(job_id.clone(), task, Utc::now())
            .await
            .map_err(|source| CreateJobError::Store {
                source,
                job_id: job_id.clone(),
            })?;

        Ok(job_id)
    }
}
