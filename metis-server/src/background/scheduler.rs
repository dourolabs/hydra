use crate::{
    app::AppState,
    background::{monitor_running_jobs, process_pending_jobs, run_spawners},
};
use std::time::Duration;
use tokio::{sync::watch, task::JoinHandle, time::sleep};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOutcome {
    Idle,
    Progress { processed: usize, failed: usize },
    TransientError { reason: String },
}

pub struct BackgroundScheduler {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
}

impl BackgroundScheduler {
    pub fn start(state: AppState) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler_config = state.config.background.scheduler.clone();

        let pending_interval = Duration::from_secs(scheduler_config.pending_interval_secs.max(1));
        let monitor_interval = Duration::from_secs(scheduler_config.monitor_interval_secs.max(1));
        let spawner_interval = Duration::from_secs(scheduler_config.spawner_interval_secs.max(1));
        let base_backoff = Duration::from_secs(scheduler_config.retry_backoff_secs.max(1));
        let max_backoff = Duration::from_secs(
            scheduler_config
                .max_backoff_secs
                .max(scheduler_config.retry_backoff_secs)
                .max(1),
        );

        let mut handles = Vec::new();

        let pending_state = state.clone();
        handles.push(spawn_worker(
            "process_pending_jobs",
            pending_interval,
            base_backoff,
            max_backoff,
            shutdown_rx.clone(),
            move || {
                let state = pending_state.clone();
                async move { process_pending_jobs(&state).await }
            },
        ));

        let monitor_state = state.clone();
        handles.push(spawn_worker(
            "monitor_running_jobs",
            monitor_interval,
            base_backoff,
            max_backoff,
            shutdown_rx.clone(),
            move || {
                let state = monitor_state.clone();
                async move { monitor_running_jobs(&state).await }
            },
        ));

        let spawner_state = state.clone();
        handles.push(spawn_worker(
            "run_spawners",
            spawner_interval,
            base_backoff,
            max_backoff,
            shutdown_rx,
            move || {
                let state = spawner_state.clone();
                async move { run_spawners(&state).await }
            },
        ));

        Self {
            shutdown_tx,
            handles,
        }
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        for handle in self.handles {
            let _ = handle.await;
        }
    }
}

fn spawn_worker<F, Fut>(
    name: &'static str,
    interval: Duration,
    base_backoff: Duration,
    max_backoff: Duration,
    shutdown_rx: watch::Receiver<bool>,
    mut worker: F,
) -> JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = WorkerOutcome> + Send + 'static,
{
    tokio::spawn(async move {
        let mut shutdown = shutdown_rx;
        let mut next_delay = interval;
        let mut backoff = base_backoff;
        let mut consecutive_errors = 0usize;

        loop {
            if wait_or_shutdown(next_delay, &mut shutdown).await {
                debug!(worker = name, "worker exiting due to shutdown");
                break;
            }

            match worker().await {
                WorkerOutcome::Idle => {
                    backoff = base_backoff;
                    consecutive_errors = 0;
                    next_delay = interval;
                }
                WorkerOutcome::Progress { processed, failed } => {
                    backoff = base_backoff;
                    consecutive_errors = 0;
                    next_delay = interval;
                    if processed > 0 || failed > 0 {
                        info!(
                            worker = name,
                            processed, failed, "worker iteration completed"
                        );
                    }
                }
                WorkerOutcome::TransientError { reason } => {
                    consecutive_errors += 1;
                    let wait = backoff.min(max_backoff);
                    next_delay = wait;
                    backoff = (backoff * 2).min(max_backoff);

                    warn!(
                        worker = name,
                        consecutive_errors,
                        backoff_secs = wait.as_secs(),
                        error = %reason,
                        "worker iteration failed; backing off"
                    );
                }
            }
        }
    })
}

async fn wait_or_shutdown(duration: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }

    tokio::select! {
        _ = sleep(duration) => false,
        _ = shutdown.changed() => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ServiceState,
        config::{AppConfig, BackgroundSection, MetisSection, SchedulerConfig, ServiceSection},
        job_engine::{JobEngine, JobStatus, MockJobEngine},
        store::{MemoryStore, Status, Task},
    };
    use chrono::Utc;
    use metis_common::{TaskId, jobs::BundleSpec};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio::time::{Duration as TokioDuration, sleep as tokio_sleep, timeout};

    fn scheduler_state_with_config(
        job_engine: Arc<dyn JobEngine>,
        scheduler: SchedulerConfig,
    ) -> AppState {
        let config = AppConfig {
            metis: MetisSection::default(),
            kubernetes: crate::config::KubernetesSection::default(),
            service: ServiceSection::default(),
            background: BackgroundSection {
                scheduler,
                ..BackgroundSection::default()
            },
        };

        AppState {
            config: Arc::new(config),
            service_state: Arc::new(ServiceState::default()),
            store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
            job_engine,
            spawners: Vec::new(),
        }
    }

    fn short_scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            pending_interval_secs: 1,
            monitor_interval_secs: 1,
            spawner_interval_secs: 1,
            retry_backoff_secs: 1,
            max_backoff_secs: 2,
        }
    }

    async fn wait_for_status(state: &AppState, task_id: &TaskId, expected: Status) {
        let deadline = TokioDuration::from_secs(2);
        timeout(deadline, async {
            loop {
                if let Ok(status) = state.store.read().await.get_status(task_id).await {
                    if status == expected {
                        break;
                    }
                }
                tokio_sleep(TokioDuration::from_millis(25)).await;
            }
        })
        .await
        .expect("status should transition in time");
    }

    #[tokio::test]
    async fn processes_pending_and_marks_failure_from_engine() {
        let engine = Arc::new(MockJobEngine::new());
        let state = scheduler_state_with_config(engine.clone(), short_scheduler_config());
        let task = Task {
            prompt: "do it".to_string(),
            context: BundleSpec::None,
            spawned_from: None,
            image: None,
            env_vars: Default::default(),
        };

        let task_id = {
            let mut store = state.store.write().await;
            store.add_task(task, Utc::now()).await.unwrap()
        };

        let scheduler = BackgroundScheduler::start(state.clone());

        wait_for_status(&state, &task_id, Status::Running).await;

        engine
            .set_job_status(&task_id, JobStatus::Failed, Some("boom".into()))
            .await;

        wait_for_status(&state, &task_id, Status::Failed).await;

        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn kills_orphaned_jobs_not_in_store() {
        let engine = Arc::new(MockJobEngine::new());
        let state = scheduler_state_with_config(engine.clone(), short_scheduler_config());
        let orphan_id = TaskId::new();
        engine.insert_job(&orphan_id, JobStatus::Running).await;

        let scheduler = BackgroundScheduler::start(state);

        timeout(TokioDuration::from_secs(2), async {
            loop {
                if let Ok(job) = engine.find_job_by_metis_id(&orphan_id).await {
                    if job.status == JobStatus::Failed {
                        break;
                    }
                }
                tokio_sleep(TokioDuration::from_millis(50)).await;
            }
        })
        .await
        .expect("orphaned job should be killed");

        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_cancels_workers_quickly() {
        let engine = Arc::new(MockJobEngine::new());
        let config = SchedulerConfig {
            pending_interval_secs: 5,
            monitor_interval_secs: 5,
            spawner_interval_secs: 5,
            retry_backoff_secs: 1,
            max_backoff_secs: 2,
        };
        let state = scheduler_state_with_config(engine, config);

        let scheduler = BackgroundScheduler::start(state.clone());
        let shutdown_result = timeout(TokioDuration::from_millis(250), scheduler.shutdown()).await;

        assert!(
            shutdown_result.is_ok(),
            "shutdown should complete without waiting full interval"
        );

        let store = state.store.read().await;
        assert!(
            store
                .list_tasks_with_status(Status::Running)
                .await
                .unwrap_or_default()
                .is_empty(),
            "no jobs should be running after immediate shutdown"
        );
    }
}
