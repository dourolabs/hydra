#![allow(dead_code)]

use anyhow::{anyhow, bail, Context, Result};
use metis::config::{AppConfig, ServerSection};
use metis::{
    client::MetisClient,
    command::output::{CommandContext, ResolvedOutputFormat},
};
use metis_common::{
    constants::{ENV_METIS_ISSUE_ID, ENV_METIS_TOKEN},
    issues::{Issue, IssueStatus, IssueType, JobSettings, UpsertIssueRequest},
    jobs::SearchJobsQuery,
    patches::{GithubPr, Patch, PatchStatus, Review, UpsertPatchRequest},
    task_status::Status,
    users::{User, Username},
    IssueId, PatchId, RepoName, TaskId,
};
use metis_server::{
    app::{AppState, ServiceState},
    background::poll_github_patches::GithubPollerWorker,
    background::scheduler::{ScheduledWorker, WorkerOutcome},
    background::spawner::AgentQueue,
    store::{MemoryStore, Store},
    test_utils::{spawn_test_server_with_state, test_app_config, GitRemote, MockJobEngine},
};
use octocrab::Octocrab;
use std::{str::FromStr, sync::Arc};
use tokio::sync::RwLock;

use super::bash_commands::{BashCommands, CommandOutput};

pub struct TestEnvironment {
    pub server: metis_server::test_utils::TestServer,
    pub app_config: AppConfig,
    pub client: MetisClient,
    pub _git_remote: GitRemote,
    pub service_repo_name: RepoName,
    pub auth_token: String,
    pub current_issue_id: metis_common::IssueId,
    #[allow(dead_code)]
    pub agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    #[allow(dead_code)]
    pub state: AppState,
    pub worker_claude_code_oauth_token: Option<String>,
}

pub fn metis_bin() -> std::path::PathBuf {
    // Cargo exposes the compiled binary location to integration tests via CARGO_BIN_EXE_<binname>
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_metis"))
}

impl TestEnvironment {
    pub async fn run_as_user(&self, commands: Vec<String>) -> Result<()> {
        let server_url = self.app_config.default_server()?.url.clone();
        for command in commands {
            if command.trim().is_empty() {
                continue;
            }

            let first_token = command.split_whitespace().next();
            let command_to_run = if first_token == Some("metis") {
                let metis_path = metis_bin();
                command.replacen("metis", &metis_path.to_string_lossy(), 1)
            } else {
                let metis_path = metis_bin();
                format!("{} {}", metis_path.to_string_lossy(), command)
            };

            let output = tokio::process::Command::new("bash")
                .arg("-c")
                .arg(&command_to_run)
                .env("METIS_SERVER_URL", &server_url)
                .env(ENV_METIS_TOKEN, &self.auth_token)
                .env(ENV_METIS_ISSUE_ID, self.current_issue_id.as_ref())
                .output()
                .await
                .context("failed to spawn metis command")?;

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow!(
                    "metis command '{command_to_run}' failed with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                    status = output.status
                ));
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn run_as_worker(
        &self,
        commands: Vec<String>,
        job_id: TaskId,
    ) -> Result<Vec<CommandOutput>> {
        self.run_as_worker_with_failure(commands, job_id, false)
            .await
    }

    pub async fn run_as_worker_with_failure(
        &self,
        commands: Vec<String>,
        job_id: TaskId,
        fail_after_run: bool,
    ) -> Result<Vec<CommandOutput>> {
        let temp_dir =
            tempfile::tempdir().context("failed to create temporary directory for worker")?;
        let worker_dir = temp_dir.path().to_path_buf();

        let bash_commands = BashCommands::new_with_failure(commands, fail_after_run);

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let run_result = metis::command::jobs::worker_run::run(
            &self.client,
            job_id,
            worker_dir,
            None,
            None,
            self.worker_claude_code_oauth_token.clone(),
            None,
            &bash_commands,
            &context,
        )
        .await;

        let outputs = bash_commands.outputs();

        if let Err(err) = run_result {
            let formatted_output = format_command_outputs(&outputs);
            return Err(anyhow!(
                "failed to run worker commands: {err}\ncommand output:\n{formatted_output}"
            ));
        }

        Ok(outputs)
    }

    #[allow(dead_code)]
    pub async fn create_issue(
        &self,
        description: impl Into<String>,
        issue_type: IssueType,
        status: IssueStatus,
        assignee: Option<String>,
        job_settings: Option<JobSettings>,
    ) -> Result<IssueId> {
        let issue = Issue::new(
            issue_type,
            description.into(),
            Username::from("test-user"),
            String::new(),
            status,
            assignee,
            job_settings,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            false,
        );
        let response = self
            .client
            .create_issue(&UpsertIssueRequest::new(issue, None))
            .await?;
        Ok(response.issue_id)
    }

    #[allow(dead_code)]
    pub async fn create_patch(
        &self,
        title: impl Into<String>,
        description: impl Into<String>,
        diff: impl Into<String>,
        status: PatchStatus,
        github: Option<GithubPr>,
        created_by: Option<TaskId>,
    ) -> Result<PatchId> {
        let patch = Patch::new(
            title.into(),
            description.into(),
            diff.into(),
            status,
            false,
            created_by,
            Vec::new(),
            self.service_repo_name.clone(),
            github,
            false,
        );
        let response = self
            .client
            .create_patch(&UpsertPatchRequest::new(patch))
            .await?;
        Ok(response.patch_id)
    }

    #[allow(dead_code)]
    pub async fn append_patch_review(&self, patch_id: &PatchId, review: Review) -> Result<()> {
        let mut record = self.client.get_patch(patch_id).await?;
        record.patch.reviews.push(review);
        self.client
            .update_patch(patch_id, &UpsertPatchRequest::new(record.patch))
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn run_github_sync(&self, interval_secs: u64) -> Result<WorkerOutcome> {
        let worker = GithubPollerWorker::new(self.state.clone(), interval_secs);
        Ok(worker.run_iteration().await)
    }
}

fn format_command_outputs(outputs: &[CommandOutput]) -> String {
    outputs
        .iter()
        .map(|output| {
            format!(
                "command: {command}\nstatus: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                command = output.command,
                status = output.status,
                stdout = output.stdout.trim_end(),
                stderr = output.stderr.trim_end(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

pub async fn init_test_server_with_remote(repo_name: &str) -> Result<TestEnvironment> {
    init_test_server_with_remote_internal(repo_name, None, None).await
}

pub async fn init_test_server_with_remote_and_github(
    repo_name: &str,
    github_app: Option<Octocrab>,
) -> Result<TestEnvironment> {
    init_test_server_with_remote_internal(repo_name, github_app, None).await
}

pub async fn init_test_server_with_remote_and_claude_token(
    repo_name: &str,
    claude_code_oauth_token: &str,
) -> Result<TestEnvironment> {
    init_test_server_with_remote_internal(
        repo_name,
        None,
        Some(claude_code_oauth_token.to_string()),
    )
    .await
}

async fn init_test_server_with_remote_internal(
    repo_name: &str,
    github_app: Option<Octocrab>,
    claude_code_oauth_token: Option<String>,
) -> Result<TestEnvironment> {
    let git_remote = GitRemote::new().context("failed to create git remote for test")?;
    let remote_url = git_remote.url().to_string();
    let service_repo_name = RepoName::from_str(repo_name)
        .with_context(|| format!("failed to parse service repo name: {repo_name}"))?;
    let (state, store, auth_token, agents) =
        app_state_with_repo(&remote_url, &service_repo_name, github_app).await?;
    let server = spawn_test_server_with_state(state.clone(), store)
        .await
        .context("failed to start test server")?;
    let server_url = server.base_url();

    let app_config = AppConfig {
        servers: vec![ServerSection {
            url: server_url.clone(),
            auth_token: None,
            default: true,
        }],
    };
    let client = MetisClient::from_config(&app_config, auth_token.clone())?;
    let mut current_issue_settings = JobSettings::default();
    current_issue_settings.repo_name = Some(service_repo_name.clone());
    current_issue_settings.image = Some("worker:latest".into());
    current_issue_settings.branch = Some("main".into());
    let current_issue_id = client
        .create_issue(&UpsertIssueRequest::new(
            Issue::new(
                IssueType::Task,
                "current issue context".into(),
                Username::from("test-user"),
                String::new(),
                IssueStatus::Open,
                None,
                Some(current_issue_settings),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
            ),
            None,
        ))
        .await?
        .issue_id;

    Ok(TestEnvironment {
        server,
        app_config,
        client,
        _git_remote: git_remote,
        service_repo_name,
        auth_token,
        current_issue_id,
        agents,
        state,
        worker_claude_code_oauth_token: claude_code_oauth_token,
    })
}

pub async fn job_id_for_prompt(client: &MetisClient, prompt: &str) -> Result<TaskId> {
    let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
    jobs.into_iter()
        .find(|job| job.task.prompt == prompt)
        .map(|job| job.job_id)
        .ok_or_else(|| anyhow!("job with prompt '{prompt}' not found"))
}

pub async fn wait_for_status(
    client: &MetisClient,
    job_id: &TaskId,
    expected: Status,
) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::time::Instant::now() > deadline {
            bail!("timed out waiting for job '{job_id}' to reach status {expected:?}");
        }

        let jobs = client.list_jobs(&SearchJobsQuery::default()).await?.jobs;
        if let Some(job) = jobs.iter().find(|job| &job.job_id == job_id) {
            if job.task.status == expected {
                return Ok(());
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn app_state_with_repo(
    remote_url: &str,
    repo_name: &RepoName,
    github_app: Option<Octocrab>,
) -> Result<(
    AppState,
    Arc<dyn Store>,
    String,
    Arc<RwLock<Vec<Arc<AgentQueue>>>>,
)> {
    let server_config = test_app_config();
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let agents = Arc::new(RwLock::new(Vec::new()));
    store
        .add_repository(
            repo_name.clone(),
            metis_common::repositories::Repository::new(
                remote_url.to_string(),
                Some("main".to_string()),
                None,
            ),
        )
        .await?;

    let (actor, auth_token) = metis_server::domain::actors::Actor::new_for_task(TaskId::new());
    store.add_actor(actor).await?;
    let user = User::new(
        Username::from("test-user"),
        1,
        auth_token.clone(),
        "gh-refresh-token".to_string(),
    );
    store.add_user(user.into()).await?;

    Ok((
        AppState::new(
            Arc::new(server_config),
            github_app,
            Arc::new(ServiceState::default()),
            store.clone(),
            Arc::new(MockJobEngine::new()),
            agents.clone(),
        ),
        store,
        auth_token,
        agents,
    ))
}
