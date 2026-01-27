#[cfg(test)]
use crate::domain::issues::{IssueDependency, IssueType};
#[cfg(test)]
use crate::domain::users::Username;
use crate::{
    app::AppState,
    config::AgentQueueConfig,
    domain::{
        issues::{Issue, IssueDependencyType, IssueStatus},
        jobs::BundleSpec,
        patches::{PatchStatus, Review},
    },
    store::{Status, StoreError, Task},
};
use anyhow::Context;
use async_trait::async_trait;
use metis_common::IssueId;
#[cfg(test)]
use metis_common::RepoName;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::str::FromStr;
use tokio::sync::RwLock;

pub const ISSUE_ID_ENV_VAR: &str = "METIS_ISSUE_ID";
pub const AGENT_NAME_ENV_VAR: &str = "METIS_AGENT_NAME";

#[async_trait]
pub trait Spawner: Send + Sync {
    fn name(&self) -> &str;
    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>>;
}

#[derive(Clone, Copy, Debug)]
struct SpawnAttempt {
    status: IssueStatus,
    attempts: u32,
}

pub struct AgentQueue {
    pub name: String,
    pub prompt: String,
    pub max_tries: u32,
    pub max_simultaneous: u32,
    spawn_attempts: RwLock<HashMap<IssueId, SpawnAttempt>>,
}

impl AgentQueue {
    pub fn from_config(config: &AgentQueueConfig) -> Self {
        Self {
            name: config.name.clone(),
            prompt: config.prompt.clone(),
            max_tries: config.max_tries,
            max_simultaneous: config.max_simultaneous,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
    }

    pub fn as_config(&self) -> AgentQueueConfig {
        AgentQueueConfig {
            name: self.name.clone(),
            prompt: self.prompt.clone(),
            max_tries: self.max_tries,
            max_simultaneous: self.max_simultaneous,
        }
    }

    async fn build_task(
        &self,
        state: &AppState,
        issue_id: &IssueId,
        issue: &Issue,
    ) -> anyhow::Result<Option<Task>> {
        let bundle = match (
            issue.job_settings.remote_url.as_ref(),
            issue.job_settings.repo_name.as_ref(),
        ) {
            (Some(remote_url), _) if !remote_url.trim().is_empty() => {
                let rev = issue
                    .job_settings
                    .branch
                    .clone()
                    .unwrap_or_else(|| "main".to_string());
                BundleSpec::GitRepository {
                    url: remote_url.trim().to_string(),
                    rev,
                }
            }
            (_, Some(repo_name)) => {
                let repository = state
                    .repository_from_store(repo_name)
                    .await
                    .context("failed to load repository for issue task")?;
                let rev = issue
                    .job_settings
                    .branch
                    .clone()
                    .or_else(|| repository.default_branch.clone());

                BundleSpec::ServiceRepository {
                    name: repo_name.clone(),
                    rev,
                }
            }
            _ => return Ok(None),
        };

        let prompt = self
            .build_prompt_for_issue(state, issue)
            .await
            .context("failed to build task prompt")?;

        let image = issue
            .job_settings
            .image
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let mut env_vars = HashMap::new();
        env_vars.insert(ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string());
        env_vars.insert(AGENT_NAME_ENV_VAR.to_string(), self.name.clone());

        Ok(Some(Task::new(
            prompt,
            bundle,
            Some(issue_id.clone()),
            image,
            env_vars,
            issue.job_settings.cpu_limit.clone(),
            issue.job_settings.memory_limit.clone(),
        )))
    }

    async fn build_prompt_for_issue(
        &self,
        state: &AppState,
        issue: &Issue,
    ) -> anyhow::Result<String> {
        let mut prompt = self.prompt.trim_end().to_string();

        if issue.issue_type == crate::domain::issues::IssueType::MergeRequest {
            if let Some(patch_id) = issue.patches.last() {
                if let Ok(patch) = state.get_patch(patch_id).await {
                    if patch.status == PatchStatus::ChangesRequested {
                        if let Some(review_summary) = build_review_summary(&patch.reviews) {
                            if !prompt.is_empty() {
                                prompt.push_str("\n\n");
                            }
                            prompt.push_str(&format!(
                                "Merge request follow-up:\nPatch {patch_id} has changes requested. Address the review feedback below and update the existing patch/branch (do not open a new patch).\n{review_summary}"
                            ));
                        }
                    }
                }
            }
        }

        Ok(prompt)
    }

    async fn register_spawn_attempt(
        &self,
        issue_id: &IssueId,
        status: IssueStatus,
        max_tries: u32,
    ) -> bool {
        let mut attempts = self.spawn_attempts.write().await;
        let entry = attempts.entry(issue_id.clone()).or_insert(SpawnAttempt {
            status,
            attempts: 0,
        });

        if entry.status != status {
            *entry = SpawnAttempt {
                status,
                attempts: 0,
            };
        }

        if entry.attempts >= max_tries {
            return false;
        }

        entry.attempts += 1;
        true
    }

    fn max_tries_for_issue(&self, issue: &Issue) -> u32 {
        issue.job_settings.max_retries.unwrap_or(self.max_tries)
    }
}

fn build_review_summary(reviews: &[Review]) -> Option<String> {
    let mut lines = Vec::new();
    for review in reviews.iter().filter(|review| !review.is_approved) {
        let contents = review.contents.trim();
        if contents.is_empty() {
            continue;
        }
        let when = review
            .submitted_at
            .map(|timestamp| format!(" @ {}", timestamp.to_rfc3339()))
            .unwrap_or_default();
        lines.push(format!("{}{}: {}", review.author, when, contents));
    }

    if lines.is_empty() {
        return None;
    }

    let mut summary = String::from("Review feedback:\n");
    for line in lines {
        summary.push_str("- ");
        summary.push_str(&line);
        summary.push('\n');
    }
    Some(summary.trim_end().to_string())
}

#[async_trait]
impl Spawner for AgentQueue {
    fn name(&self) -> &str {
        &self.name
    }

    async fn spawn(&self, state: &AppState) -> anyhow::Result<Vec<Task>> {
        let task_state = agent_task_state(state, &self.name)
            .await
            .context("failed to list tasks for agent queue")?;

        let max_simultaneous = self.max_simultaneous as usize;
        if max_simultaneous == 0 {
            return Ok(Vec::new());
        }

        let active_tasks = task_state.running_tasks + task_state.pending_tasks;
        if active_tasks >= max_simultaneous {
            return Ok(Vec::new());
        }

        let mut remaining_capacity = max_simultaneous - active_tasks;

        let issues = state
            .list_issues()
            .await
            .context("failed to list issues for agent queue")?;

        let mut tasks = Vec::new();
        for (issue_id, issue) in issues {
            if issue.assignee.as_deref() != Some(self.name.as_str()) {
                continue;
            }

            // Do not spawn tasks for closed or dropped issues.
            if matches!(issue.status, IssueStatus::Closed | IssueStatus::Dropped) {
                continue;
            }

            let is_ready = state
                .is_issue_ready(&issue_id)
                .await
                .context("failed to determine if issue is ready")?;
            if !is_ready {
                continue;
            }

            if remaining_capacity == 0 {
                break;
            }

            if task_state.existing_issue_ids.contains(&issue_id) {
                continue;
            }

            if parent_has_running_task(state, &issue).await? {
                continue;
            }

            let maybe_task = self.build_task(state, &issue_id, &issue).await?;
            let Some(task) = maybe_task else {
                continue;
            };

            let max_tries = self.max_tries_for_issue(&issue);
            if !self
                .register_spawn_attempt(&issue_id, issue.status, max_tries)
                .await
            {
                continue;
            }

            tasks.push(task);
            remaining_capacity -= 1;
        }

        Ok(tasks)
    }
}

struct AgentTaskState {
    existing_issue_ids: HashSet<IssueId>,
    running_tasks: usize,
    pending_tasks: usize,
}

async fn agent_task_state(
    state: &AppState,
    agent_name: &str,
) -> Result<AgentTaskState, StoreError> {
    let mut task_state = AgentTaskState {
        existing_issue_ids: HashSet::new(),
        running_tasks: 0,
        pending_tasks: 0,
    };
    let task_ids = state.list_tasks().await?;

    for task_id in task_ids {
        if let Ok(Task { env_vars, .. }) = state.get_task(&task_id).await {
            if !matches!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(current) if current == agent_name
            ) {
                continue;
            }

            let status = state.get_task_status(&task_id).await?;
            match status {
                Status::Pending => task_state.pending_tasks += 1,
                Status::Running => task_state.running_tasks += 1,
                _ => {}
            }

            if let Some(issue_id) = env_vars
                .get(ISSUE_ID_ENV_VAR)
                .and_then(|value| value.parse::<IssueId>().ok())
            {
                if matches!(status, Status::Pending | Status::Running) {
                    task_state.existing_issue_ids.insert(issue_id);
                }
            }
        }
    }

    Ok(task_state)
}

async fn parent_has_running_task(state: &AppState, issue: &Issue) -> Result<bool, StoreError> {
    for dependency in issue
        .dependencies
        .iter()
        .filter(|dependency| dependency.dependency_type == IssueDependencyType::ChildOf)
    {
        for task_id in state.get_tasks_for_issue(&dependency.issue_id).await? {
            if matches!(state.get_task_status(&task_id).await?, Status::Running) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issues::JobSettings;
    use crate::domain::jobs::{Bundle, BundleSpec};
    use crate::domain::patches::{Patch, PatchStatus, Review};
    use crate::{
        app::Repository,
        config::{AgentQueueConfig, DEFAULT_AGENT_MAX_SIMULTANEOUS, DEFAULT_AGENT_MAX_TRIES},
        store::Store,
        test::{TestStateHandles, test_state_with_repo_handles},
    };
    use chrono::Utc;

    fn default_user() -> Username {
        Username::from("spawner")
    }

    fn queue(agent_name: &str) -> AgentQueue {
        AgentQueue {
            name: agent_name.to_string(),
            prompt: "Fix the issue".to_string(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
            spawn_attempts: RwLock::new(HashMap::new()),
        }
    }

    fn repository() -> (RepoName, Repository) {
        let repo_name = RepoName::from_str("dourolabs/metis").expect("repo name should parse");
        let repository = Repository::new(
            "https://github.com/dourolabs/metis.git".to_string(),
            Some("main".to_string()),
            Some("repo-image".to_string()),
        );

        (repo_name, repository)
    }

    fn job_settings(repo_name: &RepoName) -> JobSettings {
        JobSettings {
            repo_name: Some(repo_name.clone()),
            image: Some("repo-image".to_string()),
            ..JobSettings::default()
        }
    }

    async fn state_with_repository() -> anyhow::Result<(TestStateHandles, RepoName)> {
        let (repo_name, repository) = repository();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository).await?;
        Ok((handles, repo_name))
    }

    async fn record_completed_task(store: &dyn Store, task: Task) -> anyhow::Result<()> {
        let task_id = store.add_task(task, Utc::now()).await?;
        store.mark_task_running(&task_id, Utc::now()).await?;
        store
            .mark_task_complete(&task_id, Ok(()), None, Utc::now())
            .await?;
        Ok(())
    }

    fn issue(
        description: &str,
        status: IssueStatus,
        assignee: Option<&str>,
        dependencies: Vec<IssueDependency>,
        repo_name: &RepoName,
    ) -> Issue {
        Issue::new(
            IssueType::Task,
            description.to_string(),
            default_user(),
            String::new(),
            status,
            assignee.map(str::to_string),
            Some(job_settings(repo_name)),
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    fn task(
        prompt: &str,
        context: BundleSpec,
        spawned_from: Option<IssueId>,
        image: Option<&str>,
        env_vars: HashMap<String, String>,
    ) -> Task {
        Task::new(
            prompt.to_string(),
            context,
            spawned_from,
            image.map(str::to_string),
            env_vars,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn spawns_tasks_for_ready_assigned_issues() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let assigned_issue_id = handles
            .store
            .add_issue(issue(
                "Fix login page",
                IssueStatus::Open,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        let in_progress_issue_id = handles
            .store
            .add_issue(issue(
                "In-progress but ready",
                IssueStatus::InProgress,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        handles
            .store
            .add_issue(issue(
                "Ignore closed",
                IssueStatus::Closed,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        let queue = queue("agent-a");
        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 2);

        let mut issue_ids = HashSet::new();
        let mut spawned_from_issue_ids = HashSet::new();
        let default_branch = "main".to_string();
        for task in tasks {
            let Task {
                prompt,
                context,
                spawned_from,
                env_vars,
                ..
            } = task;

            assert_eq!(prompt, "Fix the issue".to_string());
            assert_eq!(
                context,
                BundleSpec::ServiceRepository {
                    name: repo_name.clone(),
                    rev: Some(default_branch.clone())
                }
            );
            spawned_from_issue_ids.insert(spawned_from);
            issue_ids.insert(env_vars.get(ISSUE_ID_ENV_VAR).cloned());
            assert_eq!(
                env_vars.get(AGENT_NAME_ENV_VAR),
                Some(&"agent-a".to_string())
            );
        }

        let expected = HashSet::from([
            Some(assigned_issue_id.to_string()),
            Some(in_progress_issue_id.to_string()),
        ]);
        assert_eq!(issue_ids, expected);
        assert_eq!(
            spawned_from_issue_ids,
            HashSet::from([Some(assigned_issue_id), Some(in_progress_issue_id),])
        );

        Ok(())
    }

    #[tokio::test]
    async fn does_not_requeue_when_task_exists() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let issue_id = handles
            .store
            .add_issue(issue(
                "Already queued",
                IssueStatus::Open,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        handles
            .store
            .add_task(
                task(
                    "Fix the issue",
                    BundleSpec::None,
                    Some(issue_id.clone()),
                    Some("metis-worker:latest"),
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
            )
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn merge_request_changes_requested_includes_review_summary_in_prompt()
    -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let review_time = Utc::now();
        let patch = Patch::new(
            "Review patch".to_string(),
            "Review patch description".to_string(),
            "diff --git a/file b/file\n".to_string(),
            PatchStatus::ChangesRequested,
            false,
            None,
            vec![Review::new(
                "Please handle the edge case.".to_string(),
                false,
                "alex".to_string(),
                Some(review_time),
            )],
            repo_name.clone(),
            None,
        );
        let patch_id = handles.store.add_patch(patch).await?;
        handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::MergeRequest,
                description: "Review patch".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: job_settings(&repo_name),
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: vec![patch_id.clone()],
            })
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        let prompt = &tasks[0].prompt;
        assert!(prompt.contains("Merge request follow-up:"));
        assert!(prompt.contains(&patch_id.to_string()));
        assert!(prompt.contains("Please handle the edge case."));
        assert!(prompt.contains("Review feedback:"));

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_issue_not_ready() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let blocker_id = handles
            .store
            .add_issue(issue(
                "Blocker",
                IssueStatus::Open,
                None,
                vec![],
                &repo_name,
            ))
            .await?;

        handles
            .store
            .add_issue(issue(
                "Blocked issue",
                IssueStatus::Open,
                Some("agent-a"),
                vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    blocker_id.clone(),
                )],
                &repo_name,
            ))
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_parent_task_running() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        let parent_id = handles
            .store
            .add_issue(issue(
                "Parent issue",
                IssueStatus::Open,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        let task_id = handles
            .store
            .add_task(
                task(
                    "Parent task",
                    BundleSpec::None,
                    Some(parent_id.clone()),
                    Some("metis-worker:latest"),
                    HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), parent_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                ),
                Utc::now(),
            )
            .await?;
        handles
            .store
            .mark_task_running(&task_id, Utc::now())
            .await?;

        handles
            .store
            .add_issue(issue(
                "Child issue",
                IssueStatus::Open,
                Some("agent-a"),
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent_id.clone(),
                )],
                &repo_name,
            ))
            .await?;

        let queue = queue("agent-a");
        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn skips_when_repo_missing_but_allows_missing_image() -> anyhow::Result<()> {
        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Missing repo".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: JobSettings {
                    repo_name: None,
                    remote_url: None,
                    image: Some("metis-worker:latest".to_string()),
                    branch: None,
                    max_retries: None,
                    cpu_limit: None,
                    memory_limit: None,
                },
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;

        handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Missing image".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: JobSettings {
                    repo_name: Some(repo_name.clone()),
                    remote_url: None,
                    image: None,
                    branch: None,
                    max_retries: None,
                    cpu_limit: None,
                    memory_limit: None,
                },
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;

        let tasks = queue("agent-a").spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        let task = tasks.first().expect("task should exist");
        assert!(matches!(
            task.context,
            BundleSpec::ServiceRepository { ref name, .. } if name == &repo_name
        ));
        assert!(task.image.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn uses_job_settings_max_retries_override() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 3;

        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Override retries".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: JobSettings {
                    repo_name: Some(repo_name),
                    remote_url: None,
                    image: Some("metis-worker:latest".to_string()),
                    branch: Some("main".to_string()),
                    max_retries: Some(1),
                    cpu_limit: None,
                    memory_limit: None,
                },
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(handles.store.as_ref(), tasks.remove(0)).await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn does_not_spawn_when_at_max_simultaneous() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_simultaneous = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let issue_id = handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Already running".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: job_settings(&repo_name),
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;

        let task_id = handles
            .store
            .add_task(
                Task {
                    prompt: "Existing".to_string(),
                    context: BundleSpec::None,
                    spawned_from: Some(issue_id.clone()),
                    image: None,
                    env_vars: HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                    cpu_limit: None,
                    memory_limit: None,
                },
                Utc::now(),
            )
            .await?;
        handles
            .store
            .mark_task_running(&task_id, Utc::now())
            .await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn caps_new_tasks_to_remaining_capacity() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_simultaneous = 2;

        let (handles, repo_name) = state_with_repository().await?;
        let first_issue_id = handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "First issue".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: job_settings(&repo_name),
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;
        let second_issue_id = handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Second issue".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: job_settings(&repo_name),
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;

        handles
            .store
            .add_task(
                Task {
                    prompt: "Pending work".to_string(),
                    context: BundleSpec::None,
                    spawned_from: Some(first_issue_id.clone()),
                    image: None,
                    env_vars: HashMap::from([
                        (ISSUE_ID_ENV_VAR.to_string(), first_issue_id.to_string()),
                        (AGENT_NAME_ENV_VAR.to_string(), "agent-a".to_string()),
                    ]),
                    cpu_limit: None,
                    memory_limit: None,
                },
                Utc::now(),
            )
            .await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].env_vars.get(ISSUE_ID_ENV_VAR).map(String::as_str),
            Some(second_issue_id.as_ref())
        );

        Ok(())
    }

    #[tokio::test]
    async fn enforces_max_spawn_attempts_per_state() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 2;

        let (handles, repo_name) = state_with_repository().await?;
        handles
            .store
            .add_issue(issue(
                "Retry limited",
                IssueStatus::Open,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(handles.store.as_ref(), tasks.remove(0)).await?;

        let mut tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);
        record_completed_task(handles.store.as_ref(), tasks.remove(0)).await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert!(tasks.is_empty());

        Ok(())
    }

    #[tokio::test]
    async fn resets_attempt_counter_when_status_changes() -> anyhow::Result<()> {
        let mut queue = queue("agent-a");
        queue.max_tries = 1;

        let (handles, repo_name) = state_with_repository().await?;
        let issue_id = handles
            .store
            .add_issue(issue(
                "State change reset",
                IssueStatus::Open,
                Some("agent-a"),
                vec![],
                &repo_name,
            ))
            .await?;

        let first_run = queue.spawn(&handles.state).await?;
        assert_eq!(first_run.len(), 1);
        assert!(queue.spawn(&handles.state).await?.is_empty());

        let mut issue = handles.store.get_issue(&issue_id).await?;
        issue.status = IssueStatus::InProgress;
        handles.store.update_issue(&issue_id, issue).await?;

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        Ok(())
    }

    #[test]
    fn builds_from_config() {
        let config = AgentQueueConfig {
            name: "agent-config".to_string(),
            prompt: "Handle issues".to_string(),
            max_tries: DEFAULT_AGENT_MAX_TRIES,
            max_simultaneous: DEFAULT_AGENT_MAX_SIMULTANEOUS,
        };

        let queue = AgentQueue::from_config(&config);

        assert_eq!(queue.name, "agent-config");
        assert_eq!(queue.prompt, "Handle issues");
        assert_eq!(queue.max_tries, DEFAULT_AGENT_MAX_TRIES);
        assert_eq!(queue.max_simultaneous, DEFAULT_AGENT_MAX_SIMULTANEOUS);
    }

    #[tokio::test]
    async fn service_repo_context_uses_repo_defaults() -> anyhow::Result<()> {
        let (repo_name, repository) = repository();
        let default_branch = repository
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".into());
        let default_image = "agent-image".to_string();
        let handles = test_state_with_repo_handles(repo_name.clone(), repository.clone()).await?;
        let issue_id = handles
            .store
            .add_issue(Issue {
                issue_type: IssueType::Task,
                description: "Assigned".to_string(),
                creator: default_user(),
                progress: String::new(),
                status: IssueStatus::Open,
                assignee: Some("agent-a".to_string()),
                job_settings: JobSettings {
                    repo_name: Some(repo_name.clone()),
                    remote_url: None,
                    image: Some(default_image.clone()),
                    branch: None,
                    max_retries: None,
                    cpu_limit: None,
                    memory_limit: None,
                },
                todo_list: Vec::new(),
                dependencies: vec![],
                patches: Vec::new(),
            })
            .await?;
        let queue = queue("agent-a");

        let tasks = queue.spawn(&handles.state).await?;
        assert_eq!(tasks.len(), 1);

        let resolved = handles.state.resolve_task(&tasks[0]).await?;
        assert_eq!(
            tasks[0].context,
            BundleSpec::ServiceRepository {
                name: repo_name.clone(),
                rev: Some(default_branch.clone())
            }
        );
        assert_eq!(
            resolved.context.bundle,
            Bundle::GitRepository {
                url: "https://github.com/dourolabs/metis.git".to_string(),
                rev: default_branch,
            }
        );
        assert_eq!(resolved.image, default_image);
        assert_eq!(
            resolved
                .env_vars
                .get(ISSUE_ID_ENV_VAR)
                .map(|value| value.as_str()),
            Some(issue_id.as_ref())
        );

        Ok(())
    }
}
