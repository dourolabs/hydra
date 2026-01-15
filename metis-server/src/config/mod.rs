pub mod kube;

pub use kube::build_kube_client;

use anyhow::{Context, Result};
use metis_common::jobs::BundleSpec;
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
    pub repositories: HashMap<String, Repository>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct BackgroundSection {
    #[serde(default)]
    pub agent_queues: Vec<AgentQueueConfig>,
    #[serde(default)]
    pub github_poller: GithubPollerConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
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
pub struct SchedulerConfig {
    #[serde(default = "default_retry_backoff_secs")]
    pub retry_backoff_secs: u64,
    #[serde(default = "default_max_backoff_secs")]
    pub max_backoff_secs: u64,
    #[serde(default = "default_worker_intervals")]
    pub workers: HashMap<String, WorkerIntervalConfig>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            retry_backoff_secs: default_retry_backoff_secs(),
            max_backoff_secs: default_max_backoff_secs(),
            workers: default_worker_intervals(),
        }
    }
}

impl SchedulerConfig {
    pub fn worker_interval_secs(&self, worker_name: &str, fallback: u64) -> u64 {
        self.workers
            .get(worker_name)
            .map(|config| config.interval_secs)
            .unwrap_or(fallback)
            .max(1)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct WorkerIntervalConfig {
    #[serde(default = "default_worker_interval_secs")]
    pub interval_secs: u64,
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

const fn default_worker_interval_secs() -> u64 {
    1
}

const fn default_retry_backoff_secs() -> u64 {
    DEFAULT_RETRY_BACKOFF_SECS
}

const fn default_max_backoff_secs() -> u64 {
    DEFAULT_MAX_BACKOFF_SECS
}

fn default_worker_intervals() -> HashMap<String, WorkerIntervalConfig> {
    HashMap::from([
        (
            WORKER_PROCESS_PENDING_JOBS.to_string(),
            WorkerIntervalConfig {
                interval_secs: DEFAULT_PENDING_INTERVAL_SECS,
            },
        ),
        (
            WORKER_MONITOR_RUNNING_JOBS.to_string(),
            WorkerIntervalConfig {
                interval_secs: DEFAULT_MONITOR_INTERVAL_SECS,
            },
        ),
        (
            WORKER_RUN_SPAWNERS.to_string(),
            WorkerIntervalConfig {
                interval_secs: DEFAULT_SPAWNER_INTERVAL_SECS,
            },
        ),
    ])
}

pub const WORKER_PROCESS_PENDING_JOBS: &str = "process_pending_jobs";
pub const WORKER_MONITOR_RUNNING_JOBS: &str = "monitor_running_jobs";
pub const WORKER_RUN_SPAWNERS: &str = "run_spawners";

pub const DEFAULT_PENDING_INTERVAL_SECS: u64 = 2;
pub const DEFAULT_MONITOR_INTERVAL_SECS: u64 = 5;
pub const DEFAULT_SPAWNER_INTERVAL_SECS: u64 = 3;
pub const DEFAULT_RETRY_BACKOFF_SECS: u64 = 5;
pub const DEFAULT_MAX_BACKOFF_SECS: u64 = 60;
