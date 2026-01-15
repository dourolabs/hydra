use super::{
    ScheduledWorker, WorkerHandle, WorkerOutcome, WorkerSettings, monitor_running_jobs,
    poll_github_patches, process_pending_jobs, run_spawners,
};
use crate::{
    app::AppState,
    config::{
        DEFAULT_MONITOR_INTERVAL_SECS, DEFAULT_PENDING_INTERVAL_SECS,
        DEFAULT_SPAWNER_INTERVAL_SECS, SchedulerConfig, WORKER_MONITOR_RUNNING_JOBS,
        WORKER_POLL_GITHUB_PATCHES, WORKER_PROCESS_PENDING_JOBS, WORKER_RUN_SPAWNERS,
    },
};
use async_trait::async_trait;
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

pub fn build_scheduler_workers(state: AppState) -> Vec<WorkerHandle> {
    let scheduler_config = &state.config.background.scheduler;

    let github_interval = state.config.background.github_poller.interval_secs.max(1);
    let github_timings = scheduler_config.timings_for(WORKER_POLL_GITHUB_PATCHES, github_interval);
    let github_max_per_cycle = state
        .config
        .background
        .github_poller
        .max_patches_per_run
        .unwrap_or_else(|| poll_github_patches::max_patches_per_cycle(github_timings.interval_secs))
        .max(1);

    vec![
        WorkerHandle::new(
            settings_for(
                scheduler_config,
                WORKER_PROCESS_PENDING_JOBS,
                DEFAULT_PENDING_INTERVAL_SECS,
            ),
            Arc::new(PendingWorker {
                state: state.clone(),
            }),
        ),
        WorkerHandle::new(
            settings_for(
                scheduler_config,
                WORKER_MONITOR_RUNNING_JOBS,
                DEFAULT_MONITOR_INTERVAL_SECS,
            ),
            Arc::new(MonitorWorker {
                state: state.clone(),
            }),
        ),
        WorkerHandle::new(
            settings_for(
                scheduler_config,
                WORKER_RUN_SPAWNERS,
                DEFAULT_SPAWNER_INTERVAL_SECS,
            ),
            Arc::new(SpawnerWorker {
                state: state.clone(),
            }),
        ),
        WorkerHandle::new(
            WorkerSettings::new(
                WORKER_POLL_GITHUB_PATCHES,
                Duration::from_secs(github_timings.interval_secs),
                Duration::from_secs(github_timings.initial_backoff_secs),
                Duration::from_secs(github_timings.max_backoff_secs),
            ),
            Arc::new(GithubPollerWorker {
                state,
                max_patches_per_cycle: github_max_per_cycle,
                start_from: Arc::new(Mutex::new(0usize)),
            }),
        ),
    ]
}

fn settings_for(
    config: &SchedulerConfig,
    worker: &str,
    default_interval_secs: u64,
) -> WorkerSettings {
    let timings = config.timings_for(worker, default_interval_secs);
    WorkerSettings::new(
        worker,
        Duration::from_secs(timings.interval_secs),
        Duration::from_secs(timings.initial_backoff_secs),
        Duration::from_secs(timings.max_backoff_secs),
    )
}

#[derive(Clone)]
struct PendingWorker {
    state: AppState,
}

#[async_trait]
impl ScheduledWorker for PendingWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        process_pending_jobs(&self.state).await
    }
}

#[derive(Clone)]
struct MonitorWorker {
    state: AppState,
}

#[async_trait]
impl ScheduledWorker for MonitorWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        monitor_running_jobs(&self.state).await
    }
}

#[derive(Clone)]
struct SpawnerWorker {
    state: AppState,
}

#[async_trait]
impl ScheduledWorker for SpawnerWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        run_spawners(&self.state).await
    }
}

#[derive(Clone)]
struct GithubPollerWorker {
    state: AppState,
    max_patches_per_cycle: usize,
    start_from: Arc<Mutex<usize>>,
}

#[async_trait]
impl ScheduledWorker for GithubPollerWorker {
    async fn run_iteration(&self) -> WorkerOutcome {
        let mut start_from = self.start_from.lock().await;
        poll_github_patches(&self.state, self.max_patches_per_cycle, &mut start_from).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::ServiceState,
        background::BackgroundScheduler,
        config::{
            AppConfig, BackgroundSection, GithubPollerConfig, KubernetesSection, MetisSection,
            SchedulerConfig, ServiceSection, WORKER_PROCESS_PENDING_JOBS, WorkerSchedule,
        },
        job_engine::MockJobEngine,
        store::MemoryStore,
    };
    use metis_common::jobs::Task;
    use std::{collections::HashMap, time::Duration};
    use tokio::{sync::RwLock, time::timeout};

    fn base_state(background: BackgroundSection) -> AppState {
        AppState {
            config: Arc::new(AppConfig {
                metis: MetisSection::default(),
                kubernetes: KubernetesSection::default(),
                service: ServiceSection::default(),
                background,
            }),
            service_state: Arc::new(ServiceState::default()),
            store: Arc::new(RwLock::new(Box::new(MemoryStore::new()))),
            job_engine: Arc::new(MockJobEngine::new()),
            spawners: Vec::new(),
        }
    }

    #[tokio::test]
    async fn builder_uses_worker_overrides() {
        let mut workers = HashMap::new();
        workers.insert(
            WORKER_PROCESS_PENDING_JOBS.to_string(),
            WorkerSchedule {
                interval_secs: Some(7),
                initial_backoff_secs: Some(2),
                max_backoff_secs: Some(4),
            },
        );
        let background = BackgroundSection {
            agent_queues: Vec::new(),
            github_poller: GithubPollerConfig::default(),
            scheduler: SchedulerConfig {
                default_initial_backoff_secs: 1,
                default_max_backoff_secs: 10,
                workers,
            },
        };

        let state = base_state(background);
        let workers = build_scheduler_workers(state);
        let pending = workers
            .iter()
            .find(|worker| worker.settings.name == WORKER_PROCESS_PENDING_JOBS)
            .expect("pending worker should exist");

        assert_eq!(pending.settings.interval, Duration::from_secs(7));
        assert_eq!(pending.settings.initial_backoff, Duration::from_secs(2));
        assert_eq!(pending.settings.max_backoff, Duration::from_secs(4));
    }

    #[tokio::test]
    async fn scheduler_processes_pending_jobs() {
        let background = BackgroundSection::default();
        let engine = Arc::new(MockJobEngine::new());
        let state = AppState {
            job_engine: engine.clone(),
            ..base_state(background)
        };

        let task = Task {
            prompt: "hi".into(),
            context: metis_common::jobs::BundleSpec::None,
            spawned_from: None,
            image: None,
            env_vars: Default::default(),
        };

        let task_id = {
            let mut store = state.store.write().await;
            store.add_task(task, chrono::Utc::now()).await.unwrap()
        };

        let scheduler = BackgroundScheduler::start(build_scheduler_workers(state.clone()));

        let _ = timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(status) = state.store.read().await.get_status(&task_id).await {
                    if status == crate::store::Status::Running {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        scheduler.shutdown().await;

        let status = state.store.read().await.get_status(&task_id).await.unwrap();
        assert_eq!(status, crate::store::Status::Running);
        assert!(engine.env_vars_for_job(&task_id).is_some());
    }
}
