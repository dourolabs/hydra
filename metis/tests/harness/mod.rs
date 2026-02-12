#![allow(dead_code, unused_imports)]

pub mod user_handle;
mod worker;

use anyhow::{Context, Result};
use metis::client::MetisClient;
use metis::config::{AppConfig, ServerSection};
use metis_common::{
    repositories::Repository,
    users::{User, Username},
    RepoName, TaskId,
};
use metis_server::{
    app::{AppState, ServiceState},
    background::spawner::AgentQueue,
    domain::actors::Actor,
    store::{MemoryStore, Store},
    test_utils::{
        spawn_test_server_with_state, test_app_config, GitHubMockBuilder, GitRemote, MockJobEngine,
        TestServer,
    },
};
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tempfile::TempDir;
use tokio::sync::RwLock;

pub use user_handle::UserHandle;
pub use worker::{CommandOutput, WorkerFailure, WorkerResult};

/// Holds the GitHub mock server and the Octocrab client configured to use it.
pub struct GitHubMock {
    pub _server: httpmock::MockServer,
    pub client: octocrab::Octocrab,
}

/// Central test harness that owns all test infrastructure.
///
/// This is the single entry point for integration tests. It owns the test
/// server, app state, store, mock job engine, git remotes, temp directory,
/// user handles, and optional GitHub mock.
///
/// Create one via the builder:
/// ```ignore
/// let harness = TestHarness::builder()
///     .with_repo("acme/app")
///     .with_github()
///     .build()
///     .await?;
/// ```
///
/// Or use the shorthand for a single default repo:
/// ```ignore
/// let harness = TestHarness::new().await?;
/// ```
pub struct TestHarness {
    server: TestServer,
    state: AppState,
    store: Arc<dyn Store>,
    engine: Arc<MockJobEngine>,
    agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    _tempdir: TempDir,
    users: HashMap<String, UserHandle>,
    remotes: HashMap<RepoName, GitRemote>,
    github: Option<GitHubMock>,
}

impl TestHarness {
    /// Shorthand: create a harness with a single default repo `"test-org/test-repo"`.
    pub async fn new() -> Result<Self> {
        Self::builder()
            .with_repo("test-org/test-repo")
            .build()
            .await
    }

    /// Start building a harness with custom configuration.
    pub fn builder() -> TestHarnessBuilder {
        TestHarnessBuilder::new()
    }

    /// Return a reference to the `AppState`.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Return a reference to the shared store.
    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// Return a reference to the mock job engine.
    pub fn engine(&self) -> &Arc<MockJobEngine> {
        &self.engine
    }

    /// Return the base URL of the test server (e.g. `"http://127.0.0.1:PORT"`).
    pub fn server_url(&self) -> String {
        self.server.base_url()
    }

    /// Return a reference to the default user's `UserHandle`.
    ///
    /// The default user is always created and named `"default"`.
    pub fn default_user(&self) -> &UserHandle {
        self.user("default")
    }

    /// Return the auth token of the default user.
    ///
    /// The default user is always created and named `"default"`.
    pub fn default_user_token(&self) -> &str {
        self.users["default"].token()
    }

    /// Return a reference to the user handle with the given name.
    ///
    /// Panics if no user with that name was registered.
    pub fn user(&self, name: &str) -> &UserHandle {
        self.users
            .get(name)
            .unwrap_or_else(|| panic!("no user named '{name}' registered in the harness"))
    }

    /// Return a reference to the git remote registered under `repo_name`.
    ///
    /// Panics if no remote with that name was registered.
    pub fn remote(&self, repo_name: &str) -> &GitRemote {
        let key = RepoName::from_str(repo_name)
            .unwrap_or_else(|_| panic!("invalid repo name: {repo_name}"));
        self.remotes
            .get(&key)
            .unwrap_or_else(|| panic!("no git remote registered for '{repo_name}'"))
    }

    /// Return a reference to all registered git remotes.
    pub fn remotes(&self) -> &HashMap<RepoName, GitRemote> {
        &self.remotes
    }

    /// Return a reference to the GitHub mock, if configured.
    pub fn github(&self) -> Option<&GitHubMock> {
        self.github.as_ref()
    }

    /// Return the agents queue.
    pub fn agents(&self) -> &Arc<RwLock<Vec<Arc<AgentQueue>>>> {
        &self.agents
    }

    /// Create a `MetisClient` authenticated as the default user.
    pub fn client(&self) -> Result<MetisClient> {
        let config = AppConfig {
            servers: vec![ServerSection {
                url: self.server_url(),
                auth_token: None,
                default: true,
            }],
        };
        MetisClient::from_config(&config, self.default_user_token())
    }

    /// Create a `MetisClient` authenticated as the named user.
    pub fn client_for(&self, user_name: &str) -> Result<MetisClient> {
        let user = self.user(user_name);
        let config = AppConfig {
            servers: vec![ServerSection {
                url: self.server_url(),
                auth_token: None,
                default: true,
            }],
        };
        MetisClient::from_config(&config, user.token())
    }

    /// Run a worker for the given job, executing the provided shell commands
    /// in place of the AI model. Uses the real `worker_run::run()` pipeline.
    ///
    /// The worker executes through the full real pipeline: git clone, branch
    /// setup, env var injection, command execution, branch push, patch
    /// creation, and status update. Only the AI model invocation is replaced
    /// (via `BashCommands`).
    ///
    /// The job must already exist (e.g. via `user.create_job()`). This method
    /// ensures the required environment variables (`METIS_SERVER_URL`,
    /// `METIS_TOKEN`, `METIS_ISSUE_ID`) are set in the job context so that
    /// subprocess commands (like `metis patches create`) can reach the test
    /// server.
    pub async fn run_worker(&self, job_id: &TaskId, commands: Vec<&str>) -> Result<WorkerResult> {
        worker::run_worker_impl(self, job_id, commands, false).await
    }

    /// Run a worker that is expected to fail.
    ///
    /// Like [`run_worker`](Self::run_worker) but expects the worker commands
    /// to fail. Returns a [`WorkerFailure`] containing the error and any
    /// command outputs captured before the failure.
    pub async fn run_worker_expect_failure(
        &self,
        job_id: &TaskId,
        commands: Vec<&str>,
    ) -> Result<WorkerFailure> {
        worker::run_worker_expect_failure_impl(self, job_id, commands).await
    }
}

/// Builder for constructing a [`TestHarness`] with custom configuration.
pub struct TestHarnessBuilder {
    repos: Vec<String>,
    users: Vec<String>,
    enable_github: bool,
}

impl TestHarnessBuilder {
    fn new() -> Self {
        Self {
            repos: Vec::new(),
            users: Vec::new(),
            enable_github: false,
        }
    }

    /// Add a git remote repository to the test environment.
    ///
    /// The repository name should be in `"owner/repo"` format. A bare git
    /// repository will be created in a temporary directory and registered in
    /// the store.
    pub fn with_repo(mut self, name: &str) -> Self {
        self.repos.push(name.to_string());
        self
    }

    /// Enable the GitHub mock server.
    ///
    /// When enabled, the harness creates a `GitHubMockBuilder`-backed mock
    /// server and configures the `AppState` with the resulting `Octocrab`
    /// client.
    pub fn with_github(mut self) -> Self {
        self.enable_github = true;
        self
    }

    /// Register a named user/actor in the test environment.
    ///
    /// Each user gets a unique auth token. A `"default"` user is always
    /// created automatically; this method is for additional users.
    pub fn with_user(mut self, name: &str) -> Self {
        self.users.push(name.to_string());
        self
    }

    /// Build the harness, creating all infrastructure.
    pub async fn build(self) -> Result<TestHarness> {
        let tempdir = TempDir::new().context("failed to create tempdir for TestHarness")?;

        // Create the in-memory store and mock job engine.
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let engine = Arc::new(MockJobEngine::new());
        let agents = Arc::new(RwLock::new(Vec::new()));

        // Create git remotes and register repositories in the store.
        let mut remotes = HashMap::new();
        for repo_name_str in &self.repos {
            let git_remote = GitRemote::new()
                .with_context(|| format!("failed to create git remote for '{repo_name_str}'"))?;
            let repo_name = RepoName::from_str(repo_name_str)
                .with_context(|| format!("invalid repo name: '{repo_name_str}'"))?;
            let repository =
                Repository::new(git_remote.url().to_string(), Some("main".to_string()), None);
            store
                .add_repository(repo_name.clone(), repository)
                .await
                .with_context(|| format!("failed to add repository '{repo_name_str}' to store"))?;
            remotes.insert(repo_name, git_remote);
        }

        // Build GitHub mock if requested.
        let github_app = if self.enable_github {
            let mut builder = GitHubMockBuilder::new();
            for repo_name_str in &self.repos {
                // Parse owner/repo for the GitHub mock.
                if let Some((owner, repo)) = repo_name_str.split_once('/') {
                    builder = builder.with_installation(owner, repo);
                }
            }
            let (mock_server, octocrab) = builder
                .build()
                .context("failed to build GitHub mock server")?;
            Some((mock_server, octocrab))
        } else {
            None
        };

        let (github_mock, octocrab_client) = match github_app {
            Some((server, client)) => (
                Some(GitHubMock {
                    _server: server,
                    client: client.clone(),
                }),
                Some(client),
            ),
            None => (None, None),
        };

        // Build AppState.
        let server_config = Arc::new(test_app_config());
        let state = AppState::new(
            server_config,
            octocrab_client,
            Arc::new(ServiceState::default()),
            store.clone(),
            engine.clone(),
            agents.clone(),
        );

        // Collect user credentials. We need to create actors and users in the
        // store before spawning the server, but UserHandle construction needs
        // the server URL. So we collect (name, token) pairs first.
        let mut user_credentials: Vec<(String, String)> = Vec::new();

        // Default user
        let (default_actor, default_token) = Actor::new_for_task(TaskId::new());
        store.add_actor(default_actor).await?;
        let default_user = User::new(
            Username::from("default"),
            1,
            default_token.clone(),
            "gh-refresh-default".to_string(),
        );
        store.add_user(default_user.into()).await?;
        user_credentials.push(("default".to_string(), default_token));

        // Additional named users
        for (i, user_name) in self.users.iter().enumerate() {
            if user_name == "default" {
                continue; // Already created
            }
            let (actor, token) = Actor::new_for_task(TaskId::new());
            store.add_actor(actor).await?;
            let user = User::new(
                Username::from(user_name.as_str()),
                (i + 2) as u64, // github_id, avoid collision with default (1)
                token.clone(),
                format!("gh-refresh-{user_name}"),
            );
            store.add_user(user.into()).await?;
            user_credentials.push((user_name.clone(), token));
        }

        // Spawn the test server.
        let server = spawn_test_server_with_state(state.clone(), store.clone())
            .await
            .context("failed to spawn test server for TestHarness")?;

        // Now that we have the server URL, construct UserHandle instances.
        let server_url = server.base_url();
        let mut users = HashMap::new();
        for (name, token) in user_credentials {
            let handle = UserHandle::new(name.clone(), token, &server_url)
                .with_context(|| format!("failed to create UserHandle for '{name}'"))?;
            users.insert(name, handle);
        }

        Ok(TestHarness {
            server,
            state,
            store,
            engine,
            agents,
            _tempdir: tempdir,
            users,
            remotes,
            github: github_mock,
        })
    }
}
