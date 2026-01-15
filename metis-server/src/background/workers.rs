use super::{
    ScheduledWorker, WorkerHandle, WorkerOutcome, WorkerSettings, monitor_running_jobs,
    process_pending_jobs, run_spawners,
};
use crate::{
    app::AppState,
    config::{
        DEFAULT_MONITOR_INTERVAL_SECS, DEFAULT_PENDING_INTERVAL_SECS,
        DEFAULT_SPAWNER_INTERVAL_SECS, SchedulerConfig, WORKER_MONITOR_RUNNING_JOBS,
        WORKER_PROCESS_PENDING_JOBS, WORKER_RUN_SPAWNERS,
    },
};
use async_trait::async_trait;
use std::{sync::Arc, time::Duration};

pub fn build_scheduler_workers(state: AppState) -> Vec<WorkerHandle> {
    let scheduler_config = &state.config.background.scheduler;
    let base_backoff = Duration::from_secs(retry_backoff_secs(scheduler_config));
    let max_backoff = Duration::from_secs(max_backoff_secs(scheduler_config));

    vec![
        WorkerHandle::new(
            WorkerSettings::new(
                WORKER_PROCESS_PENDING_JOBS,
                Duration::from_secs(scheduler_config.worker_interval_secs(
                    WORKER_PROCESS_PENDING_JOBS,
                    DEFAULT_PENDING_INTERVAL_SECS,
                )),
                base_backoff,
                max_backoff,
            ),
            Arc::new(PendingWorker {
                state: state.clone(),
            }),
        ),
        WorkerHandle::new(
            WorkerSettings::new(
                WORKER_MONITOR_RUNNING_JOBS,
                Duration::from_secs(scheduler_config.worker_interval_secs(
                    WORKER_MONITOR_RUNNING_JOBS,
                    DEFAULT_MONITOR_INTERVAL_SECS,
                )),
                base_backoff,
                max_backoff,
            ),
            Arc::new(MonitorWorker {
                state: state.clone(),
            }),
        ),
        WorkerHandle::new(
            WorkerSettings::new(
                WORKER_RUN_SPAWNERS,
                Duration::from_secs(
                    scheduler_config
                        .worker_interval_secs(WORKER_RUN_SPAWNERS, DEFAULT_SPAWNER_INTERVAL_SECS),
                ),
                base_backoff,
                max_backoff,
            ),
            Arc::new(SpawnerWorker { state }),
        ),
    ]
}

fn retry_backoff_secs(config: &SchedulerConfig) -> u64 {
    config.retry_backoff_secs.max(1)
}

fn max_backoff_secs(config: &SchedulerConfig) -> u64 {
    config
        .max_backoff_secs
        .max(config.retry_backoff_secs)
        .max(1)
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
