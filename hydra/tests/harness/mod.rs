#![allow(dead_code, unused_imports)]

pub mod assertions;
pub mod concurrency;
pub mod user_handle;
mod worker;

use anyhow::{Context, Result};
use hydra::client::{HydraClient, HydraClientInterface};
use hydra::config::{AppConfig, ServerSection};
use hydra_common::{
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, SessionSettings,
        UpsertIssueRequest,
    },
    patches::{PatchStatus, UpsertPatchRequest},
    repositories::Repository,
    sessions::SearchSessionsQuery,
    task_status::Status,
    users::{User, Username},
    IssueId, PatchId, RepoName, SessionId,
};
use hydra_server::{
    app::{AppState, ServiceState},
    background::{
        monitor_running_sessions::MonitorRunningSessionsWorker,
        scheduler::{ScheduledWorker, WorkerOutcome},
        spawner::SharedSpawnAttempts,
        AgentQueue, Spawner,
    },
    domain::actors::{Actor, ActorRef},
    policy::{
        config::{PolicyConfig, PolicyEntry, PolicyList},
        integrations::github_pr_poller::GithubPollerWorker,
        registry::build_default_registry,
    },
    store::{MemoryStore, Store},
    test_utils::{
        spawn_test_server_with_state, test_app_config, test_secret_manager, GitHubMockBuilder,
        GitRemote, MockJobEngine, TestServer,
    },
};
use std::{collections::HashMap, collections::HashSet, str::FromStr, sync::Arc};
use tempfile::TempDir;

pub use assertions::{
    find_children_by_type, find_children_by_type_and_status, find_children_of,
    find_issue_by_description, find_issue_summary_by_description, find_summary_children_by_type,
    find_summary_children_by_type_and_status, find_summary_children_of, wait_until,
    IssueAssertions, IssueSummaryAssertions, JobAssertions, PatchAssertions,
};
pub use concurrency::{concurrent, test_all_orderings, Step};
// Re-export patch workflow config types for test files that construct configs directly.
pub use hydra_server::policy::automations::patch_workflow::{
    MergeRequestConfig, PatchWorkflowConfig, ReviewRequestConfig,
};
pub use user_handle::UserHandle;
pub use worker::{CommandOutput, WorkerFailure, WorkerResult};

/// Build a `PatchWorkflowConfig` with a single reviewer and an optional
/// merge-request assignee.
///
/// This covers the common test pattern where a patch creates one
/// `ReviewRequest` and one `MergeRequest` issue.
pub fn test_patch_workflow_config(
    reviewer: &str,
    merge_assignee: Option<&str>,
) -> PatchWorkflowConfig {
    PatchWorkflowConfig {
        review_requests: vec![ReviewRequestConfig {
            assignee: reviewer.to_string(),
        }],
        merge_request: Some(MergeRequestConfig {
            assignee: merge_assignee.map(|s| s.to_string()),
        }),
    }
}

/// Build a `SessionSettings` with only `repo_name` set.
pub fn test_job_settings(repo: &RepoName) -> SessionSettings {
    let mut settings = SessionSettings::default();
    settings.repo_name = Some(repo.clone());
    settings
}

/// Build a `SessionSettings` with `repo_name`, `image`, and `branch` set.
pub fn test_job_settings_full(repo: &RepoName, image: &str, branch: &str) -> SessionSettings {
    let mut settings = SessionSettings::default();
    settings.repo_name = Some(repo.clone());
    settings.image = Some(image.to_string());
    settings.branch = Some(branch.to_string());
    settings
}

/// Set a patch status to Merged via the API, triggering the
/// `close_merge_request_issues` automation.
pub async fn merge_patch(client: &dyn HydraClientInterface, patch_id: &PatchId) -> Result<()> {
    let mut patch = client.get_patch(patch_id).await?;
    patch.patch.status = PatchStatus::Merged;
    let request = UpsertPatchRequest::new(patch.patch);
    client.update_patch(patch_id, &request).await?;
    Ok(())
}

/// Create a merge-request tracking issue for a patch in tests.
///
/// The issue is created as a child of `parent_issue_id`, inheriting the
/// parent's creator and job settings.
pub async fn create_merge_request_issue(
    client: &dyn HydraClientInterface,
    patch_id: PatchId,
    assignee: String,
    parent_issue_id: IssueId,
    patch_title: String,
) -> Result<IssueId> {
    let parent_issue = client
        .get_issue(&parent_issue_id, false)
        .await
        .context("failed to fetch parent issue")?;
    let creator = parent_issue.issue.creator;
    let job_settings = parent_issue.issue.session_settings.clone();
    let description = format!("Review patch {}: {patch_title}", patch_id.as_ref());
    let issue = Issue::new(
        IssueType::MergeRequest,
        "Test Title".to_string(),
        description,
        creator,
        String::new(),
        IssueStatus::Open,
        Some(assignee),
        Some(job_settings),
        Vec::new(),
        vec![IssueDependency::new(
            IssueDependencyType::ChildOf,
            parent_issue_id,
        )],
        vec![patch_id],
        false,
    );
    let response = client
        .create_issue(&UpsertIssueRequest::new(issue, None))
        .await
        .context("failed to create merge-request issue")?;
    Ok(response.issue_id)
}

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
    _tempdir: TempDir,
    users: HashMap<String, UserHandle>,
    remotes: HashMap<RepoName, GitRemote>,
    github: Option<GitHubMock>,
    /// In-memory spawn attempt tracking shared across `step_spawner` calls,
    /// matching the behavior of the old `RunSpawnersWorker`.
    spawn_attempts: Arc<tokio::sync::RwLock<HashMap<String, SharedSpawnAttempts>>>,
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

    /// Return a mutable reference to the `AppState`.
    ///
    /// This is useful for reconfiguring the GitHub mock after initial setup
    /// (e.g. replacing the `github_app` with one that has PRs configured).
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
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

    /// Create a `HydraClient` authenticated as the default user.
    pub fn client(&self) -> Result<HydraClient> {
        let config = AppConfig {
            servers: vec![ServerSection {
                url: self.server_url(),
                auth_token: None,
                default: true,
            }],
        };
        HydraClient::from_config(&config, self.default_user_token())
    }

    /// Create a `HydraClient` authenticated as the named user.
    pub fn client_for(&self, user_name: &str) -> Result<HydraClient> {
        let user = self.user(user_name);
        let config = AppConfig {
            servers: vec![ServerSection {
                url: self.server_url(),
                auth_token: None,
                default: true,
            }],
        };
        HydraClient::from_config(&config, user.token())
    }

    /// Run a worker for the given job, executing the provided shell commands
    /// in place of the AI model. Uses the real `worker_run::run()` pipeline.
    ///
    /// The worker executes through the full real pipeline: git clone, branch
    /// setup, env var injection, command execution, branch push, patch
    /// creation, and status update. Only the AI model invocation is replaced
    /// (via `BashCommands`).
    ///
    /// The job must already exist (e.g. via `user.create_session()`). This method
    /// ensures the required environment variables (`HYDRA_SERVER_URL`,
    /// `HYDRA_TOKEN`, `HYDRA_ISSUE_ID`) are set in the job context so that
    /// subprocess commands (like `hydra patches create`) can reach the test
    /// server.
    pub async fn run_worker(
        &self,
        job_id: &SessionId,
        commands: Vec<&str>,
    ) -> Result<WorkerResult> {
        worker::run_worker_impl(self, job_id, commands, false).await
    }

    /// Run a worker that is expected to fail.
    ///
    /// Like [`run_worker`](Self::run_worker) but expects the worker commands
    /// to fail. Returns a [`WorkerFailure`] containing the error and any
    /// command outputs captured before the failure.
    pub async fn run_worker_expect_failure(
        &self,
        job_id: &SessionId,
        commands: Vec<&str>,
    ) -> Result<WorkerFailure> {
        worker::run_worker_expect_failure_impl(self, job_id, commands).await
    }

    // ── Background worker stepping ──────────────────────────────────

    /// Run one iteration of the spawner logic.
    ///
    /// Finds ready issues, creates tasks for them, and returns the IDs of
    /// newly created tasks. Returns an empty vec when no issues are ready.
    ///
    /// Uses `AgentQueue::spawn()` directly (the same logic underlying the
    /// `spawn_sessions` automation) so that tests can trigger spawning
    /// synchronously without needing the event-driven automation.
    pub async fn step_spawner(&self) -> Result<Vec<SessionId>> {
        let before: HashSet<SessionId> = self
            .state
            .list_sessions()
            .await
            .context("failed to list tasks before step_spawner")?
            .into_iter()
            .collect();

        let agents = self
            .state
            .list_agents()
            .await
            .context("failed to list agents in step_spawner")?;

        let mut queues = Vec::with_capacity(agents.len());
        {
            let mut attempts_map = self.spawn_attempts.write().await;
            for agent in agents {
                let shared = attempts_map
                    .entry(agent.name.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::RwLock::new(HashMap::new())))
                    .clone();
                queues.push(AgentQueue::new(agent, shared));
            }
        }

        for queue in &queues {
            match queue.spawn(&self.state).await {
                Ok(tasks) => {
                    for task in tasks {
                        self.state
                            .add_session(
                                task,
                                chrono::Utc::now(),
                                ActorRef::System {
                                    worker_name: "step_spawner".into(),
                                    on_behalf_of: None,
                                },
                            )
                            .await
                            .context("failed to add session in step_spawner")?;
                    }
                }
                Err(err) => {
                    anyhow::bail!("step_spawner failed: {err}");
                }
            }
        }

        let after = self
            .state
            .list_sessions()
            .await
            .context("failed to list tasks after step_spawner")?;

        let new_ids: Vec<SessionId> = after
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();

        Ok(new_ids)
    }

    /// Wait for sessions in `Created` status to be processed by the
    /// `start_created_sessions` automation.
    ///
    /// The automation fires automatically via the event bus when sessions
    /// are created or updated. This method polls until no sessions remain
    /// in `Created` status, with a timeout to avoid hanging.
    pub async fn step_pending_jobs(&self) -> Result<()> {
        let timeout = std::time::Duration::from_secs(5);
        let poll_interval = std::time::Duration::from_millis(25);
        let deadline = tokio::time::Instant::now() + timeout;

        // Yield once to let the automation runner start processing.
        tokio::task::yield_now().await;

        loop {
            let query = SearchSessionsQuery::new(None, None, None, vec![Status::Created]);
            let created_sessions = self
                .state
                .list_sessions_with_query(&query)
                .await
                .context("failed to query sessions in step_pending_jobs")?;
            if created_sessions.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "step_pending_jobs timed out after {:.1}s: {} session(s) still in Created status",
                    timeout.as_secs_f64(),
                    created_sessions.len()
                );
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Run one iteration of the GitHub poller worker.
    ///
    /// Synchronizes open patches with their GitHub PR state (reviews, CI
    /// status, merge status). Requires the harness to have been built with
    /// `.with_github()`.
    pub async fn step_github_sync(&self) -> Result<()> {
        let worker = GithubPollerWorker::new(self.state.clone(), 60);
        let outcome = worker.run_iteration().await;

        if let WorkerOutcome::TransientError { reason } = outcome {
            anyhow::bail!("step_github_sync failed: {reason}");
        }

        Ok(())
    }

    /// Run one iteration of the running-jobs monitor.
    ///
    /// Reconciles task status with the job engine, reaps orphaned jobs,
    /// and cleans up tasks whose parent issues have been deleted.
    pub async fn step_monitor_jobs(&self) -> Result<()> {
        let worker = MonitorRunningSessionsWorker::new(self.state.clone());
        let outcome = worker.run_iteration().await;

        if let WorkerOutcome::TransientError { reason } = outcome {
            anyhow::bail!("step_monitor_jobs failed: {reason}");
        }

        Ok(())
    }

    /// Convenience: run spawner + wait for sessions to be processed.
    ///
    /// This is the common pattern for "schedule work": the spawner
    /// creates tasks from ready issues, and the `start_created_sessions`
    /// automation automatically transitions them from Created to
    /// Pending/Running via the event bus. Returns all task IDs created
    /// by the spawner.
    pub async fn step_schedule(&self) -> Result<Vec<SessionId>> {
        let created = self.step_spawner().await?;
        self.step_pending_jobs().await?;
        Ok(created)
    }
}

/// Builder for constructing a [`TestHarness`] with custom configuration.
pub struct TestHarnessBuilder {
    repos: Vec<String>,
    users: Vec<String>,
    enable_github: bool,
    patch_workflow_config: Option<PatchWorkflowConfig>,
    agent_configs: Vec<(String, String)>,
    assignment_agent: Option<String>,
}

impl TestHarnessBuilder {
    fn new() -> Self {
        Self {
            repos: Vec::new(),
            users: Vec::new(),
            enable_github: false,
            patch_workflow_config: None,
            agent_configs: Vec::new(),
            assignment_agent: None,
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

    /// Configure the patch_workflow automation with custom parameters.
    ///
    /// Overrides the default patch_workflow config (which creates a
    /// MergeRequest issue with no assignee). Use this to set custom
    /// reviewer assignments, `$patch_creator` support, or per-repo
    /// overrides.
    pub fn with_patch_workflow_config(mut self, config: PatchWorkflowConfig) -> Self {
        self.patch_workflow_config = Some(config);
        self
    }

    /// Register an agent with the given name and prompt.
    ///
    /// Agents registered here are seeded in the database and available
    /// immediately when the harness is built.
    pub fn with_agent(mut self, name: &str, prompt: &str) -> Self {
        self.agent_configs
            .push((name.to_string(), prompt.to_string()));
        self
    }

    /// Set which agent queue acts as the assignment agent.
    ///
    /// The assignment agent automatically picks up unassigned issues (those
    /// with no `assignee` field). Other agents only pick up issues assigned
    /// to them by name.
    pub fn with_assignment_agent(mut self, name: &str) -> Self {
        self.assignment_agent = Some(name.to_string());
        self
    }

    /// Build the harness, creating all infrastructure.
    pub async fn build(self) -> Result<TestHarness> {
        let tempdir = TempDir::new().context("failed to create tempdir for TestHarness")?;

        // Create the in-memory store and mock job engine.
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let engine = Arc::new(MockJobEngine::new());

        // Pre-populate agent queues from builder config by adding to DB store.
        use hydra_server::domain::{agents::Agent, documents::Document};
        for (name, prompt) in &self.agent_configs {
            let is_assignment = self
                .assignment_agent
                .as_ref()
                .map(|a| a == name)
                .unwrap_or(false);
            let agent = Agent::new(
                name.clone(),
                format!("/agents/{name}/prompt.md"),
                3,
                10,
                is_assignment,
                Vec::new(),
            );
            store
                .add_agent(agent)
                .await
                .with_context(|| format!("failed to add agent '{name}' to store"))?;

            let doc = Document {
                title: format!("{name} prompt"),
                body_markdown: prompt.clone(),
                path: Some(format!("/agents/{name}/prompt.md").parse().unwrap()),
                created_by: None,
                deleted: false,
            };
            store
                .add_document(doc, &ActorRef::test())
                .await
                .with_context(|| format!("failed to add prompt for agent '{name}'"))?;
        }
        // Create git remotes and register repositories in the store.
        let mut remotes = HashMap::new();
        for repo_name_str in &self.repos {
            let git_remote = GitRemote::new()
                .with_context(|| format!("failed to create git remote for '{repo_name_str}'"))?;
            let repo_name = RepoName::from_str(repo_name_str)
                .with_context(|| format!("invalid repo name: '{repo_name_str}'"))?;
            let repository = Repository::new(
                git_remote.url().to_string(),
                Some("main".to_string()),
                None,
                None,
            );
            store
                .add_repository(repo_name.clone(), repository, &ActorRef::test())
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
        let mut state = AppState::new(
            server_config,
            octocrab_client,
            Arc::new(ServiceState::default()),
            store.clone(),
            engine.clone(),
            test_secret_manager(),
        );

        // Override the policy engine if a custom patch_workflow config was provided.
        if let Some(pwc) = self.patch_workflow_config {
            let params = serde_yaml_ng::to_value(&pwc)
                .context("failed to serialize PatchWorkflowConfig to YAML")?;
            let policy_config = PolicyConfig {
                global: PolicyList {
                    restrictions: vec![
                        PolicyEntry::Name("issue_lifecycle_validation".to_string()),
                        PolicyEntry::Name("task_state_machine".to_string()),
                        PolicyEntry::Name("duplicate_branch_name".to_string()),
                        PolicyEntry::Name("running_job_validation".to_string()),
                        PolicyEntry::Name("require_creator".to_string()),
                    ],
                    automations: vec![
                        PolicyEntry::Name("cascade_issue_status".to_string()),
                        PolicyEntry::Name("kill_tasks_on_issue_failure".to_string()),
                        PolicyEntry::Name("close_merge_request_issues".to_string()),
                        PolicyEntry::Name("sync_review_request_issues".to_string()),
                        PolicyEntry::WithParams {
                            name: "patch_workflow".to_string(),
                            params,
                        },
                        PolicyEntry::Name("github_pr_sync".to_string()),
                        PolicyEntry::Name("start_created_sessions".to_string()),
                    ],
                },
            };
            let registry = build_default_registry();
            let engine = registry
                .build(&policy_config)
                .map_err(|e| anyhow::anyhow!("failed to build policy engine: {e}"))?;
            state = state.with_policy_engine(engine);
        }

        // Collect user credentials. We need to create actors and users in the
        // store before spawning the server, but UserHandle construction needs
        // the server URL. So we collect (name, token) pairs first.
        let mut user_credentials: Vec<(String, String)> = Vec::new();

        // Default user
        let (default_actor, default_token) =
            Actor::new_for_session(SessionId::new(), Username::from("default").into());
        store.add_actor(default_actor, &ActorRef::test()).await?;
        let default_user = User::new(Username::from("default"), Some(1), false);
        store
            .add_user(default_user.into(), &ActorRef::test())
            .await?;
        user_credentials.push(("default".to_string(), default_token));

        // Additional named users
        for (i, user_name) in self.users.iter().enumerate() {
            if user_name == "default" {
                continue; // Already created
            }
            let (actor, token) =
                Actor::new_for_session(SessionId::new(), Username::from(user_name.as_str()).into());
            store.add_actor(actor, &ActorRef::test()).await?;
            let user = User::new(
                Username::from(user_name.as_str()),
                Some((i + 2) as u64), // github_id, avoid collision with default (1)
                false,
            );
            store.add_user(user.into(), &ActorRef::test()).await?;
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
            _tempdir: tempdir,
            users,
            remotes,
            github: github_mock,
            spawn_attempts: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        })
    }
}
