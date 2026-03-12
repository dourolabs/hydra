use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures::channel::mpsc;
use metis_common::constants::{ENV_METIS_ID, ENV_METIS_SERVER_URL, ENV_METIS_TOKEN};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{error, info, warn};

use super::{JobEngine, JobEngineError, JobStatus, MetisJob, TaskId};
use crate::domain::actors::Actor;

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
    log_file: std::path::PathBuf,
    status_rx: tokio::sync::watch::Receiver<ProcessStatus>,
    pid: u32,
}

/// A job engine that runs worker-run as host subprocesses without Docker.
pub struct LocalJobEngine {
    server_url: String,
    /// Maps metis_id -> process info for tracking.
    processes: DashMap<TaskId, ProcessInfo>,
    /// Temp directory for log files.
    log_dir: std::path::PathBuf,
}

impl LocalJobEngine {
    pub fn new(server_url: String) -> Self {
        let log_dir = std::env::temp_dir().join("metis-local-jobs");
        let _ = std::fs::create_dir_all(&log_dir);
        Self {
            server_url,
            processes: DashMap::new(),
            log_dir,
        }
    }

    fn build_env_vars(
        &self,
        metis_id: &TaskId,
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

    fn log_file_path(&self, metis_id: &TaskId) -> std::path::PathBuf {
        self.log_dir.join(format!("{metis_id}.log"))
    }

    fn build_metis_job(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
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

        Ok(MetisJob {
            id: metis_id.clone(),
            status,
            creation_time: Some(info.creation_time),
            start_time: Some(info.creation_time),
            completion_time: match status {
                JobStatus::Complete | JobStatus::Failed => Some(Utc::now()),
                _ => None,
            },
            failure_message,
        })
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
        metis_id: &TaskId,
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

        let exe = std::env::current_exe().map_err(|e| {
            JobEngineError::Internal(format!("Failed to determine current executable: {e}"))
        })?;

        let log_path = self.log_file_path(metis_id);
        let log_file = std::fs::File::create(&log_path)
            .map_err(|e| JobEngineError::Internal(format!("Failed to create log file: {e}")))?;
        let stderr_log_file = log_file.try_clone().map_err(|e| {
            JobEngineError::Internal(format!("Failed to clone log file handle: {e}"))
        })?;

        let env = self.build_env_vars(metis_id, auth_token, env_vars);

        let mut child = Command::new(&exe)
            .args(["jobs", "worker-run", metis_id.as_ref(), ".", "--tempdir"])
            .envs(&env)
            .stdout(std::process::Stdio::from(log_file))
            .stderr(std::process::Stdio::from(stderr_log_file))
            .kill_on_drop(false)
            .spawn()
            .map_err(|e| JobEngineError::Internal(format!("Failed to spawn subprocess: {e}")))?;

        let pid = child.id().unwrap_or(0);
        let creation_time = Utc::now();

        let (status_tx, status_rx) = tokio::sync::watch::channel(ProcessStatus::Running);
        let status_tx = Arc::new(status_tx);

        self.processes.insert(
            metis_id.clone(),
            ProcessInfo {
                creation_time,
                log_file: log_path,
                status_rx,
                pid,
            },
        );

        info!(metis_id = %metis_id, pid = pid, "local subprocess spawned");

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
                    let _ = status_tx.send(new_status);
                    info!(
                        metis_id = %task_id,
                        exit_status = %exit_status,
                        "local subprocess exited"
                    );
                }
                Err(e) => {
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

        for entry in self.processes.iter() {
            match self.build_metis_job(entry.key()) {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    warn!(metis_id = %entry.key(), error = %e, "skipping process in list");
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

    async fn find_job_by_metis_id(&self, metis_id: &TaskId) -> Result<MetisJob, JobEngineError> {
        self.build_metis_job(metis_id)
    }

    async fn get_logs(
        &self,
        job_id: &TaskId,
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
        job_id: &TaskId,
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

    async fn kill_job(&self, metis_id: &TaskId) -> Result<(), JobEngineError> {
        let info = self
            .processes
            .get(metis_id)
            .ok_or_else(|| JobEngineError::NotFound(metis_id.clone()))?;

        let pid = info.pid;
        let is_running = *info.status_rx.borrow() == ProcessStatus::Running;
        drop(info);

        if pid > 0 && is_running {
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

        self.processes.remove(metis_id);
        info!(metis_id = %metis_id, pid = pid, "local subprocess killed");

        Ok(())
    }
}
