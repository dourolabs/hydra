use crate::{
    background::AgentQueue,
    config::{AgentQueueConfig, AppConfig, non_empty},
    domain::{
        actors::Actor,
        issues::{
            Issue, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType, JobSettings,
            TodoItem, UpsertIssueRequest,
        },
        jobs::{BundleSpec, CreateJobRequest},
        patches::{GithubPr, Patch, PatchStatus, UpsertPatchRequest},
        users::{User, UserSummary, Username},
    },
    job_engine::{JobEngine, JobEngineError, JobStatus},
    store::{Status, Store, StoreError, Task, TaskError, TaskStatusLog},
};
use chrono::{DateTime, Duration, Utc};
use metis_common::{
    PatchId, RepoName, TaskId, Versioned,
    api::v1 as api,
    constants::ENV_METIS_ID,
    issues::IssueId,
    job_status::{JobStatusUpdate, SetJobStatusResponse},
    merge_queues::MergeQueue,
};
use octocrab::Octocrab;
use serde::Deserialize;
use std::{collections::HashSet, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use super::{
    MergeQueueError, Repository, RepositoryError, RepositoryRecord, ServiceState,
    TaskResolutionError,
};

/// Shared application state and application-specific coordination such as issue lifecycle validation.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub github_app: Option<Octocrab>,
    pub service_state: Arc<ServiceState>,
    store: Arc<dyn Store>,
    pub job_engine: Arc<dyn JobEngine>,
    agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
}

#[derive(Debug, Error)]
pub enum CreateJobError {
    #[error(transparent)]
    TaskResolution(#[from] TaskResolutionError),
    #[error("failed to load issue '{issue_id}' for job creation")]
    IssueLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("failed to store job {job_id}")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum SetJobStatusError {
    #[error("job '{job_id}' not found in store")]
    NotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to update status for job '{job_id}'")]
    Store {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum UpsertPatchError {
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("created_by must reference a running job")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("patch '{patch_id}' not found")]
    PatchNotFound {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("patch store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
    #[error("github sync requires an authenticated actor")]
    GithubActorMissing,
    #[error("failed to load github token for actor '{actor}': {message}")]
    GithubTokenLookup { actor: String, message: String },
    #[error("failed to create github client for actor '{actor}'")]
    GithubUserClient {
        #[source]
        source: octocrab::Error,
        actor: String,
    },
    #[error("github sync requires a base ref")]
    GithubBaseRefMissing,
    #[error("failed to load repository '{repo_name}' for github sync")]
    GithubRepositoryLookup {
        #[source]
        source: StoreError,
        repo_name: RepoName,
    },
    #[error("failed to update github pull request '{owner}/{repo}#{number}'")]
    GithubPullRequestUpdate {
        #[source]
        source: octocrab::Error,
        owner: String,
        repo: String,
        number: u64,
    },
    #[error("failed to create github pull request for '{owner}/{repo}'")]
    GithubPullRequestCreate {
        #[source]
        source: octocrab::Error,
        owner: String,
        repo: String,
    },
    #[error("failed to load merge-request issues for patch '{patch_id}'")]
    MergeRequestLookup {
        #[source]
        source: StoreError,
        patch_id: PatchId,
    },
    #[error("failed to update merge-request issue '{issue_id}' for patch '{patch_id}'")]
    MergeRequestUpdate {
        #[source]
        source: StoreError,
        patch_id: PatchId,
        issue_id: IssueId,
    },
}

#[derive(Debug, Error)]
pub enum UpsertIssueError {
    #[error("job_id may only be provided when creating an issue")]
    JobIdProvidedForUpdate,
    #[error("issue creator must be set")]
    MissingCreator,
    #[error("issue dependency '{dependency_id}' not found")]
    MissingDependency {
        #[source]
        source: StoreError,
        dependency_id: IssueId,
    },
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: Option<IssueId>,
    },
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: TaskId,
    },
    #[error("job_id must reference a running job")]
    JobNotRunning {
        job_id: TaskId,
        status: Option<Status>,
    },
    #[error("failed to read tasks for dropped issue '{issue_id}'")]
    TaskLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("failed to kill task '{job_id}' for dropped issue '{issue_id}'")]
    KillTask {
        #[source]
        source: JobEngineError,
        issue_id: IssueId,
        job_id: TaskId,
    },
}

#[derive(Debug, Error)]
pub enum UpdateTodoListError {
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("todo item number {item_number} is out of range for issue '{issue_id}'")]
    InvalidItemNumber {
        issue_id: IssueId,
        item_number: usize,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent '{name}' already exists")]
    AlreadyExists { name: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
}

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("invalid github token: {0}")]
    InvalidGithubToken(String),
    #[error("github user '{username}' is not in an allowed organization")]
    ForbiddenGithubOrg { username: String },
    #[error("login store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    pub fn new(
        config: Arc<AppConfig>,
        github_app: Option<Octocrab>,
        service_state: Arc<ServiceState>,
        store: Arc<dyn Store>,
        job_engine: Arc<dyn JobEngine>,
        agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    ) -> Self {
        Self {
            config,
            github_app,
            service_state,
            store,
            job_engine,
            agents,
        }
    }

    pub async fn login_with_github_token(
        &self,
        github_token: String,
        github_refresh_token: String,
    ) -> Result<api::login::LoginResponse, LoginError> {
        let (user, _actor, login_token) = self
            .create_actor_for_github_token(github_token, github_refresh_token)
            .await?;

        let user_summary: api::users::UserSummary = UserSummary::from(user).into();

        Ok(api::login::LoginResponse::new(login_token, user_summary))
    }

    async fn create_actor_for_github_token(
        &self,
        github_token: String,
        github_refresh_token: String,
    ) -> Result<(User, Actor, String), LoginError> {
        let github_client = Octocrab::builder()
            .base_uri(self.config.github_app.api_base_url().to_string())
            .map_err(|err| LoginError::Store {
                source: StoreError::Internal(format!("failed to parse github api base url: {err}")),
            })?
            .personal_token(github_token.clone())
            .build()
            .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;

        let github_user = github_client
            .current()
            .user()
            .await
            .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;
        let username = Username::from(github_user.login);

        let allowed_orgs = &self.config.metis.allowed_orgs;
        if !allowed_orgs.is_empty() {
            #[derive(Deserialize)]
            struct GithubOrg {
                login: String,
            }

            let orgs: Vec<GithubOrg> = github_client
                .get("/user/orgs", None::<&()>)
                .await
                .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;

            let is_allowed = orgs.iter().any(|org| {
                allowed_orgs
                    .iter()
                    .any(|allowed| org.login.eq_ignore_ascii_case(allowed))
            });

            if !is_allowed {
                return Err(LoginError::ForbiddenGithubOrg {
                    username: username.to_string(),
                });
            }
        }

        let user = User {
            username: username.clone(),
            github_user_id: github_user.id.into_inner(),
            github_token,
            github_refresh_token,
        };

        let (actor, auth_token) = Actor::new_for_user(username);

        let store = self.store.as_ref();
        if let Err(err) = store.add_user(user.clone()).await {
            match err {
                StoreError::UserAlreadyExists(_) => {
                    self.set_user_github_token(
                        &user.username,
                        user.github_token.clone(),
                        user.github_user_id,
                        user.github_refresh_token.clone(),
                    )
                    .await
                    .map_err(|source| LoginError::Store { source })?;
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        if let Err(err) = store.add_actor(actor.clone()).await {
            match err {
                StoreError::ActorAlreadyExists(_) => {
                    store
                        .update_actor(actor.clone())
                        .await
                        .map_err(|source| LoginError::Store { source })?;
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        Ok((user, actor, auth_token))
    }

    async fn create_actor_for_task(&self, task_id: TaskId) -> Result<(Actor, String), StoreError> {
        let (actor, auth_token) = Actor::new_for_task(task_id);
        let store = self.store.as_ref();
        store.add_actor(actor.clone()).await?;
        Ok((actor, auth_token))
    }

    pub async fn get_issue(&self, issue_id: &IssueId) -> Result<Versioned<Issue>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue(issue_id).await
    }

    pub async fn get_issue_versions(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue_versions(issue_id).await
    }

    pub async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        let store = self.store.as_ref();
        store.search_issue_graph(filters).await
    }

    pub async fn get_patch(&self, patch_id: &PatchId) -> Result<Versioned<Patch>, StoreError> {
        let store = self.store.as_ref();
        store.get_patch(patch_id).await
    }

    pub async fn get_patch_versions(
        &self,
        patch_id: &PatchId,
    ) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let store = self.store.as_ref();
        store.get_patch_versions(patch_id).await
    }

    pub async fn list_patches(&self) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_patches().await
    }

    pub async fn get_status_log(&self, task_id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        let store = self.store.as_ref();
        store.get_status_log(task_id).await
    }

    pub async fn get_actor(&self, name: &str) -> Result<Actor, StoreError> {
        let store = self.store.as_ref();
        store.get_actor(name).await.map(|actor| actor.item)
    }

    pub async fn get_user(&self, username: &Username) -> Result<User, StoreError> {
        let store = self.store.as_ref();
        store.get_user(username).await.map(|user| user.item)
    }

    pub async fn set_user_github_token(
        &self,
        username: &Username,
        github_token: String,
        github_user_id: u64,
        github_refresh_token: String,
    ) -> Result<User, StoreError> {
        let store = self.store.as_ref();
        let mut user = store.get_user(username).await?.item;
        user.github_token = github_token;
        user.github_user_id = github_user_id;
        user.github_refresh_token = github_refresh_token;
        store.update_user(user).await.map(|user| user.item)
    }

    async fn sync_patch_with_github(
        &self,
        actor: &Actor,
        patch: &mut Patch,
        head_ref: &str,
    ) -> Result<(), UpsertPatchError> {
        let (owner, repo) = match patch.github.as_ref() {
            Some(github) => (github.owner.clone(), github.repo.clone()),
            None => (
                patch.service_repo_name.organization.clone(),
                patch.service_repo_name.repo.clone(),
            ),
        };
        let client = self.github_user_client(actor).await?;

        if let Some(existing) = patch.github.as_ref() {
            let pr = client
                .pulls(&owner, &repo)
                .update(existing.number)
                .title(patch.title.clone())
                .body(patch.description.clone())
                .send()
                .await
                .map_err(|source| UpsertPatchError::GithubPullRequestUpdate {
                    source,
                    owner: owner.clone(),
                    repo: repo.clone(),
                    number: existing.number,
                })?;

            let mut updated = existing.clone();
            updated.head_ref = Some(pr.head.ref_field.clone());
            updated.base_ref = Some(pr.base.ref_field.clone());
            updated.url = pr.html_url.as_ref().map(ToString::to_string);
            patch.github = Some(updated);
            return Ok(());
        }

        let base_ref = match patch
            .github
            .as_ref()
            .and_then(|github| github.base_ref.as_ref())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            Some(base_ref) => base_ref,
            None => {
                let repository = self
                    .repository_from_store(&patch.service_repo_name)
                    .await
                    .map_err(|source| UpsertPatchError::GithubRepositoryLookup {
                        source,
                        repo_name: patch.service_repo_name.clone(),
                    })?;
                repository
                    .default_branch
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or(UpsertPatchError::GithubBaseRefMissing)?
            }
        };

        let pr = client
            .pulls(&owner, &repo)
            .create(patch.title.clone(), head_ref, base_ref)
            .body(patch.description.clone())
            .send()
            .await
            .map_err(|source| UpsertPatchError::GithubPullRequestCreate {
                source,
                owner: owner.clone(),
                repo: repo.clone(),
            })?;

        patch.github = Some(GithubPr::new(
            owner,
            repo,
            pr.number,
            Some(pr.head.ref_field.clone()),
            Some(pr.base.ref_field.clone()),
            pr.html_url.as_ref().map(ToString::to_string),
            patch.github.as_ref().and_then(|github| github.ci.clone()),
        ));

        Ok(())
    }

    async fn github_user_client(&self, actor: &Actor) -> Result<Octocrab, UpsertPatchError> {
        let token = actor.get_github_token(self).await.map_err(|err| {
            UpsertPatchError::GithubTokenLookup {
                actor: actor.name(),
                message: err.message().to_string(),
            }
        })?;

        Octocrab::builder()
            .base_uri(self.config.github_app.api_base_url().to_string())
            .map_err(|source| UpsertPatchError::GithubUserClient {
                source,
                actor: actor.name(),
            })?
            .personal_token(token.github_token)
            .build()
            .map_err(|source| UpsertPatchError::GithubUserClient {
                source,
                actor: actor.name(),
            })
    }

    pub async fn list_repositories(&self) -> Result<Vec<RepositoryRecord>, RepositoryError> {
        let store = self.store.as_ref();
        let repositories = store
            .list_repositories()
            .await
            .map_err(|source| RepositoryError::Store { source })?;

        Ok(repositories
            .into_iter()
            .map(|(name, repository)| RepositoryRecord::from((name, repository.item)))
            .collect())
    }

    pub async fn create_repository(
        &self,
        name: RepoName,
        config: Repository,
    ) -> Result<RepositoryRecord, RepositoryError> {
        {
            let store = self.store.as_ref();
            store
                .add_repository(name.clone(), config.clone())
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryAlreadyExists(name) => {
                        RepositoryError::AlreadyExists(name)
                    }
                    other => RepositoryError::Store { source: other },
                })?;
        }

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
    ) -> Result<RepositoryRecord, RepositoryError> {
        {
            let store = self.store.as_ref();
            store
                .update_repository(name.clone(), config.clone())
                .await
                .map_err(|source| match source {
                    StoreError::RepositoryNotFound(_) => RepositoryError::NotFound(name.clone()),
                    StoreError::RepositoryAlreadyExists(_) => {
                        RepositoryError::AlreadyExists(name.clone())
                    }
                    other => RepositoryError::Store { source: other },
                })?;
        }

        self.service_state.clear_cache(&name).await;

        Ok(RepositoryRecord::from((name, config)))
    }

    pub async fn list_agent_configs(&self) -> Vec<AgentQueueConfig> {
        self.agents
            .read()
            .await
            .iter()
            .map(|agent| agent.as_config())
            .collect()
    }

    pub async fn get_agent_config(&self, name: &str) -> Option<AgentQueueConfig> {
        self.agents
            .read()
            .await
            .iter()
            .find(|agent| agent.name == name)
            .map(|agent| agent.as_config())
    }

    pub async fn agent_queues(&self) -> Vec<Arc<AgentQueue>> {
        self.agents.read().await.clone()
    }

    #[allow(unused)]
    pub async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        let store = self.store.as_ref();
        store.list_tasks_with_status(status).await
    }

    #[allow(unused)]
    pub async fn add_task(
        &self,
        task: Task,
        created_at: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        let store = self.store.as_ref();
        store.add_task(task, created_at).await
    }

    pub async fn create_agent(
        &self,
        agent: AgentQueueConfig,
    ) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;
        if agents.iter().any(|existing| existing.name == agent.name) {
            return Err(AgentError::AlreadyExists {
                name: agent.name.clone(),
            });
        }

        let created = Arc::new(AgentQueue::from_config(&agent));
        agents.push(created.clone());

        Ok(created.as_config())
    }

    pub async fn update_agent(
        &self,
        agent_name: &str,
        updated: AgentQueueConfig,
    ) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;

        if updated.name != agent_name && agents.iter().any(|existing| existing.name == updated.name)
        {
            return Err(AgentError::AlreadyExists {
                name: updated.name.clone(),
            });
        }

        let Some(index) = agents.iter().position(|agent| agent.name == agent_name) else {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        };

        let replacement = Arc::new(AgentQueue::from_config(&updated));
        agents[index] = replacement.clone();

        Ok(replacement.as_config())
    }

    pub async fn delete_agent(&self, agent_name: &str) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;

        let Some(index) = agents.iter().position(|agent| agent.name == agent_name) else {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        };

        let removed = agents.remove(index);
        Ok(removed.as_config())
    }

    pub async fn create_job(&self, request: CreateJobRequest) -> Result<TaskId, CreateJobError> {
        let job_id = TaskId::new();

        let mut env_vars = request.variables;
        env_vars.insert(ENV_METIS_ID.to_string(), job_id.to_string());

        let issue = match request.issue_id.as_ref() {
            Some(issue_id) => {
                let store = self.store.as_ref();
                Some(store.get_issue(issue_id).await.map_err(|source| {
                    CreateJobError::IssueLookup {
                        source,
                        issue_id: issue_id.clone(),
                    }
                })?)
            }
            None => None,
        };
        let job_settings = issue
            .as_ref()
            .map(|issue| self.apply_job_settings_defaults(issue.item.job_settings.clone()))
            .filter(|settings| !JobSettings::is_default(settings));

        let mut context = request.context;
        let image = job_settings
            .as_ref()
            .and_then(|settings| settings.image.clone())
            .or(request.image);
        let cpu_limit = job_settings
            .as_ref()
            .and_then(|settings| settings.cpu_limit.clone());
        let memory_limit = job_settings
            .as_ref()
            .and_then(|settings| settings.memory_limit.clone());

        if let Some(settings) = job_settings {
            if let Some(remote_url) = settings.remote_url.clone() {
                let rev = settings
                    .branch
                    .clone()
                    .or_else(|| match &context {
                        BundleSpec::GitRepository { rev, .. } => Some(rev.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "main".to_string());
                context = BundleSpec::GitRepository {
                    url: remote_url,
                    rev,
                };
            } else if let Some(repo_name) = settings.repo_name.clone() {
                context = BundleSpec::ServiceRepository {
                    name: repo_name,
                    rev: settings.branch.clone(),
                };
            } else if let (Some(branch), BundleSpec::GitRepository { url, .. }) =
                (settings.branch.clone(), &context)
            {
                context = BundleSpec::GitRepository {
                    url: url.clone(),
                    rev: branch,
                };
            }
        }

        let task = Task::new(
            request.prompt,
            context,
            request.issue_id.clone(),
            image,
            env_vars,
            cpu_limit,
            memory_limit,
        );

        self.resolve_task(&task).await?;

        let store = self.store.as_ref();
        store
            .add_task_with_id(job_id.clone(), task, Utc::now())
            .await
            .map_err(|source| CreateJobError::Store {
                source,
                job_id: job_id.clone(),
            })?;

        Ok(job_id)
    }

    pub(crate) fn apply_job_settings_defaults(&self, mut settings: JobSettings) -> JobSettings {
        if settings.model.is_none() {
            if let Some(default_model) =
                self.config.job.default_model.as_deref().and_then(non_empty)
            {
                settings.model = Some(default_model.to_string());
            }
        }

        settings
    }

    pub async fn set_job_status(
        &self,
        job_id: TaskId,
        status: JobStatusUpdate,
    ) -> Result<SetJobStatusResponse, SetJobStatusError> {
        {
            let store = self.store.as_ref();

            store
                .get_task(&job_id)
                .await
                .map(|_| ())
                .map_err(|source| SetJobStatusError::NotFound {
                    source,
                    job_id: job_id.clone(),
                })?;

            self.transition_task_to_completion(
                &job_id,
                status.to_result().map_err(TaskError::from),
                status.last_message(),
            )
            .await
            .map_err(|source| SetJobStatusError::Store {
                source,
                job_id: job_id.clone(),
            })?;
        }

        Ok(SetJobStatusResponse::new(job_id, status.as_status()))
    }

    pub async fn start_pending_task(&self, task_id: TaskId) {
        let job_config = self.config.job.clone();
        let (resolved, cpu_limit, memory_limit) = {
            let store = self.store.as_ref();
            match store.get_task(&task_id).await {
                Ok(task) => match self.resolve_task(&task.item).await {
                    Ok(resolved) => (
                        resolved,
                        task.item.cpu_limit.clone(),
                        task.item.memory_limit.clone(),
                    ),
                    Err(err) => {
                        warn!(
                            metis_id = %task_id,
                            error = %err,
                            "failed to resolve task for spawning"
                        );
                        return;
                    }
                },
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to load task for spawning"
                    );
                    return;
                }
            }
        };

        let cpu_limit = cpu_limit.unwrap_or_else(|| job_config.cpu_limit.clone());
        let memory_limit = memory_limit.unwrap_or_else(|| job_config.memory_limit.clone());
        let cpu_request = job_config.cpu_request.clone();
        let memory_request = job_config.memory_request.clone();

        let (actor, auth_token) = match self.create_actor_for_task(task_id.clone()).await {
            Ok(values) => values,
            Err(err) => {
                let failure_reason = format!("Failed to create actor for task: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                    )
                    .await
                {
                    error!(
                        metis_id = %task_id,
                        error = %update_err,
                        "failed to set task status to Failed (actor creation failed)"
                    );
                } else {
                    info!(
                        metis_id = %task_id,
                        "set task status to Failed (actor creation failed)"
                    );
                }
                return;
            }
        };

        match self
            .job_engine
            .create_job(
                &task_id,
                &actor,
                &auth_token,
                &resolved.image,
                &resolved.env_vars,
                cpu_limit,
                memory_limit,
                cpu_request,
                memory_request,
            )
            .await
        {
            Ok(()) => match self.transition_task_to_pending(&task_id).await {
                Ok(_) => {
                    info!(
                        metis_id = %task_id,
                        "set task status to Pending (spawned)"
                    );
                }
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to set task to Pending after spawn"
                    );
                }
            },
            Err(err) => {
                let failure_reason = format!("Failed to create Kubernetes job: {err}");
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                    )
                    .await
                {
                    error!(
                        metis_id = %task_id,
                        error = %update_err,
                        "failed to set task status to Failed (spawn failed)"
                    );
                } else {
                    info!(
                        metis_id = %task_id,
                        "set task status to Failed (spawn failed)"
                    );
                }
            }
        }
    }

    pub async fn reap_orphaned_jobs(&self) {
        let job_engine_jobs = match self.job_engine.list_jobs().await {
            Ok(jobs) => jobs,
            Err(err) => {
                error!(error = %err, "failed to list jobs in job engine");
                return;
            }
        };

        if job_engine_jobs.is_empty() {
            return;
        }

        let store_task_ids: Vec<TaskId> = {
            let store = self.store.as_ref();
            match store.list_tasks().await {
                Ok(tasks) => tasks.into_iter().map(|(id, _)| id).collect(),
                Err(err) => {
                    error!(error = %err, "failed to list tasks from store for job reconciliation");
                    return;
                }
            }
        };

        let store_task_set: HashSet<_> = store_task_ids.into_iter().collect();
        let orphaned_jobs: Vec<_> = job_engine_jobs
            .into_iter()
            .filter(|job| !store_task_set.contains(&job.id))
            .collect();

        if !orphaned_jobs.is_empty() {
            info!(
                count = orphaned_jobs.len(),
                "killing jobs present in engine but missing from store"
            );
        }

        for job in orphaned_jobs {
            match self.job_engine.kill_job(&job.id).await {
                Ok(()) => {
                    info!(metis_id = %job.id, "killed job not present in store");
                }
                Err(err) => {
                    warn!(metis_id = %job.id, error = %err, "failed to kill job not present in store");
                }
            }
        }
    }

    pub async fn reconcile_running_task(&self, task_id: TaskId) {
        let current_status = {
            let store = self.store.as_ref();
            match store.get_task(&task_id).await {
                Ok(task) => task.item.status,
                Err(err) => {
                    warn!(
                        metis_id = %task_id,
                        error = %err,
                        "failed to load task while reconciling status"
                    );
                    return;
                }
            }
        };

        match self.job_engine.find_job_by_metis_id(&task_id).await {
            Ok(job) => match job.status {
                JobStatus::Pending => {}
                JobStatus::Running => {
                    if current_status == Status::Pending {
                        match self.transition_task_to_running(&task_id).await {
                            Ok(_) => {
                                info!(
                                    metis_id = %task_id,
                                    "set task status to Running (pod started)"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    metis_id = %task_id,
                                    error = %err,
                                    "failed to set task to Running after pod start"
                                );
                            }
                        }
                    }
                }
                JobStatus::Complete => {
                    warn!(
                        metis_id = %task_id,
                        "Job completed in job engine without submitting results."
                    );

                    let completion_time = job.completion_time.unwrap_or_else(Utc::now);
                    let duration_since_completion =
                        Utc::now().signed_duration_since(completion_time);

                    if duration_since_completion < Duration::seconds(60) {
                        return;
                    }

                    let failure_reason =
                        "Job completed without submitting results (timeout after 1 minute)"
                            .to_string();
                    match self
                        .transition_task_to_completion(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            warn!(metis_id = %task_id, "task marked failed due to missing results after job completion timeout");
                        }
                        Err(err) => {
                            warn!(metis_id = %task_id, error = %err, "failed to mark task failed after missing results timeout");
                        }
                    }
                }
                JobStatus::Failed => {
                    let failure_reason = job
                        .failure_message
                        .unwrap_or_else(|| "Job failed for an undetermined reason".to_string());
                    match self
                        .transition_task_to_completion(
                            &task_id,
                            Err(TaskError::JobEngineError {
                                reason: failure_reason,
                            }),
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            info!(metis_id = %task_id, "updated task status to Failed from job engine");
                        }
                        Err(err) => {
                            warn!(metis_id = %task_id, error = %err, "failed to update task status to Failed");
                        }
                    }
                }
            },
            Err(JobEngineError::NotFound(_)) => {
                warn!(
                    metis_id = %task_id,
                    "job not found in job engine, marking as failed"
                );

                let failure_reason = "Job not found in job engine".to_string();
                if let Err(update_err) = self
                    .transition_task_to_completion(
                        &task_id,
                        Err(TaskError::JobEngineError {
                            reason: failure_reason,
                        }),
                        None,
                    )
                    .await
                {
                    error!(metis_id = %task_id, error = %update_err, "failed to set task status to Failed");
                }
            }
            Err(err) => {
                error!(
                    metis_id = %task_id,
                    error = %err,
                    "failed to check job status in job engine"
                );
            }
        }
    }

    pub async fn upsert_patch(
        &self,
        actor: Option<&Actor>,
        patch_id: Option<PatchId>,
        request: UpsertPatchRequest,
    ) -> Result<PatchId, UpsertPatchError> {
        let UpsertPatchRequest {
            mut patch,
            sync_github_branch,
        } = request;

        let mut should_close_merge_requests = false;
        let store = self.store.as_ref();
        let patch_id = match patch_id {
            Some(id) => {
                let existing_patch = store.get_patch(&id).await.map_err(|source| match source {
                    StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                        patch_id: id.clone(),
                        source,
                    },
                    other => UpsertPatchError::Store { source: other },
                })?;
                let new_status = patch.status;
                should_close_merge_requests =
                    matches!(
                        existing_patch.item.status,
                        PatchStatus::Open | PatchStatus::ChangesRequested
                    ) && matches!(new_status, PatchStatus::Closed | PatchStatus::Merged);

                patch.created_by = existing_patch.item.created_by;
                if let Some(sync_github_branch) = sync_github_branch {
                    if patch.github.is_none() {
                        patch.github = existing_patch.item.github.clone();
                    }

                    let actor = actor.ok_or(UpsertPatchError::GithubActorMissing)?;
                    self.sync_patch_with_github(actor, &mut patch, &sync_github_branch)
                        .await?;
                }

                store
                    .update_patch(&id, patch)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(_) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source,
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?;

                id
            }
            None => {
                if let Some(ref job_id) = patch.created_by {
                    let status = store
                        .get_task(job_id)
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertPatchError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertPatchError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?
                        .item
                        .status;

                    if status != Status::Running {
                        return Err(UpsertPatchError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }
                }

                if let Some(sync_github_branch) = sync_github_branch {
                    let actor = actor.ok_or(UpsertPatchError::GithubActorMissing)?;
                    self.sync_patch_with_github(actor, &mut patch, &sync_github_branch)
                        .await?;
                }

                store
                    .add_patch(patch)
                    .await
                    .map_err(|source| match source {
                        StoreError::PatchNotFound(id) => UpsertPatchError::PatchNotFound {
                            patch_id: id.clone(),
                            source: StoreError::PatchNotFound(id),
                        },
                        other => UpsertPatchError::Store { source: other },
                    })?
            }
        };

        if should_close_merge_requests {
            let merge_request_issue_ids =
                store
                    .get_issues_for_patch(&patch_id)
                    .await
                    .map_err(|source| UpsertPatchError::MergeRequestLookup {
                        patch_id: patch_id.clone(),
                        source,
                    })?;

            let mut closed_issue_ids = Vec::new();
            for issue_id in merge_request_issue_ids {
                let issue = store.get_issue(&issue_id).await.map_err(|source| {
                    UpsertPatchError::MergeRequestUpdate {
                        patch_id: patch_id.clone(),
                        issue_id: issue_id.clone(),
                        source,
                    }
                })?;
                let mut issue = issue.item;

                if issue.issue_type != IssueType::MergeRequest
                    || matches!(issue.status, IssueStatus::Closed | IssueStatus::Dropped)
                {
                    continue;
                }

                issue.status = IssueStatus::Closed;
                store
                    .update_issue(&issue_id, issue)
                    .await
                    .map_err(|source| UpsertPatchError::MergeRequestUpdate {
                        patch_id: patch_id.clone(),
                        issue_id: issue_id.clone(),
                        source,
                    })?;
                closed_issue_ids.push(issue_id);
            }

            if !closed_issue_ids.is_empty() {
                let issues = closed_issue_ids
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::info!(
                    patch_id = %patch_id,
                    issues = %issues,
                    "closed merge-request issues for patch"
                );
            }
        }

        tracing::info!(patch_id = %patch_id, "patch stored successfully");

        Ok(patch_id)
    }

    pub async fn upsert_issue(
        &self,
        issue_id: Option<IssueId>,
        request: UpsertIssueRequest,
    ) -> Result<IssueId, UpsertIssueError> {
        let UpsertIssueRequest {
            mut issue, job_id, ..
        } = request;
        let mut tasks_to_kill = Vec::new();

        let store = self.store.as_ref();

        let issue_id = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();
                if updated_issue.creator.as_ref().trim().is_empty() {
                    return Err(UpsertIssueError::MissingCreator);
                }
                let is_dropping = updated_issue.status == IssueStatus::Dropped;

                if let Err(source) =
                    validate_issue_lifecycle(store, Some(&id), &updated_issue).await
                {
                    return Err(match source {
                        StoreError::IssueNotFound(_) => UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        },
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: Some(id),
                        },
                    });
                }

                match store.update_issue(&id, updated_issue).await {
                    Ok(()) => {
                        if is_dropping {
                            tasks_to_kill =
                                active_tasks_for_issue(store, &id).await.map_err(|source| {
                                    UpsertIssueError::TaskLookup {
                                        source,
                                        issue_id: id.clone(),
                                    }
                                })?;

                            let child_tasks = drop_issue_children(store, &id).await?;
                            tasks_to_kill.extend(child_tasks);
                        }
                        id
                    }
                    Err(source @ StoreError::IssueNotFound(_)) => {
                        return Err(UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        });
                    }
                    Err(StoreError::InvalidDependency(dependency_id)) => {
                        return Err(UpsertIssueError::MissingDependency {
                            dependency_id: dependency_id.clone(),
                            source: StoreError::InvalidDependency(dependency_id),
                        });
                    }
                    Err(source) => {
                        return Err(UpsertIssueError::Store {
                            source,
                            issue_id: Some(id),
                        });
                    }
                }
            }
            None => {
                if let Some(ref job_id) = job_id {
                    let status = store
                        .get_task(job_id)
                        .await
                        .map_err(|source| match source {
                            StoreError::TaskNotFound(_) => UpsertIssueError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertIssueError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?
                        .item
                        .status;

                    if status != Status::Running {
                        return Err(UpsertIssueError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }
                }

                if issue.creator.as_ref().trim().is_empty() {
                    if let Some(parent_dependency) = issue.dependencies.iter().find(|dependency| {
                        dependency.dependency_type == IssueDependencyType::ChildOf
                    }) {
                        match store.get_issue(&parent_dependency.issue_id).await {
                            Ok(parent_issue) => {
                                issue.creator = parent_issue.item.creator;
                            }
                            Err(source @ StoreError::IssueNotFound(_)) => {
                                return Err(UpsertIssueError::MissingDependency {
                                    dependency_id: parent_dependency.issue_id.clone(),
                                    source,
                                });
                            }
                            Err(source) => {
                                return Err(UpsertIssueError::Store {
                                    source,
                                    issue_id: None,
                                });
                            }
                        }
                    }
                }
                if issue.creator.as_ref().trim().is_empty() {
                    return Err(UpsertIssueError::MissingCreator);
                }

                if let Err(source) = validate_issue_lifecycle(store, None, &issue).await {
                    return Err(match source {
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: None,
                        },
                    });
                }

                store
                    .add_issue(issue)
                    .await
                    .map_err(|source| match source {
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: None,
                        },
                    })?
            }
        };

        for job_id in tasks_to_kill {
            match self.job_engine.kill_job(&job_id).await {
                Ok(()) => info!(
                    issue_id = %issue_id,
                    job_id = %job_id,
                    "killed task for dropped issue"
                ),
                Err(JobEngineError::NotFound(_)) => info!(
                    issue_id = %issue_id,
                    job_id = %job_id,
                    "task already missing while dropping issue"
                ),
                Err(source) => {
                    return Err(UpsertIssueError::KillTask {
                        issue_id: issue_id.clone(),
                        job_id,
                        source,
                    });
                }
            }
        }

        info!(issue_id = %issue_id, "issue stored successfully");

        Ok(issue_id)
    }

    pub async fn add_todo_item(
        &self,
        issue_id: IssueId,
        item: TodoItem,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        issue.todo_list.push(item);
        let todo_list = issue.todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn replace_todo_list(
        &self,
        issue_id: IssueId,
        todo_list: Vec<TodoItem>,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        issue.todo_list = todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn set_todo_item_status(
        &self,
        issue_id: IssueId,
        item_number: usize,
        is_done: bool,
    ) -> Result<Vec<TodoItem>, UpdateTodoListError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(&issue_id).await.map_err(|source| {
            UpdateTodoListError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = issue.item;

        if item_number == 0 {
            return Err(UpdateTodoListError::InvalidItemNumber {
                issue_id,
                item_number,
            });
        }
        let index = item_number - 1;
        let item =
            issue
                .todo_list
                .get_mut(index)
                .ok_or(UpdateTodoListError::InvalidItemNumber {
                    issue_id: issue_id.clone(),
                    item_number,
                })?;
        item.is_done = is_done;

        let todo_list = issue.todo_list.clone();
        store
            .update_issue(&issue_id, issue)
            .await
            .map_err(|source| UpdateTodoListError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        Ok(todo_list)
    }

    pub async fn merge_queue(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .get_merge_queue(service_repo_name, &config, branch_name)
            .await
    }

    pub async fn enqueue_merge_queue_patch(
        &self,
        service_repo_name: &RepoName,
        branch_name: &str,
        patch_id: PatchId,
    ) -> Result<MergeQueue, MergeQueueError> {
        let config = self
            .repository_from_store(service_repo_name)
            .await
            .map_err(|source| match source {
                StoreError::RepositoryNotFound(_) => {
                    MergeQueueError::UnknownRepository(service_repo_name.clone())
                }
                other => MergeQueueError::RepositoryLookup {
                    repo_name: service_repo_name.clone(),
                    source: other,
                },
            })?;

        let patch = self.load_patch(patch_id.clone()).await?;

        self.service_state
            .ensure_cached(service_repo_name, &config)
            .await?;
        self.service_state
            .add_patch_to_merge_queue(service_repo_name, &config, branch_name, patch_id, &patch)
            .await
    }

    pub async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let store = self.store.as_ref();
        issue_ready(store, issue_id).await
    }

    pub async fn list_issues(&self) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_issues().await
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        let store = self.store.as_ref();
        store
            .list_tasks()
            .await
            .map(|tasks| tasks.into_iter().map(|(id, _)| id).collect())
    }

    pub async fn transition_task_to_running(
        &self,
        task_id: &TaskId,
    ) -> Result<Versioned<Task>, StoreError> {
        let store = self.store.as_ref();
        let latest = store.get_task(task_id).await?;
        if !matches!(latest.item.status, Status::Created | Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Running;
        updated.last_message = None;
        updated.error = None;

        store.update_task(task_id, updated).await
    }

    pub async fn transition_task_to_completion(
        &self,
        task_id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
    ) -> Result<Versioned<Task>, StoreError> {
        let store = self.store.as_ref();
        let latest = store.get_task(task_id).await?;
        let can_transition = match latest.item.status {
            Status::Created => result.is_err(),
            Status::Pending | Status::Running => true,
            _ => false,
        };
        if !can_transition {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        match result {
            Ok(()) => {
                updated.status = Status::Complete;
                updated.last_message = last_message;
                updated.error = None;
            }
            Err(error) => {
                updated.status = Status::Failed;
                updated.last_message = None;
                updated.error = Some(error);
            }
        }

        store.update_task(task_id, updated).await
    }

    pub async fn transition_task_to_pending(
        &self,
        task_id: &TaskId,
    ) -> Result<Versioned<Task>, StoreError> {
        let store = self.store.as_ref();
        let latest = store.get_task(task_id).await?;
        if latest.item.status != Status::Created {
            return Err(StoreError::InvalidStatusTransition);
        }

        let mut updated = latest.item;
        updated.status = Status::Pending;
        updated.last_message = None;
        updated.error = None;

        store.update_task(task_id, updated).await
    }

    pub async fn get_task(&self, task_id: &TaskId) -> Result<Task, StoreError> {
        let store = self.store.as_ref();
        store.get_task(task_id).await.map(|task| task.item)
    }

    pub async fn get_task_versions(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<Versioned<Task>>, StoreError> {
        let store = self.store.as_ref();
        store.get_task_versions(task_id).await
    }

    pub async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        let store = self.store.as_ref();
        store.get_tasks_for_issue(issue_id).await
    }

    pub async fn repository_from_store(&self, name: &RepoName) -> Result<Repository, StoreError> {
        let store = self.store.as_ref();
        store.get_repository(name).await.map(|repo| repo.item)
    }

    async fn load_patch(&self, patch_id: PatchId) -> Result<Patch, MergeQueueError> {
        let store = self.store.as_ref();
        match store.get_patch(&patch_id).await {
            Ok(patch) => Ok(patch.item),
            Err(StoreError::PatchNotFound(_)) => Err(MergeQueueError::PatchNotFound { patch_id }),
            Err(source) => Err(MergeQueueError::PatchLookup { patch_id, source }),
        }
    }
}

fn join_issue_ids(ids: &[IssueId]) -> String {
    let mut values: Vec<String> = ids.iter().map(ToString::to_string).collect();
    values.sort();
    values.join(", ")
}

fn join_item_numbers(numbers: &[usize]) -> String {
    let mut values = numbers.to_vec();
    values.sort();
    values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<String>>()
        .join(", ")
}

async fn issue_ready(store: &dyn Store, issue_id: &IssueId) -> Result<bool, StoreError> {
    let issue = store.get_issue(issue_id).await?;
    let issue = issue.item;

    match issue.status {
        IssueStatus::Closed | IssueStatus::Dropped => Ok(false),
        IssueStatus::Open => {
            for dependency in issue
                .dependencies
                .iter()
                .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
            {
                let blocker = store.get_issue(&dependency.issue_id).await?;
                if blocker.item.status != IssueStatus::Closed {
                    return Ok(false);
                }
            }

            Ok(true)
        }
        IssueStatus::InProgress => {
            for child_id in store.get_issue_children(issue_id).await? {
                let child = store.get_issue(&child_id).await?;
                if child.item.status != IssueStatus::Closed {
                    return Ok(false);
                }
            }

            Ok(true)
        }
    }
}

async fn validate_issue_lifecycle(
    store: &dyn Store,
    issue_id: Option<&IssueId>,
    issue: &Issue,
) -> Result<(), StoreError> {
    if issue.status != IssueStatus::Closed {
        return Ok(());
    }

    let mut open_blockers = Vec::new();
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::BlockedOn)
    {
        let blocker = store
            .get_issue(&dependency.issue_id)
            .await
            .map_err(|err| match err {
                StoreError::IssueNotFound(missing_id) => StoreError::InvalidDependency(missing_id),
                other => other,
            })?;

        if blocker.item.status != IssueStatus::Closed {
            open_blockers.push(dependency.issue_id.clone());
        }
    }

    let mut open_todos = Vec::new();
    for (index, item) in issue.todo_list.iter().enumerate() {
        if !item.is_done {
            open_todos.push(index + 1);
        }
    }

    if !open_todos.is_empty() {
        return Err(StoreError::InvalidIssueStatus(format!(
            "cannot close issue with incomplete todo items: {}",
            join_item_numbers(&open_todos)
        )));
    }

    if let Some(issue_id) = issue_id {
        let mut open_children = Vec::new();
        for child_id in store.get_issue_children(issue_id).await? {
            let child = store.get_issue(&child_id).await?;
            if child.item.status != IssueStatus::Closed {
                open_children.push(child_id);
            }
        }

        if !open_children.is_empty() {
            return Err(StoreError::InvalidIssueStatus(format!(
                "cannot close issue with open child issues: {}",
                join_issue_ids(&open_children)
            )));
        }
    }

    if !open_blockers.is_empty() {
        return Err(StoreError::InvalidIssueStatus(format!(
            "blocked issues cannot close until blockers are closed: {}",
            join_issue_ids(&open_blockers)
        )));
    }

    Ok(())
}

async fn drop_issue_children(
    store: &dyn Store,
    issue_id: &IssueId,
) -> Result<Vec<TaskId>, UpsertIssueError> {
    let mut tasks_to_kill = Vec::new();
    let mut to_visit =
        store
            .get_issue_children(issue_id)
            .await
            .map_err(|source| UpsertIssueError::Store {
                source,
                issue_id: Some(issue_id.clone()),
            })?;
    let mut visited: HashSet<IssueId> = HashSet::new();

    while let Some(child_id) = to_visit.pop() {
        if !visited.insert(child_id.clone()) {
            continue;
        }

        let child_issue =
            store
                .get_issue(&child_id)
                .await
                .map_err(|source| UpsertIssueError::Store {
                    source,
                    issue_id: Some(child_id.clone()),
                })?;
        let mut child_issue = child_issue.item;

        if child_issue.status != IssueStatus::Dropped {
            child_issue.status = IssueStatus::Dropped;
            store
                .update_issue(&child_id, child_issue)
                .await
                .map_err(|source| UpsertIssueError::Store {
                    source,
                    issue_id: Some(child_id.clone()),
                })?;
        }

        let active_child_tasks =
            active_tasks_for_issue(store, &child_id)
                .await
                .map_err(|source| UpsertIssueError::TaskLookup {
                    source,
                    issue_id: child_id.clone(),
                })?;
        tasks_to_kill.extend(active_child_tasks);

        let grandchildren = store
            .get_issue_children(&child_id)
            .await
            .map_err(|source| UpsertIssueError::Store {
                source,
                issue_id: Some(child_id.clone()),
            })?;
        to_visit.extend(grandchildren);
    }

    Ok(tasks_to_kill)
}

async fn active_tasks_for_issue(
    store: &dyn Store,
    issue_id: &IssueId,
) -> Result<Vec<TaskId>, StoreError> {
    let task_ids = store.get_tasks_for_issue(issue_id).await?;

    let mut active_task_ids = Vec::new();
    for task_id in task_ids {
        let status = store.get_task(&task_id).await?.item.status;
        if matches!(status, Status::Created | Status::Pending | Status::Running) {
            active_task_ids.push(task_id);
        }
    }

    Ok(active_task_ids)
}

#[cfg(test)]
mod tests {
    use super::{LoginError, UpsertIssueError};
    use crate::{
        app::{AppState, ServiceState},
        domain::{
            actors::Actor,
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, JobSettings,
                TodoItem, UpsertIssueRequest,
            },
            jobs::{BundleSpec, Task},
            patches::{GithubPr, Patch, PatchStatus, UpsertPatchRequest},
            users::{User, Username},
        },
        job_engine::{JobEngine, JobStatus},
        store::{MemoryStore, Status, StoreError, TaskError},
        test_utils::{
            MockJobEngine, add_repository, github_user_response, test_app_config, test_state,
            test_state_with_engine, test_state_with_github_api_base_url,
        },
    };
    use chrono::{Duration, Utc};
    use httpmock::Method::PATCH;
    use httpmock::prelude::*;
    use metis_common::{IssueId, RepoName, TaskId};
    use serde_json::json;
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;

    fn sample_task() -> Task {
        Task::new(
            "Spawn me".to_string(),
            BundleSpec::None,
            None,
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
        )
    }

    fn task_for_issue(issue_id: &IssueId) -> Task {
        Task::new(
            "Spawn me".to_string(),
            BundleSpec::None,
            Some(issue_id.clone()),
            Some("worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
        )
    }

    fn state_with_default_model(model: &str) -> AppState {
        let mut config = test_app_config();
        config.job.default_model = Some(model.to_string());
        AppState::new(
            Arc::new(config),
            None,
            Arc::new(ServiceState::default()),
            Arc::new(MemoryStore::new()),
            Arc::new(MockJobEngine::new()),
            Arc::new(RwLock::new(Vec::new())),
        )
    }

    fn github_pull_request_response(
        number: u64,
        head_ref: &str,
        base_ref: &str,
        html_url: &str,
    ) -> serde_json::Value {
        json!({
            "url": format!("https://api.example.com/pulls/{number}"),
            "id": number,
            "number": number,
            "head": {
                "ref": head_ref,
                "sha": "abc123"
            },
            "base": {
                "ref": base_ref,
                "sha": "def456"
            },
            "html_url": html_url
        })
    }

    fn issue_with_status(
        description: &str,
        status: IssueStatus,
        dependencies: Vec<IssueDependency>,
    ) -> Issue {
        Issue::new(
            IssueType::Task,
            description.to_string(),
            Username::from("creator"),
            String::new(),
            status,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn login_persists_user_and_actor() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let response = handles
            .state
            .login_with_github_token("gh-token".to_string(), "gh-refresh".to_string())
            .await
            .expect("login should succeed");

        assert!(!response.login_token.is_empty());
        assert_eq!(response.user.username.as_str(), "octo");

        let store_read = handles.store.as_ref();
        let user = store_read.get_user(&Username::from("octo")).await?;
        let actors = store_read.list_actors().await?;
        assert_eq!(actors.len(), 1);
        assert_eq!(user.item.username.as_str(), "octo");

        Ok(())
    }

    #[tokio::test]
    async fn login_returns_error_for_invalid_token() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(401);
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let err = handles
            .state
            .login_with_github_token("bad-token".to_string(), "gh-refresh".to_string())
            .await
            .expect_err("login should fail for invalid token");

        assert!(matches!(err, LoginError::InvalidGithubToken(_)));
        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_sync_github_updates_existing_pr() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let user_mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });
        let update_mock = github_server.mock(|when, then| {
            when.method(PATCH)
                .path("/repos/octo/repo/pulls/42")
                .json_body_partial(r#"{"title":"Updated title","body":"Updated description"}"#);
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_pull_request_response(
                    42,
                    "feature",
                    "main",
                    "https://example.com/pr/42",
                ));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let username = Username::from("octo");
        let user = User::new(
            username.clone(),
            42,
            "token-123".to_string(),
            "refresh-123".to_string(),
        );
        handles.store.as_ref().add_user(user).await?;
        let (actor, _auth_token) = Actor::new_for_user(username);
        handles.store.as_ref().add_actor(actor.clone()).await?;
        let repo_name = RepoName::new("octo", "repo")?;
        let existing_patch = Patch::new(
            "Original".to_string(),
            "Original description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Some(TaskId::new()),
            Vec::new(),
            repo_name.clone(),
            Some(GithubPr::new(
                "octo".to_string(),
                "repo".to_string(),
                42,
                Some("old-head".to_string()),
                Some("old-base".to_string()),
                None,
                None,
            )),
        );

        let patch_id = handles.store.as_ref().add_patch(existing_patch).await?;

        let request_patch = Patch::new(
            "Updated title".to_string(),
            "Updated description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            repo_name,
            None,
        );
        let request = UpsertPatchRequest {
            patch: request_patch,
            sync_github_branch: Some(String::from("feature")),
        };

        handles
            .state
            .upsert_patch(Some(&actor), Some(patch_id.clone()), request)
            .await?;

        let stored_patch = handles.store.as_ref().get_patch(&patch_id).await?;
        let github = stored_patch
            .item
            .github
            .expect("github metadata should be preserved");
        assert_eq!(github.number, 42);
        assert_eq!(github.owner, "octo");
        assert_eq!(github.repo, "repo");
        assert_eq!(github.head_ref.as_deref(), Some("feature"));
        assert_eq!(github.base_ref.as_deref(), Some("main"));
        assert_eq!(github.url.as_deref(), Some("https://example.com/pr/42"));

        user_mock.assert_async().await;
        update_mock.assert_async().await;

        Ok(())
    }

    #[tokio::test]
    async fn upsert_patch_sync_github_creates_pr_and_persists_github() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let user_mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });
        let create_mock = github_server.mock(|when, then| {
            when.method(POST)
                .path("/repos/octo/repo/pulls")
                .json_body_partial(
                    r#"{"title":"New patch","head":"metis-t-test","base":"main","body":"New patch description"}"#,
                );
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_pull_request_response(
                    99,
                    "metis-t-test",
                    "main",
                    "https://example.com/pr/99",
                ));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let username = Username::from("octo");
        let user = User::new(
            username.clone(),
            42,
            "token-456".to_string(),
            "refresh-456".to_string(),
        );
        handles.store.as_ref().add_user(user).await?;
        let (actor, _auth_token) = Actor::new_for_user(username);
        handles.store.as_ref().add_actor(actor.clone()).await?;
        let repo_name = RepoName::new("octo", "repo")?;
        add_repository(
            &handles.state,
            repo_name.clone(),
            crate::app::Repository::new(
                "https://example.com/repo.git".to_string(),
                Some("main".to_string()),
                None,
            ),
        )
        .await?;

        let task_id = TaskId::try_from("t-test".to_string())?;
        let mut task = sample_task();
        let created_at = Utc::now();
        handles
            .store
            .as_ref()
            .add_task_with_id(task_id.clone(), task.clone(), created_at)
            .await?;
        task.status = Status::Running;
        handles.store.as_ref().update_task(&task_id, task).await?;
        let patch = Patch::new(
            "New patch".to_string(),
            "New patch description".to_string(),
            "diff".to_string(),
            PatchStatus::Open,
            false,
            Some(task_id),
            Vec::new(),
            repo_name,
            None,
        );
        let request = UpsertPatchRequest {
            patch,
            sync_github_branch: Some(String::from("metis-t-test")),
        };

        let patch_id = handles
            .state
            .upsert_patch(Some(&actor), None, request)
            .await?;
        let stored_patch = handles.store.as_ref().get_patch(&patch_id).await?;
        let github = stored_patch
            .item
            .github
            .expect("github metadata should be created");

        assert_eq!(github.number, 99);
        assert_eq!(github.owner, "octo");
        assert_eq!(github.repo, "repo");
        assert_eq!(github.head_ref.as_deref(), Some("metis-t-test"));
        assert_eq!(github.base_ref.as_deref(), Some("main"));
        assert_eq!(github.url.as_deref(), Some("https://example.com/pr/99"));

        user_mock.assert_async().await;
        create_mock.assert_async().await;

        Ok(())
    }

    #[tokio::test]
    async fn open_issue_ready_when_not_blocked() {
        let state = test_state();

        let issue_id = {
            let store = state.store.as_ref();
            store
                .add_issue(issue_with_status("open", IssueStatus::Open, vec![]))
                .await
                .unwrap()
        };

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_blocked_on_open_issue() {
        let state = test_state();

        let (blocker_id, blocked_issue_id) = {
            let store = state.store.as_ref();
            let blocker_id = store
                .add_issue(issue_with_status("blocker", IssueStatus::Open, vec![]))
                .await
                .unwrap();
            let blocked_issue_id = store
                .add_issue(issue_with_status(
                    "blocked",
                    IssueStatus::Open,
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id.clone(),
                    )],
                ))
                .await
                .unwrap();

            (blocker_id, blocked_issue_id)
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue(
                    &blocker_id,
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_issue_ready_after_children_closed() {
        let state = test_state();

        let (parent_id, child_id, child_dependencies) = {
            let store = state.store.as_ref();
            let parent_id = store
                .add_issue(issue_with_status("parent", IssueStatus::InProgress, vec![]))
                .await
                .unwrap();
            let child_dependencies = vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )];
            let child_id = store
                .add_issue(issue_with_status(
                    "child",
                    IssueStatus::Open,
                    child_dependencies.clone(),
                ))
                .await
                .unwrap();

            (parent_id, child_id, child_dependencies)
        };

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue(
                    &child_id,
                    issue_with_status("child", IssueStatus::Closed, child_dependencies),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_is_not_ready() {
        let state = test_state();

        let issue_id = {
            let store = state.store.as_ref();
            store
                .add_issue(issue_with_status("dropped", IssueStatus::Dropped, vec![]))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let state = test_state();

        let blocked_issue_id = {
            let store = state.store.as_ref();
            let blocker_id = store
                .add_issue(issue_with_status("blocker", IssueStatus::Dropped, vec![]))
                .await
                .unwrap();
            store
                .add_issue(issue_with_status(
                    "blocked",
                    IssueStatus::Open,
                    vec![IssueDependency::new(
                        IssueDependencyType::BlockedOn,
                        blocker_id,
                    )],
                ))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn closed_issue_is_not_ready() {
        let state = test_state();

        let issue_id = {
            let store = state.store.as_ref();
            store
                .add_issue(issue_with_status("closed", IssueStatus::Closed, vec![]))
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn start_pending_task_spawns_and_marks_pending() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let config = state.config.clone();
        let task = sample_task();

        let task_id = {
            let store = state.store.as_ref();
            store.add_task(task, Utc::now()).await.unwrap()
        };

        state.start_pending_task(task_id.clone()).await;

        {
            let store = state.store.as_ref();
            let status = store.get_task(&task_id).await.unwrap().item.status;
            assert_eq!(status, Status::Pending);
        }

        assert!(job_engine.env_vars_for_job(&task_id).is_some());
        let limits = job_engine
            .resource_limits_for_job(&task_id)
            .expect("resource limits should be recorded");
        assert_eq!(
            limits,
            (
                config.job.cpu_limit.clone(),
                config.job.memory_limit.clone()
            )
        );
    }

    #[tokio::test]
    async fn start_pending_task_uses_task_resource_limits() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let job_settings = JobSettings {
            cpu_limit: Some("750m".to_string()),
            memory_limit: Some("2Gi".to_string()),
            ..Default::default()
        };

        let issue_id = {
            let store = state.store.as_ref();
            store
                .add_issue(Issue {
                    issue_type: IssueType::Task,
                    description: "with limits".to_string(),
                    creator: Username::from("creator"),
                    progress: String::new(),
                    status: IssueStatus::Open,
                    assignee: None,
                    job_settings: job_settings.clone(),
                    todo_list: Vec::new(),
                    dependencies: Vec::new(),
                    patches: Vec::new(),
                })
                .await
                .unwrap()
        };

        let task_id = {
            let store = state.store.as_ref();
            let mut task = task_for_issue(&issue_id);
            task.cpu_limit = job_settings.cpu_limit.clone();
            task.memory_limit = job_settings.memory_limit.clone();
            store.add_task(task, Utc::now()).await.unwrap()
        };

        state.start_pending_task(task_id.clone()).await;

        let limits = job_engine
            .resource_limits_for_job(&task_id)
            .expect("resource limits should be recorded");
        assert_eq!(limits, ("750m".to_string(), "2Gi".to_string()));
    }

    #[test]
    fn apply_job_settings_defaults_sets_model() {
        let state = state_with_default_model("gpt-4o");
        let job_settings = JobSettings::default();

        let resolved = state.apply_job_settings_defaults(job_settings);

        assert_eq!(resolved.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn apply_job_settings_defaults_preserves_explicit_model() {
        let state = state_with_default_model("gpt-4o");
        let job_settings = JobSettings {
            model: Some("custom-model".to_string()),
            ..Default::default()
        };

        let resolved = state.apply_job_settings_defaults(job_settings);

        assert_eq!(resolved.model.as_deref(), Some("custom-model"));
    }

    #[tokio::test]
    async fn reap_orphaned_jobs_kills_jobs_missing_from_store() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let tracked_task_id = {
            let store = state.store.as_ref();
            store.add_task(sample_task(), Utc::now()).await.unwrap()
        };
        let orphan_task_id = TaskId::new();

        job_engine
            .insert_job(&tracked_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&orphan_task_id, JobStatus::Running)
            .await;

        state.reap_orphaned_jobs().await;

        let tracked_status = job_engine
            .find_job_by_metis_id(&tracked_task_id)
            .await
            .expect("tracked job should exist")
            .status;
        assert_eq!(tracked_status, JobStatus::Running);

        let orphan_status = job_engine
            .find_job_by_metis_id(&orphan_task_id)
            .await
            .expect("orphan job should exist")
            .status;
        assert_eq!(orphan_status, JobStatus::Failed);
    }

    #[tokio::test]
    async fn reconcile_running_task_marks_missing_jobs_failed() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);
        let task_id = {
            let store = state.store.as_ref();
            let task_id = store.add_task(sample_task(), Utc::now()).await.unwrap();
            state
                .transition_task_to_pending(&task_id)
                .await
                .expect("task should transition to pending");
            task_id
        };

        state.reconcile_running_task(task_id.clone()).await;

        let store = state.store.as_ref();
        assert_eq!(
            store.get_task(&task_id).await.unwrap().item.status,
            Status::Failed
        );

        let status_log = store.get_status_log(&task_id).await.unwrap();
        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => {
                assert_eq!(reason, "Job not found in job engine");
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reconcile_running_task_times_out_completed_jobs_without_results() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let completion_time = Utc::now() - Duration::seconds(90);

        let task_id = {
            let store = state.store.as_ref();
            let task_id = store.add_task(sample_task(), Utc::now()).await.unwrap();
            state
                .transition_task_to_pending(&task_id)
                .await
                .expect("task should transition to pending");
            task_id
        };

        job_engine
            .insert_job_with_metadata(&task_id, JobStatus::Complete, Some(completion_time), None)
            .await;

        state.reconcile_running_task(task_id.clone()).await;

        let store = state.store.as_ref();
        assert_eq!(
            store.get_task(&task_id).await.unwrap().item.status,
            Status::Failed
        );
        let status_log = store.get_status_log(&task_id).await.unwrap();
        assert!(status_log.end_time().is_some());

        match status_log.result().expect("task should be finished") {
            Err(TaskError::JobEngineError { reason }) => assert_eq!(
                reason,
                "Job completed without submitting results (timeout after 1 minute)"
            ),
            other => panic!("unexpected result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dropping_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue.clone(), None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue.clone(), None))
            .await
            .unwrap();

        let grandchild_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, child_id.clone());
        let grandchild_issue = issue_with_status(
            "grandchild",
            IssueStatus::Open,
            vec![grandchild_dependency.clone()],
        );
        let grandchild_id = state
            .upsert_issue(
                None,
                UpsertIssueRequest::new(grandchild_issue.clone(), None),
            )
            .await
            .unwrap();

        let (parent_task_id, child_task_id, grandchild_task_id) = {
            let store = state.store.as_ref();
            let parent_task_id = store
                .add_task(task_for_issue(&parent_id), Utc::now())
                .await
                .unwrap();
            let child_task_id = store
                .add_task(task_for_issue(&child_id), Utc::now())
                .await
                .unwrap();
            let grandchild_task_id = store
                .add_task(task_for_issue(&grandchild_id), Utc::now())
                .await
                .unwrap();
            (parent_task_id, child_task_id, grandchild_task_id)
        };

        job_engine
            .insert_job(&parent_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&grandchild_task_id, JobStatus::Running)
            .await;

        let mut dropped_parent = parent_issue.clone();
        dropped_parent.status = IssueStatus::Dropped;
        state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(dropped_parent, None),
            )
            .await
            .unwrap();

        {
            let store = state.store.as_ref();
            assert_eq!(
                store.get_issue(&child_id).await.unwrap().item.status,
                IssueStatus::Dropped
            );
            assert_eq!(
                store.get_issue(&grandchild_id).await.unwrap().item.status,
                IssueStatus::Dropped
            );
        }

        for task_id in [parent_task_id, child_task_id, grandchild_task_id] {
            let job = job_engine
                .find_job_by_metis_id(&task_id)
                .await
                .expect("job should exist");
            assert_eq!(job.status, JobStatus::Failed);
        }
    }

    #[tokio::test]
    async fn closing_issue_requires_closed_blockers() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let blocker_issue = issue_with_status("blocker", IssueStatus::Open, vec![]);
        let blocker_id = state
            .upsert_issue(None, UpsertIssueRequest::new(blocker_issue, None))
            .await
            .unwrap();

        let blocked_dependencies = vec![IssueDependency::new(
            IssueDependencyType::BlockedOn,
            blocker_id.clone(),
        )];
        let blocked_issue =
            issue_with_status("blocked", IssueStatus::Open, blocked_dependencies.clone());
        let blocked_issue_id = state
            .upsert_issue(None, UpsertIssueRequest::new(blocked_issue, None))
            .await
            .unwrap();

        let err = state
            .upsert_issue(
                Some(blocked_issue_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocked", IssueStatus::Closed, blocked_dependencies.clone()),
                    None,
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == blocked_issue_id && message.contains(&blocker_id.to_string())
        ));

        state
            .upsert_issue(
                Some(blocker_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap();

        state
            .upsert_issue(
                Some(blocked_issue_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("blocked", IssueStatus::Closed, blocked_dependencies),
                    None,
                ),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closing_parent_requires_closed_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let err = state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("parent", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == parent_id && message.contains(&child_id.to_string())
        ));

        state
            .upsert_issue(
                Some(child_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("child", IssueStatus::Closed, vec![child_dependency.clone()]),
                    None,
                ),
            )
            .await
            .unwrap();

        state
            .upsert_issue(
                Some(parent_id.clone()),
                UpsertIssueRequest::new(
                    issue_with_status("parent", IssueStatus::Closed, vec![]),
                    None,
                ),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn closing_issue_requires_completed_todos() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut issue = issue_with_status("todo", IssueStatus::Open, vec![]);
        issue
            .todo_list
            .push(TodoItem::new("finish task".to_string(), false));
        let issue_id = state
            .upsert_issue(None, UpsertIssueRequest::new(issue.clone(), None))
            .await
            .unwrap();

        let mut closed_issue = issue.clone();
        closed_issue.status = IssueStatus::Closed;

        let err = state
            .upsert_issue(
                Some(issue_id.clone()),
                UpsertIssueRequest::new(closed_issue.clone(), None),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            UpsertIssueError::Store {
                source: StoreError::InvalidIssueStatus(message),
                issue_id: Some(id),
            } if id == issue_id && message.contains("incomplete todo items")
        ));

        state
            .set_todo_item_status(issue_id.clone(), 1, true)
            .await
            .unwrap();

        closed_issue
            .todo_list
            .iter_mut()
            .for_each(|item| item.is_done = true);

        state
            .upsert_issue(
                Some(issue_id.clone()),
                UpsertIssueRequest::new(closed_issue, None),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_issue_inherits_creator_from_parent_when_empty() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        parent_issue.creator = Username::from("parent-creator");
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let mut child_issue = issue_with_status("child", IssueStatus::Open, vec![child_dependency]);
        child_issue.creator = Username::from("");
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let store = state.store.as_ref();
        let stored_child = store.get_issue(&child_id).await.unwrap();
        assert_eq!(stored_child.item.creator, Username::from("parent-creator"));
    }

    #[tokio::test]
    async fn create_issue_preserves_explicit_creator_with_parent() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
        parent_issue.creator = Username::from("parent-creator");
        let parent_id = state
            .upsert_issue(None, UpsertIssueRequest::new(parent_issue, None))
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let mut child_issue = issue_with_status("child", IssueStatus::Open, vec![child_dependency]);
        child_issue.creator = Username::from("explicit-creator");
        let child_id = state
            .upsert_issue(None, UpsertIssueRequest::new(child_issue, None))
            .await
            .unwrap();

        let store = state.store.as_ref();
        let stored_child = store.get_issue(&child_id).await.unwrap();
        assert_eq!(
            stored_child.item.creator,
            Username::from("explicit-creator")
        );
    }

    #[tokio::test]
    async fn create_issue_without_parent_rejects_empty_creator() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine);

        let mut issue = issue_with_status("solo", IssueStatus::Open, vec![]);
        issue.creator = Username::from("");
        let err = state
            .upsert_issue(None, UpsertIssueRequest::new(issue, None))
            .await
            .unwrap_err();
        assert!(matches!(err, UpsertIssueError::MissingCreator));
    }
}
