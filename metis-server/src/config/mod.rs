pub mod kube;

pub use kube::build_kube_client;

use anyhow::{Context, Result, ensure};
use octocrab::models::AppId;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    #[serde(default)]
    pub metis: MetisSection,
    pub job: JobSection,
    #[serde(default)]
    pub kubernetes: KubernetesSection,
    #[serde(default)]
    pub database: DatabaseSection,
    pub github_app: GithubAppSection,
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
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        config
            .validate()
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        self.github_app.validate()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetisSection {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub server_hostname: String,
    #[serde(default)]
    pub allowed_orgs: Vec<String>,
    #[serde(default, rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
}

impl Default for MetisSection {
    fn default() -> Self {
        Self {
            namespace: default_namespace(),
            server_hostname: String::new(),
            allowed_orgs: Vec::new(),
            openai_api_key: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct JobSection {
    #[serde(default)]
    pub default_image: String,
    #[serde(default = "default_cpu_limit")]
    pub cpu_limit: String,
    #[serde(default = "default_memory_limit")]
    pub memory_limit: String,
    #[serde(default = "default_cpu_request")]
    pub cpu_request: String,
    #[serde(default = "default_memory_request")]
    pub memory_request: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseSection {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl DatabaseSection {
    pub fn database_url(&self) -> Option<String> {
        self.url.as_deref().and_then(non_empty).map(str::to_owned)
    }

    pub fn idle_timeout(&self) -> Option<u64> {
        if self.idle_timeout_secs == 0 {
            None
        } else {
            Some(self.idle_timeout_secs)
        }
    }
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            url: None,
            min_connections: default_min_connections(),
            max_connections: default_max_connections(),
            connect_timeout_secs: default_connect_timeout_secs(),
            idle_timeout_secs: default_idle_timeout_secs(),
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
    #[serde(default)]
    pub image_pull_secrets: Vec<String>,
}

impl Default for KubernetesSection {
    fn default() -> Self {
        Self {
            in_cluster: false,
            config_path: default_kubeconfig_path(),
            context: String::new(),
            cluster_name: String::new(),
            api_server: String::new(),
            image_pull_secrets: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GithubAppSection {
    pub app_id: u64,
    pub client_id: String,
    pub client_secret: String,
    pub private_key: String,
    #[serde(default = "default_github_api_base_url")]
    pub api_base_url: String,
    #[serde(default = "default_github_oauth_base_url")]
    pub oauth_base_url: String,
}

impl GithubAppSection {
    pub fn app_id(&self) -> AppId {
        AppId(self.app_id)
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn client_secret(&self) -> &str {
        &self.client_secret
    }

    pub fn private_key(&self) -> &str {
        &self.private_key
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    pub fn oauth_base_url(&self) -> &str {
        &self.oauth_base_url
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.app_id > 0,
            "github_app.app_id must be a positive integer"
        );
        ensure!(
            non_empty(&self.client_id).is_some(),
            "github_app.client_id must be set"
        );
        ensure!(
            non_empty(&self.client_secret).is_some(),
            "github_app.client_secret must be set"
        );
        ensure!(
            non_empty(&self.private_key).is_some(),
            "github_app.private_key must be set"
        );
        ensure!(
            non_empty(&self.api_base_url).is_some(),
            "github_app.api_base_url must be set"
        );
        ensure!(
            non_empty(&self.oauth_base_url).is_some(),
            "github_app.oauth_base_url must be set"
        );
        Ok(())
    }
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
pub struct AgentQueueConfig {
    pub name: String,
    pub prompt: String,
    #[serde(default = "default_agent_max_tries")]
    pub max_tries: u32,
    #[serde(default = "default_agent_max_simultaneous")]
    pub max_simultaneous: u32,
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

pub const DEFAULT_AGENT_MAX_TRIES: u32 = 3;
pub const DEFAULT_AGENT_MAX_SIMULTANEOUS: u32 = u32::MAX;
const fn default_min_connections() -> u32 {
    1
}

const fn default_max_connections() -> u32 {
    5
}

const fn default_connect_timeout_secs() -> u64 {
    5
}

const fn default_idle_timeout_secs() -> u64 {
    300
}

fn default_kubeconfig_path() -> String {
    "~/.kube/config".to_string()
}

fn default_github_api_base_url() -> String {
    "https://api.github.com".to_string()
}

fn default_github_oauth_base_url() -> String {
    "https://github.com".to_string()
}

const fn default_agent_max_tries() -> u32 {
    DEFAULT_AGENT_MAX_TRIES
}

const fn default_agent_max_simultaneous() -> u32 {
    DEFAULT_AGENT_MAX_SIMULTANEOUS
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

fn default_cpu_limit() -> String {
    "500m".to_string()
}

fn default_memory_limit() -> String {
    "1Gi".to_string()
}

fn default_cpu_request() -> String {
    default_cpu_limit()
}

fn default_memory_request() -> String {
    default_memory_limit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn error_chain_contains(error: &anyhow::Error, needle: &str) -> bool {
        error
            .chain()
            .any(|cause| cause.to_string().contains(needle))
    }

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

    #[test]
    fn database_url_prefers_config_value() {
        let database = DatabaseSection {
            url: Some("postgres://config-value".to_string()),
            ..Default::default()
        };

        assert_eq!(
            database.database_url(),
            Some("postgres://config-value".to_string())
        );
    }

    #[test]
    fn database_url_returns_none_when_unset() {
        let database = DatabaseSection {
            url: None,
            ..Default::default()
        };

        assert_eq!(database.database_url(), None);
    }

    #[test]
    fn database_url_ignores_blank_values() {
        let database = DatabaseSection {
            url: Some(" ".to_string()),
            ..Default::default()
        };

        assert_eq!(database.database_url(), None);
    }

    #[test]
    fn github_app_section_rejects_blank_client_id() {
        let github_app = GithubAppSection {
            app_id: 42,
            client_id: "  ".to_string(),
            client_secret: "\n".to_string(),
            private_key: "key".to_string(),
            api_base_url: "https://api.github.com".to_string(),
            oauth_base_url: "https://github.com".to_string(),
        };

        let error = github_app
            .validate()
            .expect_err("expected blank client_id to fail validation");
        assert!(
            error
                .to_string()
                .contains("github_app.client_id must be set")
        );
    }

    #[test]
    fn config_requires_github_app_private_key() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[github_app]
app_id = 1
client_id = "client-id"
client_secret = "client-secret"
"#,
        )?;

        let error = AppConfig::load(&path).expect_err("expected missing private_key");
        assert!(error_chain_contains(&error, "missing field `private_key`"));

        Ok(())
    }

    #[test]
    fn config_loads_allowed_orgs() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[metis]
allowed_orgs = ["octo-org", "metis"]

[job]
default_image = "worker:latest"
cpu_limit = "500m"
memory_limit = "1Gi"
cpu_request = "500m"
memory_request = "1Gi"

[github_app]
app_id = 1
client_id = "client-id"
client_secret = "client-secret"
private_key = "private-key"
"#,
        )?;

        let config = AppConfig::load(&path)?;
        assert_eq!(config.metis.allowed_orgs, vec!["octo-org", "metis"]);

        Ok(())
    }
}
