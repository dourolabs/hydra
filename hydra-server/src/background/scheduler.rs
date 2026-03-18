#![cfg_attr(not(test), allow(dead_code))]

use crate::{
    app::AppState,
    background::{
        cleanup_branches::CleanupBranchesWorker,
        monitor_running_sessions::MonitorRunningSessionsWorker,
    },
    config::WorkerSchedulerConfig,
    policy::integrations::github_pr_poller::GithubPollerWorker,
};
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

pub fn start_background_scheduler(state: AppState) -> BackgroundScheduler {
    let scheduler_config = state.config.background.scheduler.clone();
    let monitor_interval_secs = scheduler_config
        .monitor_running_sessions
        .interval_secs
        .max(1);
    let github_interval_secs = scheduler_config
        .github_poller
        .interval_secs
        .max(state.config.background.github_poller.interval_secs)
        .max(1);
    let cleanup_branches_interval_secs = scheduler_config.cleanup_branches.interval_secs.max(1);
    log_worker_config(
        "monitor_running_sessions",
        monitor_interval_secs,
        &scheduler_config.monitor_running_sessions,
    );
    log_worker_config(
        "github_poller",
        github_interval_secs,
        &scheduler_config.github_poller,
    );
    log_worker_config(
        "cleanup_branches",
        cleanup_branches_interval_secs,
        &scheduler_config.cleanup_branches,
    );

    let workers = vec![
        WorkerHandle::new(
            worker_settings_from_config(
                "monitor_running_sessions",
                monitor_interval_secs,
                &scheduler_config.monitor_running_sessions,
            ),
            Arc::new(MonitorRunningSessionsWorker::new(state.clone())),
        ),
        WorkerHandle::new(
            worker_settings_from_config(
                "github_poller",
                github_interval_secs,
                &scheduler_config.github_poller,
            ),
            Arc::new(GithubPollerWorker::new(state.clone(), github_interval_secs)),
        ),
        WorkerHandle::new(
            worker_settings_from_config(
                "cleanup_branches",
                cleanup_branches_interval_secs,
                &scheduler_config.cleanup_branches,
            ),
            Arc::new(CleanupBranchesWorker::new(state)),
        ),
    ];

    BackgroundScheduler::start(workers)
}

fn log_worker_config(name: &str, interval_secs: u64, config: &WorkerSchedulerConfig) {
    debug!(
        worker = name,
        interval_secs,
        initial_backoff_secs = config.initial_backoff_secs,
        max_backoff_secs = config.max_backoff_secs,
        "scheduler worker configured"
    );
}

fn worker_settings_from_config(
    name: &str,
    interval_secs: u64,
    config: &WorkerSchedulerConfig,
) -> WorkerSettings {
    WorkerSettings::new(
        name.to_string(),
        Duration::from_secs(interval_secs.max(1)),
        Duration::from_secs(config.initial_backoff_secs.max(1)),
        Duration::from_secs(
            config
                .max_backoff_secs
                .max(config.initial_backoff_secs.max(1)),
        ),
    )
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
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };
    use tokio::time::timeout;

    #[derive(Clone)]
    struct CountingWorker {
        calls: Arc<AtomicUsize>,
        outcome: WorkerOutcome,
    }

    #[async_trait]
    impl ScheduledWorker for CountingWorker {
        async fn run_iteration(&self) -> WorkerOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcome.clone()
        }
    }

    struct SequenceWorker {
        outcomes: Mutex<VecDeque<WorkerOutcome>>,
        call_times: Arc<Mutex<Vec<Instant>>>,
    }

    #[async_trait]
    impl ScheduledWorker for SequenceWorker {
        async fn run_iteration(&self) -> WorkerOutcome {
            {
                let mut guard = self.call_times.lock().expect("call_times poisoned");
                guard.push(Instant::now());
            }

            let mut outcomes = self.outcomes.lock().expect("outcomes poisoned");
            outcomes.pop_front().unwrap_or(WorkerOutcome::Idle)
        }
    }

    #[tokio::test]
    async fn runs_worker_until_shutdown() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = Arc::new(CountingWorker {
            calls: calls.clone(),
            outcome: WorkerOutcome::Progress {
                processed: 1,
                failed: 0,
            },
        });

        let scheduler = BackgroundScheduler::start(vec![WorkerHandle::new(
            WorkerSettings::new(
                "counter",
                Duration::from_millis(150),
                Duration::from_millis(120),
                Duration::from_millis(200),
            ),
            worker,
        )]);

        sleep(Duration::from_millis(380)).await;
        scheduler.shutdown().await;

        assert!(
            calls.load(Ordering::SeqCst) >= 2,
            "worker should have run multiple iterations"
        );
    }

    #[tokio::test]
    async fn backs_off_after_error_and_recovers() {
        let call_times = Arc::new(Mutex::new(Vec::new()));
        let worker = Arc::new(SequenceWorker {
            outcomes: Mutex::new(VecDeque::from([
                WorkerOutcome::TransientError {
                    reason: "boom".to_string(),
                },
                WorkerOutcome::Progress {
                    processed: 1,
                    failed: 0,
                },
                WorkerOutcome::Progress {
                    processed: 1,
                    failed: 0,
                },
            ])),
            call_times: call_times.clone(),
        });

        let scheduler = BackgroundScheduler::start(vec![WorkerHandle::new(
            WorkerSettings::new(
                "backoff",
                Duration::from_millis(120),
                Duration::from_millis(250),
                Duration::from_millis(250),
            ),
            worker,
        )]);

        sleep(Duration::from_millis(800)).await;
        scheduler.shutdown().await;

        let times = call_times.lock().expect("call_times poisoned");
        assert!(
            times.len() >= 3,
            "expected at least three iterations, got {}",
            times.len()
        );

        let delta_error = times[1].duration_since(times[0]);
        let delta_recovery = times[2].duration_since(times[1]);

        assert!(
            delta_error >= Duration::from_millis(200),
            "backoff interval should be applied after an error (got {delta_error:?})"
        );
        assert!(
            delta_recovery <= Duration::from_millis(180),
            "interval should reset after progress (got {delta_recovery:?})"
        );
        assert!(
            delta_error > delta_recovery,
            "recovery interval should be shorter than error backoff"
        );
    }

    #[tokio::test]
    async fn shutdown_unblocks_sleeping_worker() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = Arc::new(CountingWorker {
            calls: calls.clone(),
            outcome: WorkerOutcome::Idle,
        });

        let scheduler = BackgroundScheduler::start(vec![WorkerHandle::new(
            WorkerSettings::new(
                "slow",
                Duration::from_secs(5),
                Duration::from_millis(50),
                Duration::from_millis(50),
            ),
            worker,
        )]);

        timeout(Duration::from_millis(200), scheduler.shutdown())
            .await
            .expect("shutdown should not wait full interval");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "worker should not run before shutdown signal is processed"
        );
    }
}
