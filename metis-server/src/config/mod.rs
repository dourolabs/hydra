pub mod kube;

pub use kube::build_kube_client;

use anyhow::{Context, Result};
use metis_common::{RepoName, jobs::BundleSpec};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default)]
    pub metis: MetisSection,
    #[serde(default)]
    pub kubernetes: KubernetesSection,
    #[serde(default)]
    pub service: ServiceSection,
    #[serde(default)]
    pub background: BackgroundSection,
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let resolved_path = expand_path(path);
        let contents = fs::read_to_string(&resolved_path).with_context(|| {
            format!(
                "Unable to read configuration file '{}'",
                resolved_path.display()
            )
        })?;
        toml::from_str(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetisSection {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default = "default_worker_image")]
    pub worker_image: String,
    #[serde(default)]
    pub server_hostname: String,
    #[serde(default, rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
}

impl Default for MetisSection {
    fn default() -> Self {
        Self {
            namespace: default_namespace(),
            worker_image: default_worker_image(),
            server_hostname: String::new(),
            openai_api_key: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct KubernetesSection {
    #[serde(default)]
    pub in_cluster: bool,
    #[serde(default = "default_kubeconfig_path")]
    pub config_path: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub cluster_name: String,
    #[serde(default)]
    pub api_server: String,
}

impl Default for KubernetesSection {
    fn default() -> Self {
        Self {
            in_cluster: false,
            config_path: default_kubeconfig_path(),
            context: String::new(),
            cluster_name: String::new(),
            api_server: String::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceSection {
    #[serde(default)]
    pub repositories: HashMap<RepoName, Repository>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct BackgroundSection {
    #[serde(default)]
    pub agent_queues: Vec<AgentQueueConfig>,
    #[serde(default)]
    pub github_poller: GithubPollerConfig,
    #[serde(default)]
    pub scheduler: SchedulerSection,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Repository {
    pub remote_url: String,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub default_image: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentQueueConfig {
    pub name: String,
    pub prompt: String,
    #[serde(default = "default_bundle_spec")]
    pub context: BundleSpec,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default = "default_agent_max_tries")]
    pub max_tries: u32,
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubPollerConfig {
    #[serde(default = "default_github_poll_interval_secs")]
    pub interval_secs: u64,
}

impl Default for GithubPollerConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_github_poll_interval_secs(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SchedulerSection {
    #[serde(default = "default_process_pending_scheduler")]
    pub process_pending_jobs: WorkerSchedulerConfig,
    #[serde(default = "default_monitor_running_scheduler")]
    pub monitor_running_jobs: WorkerSchedulerConfig,
    #[serde(default = "default_run_spawners_scheduler")]
    pub run_spawners: WorkerSchedulerConfig,
    #[serde(default = "default_github_poller_scheduler")]
    pub github_poller: WorkerSchedulerConfig,
}

impl Default for SchedulerSection {
    fn default() -> Self {
        Self {
            process_pending_jobs: default_process_pending_scheduler(),
            monitor_running_jobs: default_monitor_running_scheduler(),
            run_spawners: default_run_spawners_scheduler(),
            github_poller: default_github_poller_scheduler(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerSchedulerConfig {
    #[serde(default = "default_scheduler_interval_secs")]
    pub interval_secs: u64,
    #[serde(default = "default_scheduler_initial_backoff_secs")]
    pub initial_backoff_secs: u64,
    #[serde(default = "default_scheduler_max_backoff_secs")]
    pub max_backoff_secs: u64,
}

impl Default for WorkerSchedulerConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_scheduler_interval_secs(),
            initial_backoff_secs: default_scheduler_initial_backoff_secs(),
            max_backoff_secs: default_scheduler_max_backoff_secs(),
        }
    }
}

pub(crate) fn expand_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    match path.to_str() {
        Some(raw) if raw.starts_with('~') => PathBuf::from(shellexpand::tilde(raw).into_owned()),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn non_empty(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.trim())
    }
}

fn default_namespace() -> String {
    "default".to_string()
}

fn default_worker_image() -> String {
    "metis-worker:latest".to_string()
}

pub const DEFAULT_AGENT_MAX_TRIES: u32 = 3;

fn default_kubeconfig_path() -> String {
    "~/.kube/config".to_string()
}

fn default_bundle_spec() -> BundleSpec {
    BundleSpec::None
}

const fn default_agent_max_tries() -> u32 {
    DEFAULT_AGENT_MAX_TRIES
}

const fn default_github_poll_interval_secs() -> u64 {
    60
}

const fn default_scheduler_interval_secs() -> u64 {
    60
}

const fn default_scheduler_initial_backoff_secs() -> u64 {
    1
}

const fn default_scheduler_max_backoff_secs() -> u64 {
    30
}

fn default_process_pending_scheduler() -> WorkerSchedulerConfig {
    WorkerSchedulerConfig {
        interval_secs: 2,
        ..Default::default()
    }
}

fn default_monitor_running_scheduler() -> WorkerSchedulerConfig {
    WorkerSchedulerConfig {
        interval_secs: 5,
        ..Default::default()
    }
}

fn default_run_spawners_scheduler() -> WorkerSchedulerConfig {
    WorkerSchedulerConfig {
        interval_secs: 3,
        ..Default::default()
    }
}

fn default_github_poller_scheduler() -> WorkerSchedulerConfig {
    WorkerSchedulerConfig {
        interval_secs: default_github_poll_interval_secs(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_defaults_match_worker_intervals() {
        let background = BackgroundSection::default();
        let scheduler = background.scheduler;

        assert_eq!(scheduler.process_pending_jobs.interval_secs, 2);
        assert_eq!(scheduler.monitor_running_jobs.interval_secs, 5);
        assert_eq!(scheduler.run_spawners.interval_secs, 3);
        assert_eq!(
            scheduler.github_poller.interval_secs,
            default_github_poll_interval_secs()
        );

        assert_eq!(scheduler.process_pending_jobs.initial_backoff_secs, 1);
        assert_eq!(scheduler.process_pending_jobs.max_backoff_secs, 30);
        assert_eq!(scheduler.monitor_running_jobs.initial_backoff_secs, 1);
        assert_eq!(scheduler.monitor_running_jobs.max_backoff_secs, 30);
        assert_eq!(scheduler.run_spawners.initial_backoff_secs, 1);
        assert_eq!(scheduler.run_spawners.max_backoff_secs, 30);
        assert_eq!(scheduler.github_poller.initial_backoff_secs, 1);
        assert_eq!(scheduler.github_poller.max_backoff_secs, 30);
    }
}
