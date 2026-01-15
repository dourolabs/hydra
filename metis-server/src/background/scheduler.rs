use async_trait::async_trait;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle, time::sleep};
use tracing::{debug, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOutcome {
    Idle,
    Progress { processed: usize, failed: usize },
    TransientError { reason: String },
}

#[async_trait]
pub trait ScheduledWorker: Send + Sync {
    async fn run_iteration(&self) -> WorkerOutcome;
}

#[derive(Clone)]
pub struct WorkerSettings {
    pub name: String,
    pub interval: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl WorkerSettings {
    pub fn new(
        name: impl Into<String>,
        interval: Duration,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Self {
        let interval = interval.max(Duration::from_millis(100));
        let initial_backoff = initial_backoff.max(Duration::from_millis(100));
        let max_backoff = max_backoff.max(initial_backoff);

        Self {
            name: name.into(),
            interval,
            initial_backoff,
            max_backoff,
        }
    }
}

pub struct WorkerHandle {
    pub settings: WorkerSettings,
    pub worker: Arc<dyn ScheduledWorker>,
}

impl WorkerHandle {
    pub fn new(settings: WorkerSettings, worker: Arc<dyn ScheduledWorker>) -> Self {
        Self { settings, worker }
    }
}

pub struct BackgroundScheduler {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
}

impl BackgroundScheduler {
    pub fn start(workers: Vec<WorkerHandle>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handles = workers
            .into_iter()
            .map(|handle| spawn_worker(handle, shutdown_rx.clone()))
            .collect();

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

fn spawn_worker(handle: WorkerHandle, shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut shutdown = shutdown_rx;
        let WorkerHandle { settings, worker } = handle;
        let WorkerSettings {
            name,
            interval,
            initial_backoff,
            max_backoff,
        } = settings;

        let mut next_delay = interval;
        let mut backoff = initial_backoff;
        let mut consecutive_errors = 0usize;

        loop {
            if wait_or_shutdown(next_delay, &mut shutdown).await {
                debug!(worker = %name, "worker exiting due to shutdown");
                break;
            }

            match worker.run_iteration().await {
                WorkerOutcome::Idle => {
                    backoff = initial_backoff;
                    consecutive_errors = 0;
                    next_delay = interval;
                }
                WorkerOutcome::Progress { processed, failed } => {
                    backoff = initial_backoff;
                    consecutive_errors = 0;
                    next_delay = interval;
                    if processed > 0 || failed > 0 {
                        info!(
                            worker = %name,
                            processed,
                            failed,
                            "worker iteration completed"
                        );
                    }
                }
                WorkerOutcome::TransientError { reason } => {
                    consecutive_errors += 1;
                    let wait = backoff.min(max_backoff);
                    next_delay = wait;
                    backoff = (backoff * 2).min(max_backoff);

                    warn!(
                        worker = %name,
                        consecutive_errors,
                        backoff_secs = wait.as_secs_f64(),
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
        app::{AppState, ServiceState},
        background::build_scheduler_workers,
        config::{
            AppConfig, BackgroundSection, MetisSection, SchedulerConfig, ServiceSection,
            WORKER_MONITOR_RUNNING_JOBS, WORKER_PROCESS_PENDING_JOBS, WORKER_RUN_SPAWNERS,
            WorkerIntervalConfig,
        },
        job_engine::{JobEngine, JobStatus, MockJobEngine},
        store::{MemoryStore, Status, Task},
    };
    use chrono::Utc;
    use metis_common::{TaskId, jobs::BundleSpec};
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;
    use tokio::time::{Duration as TokioDuration, sleep as tokio_sleep, timeout};

    fn scheduler_state_with_config(
        job_engine: Arc<dyn JobEngine>,
        scheduler: SchedulerConfig,
    ) -> AppState {
        let app_config = AppConfig {
            metis: MetisSection::default(),
            kubernetes: crate::config::KubernetesSection::default(),
            service: ServiceSection::default(),
            background: BackgroundSection {
                scheduler,
                ..BackgroundSection::default()
            },
        };

        AppState {
            config: Arc::new(app_config),
            service_state: Arc::new(ServiceState::default()),
            store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
            job_engine,
            spawners: Vec::new(),
        }
    }

    fn scheduler_intervals(interval_secs: u64) -> SchedulerConfig {
        SchedulerConfig {
            retry_backoff_secs: 1,
            max_backoff_secs: 2,
            workers: HashMap::from([
                (
                    WORKER_PROCESS_PENDING_JOBS.to_string(),
                    WorkerIntervalConfig { interval_secs },
                ),
                (
                    WORKER_MONITOR_RUNNING_JOBS.to_string(),
                    WorkerIntervalConfig { interval_secs },
                ),
                (
                    WORKER_RUN_SPAWNERS.to_string(),
                    WorkerIntervalConfig { interval_secs },
                ),
            ]),
        }
    }

    async fn wait_for_status(state: &AppState, task_id: &TaskId, expected: Status) {
        let deadline = TokioDuration::from_secs(3);
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
        let state =
            scheduler_state_with_config(engine.clone(), scheduler_intervals(1 /* second */));
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

        let scheduler = BackgroundScheduler::start(build_scheduler_workers(state.clone()));

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
        let state =
            scheduler_state_with_config(engine.clone(), scheduler_intervals(1 /* second */));
        let orphan_id = TaskId::new();
        engine.insert_job(&orphan_id, JobStatus::Running).await;

        let scheduler = BackgroundScheduler::start(build_scheduler_workers(state));

        timeout(TokioDuration::from_secs(3), async {
            loop {
                if let Ok(job) = engine.find_job_by_metis_id(&orphan_id).await {
                    if matches!(job.status, JobStatus::Failed) {
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
        let state = scheduler_state_with_config(engine, scheduler_intervals(5));

        let scheduler = BackgroundScheduler::start(build_scheduler_workers(state.clone()));
        let shutdown_result = timeout(TokioDuration::from_millis(300), scheduler.shutdown()).await;

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
