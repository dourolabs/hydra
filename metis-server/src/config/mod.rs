pub mod kube;

pub use kube::build_kube_client;

use anyhow::{Context, Result, ensure};
use metis_common::{BuildCacheContext, BuildCacheSettings, BuildCacheStorageConfig};
use octocrab::models::AppId;
use serde::Deserialize;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::policy::config::PolicyConfig;

/// Authentication configuration for the server, modeled as a tagged enum so
/// the type system enforces which fields are required for each mode.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "auth_mode")]
pub enum AuthConfig {
    /// Local single-player mode: a default user actor is auto-created on
    /// startup. Requires a GitHub personal access token (PAT) so GitHub API
    /// consumers (PR sync, patch assets, etc.) can function.
    #[serde(rename = "local")]
    Local {
        /// GitHub personal access token. Required scopes: `repo`.
        github_token: String,
        /// Optional username for the local actor. Defaults to `"local"` when
        /// omitted, producing actor name `u-local`.
        #[serde(default)]
        username: Option<String>,
    },
    /// GitHub OAuth mode: users authenticate via the GitHub device flow.
    /// Requires the `github_app` section in the config.
    #[serde(rename = "github")]
    Github { github_app: GithubAppSection },
}

impl Default for AuthConfig {
    fn default() -> Self {
        // Cannot provide a sensible default for local mode (github_token is
        // mandatory), so default to Github with an empty GithubAppSection that
        // will fail validation. In practice this path is only hit by serde when
        // the `auth` key is omitted from the config file, and `validate()` will
        // catch it.
        Self::Local {
            github_token: String::new(),
            username: None,
        }
    }
}

impl fmt::Display for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { .. } => write!(f, "local"),
            Self::Github { .. } => write!(f, "github"),
        }
    }
}

impl AuthConfig {
    /// Returns `true` if the server is running in local single-player mode.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    /// Returns the GitHub App configuration, if this is GitHub auth mode.
    pub fn github_app(&self) -> Option<&GithubAppSection> {
        match self {
            Self::Github { github_app } => Some(github_app),
            Self::Local { .. } => None,
        }
    }

    /// Returns the GitHub personal access token for local mode.
    pub fn github_token(&self) -> Option<&str> {
        match self {
            Self::Local { github_token, .. } => Some(github_token.as_str()),
            Self::Github { .. } => None,
        }
    }

    /// Returns the local username, defaulting to `"local"` when unset.
    ///
    /// Returns `None` for GitHub auth mode.
    pub fn local_username(&self) -> Option<&str> {
        match self {
            Self::Local { username, .. } => Some(username.as_deref().unwrap_or("local")),
            Self::Github { .. } => None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub metis: MetisSection,
    pub job: JobSection,
    #[serde(default)]
    pub kubernetes: KubernetesSection,
    #[serde(default)]
    pub database: DatabaseSection,
    #[serde(default, flatten)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub background: BackgroundSection,
    #[serde(default)]
    pub build_cache: BuildCacheSection,
    /// Optional policy engine configuration. When absent, all built-in
    /// policies are enabled with default parameters.
    #[serde(default)]
    pub policies: Option<PolicyConfig>,
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
        let config: Self = serde_yaml_ng::from_str(&contents)
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        config
            .validate()
            .with_context(|| format!("Invalid configuration in '{}'", resolved_path.display()))?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        self.metis.validate()?;
        match &self.auth {
            AuthConfig::Local { github_token, .. } => {
                ensure!(
                    non_empty(github_token).is_some(),
                    "github_token is required when auth_mode is 'local'"
                );
            }
            AuthConfig::Github { github_app } => {
                github_app.validate()?;
            }
        }
        self.background.validate()?;
        self.build_cache.validate()?;
        self.validate_policies()
    }

    /// Return the GitHub API base URL regardless of auth mode.
    ///
    /// In GitHub mode this comes from the `github_app` section; in local mode it
    /// falls back to the public GitHub API. Downstream consumers should call
    /// this instead of reaching into `auth` directly.
    pub fn github_api_base_url(&self) -> &str {
        self.auth
            .github_app()
            .map(|gh| gh.api_base_url())
            .unwrap_or("https://api.github.com")
    }

    fn validate_policies(&self) -> Result<()> {
        let Some(config) = &self.policies else {
            return Ok(());
        };
        let registry = crate::policy::registry::build_default_registry();
        registry.validate_config(config)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetisSection {
    #[serde(default = "default_namespace")]
    pub namespace: String,
    #[serde(default)]
    pub server_hostname: String,
    #[serde(rename = "METIS_SECRET_ENCRYPTION_KEY")]
    pub secret_encryption_key: String,
    #[serde(default)]
    pub allowed_orgs: Vec<String>,
    #[serde(default, rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    #[serde(default, rename = "ANTHROPIC_API_KEY")]
    pub anthropic_api_key: Option<String>,
    #[serde(default, rename = "CLAUDE_CODE_OAUTH_TOKEN")]
    pub claude_code_oauth_token: Option<String>,
}

impl MetisSection {
    fn validate(&self) -> Result<()> {
        ensure!(
            non_empty(&self.secret_encryption_key).is_some(),
            "metis.METIS_SECRET_ENCRYPTION_KEY must be set"
        );
        // Validate it's valid base64 and exactly 32 bytes
        {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(self.secret_encryption_key.trim())
                .context("metis.METIS_SECRET_ENCRYPTION_KEY is not valid base64")?;
            ensure!(
                bytes.len() == 32,
                "metis.METIS_SECRET_ENCRYPTION_KEY must decode to exactly 32 bytes (got {})",
                bytes.len()
            );
        }
        ensure!(
            self.allowed_orgs.iter().all(|org| non_empty(org).is_some()),
            "metis.allowed_orgs must not contain empty values"
        );
        Ok(())
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MetisSection {
    fn default() -> Self {
        use base64::Engine;
        Self {
            namespace: default_namespace(),
            server_hostname: String::new(),
            secret_encryption_key: base64::engine::general_purpose::STANDARD.encode([42u8; 32]),
            allowed_orgs: Vec::new(),
            openai_api_key: None,
            anthropic_api_key: None,
            claude_code_oauth_token: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct BuildCacheSection {
    #[serde(default)]
    pub storage: Option<BuildCacheStorageConfig>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub home_include: Vec<String>,
    #[serde(default)]
    pub home_exclude: Vec<String>,
    #[serde(default)]
    pub max_entries_per_repo: Option<usize>,
}

impl BuildCacheSection {
    pub fn to_context(&self) -> Option<BuildCacheContext> {
        let storage = self.storage.clone()?;
        let mut settings = BuildCacheSettings::default();
        if !self.include.is_empty() {
            settings.include = self.include.clone();
        }
        if !self.exclude.is_empty() {
            settings.exclude = self.exclude.clone();
        }
        if !self.home_include.is_empty() {
            settings.home_include = self.home_include.clone();
        }
        if !self.home_exclude.is_empty() {
            settings.home_exclude = self.home_exclude.clone();
        }
        if self.max_entries_per_repo.is_some() {
            settings.max_entries_per_repo = self.max_entries_per_repo;
        }
        Some(BuildCacheContext { storage, settings })
    }

    fn validate(&self) -> Result<()> {
        let Some(storage) = self.storage.as_ref() else {
            return Ok(());
        };
        match storage {
            BuildCacheStorageConfig::FileSystem { root_dir } => {
                ensure!(
                    non_empty(root_dir).is_some(),
                    "build_cache.storage.root_dir must be set"
                );
            }
            BuildCacheStorageConfig::S3 {
                endpoint_url,
                bucket,
                region,
                access_key_id,
                secret_access_key,
                session_token,
            } => {
                ensure!(
                    non_empty(endpoint_url).is_some(),
                    "build_cache.storage.endpoint_url must be set"
                );
                ensure!(
                    non_empty(bucket).is_some(),
                    "build_cache.storage.bucket must be set"
                );
                ensure!(
                    non_empty(region).is_some(),
                    "build_cache.storage.region must be set"
                );
                let has_access = access_key_id.as_ref().and_then(|v| non_empty(v)).is_some();
                let has_secret = secret_access_key
                    .as_ref()
                    .and_then(|v| non_empty(v))
                    .is_some();
                ensure!(
                    has_access == has_secret,
                    "build_cache.storage requires access_key_id and secret_access_key together"
                );
                ensure!(
                    session_token.as_ref().and_then(|v| non_empty(v)).is_none() || has_access,
                    "build_cache.storage.session_token requires access_key_id and secret_access_key"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct JobSection {
    #[serde(default)]
    pub default_image: String,
    #[serde(default)]
    pub default_model: Option<String>,
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
    pub github_poller: GithubPollerConfig,
    #[serde(default)]
    pub scheduler: SchedulerSection,
}

impl BackgroundSection {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
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
    #[serde(default = "default_cleanup_branches_scheduler")]
    pub cleanup_branches: WorkerSchedulerConfig,
}

impl Default for SchedulerSection {
    fn default() -> Self {
        Self {
            process_pending_jobs: default_process_pending_scheduler(),
            monitor_running_jobs: default_monitor_running_scheduler(),
            run_spawners: default_run_spawners_scheduler(),
            github_poller: default_github_poller_scheduler(),
            cleanup_branches: default_cleanup_branches_scheduler(),
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

pub const DEFAULT_AGENT_MAX_TRIES: i32 = 3;
pub const DEFAULT_AGENT_MAX_SIMULTANEOUS: i32 = i32::MAX;
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

fn default_cleanup_branches_scheduler() -> WorkerSchedulerConfig {
    WorkerSchedulerConfig {
        interval_secs: 300,
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

    /// Valid base64-encoded 32-byte key for test configs.
    const TEST_SECRET_KEY: &str = "KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=";

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
        assert_eq!(scheduler.cleanup_branches.interval_secs, 300);

        assert_eq!(scheduler.process_pending_jobs.initial_backoff_secs, 1);
        assert_eq!(scheduler.process_pending_jobs.max_backoff_secs, 30);
        assert_eq!(scheduler.monitor_running_jobs.initial_backoff_secs, 1);
        assert_eq!(scheduler.monitor_running_jobs.max_backoff_secs, 30);
        assert_eq!(scheduler.run_spawners.initial_backoff_secs, 1);
        assert_eq!(scheduler.run_spawners.max_backoff_secs, 30);
        assert_eq!(scheduler.github_poller.initial_backoff_secs, 1);
        assert_eq!(scheduler.github_poller.max_backoff_secs, 30);
        assert_eq!(scheduler.cleanup_branches.initial_backoff_secs, 1);
        assert_eq!(scheduler.cleanup_branches.max_backoff_secs, 30);
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
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: github

job:
  default_image: "metis-worker:latest"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
"#
            ),
        )?;

        let error = AppConfig::load(&path).expect_err("expected missing private_key");
        assert!(error_chain_contains(&error, "missing field `private_key`"));

        Ok(())
    }

    #[test]
    fn config_allows_empty_allowed_orgs() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"
  allowed_orgs: []

auth_mode: github

job:
  default_image: "metis-worker:latest"
  cpu_limit: "500m"
  memory_limit: "1Gi"
  cpu_request: "500m"
  memory_request: "1Gi"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert!(config.metis.allowed_orgs.is_empty());

        Ok(())
    }

    #[test]
    fn config_rejects_blank_allowed_orgs() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"
  allowed_orgs: [" ", ""]

auth_mode: github

job:
  default_image: "metis-worker:latest"
  cpu_limit: "500m"
  memory_limit: "1Gi"
  cpu_request: "500m"
  memory_request: "1Gi"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#
            ),
        )?;

        let error = AppConfig::load(&path).expect_err("expected blank allowed_orgs entry");
        assert!(error_chain_contains(
            &error,
            "metis.allowed_orgs must not contain empty values"
        ));

        Ok(())
    }

    #[test]
    fn config_rejects_missing_secret_encryption_key() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            r#"
metis:
  allowed_orgs: []

auth_mode: github

job:
  default_image: "metis-worker:latest"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#,
        )?;

        let error = AppConfig::load(&path).expect_err("expected missing secret_encryption_key");
        assert!(error_chain_contains(
            &error,
            "missing field `METIS_SECRET_ENCRYPTION_KEY`"
        ));

        Ok(())
    }

    #[test]
    fn config_rejects_invalid_base64_secret_encryption_key() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "not-valid-base64!!!"

auth_mode: github

job:
  default_image: "metis-worker:latest"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#,
        )?;

        let error = AppConfig::load(&path).expect_err("expected invalid base64");
        assert!(error_chain_contains(
            &error,
            "metis.METIS_SECRET_ENCRYPTION_KEY is not valid base64"
        ));

        Ok(())
    }

    #[test]
    fn config_rejects_wrong_length_secret_encryption_key() -> anyhow::Result<()> {
        use base64::Engine;
        let short_key = base64::engine::general_purpose::STANDARD.encode([42u8; 16]);

        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{short_key}"

auth_mode: github

job:
  default_image: "metis-worker:latest"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#
            ),
        )?;

        let error = AppConfig::load(&path).expect_err("expected wrong-length key");
        assert!(error_chain_contains(
            &error,
            "metis.METIS_SECRET_ENCRYPTION_KEY must decode to exactly 32 bytes"
        ));

        Ok(())
    }

    #[test]
    fn config_loads_without_github_app_in_local_mode() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: local
github_token: "ghp_test_token"

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert!(config.auth.is_local());
        assert!(config.auth.github_app().is_none());

        Ok(())
    }

    #[test]
    fn config_defaults_auth_mode_to_local() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: local
github_token: "ghp_test_token"

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert!(config.auth.is_local());

        Ok(())
    }

    #[test]
    fn config_rejects_github_mode_without_github_app() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: github

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let error = AppConfig::load(&path).expect_err("expected missing github_app");
        assert!(error_chain_contains(&error, "missing field `github_app`"));

        Ok(())
    }

    #[test]
    fn config_local_mode_defaults_username_to_local() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: local
github_token: "ghp_test_token"

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert_eq!(config.auth.local_username(), Some("local"));

        Ok(())
    }

    #[test]
    fn config_local_mode_accepts_custom_username() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: local
github_token: "ghp_test_token"
username: "alice"

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert_eq!(config.auth.local_username(), Some("alice"));

        Ok(())
    }

    #[test]
    fn config_github_mode_local_username_returns_none() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: github

job:
  default_image: "metis-worker:latest"

github_app:
  app_id: 1
  client_id: "client-id"
  client_secret: "client-secret"
  api_base_url: "https://api.github.com"
  oauth_base_url: "https://github.com"
  private_key: "private-key"
"#
            ),
        )?;

        let config = AppConfig::load(&path)?;
        assert_eq!(config.auth.local_username(), None);

        Ok(())
    }

    #[test]
    fn config_rejects_local_mode_without_github_token() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"
metis:
  METIS_SECRET_ENCRYPTION_KEY: "{TEST_SECRET_KEY}"

auth_mode: local

job:
  default_image: "metis-worker:latest"
"#
            ),
        )?;

        let error = AppConfig::load(&path).expect_err("expected missing github_token");
        assert!(error_chain_contains(&error, "missing field `github_token`"));

        Ok(())
    }
}
