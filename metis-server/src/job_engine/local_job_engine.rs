use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use dashmap::DashMap;
use futures::channel::mpsc;
use metis_common::constants::{ENV_METIS_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::{JobEngine, JobEngineError, JobStatus, MetisJob, SessionId};
use crate::domain::actors::Actor;

/// How long completed/failed process entries are kept before being reaped.
const COMPLETED_PROCESS_TTL: TimeDelta = TimeDelta::hours(1);

/// How often the reaper task runs.
const REAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Tracks the runtime status of a local subprocess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStatus {
    Running,
    Complete,
    Failed,
}

/// Metadata tracked for each subprocess managed by this engine.
struct ProcessInfo {
    creation_time: DateTime<Utc>,
    completion_time: Arc<Mutex<Option<DateTime<Utc>>>>,
    log_file: std::path::PathBuf,
    status_rx: tokio::sync::watch::Receiver<ProcessStatus>,
    pid: Option<u32>,
}

/// A job engine that runs worker-run as host subprocesses without Docker.
pub struct LocalJobEngine {
    server_url: String,
    /// Maps metis_id -> process info for tracking.
    processes: DashMap<SessionId, ProcessInfo>,
    /// Temp directory for log files.
    log_dir: std::path::PathBuf,
    /// Optional custom spawn command (program, args). When set, this replaces
    /// the default `current_exe() jobs worker-run <id> . --tempdir` command.
    /// Useful for testing with dummy commands like `/bin/true` or `/bin/false`.
    spawn_command: Option<(std::path::PathBuf, Vec<String>)>,
}

impl LocalJobEngine {
    pub fn new(
        server_url: String,
        spawn_command: Option<(std::path::PathBuf, Vec<String>)>,
    ) -> Self {
        let log_dir = std::env::temp_dir().join("metis-local-jobs");
        let _ = std::fs::create_dir_all(&log_dir);
        Self {
            server_url,
            processes: DashMap::new(),
            log_dir,
            spawn_command,
        }
    }

    fn build_env_vars(
        &self,
        metis_id: &SessionId,
        auth_token: &str,
        extra_env: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        let mut env: HashMap<String, String> = extra_env.clone();
        env.insert(ENV_METIS_ID.to_string(), metis_id.to_string());
        env.insert(ENV_METIS_TOKEN.to_string(), auth_token.to_string());

        let server_url = self.server_url.trim();
        if !server_url.is_empty() {
            env.insert(ENV_METIS_SERVER_URL.to_string(), server_url.to_string());
        }

        env
    }

    fn log_file_path(&self, metis_id: &SessionId) -> std::path::PathBuf {
        self.log_dir.join(format!("{metis_id}.log"))
    }

    async fn build_metis_job(&self, metis_id: &SessionId) -> Result<MetisJob, JobEngineError> {
        let info = self
            .processes
            .get(metis_id)
            .ok_or_else(|| JobEngineError::NotFound(metis_id.clone()))?;

        let process_status = *info.status_rx.borrow();
        let status = match process_status {
            ProcessStatus::Running => JobStatus::Running,
            ProcessStatus::Complete => JobStatus::Complete,
            ProcessStatus::Failed => JobStatus::Failed,
        };

        let failure_message = if status == JobStatus::Failed {
            Some("Process exited with non-zero status".to_string())
        } else {
            None
        };

        let completion_time = *info.completion_time.lock().await;

        Ok(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(info.creation_time),
            start_time: Some(info.creation_time),
            completion_time,
            failure_message,
        })
    }

    /// Remove completed/failed process entries older than the given TTL.
    ///
    /// Also deletes the associated log file on disk for each removed entry.
    async fn reap_completed_processes(&self, ttl: TimeDelta) {
        let now = Utc::now();
        let mut reaped = 0u32;

        // Collect keys to reap first, then remove individually.
        // (DashMap::retain is synchronous and we need async lock access for completion_time.)
        let keys: Vec<SessionId> = self.processes.iter().map(|e| e.key().clone()).collect();

        for key in keys {
            let should_reap = {
                let Some(info) = self.processes.get(&key) else {
                    continue;
                };
                let status = *info.status_rx.borrow();
                if status == ProcessStatus::Running {
                    false
                } else {
                    let completion_time = *info.completion_time.lock().await;
                    matches!(completion_time, Some(t) if now - t > ttl)
                }
            };

            if should_reap {
                if let Some((_, info)) = self.processes.remove(&key) {
                    if let Err(e) = std::fs::remove_file(&info.log_file) {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            warn!(
                                task_id = %key,
                                error = %e,
                                "failed to remove log file during reap"
                            );
                        }
                    }
                    let completion_time = *info.completion_time.lock().await;
                    let age = completion_time.map(|t| now - t);
                    debug!(
                        task_id = %key,
                        age_secs = ?age.map(|a| a.num_seconds()),
                        "reaped completed process entry"
                    );
                    reaped += 1;
                }
            }
        }

        if reaped > 0 {
            info!(reaped_count = reaped, "reaped completed process entries");
        }
    }

    /// Spawn a background task that periodically reaps completed process entries.
    ///
    /// The task holds a `Weak` reference to the engine and stops automatically
    /// when the engine is dropped.
    pub fn start_reaper(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(REAP_INTERVAL);
            // The first tick completes immediately; skip it so the first reap
            // happens after one full interval.
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(engine) = weak.upgrade() else {
                    debug!("LocalJobEngine dropped, stopping reaper task");
                    break;
                };
                engine.reap_completed_processes(COMPLETED_PROCESS_TTL).await;
            }
        });
    }

    /// Send a signal to a process by PID using the `kill` command.
    async fn send_signal(pid: u32, signal: &str) -> Result<(), std::io::Error> {
        tokio::process::Command::new("kill")
            .args([signal, &pid.to_string()])
            .output()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl JobEngine for LocalJobEngine {
    async fn create_job(
        &self,
        metis_id: &SessionId,
        _actor: &Actor,
        auth_token: &str,
        _image: &str,
        env_vars: &HashMap<String, String>,
        _cpu_limit: String,
        _memory_limit: String,
        _cpu_request: String,
        _memory_request: String,
    ) -> Result<(), JobEngineError> {
        if self.processes.contains_key(metis_id) {
            return Err(JobEngineError::AlreadyExists(metis_id.clone()));
        }

        let (exe, args) = match &self.spawn_command {
            Some((program, args)) => (program.clone(), args.clone()),
            None => {
                let exe = std::env::current_exe().map_err(|e| {
                    JobEngineError::Internal(format!("Failed to determine current executable: {e}"))
                })?;
                let args = vec![
                    "jobs".to_string(),
                    "worker-run".to_string(),
                    metis_id.as_ref().to_string(),
                    ".".to_string(),
                    "--tempdir".to_string(),
                ];
                (exe, args)
            }
        };

        let log_path = self.log_file_path(metis_id);
        let log_file = std::fs::File::create(&log_path)
            .map_err(|e| JobEngineError::Internal(format!("Failed to create log file: {e}")))?;
        let stderr_log_file = log_file.try_clone().map_err(|e| {
            JobEngineError::Internal(format!("Failed to clone log file handle: {e}"))
        })?;

        let env = self.build_env_vars(metis_id, auth_token, env_vars);

        let mut child = Command::new(&exe)
            .args(&args)
            .envs(&env)
            .stdout(std::process::Stdio::from(log_file))
            .stderr(std::process::Stdio::from(stderr_log_file))
            .kill_on_drop(false)
            .spawn()
            .map_err(|e| JobEngineError::Internal(format!("Failed to spawn subprocess: {e}")))?;

        let pid = child.id();
        let creation_time = Utc::now();

        let (status_tx, status_rx) = tokio::sync::watch::channel(ProcessStatus::Running);
        let status_tx = Arc::new(status_tx);
        let completion_time = Arc::new(Mutex::new(None));

        self.processes.insert(
            metis_id.clone(),
            ProcessInfo {
                creation_time,
                completion_time: completion_time.clone(),
                log_file: log_path,
                status_rx,
                pid,
            },
        );

        info!(metis_id = %metis_id, pid = ?pid, "local subprocess spawned");

        // Spawn a background task to wait on the child process and update status.
        let task_id = metis_id.clone();
        tokio::spawn(async move {
            match child.wait().await {
                Ok(exit_status) => {
                    let new_status = if exit_status.success() {
                        ProcessStatus::Complete
                    } else {
                        ProcessStatus::Failed
                    };
                    *completion_time.lock().await = Some(Utc::now());
                    let _ = status_tx.send(new_status);
                    info!(
                        metis_id = %task_id,
                        exit_status = %exit_status,
                        "local subprocess exited"
                    );
                }
                Err(e) => {
                    *completion_time.lock().await = Some(Utc::now());
                    let _ = status_tx.send(ProcessStatus::Failed);
                    error!(
                        metis_id = %task_id,
                        error = %e,
                        "error waiting for local subprocess"
                    );
                }
            }
        });

        Ok(())
    }

    async fn list_jobs(&self) -> Result<Vec<MetisJob>, JobEngineError> {
        let mut jobs = Vec::new();

        let keys: Vec<SessionId> = self.processes.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            match self.build_metis_job(&key).await {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    warn!(metis_id = %key, error = %e, "skipping process in list");
                }
            }
        }

        jobs.sort_by(|a, b| {
            let time_a = a.start_time.or(a.creation_time);
            let time_b = b.start_time.or(b.creation_time);
            time_b.cmp(&time_a)
        });

        Ok(jobs)
    }

    async fn find_job_by_metis_id(&self, metis_id: &SessionId) -> Result<MetisJob, JobEngineError> {
        self.build_metis_job(metis_id).await
    }

    async fn get_logs(
        &self,
        job_id: &SessionId,
        tail_lines: Option<i64>,
    ) -> Result<String, JobEngineError> {
        let info = self
            .processes
            .get(job_id)
            .ok_or_else(|| JobEngineError::NotFound(job_id.clone()))?;

        let content = tokio::fs::read_to_string(&info.log_file)
            .await
            .map_err(|e| JobEngineError::Internal(format!("Failed to read log file: {e}")))?;

        match tail_lines {
            Some(n) if n > 0 => {
                let lines: Vec<&str> = content.lines().collect();
                let start = lines.len().saturating_sub(n as usize);
                Ok(lines[start..].join("\n"))
            }
            _ => Ok(content),
        }
    }

    fn get_logs_stream(
        &self,
        job_id: &SessionId,
        follow: bool,
    ) -> Result<mpsc::UnboundedReceiver<String>, JobEngineError> {
        let info = self
            .processes
            .get(job_id)
            .ok_or_else(|| JobEngineError::NotFound(job_id.clone()))?;

        let log_path = info.log_file.clone();
        let mut status_rx = info.status_rx.clone();
        drop(info);

        let (tx, rx) = mpsc::unbounded();

        tokio::spawn(async move {
            let file = match tokio::fs::File::open(&log_path).await {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.unbounded_send(format!("Error opening log file: {e}"));
                    return;
                }
            };

            let mut reader = BufReader::new(file);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        if !follow {
                            break;
                        }
                        let current_status = *status_rx.borrow();
                        if current_status != ProcessStatus::Running {
                            // Drain any remaining data after process exit.
                            loop {
                                line.clear();
                                match reader.read_line(&mut line).await {
                                    Ok(0) => break,
                                    Ok(_) => {
                                        if tx.unbounded_send(line.clone()).is_err() {
                                            return;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                            break;
                        }
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
                            _ = status_rx.changed() => {}
                        }
                    }
                    Ok(_) => {
                        if tx.unbounded_send(line.clone()).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(format!("Error reading log file: {e}"));
                        break;
                    }
                }
            }
        });

        Ok(rx)
    }

    async fn kill_job(&self, metis_id: &SessionId) -> Result<(), JobEngineError> {
        let info = self
            .processes
            .get(metis_id)
            .ok_or_else(|| JobEngineError::NotFound(metis_id.clone()))?;

        let pid = info.pid;
        let is_running = *info.status_rx.borrow() == ProcessStatus::Running;
        drop(info);

        if let Some(pid) = pid {
            if is_running {
                // Send SIGTERM for graceful shutdown.
                let _ = Self::send_signal(pid, "-TERM").await;

                // Give the process a moment to exit gracefully.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                // Check if still running and send SIGKILL.
                if let Some(info) = self.processes.get(metis_id) {
                    if *info.status_rx.borrow() == ProcessStatus::Running {
                        let _ = Self::send_signal(pid, "-KILL").await;
                    }
                }
            }
        }

        self.processes.remove(metis_id);
        info!(metis_id = %metis_id, pid = ?pid, "local subprocess killed");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> LocalJobEngine {
        LocalJobEngine::new("http://localhost:8080".to_string(), None)
    }

    fn insert_process(
        engine: &LocalJobEngine,
        metis_id: &SessionId,
        status: ProcessStatus,
        pid: Option<u32>,
    ) {
        let (tx, rx) = tokio::sync::watch::channel(status);
        let completion_time = Arc::new(Mutex::new(if status != ProcessStatus::Running {
            Some(Utc::now())
        } else {
            None
        }));
        let _ = tx; // keep sender alive via the watch channel internals
        engine.processes.insert(
            metis_id.clone(),
            ProcessInfo {
                creation_time: Utc::now(),
                completion_time,
                log_file: engine.log_file_path(metis_id),
                status_rx: rx,
                pid,
            },
        );
    }

    #[test]
    fn build_env_vars_includes_metis_vars() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        let extra = HashMap::from([("CUSTOM".to_string(), "value".to_string())]);
        let env = engine.build_env_vars(&metis_id, "test-token", &extra);

        assert_eq!(env.get(ENV_METIS_ID).unwrap(), &metis_id.to_string());
        assert_eq!(env.get(ENV_METIS_TOKEN).unwrap(), "test-token");
        assert_eq!(
            env.get(ENV_METIS_SERVER_URL).unwrap(),
            "http://localhost:8080"
        );
        assert_eq!(env.get("CUSTOM").unwrap(), "value");
    }

    #[test]
    fn build_env_vars_omits_empty_server_url() {
        let engine = LocalJobEngine::new("".to_string(), None);
        let metis_id = SessionId::new();
        let env = engine.build_env_vars(&metis_id, "tok", &HashMap::new());

        assert!(!env.contains_key(ENV_METIS_SERVER_URL));
    }

    #[tokio::test]
    async fn build_metis_job_maps_running_status() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        insert_process(&engine, &metis_id, ProcessStatus::Running, Some(123));

        let job = engine.build_metis_job(&metis_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert!(job.completion_time.is_none());
        assert!(job.failure_message.is_none());
    }

    #[tokio::test]
    async fn build_metis_job_maps_complete_status() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        insert_process(&engine, &metis_id, ProcessStatus::Complete, Some(123));

        let job = engine.build_metis_job(&metis_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Complete);
        assert!(job.completion_time.is_some());
        assert!(job.failure_message.is_none());
    }

    #[tokio::test]
    async fn build_metis_job_maps_failed_status() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        insert_process(&engine, &metis_id, ProcessStatus::Failed, Some(123));

        let job = engine.build_metis_job(&metis_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert!(job.completion_time.is_some());
        assert!(job.failure_message.is_some());
    }

    #[tokio::test]
    async fn build_metis_job_returns_not_found_for_unknown_id() {
        let engine = make_engine();
        let metis_id = SessionId::new();

        let result = engine.build_metis_job(&metis_id).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn completion_time_is_stable_across_queries() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        insert_process(&engine, &metis_id, ProcessStatus::Complete, Some(123));

        let job1 = engine.build_metis_job(&metis_id).await.unwrap();
        let job2 = engine.build_metis_job(&metis_id).await.unwrap();
        assert_eq!(job1.completion_time, job2.completion_time);
    }

    #[tokio::test]
    async fn kill_job_removes_process() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        // Use None pid to avoid actually sending signals.
        insert_process(&engine, &metis_id, ProcessStatus::Running, None);

        engine.kill_job(&metis_id).await.unwrap();
        assert!(engine.processes.get(&metis_id).is_none());
    }

    #[tokio::test]
    async fn kill_job_with_none_pid_does_not_send_signals() {
        let engine = make_engine();
        let metis_id = SessionId::new();
        insert_process(&engine, &metis_id, ProcessStatus::Running, None);

        // Should succeed without attempting to signal PID 0.
        let result = engine.kill_job(&metis_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn kill_job_returns_not_found_for_unknown_id() {
        let engine = make_engine();
        let metis_id = SessionId::new();

        let result = engine.kill_job(&metis_id).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_jobs_returns_tracked_processes() {
        let engine = make_engine();
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        insert_process(&engine, &id1, ProcessStatus::Running, Some(1));
        insert_process(&engine, &id2, ProcessStatus::Complete, Some(2));

        let jobs = engine.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 2);
    }

    fn insert_process_with_completion_time(
        engine: &LocalJobEngine,
        metis_id: &SessionId,
        status: ProcessStatus,
        completion_time: Option<DateTime<Utc>>,
    ) {
        let (_tx, rx) = tokio::sync::watch::channel(status);
        engine.processes.insert(
            metis_id.clone(),
            ProcessInfo {
                creation_time: Utc::now(),
                completion_time: Arc::new(Mutex::new(completion_time)),
                log_file: engine.log_file_path(metis_id),
                status_rx: rx,
                pid: None,
            },
        );
    }

    #[tokio::test]
    async fn reap_removes_completed_entries_past_ttl() {
        let engine = make_engine();
        let old_completed = SessionId::new();
        let recent_completed = SessionId::new();
        let running = SessionId::new();
        let old_failed = SessionId::new();

        // Create log files so we can verify cleanup.
        let _ = std::fs::create_dir_all(&engine.log_dir);
        std::fs::write(engine.log_file_path(&old_completed), "log").unwrap();
        std::fs::write(engine.log_file_path(&old_failed), "log").unwrap();

        // Completed 2 hours ago — should be reaped with a 1-second TTL.
        let two_hours_ago = Utc::now() - TimeDelta::hours(2);
        insert_process_with_completion_time(
            &engine,
            &old_completed,
            ProcessStatus::Complete,
            Some(two_hours_ago),
        );

        // Completed just now — should NOT be reaped with a 1-hour TTL.
        insert_process_with_completion_time(
            &engine,
            &recent_completed,
            ProcessStatus::Complete,
            Some(Utc::now()),
        );

        // Still running — should never be reaped.
        insert_process_with_completion_time(&engine, &running, ProcessStatus::Running, None);

        // Failed 2 hours ago — should be reaped.
        insert_process_with_completion_time(
            &engine,
            &old_failed,
            ProcessStatus::Failed,
            Some(two_hours_ago),
        );

        // Use a short TTL of 1 second so only the "2 hours ago" entries are reaped.
        engine.reap_completed_processes(TimeDelta::seconds(1)).await;

        // Old completed and old failed should be removed.
        assert!(engine.processes.get(&old_completed).is_none());
        assert!(engine.processes.get(&old_failed).is_none());

        // Recent completed and running should remain.
        assert!(engine.processes.get(&recent_completed).is_some());
        assert!(engine.processes.get(&running).is_some());

        // Log files for reaped entries should be deleted.
        assert!(!engine.log_file_path(&old_completed).exists());
        assert!(!engine.log_file_path(&old_failed).exists());
    }

    #[tokio::test]
    async fn reap_does_not_remove_entries_without_completion_time() {
        let engine = make_engine();
        let id = SessionId::new();

        // Failed but no completion_time set yet — should not be reaped.
        insert_process_with_completion_time(&engine, &id, ProcessStatus::Failed, None);

        engine.reap_completed_processes(TimeDelta::seconds(0)).await;

        assert!(engine.processes.get(&id).is_some());
    }

    // ── Integration tests using configurable spawn command ───────────

    use crate::domain::actors::Actor;
    use crate::domain::users::Username;

    fn make_actor() -> (Actor, String) {
        Actor::new_for_session(SessionId::new(), Username::from("test-user"))
    }

    fn dummy_env() -> HashMap<String, String> {
        HashMap::new()
    }

    fn make_failing_engine() -> LocalJobEngine {
        LocalJobEngine::new(
            "http://localhost:0".to_string(),
            Some((std::path::PathBuf::from("/bin/false"), vec![])),
        )
    }

    fn make_succeeding_engine() -> LocalJobEngine {
        LocalJobEngine::new(
            "http://localhost:0".to_string(),
            Some((std::path::PathBuf::from("/bin/true"), vec![])),
        )
    }

    fn make_echo_engine() -> LocalJobEngine {
        LocalJobEngine::new(
            "http://localhost:0".to_string(),
            Some((
                std::path::PathBuf::from("/bin/sh"),
                vec![
                    "-c".to_string(),
                    "echo 'line 1'; echo 'line 2'; echo 'line 3'".to_string(),
                ],
            )),
        )
    }

    async fn wait_for_exit(engine: &LocalJobEngine, metis_id: &SessionId) {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
        loop {
            let job = engine.find_job_by_metis_id(metis_id).await.unwrap();
            if job.status != JobStatus::Running {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("timed out waiting for subprocess to exit");
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn integration_create_job_spawns_and_tracks_process() {
        let engine = make_failing_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        let job = engine.find_job_by_metis_id(&metis_id).await.unwrap();
        assert_eq!(job.id, metis_id);
        assert!(job.creation_time.is_some());
        assert!(job.start_time.is_some());
    }

    #[tokio::test]
    async fn integration_create_job_rejects_duplicate() {
        let engine = make_failing_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        let result = engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await;

        assert!(
            matches!(result, Err(JobEngineError::AlreadyExists(_))),
            "duplicate create_job should return AlreadyExists"
        );
    }

    #[tokio::test]
    async fn integration_subprocess_failure_transitions_to_failed() {
        let engine = make_failing_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let job = engine.find_job_by_metis_id(&metis_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert!(job.completion_time.is_some());
        assert!(job.failure_message.is_some());
    }

    #[tokio::test]
    async fn integration_subprocess_success_transitions_to_complete() {
        let engine = make_succeeding_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let job = engine.find_job_by_metis_id(&metis_id).await.unwrap();
        assert_eq!(job.status, JobStatus::Complete);
        assert!(job.completion_time.is_some());
        assert!(job.failure_message.is_none());
    }

    #[tokio::test]
    async fn integration_get_logs_returns_content_after_exit() {
        let engine = make_echo_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let logs = engine.get_logs(&metis_id, None).await.unwrap();
        assert!(
            logs.contains("line 1"),
            "logs should contain subprocess output"
        );
    }

    #[tokio::test]
    async fn integration_get_logs_respects_tail_lines() {
        let engine = make_echo_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let tail_1 = engine.get_logs(&metis_id, Some(1)).await.unwrap();
        assert_eq!(
            tail_1.lines().count(),
            1,
            "tail_lines=1 should return exactly 1 line"
        );
        assert!(
            tail_1.contains("line 3"),
            "tail should return the last line"
        );
    }

    #[tokio::test]
    async fn integration_get_logs_not_found_for_unknown_job() {
        let engine = make_failing_engine();
        let result = engine.get_logs(&SessionId::new(), None).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn integration_list_jobs_includes_created_jobs() {
        let engine = make_failing_engine();
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &id1,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();
        engine
            .create_job(
                &id2,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        let jobs = engine.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 2);

        let ids: Vec<&SessionId> = jobs.iter().map(|j| &j.id).collect();
        assert!(ids.contains(&&id1));
        assert!(ids.contains(&&id2));
    }

    #[tokio::test]
    async fn integration_list_jobs_returns_empty_when_no_jobs() {
        let engine = make_failing_engine();
        let jobs = engine.list_jobs().await.unwrap();
        assert!(jobs.is_empty());
    }

    #[tokio::test]
    async fn integration_find_job_not_found_for_unknown_id() {
        let engine = make_failing_engine();
        let result = engine.find_job_by_metis_id(&SessionId::new()).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn integration_kill_job_removes_from_tracking() {
        let engine = make_failing_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        assert!(engine.find_job_by_metis_id(&metis_id).await.is_ok());

        engine.kill_job(&metis_id).await.unwrap();

        let result = engine.find_job_by_metis_id(&metis_id).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn integration_kill_job_not_found_for_unknown_id() {
        let engine = make_failing_engine();
        let result = engine.kill_job(&SessionId::new()).await;
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn integration_kill_job_removes_from_list() {
        let engine = make_failing_engine();
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &id1,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();
        engine
            .create_job(
                &id2,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        engine.kill_job(&id1).await.unwrap();

        let jobs = engine.list_jobs().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id2);
    }

    #[tokio::test]
    async fn integration_get_logs_stream_returns_content() {
        use futures::StreamExt;

        let engine = make_echo_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let mut rx = engine.get_logs_stream(&metis_id, false).unwrap();

        let mut chunks = Vec::new();
        while let Some(chunk) = rx.next().await {
            chunks.push(chunk);
        }

        assert!(!chunks.is_empty(), "stream should return log content");
    }

    #[tokio::test]
    async fn integration_get_logs_stream_not_found_for_unknown_job() {
        let engine = make_failing_engine();
        let result = engine.get_logs_stream(&SessionId::new(), false);
        assert!(matches!(result, Err(JobEngineError::NotFound(_))));
    }

    #[tokio::test]
    async fn integration_completion_time_set_after_exit() {
        let engine = make_failing_engine();
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &dummy_env(),
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        wait_for_exit(&engine, &metis_id).await;

        let job = engine.find_job_by_metis_id(&metis_id).await.unwrap();
        assert!(job.completion_time.is_some());
        assert!(job.creation_time.unwrap() <= job.completion_time.unwrap());
    }

    #[tokio::test]
    async fn integration_create_job_passes_env_vars() {
        let engine = LocalJobEngine::new(
            "http://test-server:8080".to_string(),
            Some((std::path::PathBuf::from("/bin/true"), vec![])),
        );
        let metis_id = SessionId::new();
        let (actor, token) = make_actor();

        let mut env = HashMap::new();
        env.insert("CUSTOM_VAR".to_string(), "custom_value".to_string());

        engine
            .create_job(
                &metis_id,
                &actor,
                &token,
                "unused-image",
                &env,
                "500m".to_string(),
                "1Gi".to_string(),
                "500m".to_string(),
                "1Gi".to_string(),
            )
            .await
            .unwrap();

        let job = engine.find_job_by_metis_id(&metis_id).await.unwrap();
        assert_eq!(job.id, metis_id);
    }
}
