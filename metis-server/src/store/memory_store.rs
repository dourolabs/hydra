use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};

use super::issue_graph::IssueGraphContext;
use super::{Status, Store, StoreError, Task, TaskError, TaskStatusLog};
use crate::domain::{
    issues::{Issue, IssueDependency, IssueDependencyType, IssueGraphFilter},
    patches::Patch,
    task_status::Event,
    users::{User, Username},
};
use metis_common::{IssueId, PatchId, TaskId};

/// An in-memory implementation of the Store trait.
///
/// This store keeps tasks, issues, and patches in HashMaps for fast lookups.
/// It is not thread-safe and should only be used in single-threaded contexts.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: HashMap<TaskId, Task>,
    /// Maps issue IDs to their Issue data
    issues: HashMap<IssueId, Issue>,
    /// Maps patch IDs to their Patch data
    patches: HashMap<PatchId, Patch>,
    /// Maps parent issue IDs to their child issue IDs declared via child-of dependencies
    issue_children: HashMap<IssueId, Vec<IssueId>>,
    /// Maps blocking issue IDs to the issues that are blocked on them
    issue_blocked_on: HashMap<IssueId, Vec<IssueId>>,
    /// Maps issue IDs to tasks spawned from them
    issue_tasks: HashMap<IssueId, Vec<TaskId>>,
    /// Maps patch IDs to the issues that reference them
    patch_issues: HashMap<PatchId, Vec<IssueId>>,
    /// Maps task IDs to their TaskStatusLog
    status_logs: HashMap<TaskId, TaskStatusLog>,
    /// Maps usernames to their User data
    users: HashMap<Username, User>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            issues: HashMap::new(),
            patches: HashMap::new(),
            issue_children: HashMap::new(),
            issue_blocked_on: HashMap::new(),
            issue_tasks: HashMap::new(),
            patch_issues: HashMap::new(),
            status_logs: HashMap::new(),
            users: HashMap::new(),
        }
    }

    /// Updates issue adjacency indexes to match the provided dependency list.
    fn apply_issue_dependency_delta(
        &mut self,
        issue_id: &IssueId,
        previous: &[IssueDependency],
        updated: &[IssueDependency],
    ) {
        for dependency in previous {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    if let Some(children) = self.issue_children.get_mut(&dependency.issue_id) {
                        children.retain(|child_id| child_id != issue_id);
                        if children.is_empty() {
                            self.issue_children.remove(&dependency.issue_id);
                        }
                    }
                }
                IssueDependencyType::BlockedOn => {
                    if let Some(blocked) = self.issue_blocked_on.get_mut(&dependency.issue_id) {
                        blocked.retain(|blocked_id| blocked_id != issue_id);
                        if blocked.is_empty() {
                            self.issue_blocked_on.remove(&dependency.issue_id);
                        }
                    }
                }
            }
        }

        for dependency in updated {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    let children = self
                        .issue_children
                        .entry(dependency.issue_id.clone())
                        .or_default();
                    if !children.contains(issue_id) {
                        children.push(issue_id.clone());
                    }
                }
                IssueDependencyType::BlockedOn => {
                    let blocked = self
                        .issue_blocked_on
                        .entry(dependency.issue_id.clone())
                        .or_default();
                    if !blocked.contains(issue_id) {
                        blocked.push(issue_id.clone());
                    }
                }
            }
        }
    }

    fn apply_issue_patch_delta(
        &mut self,
        issue_id: &IssueId,
        previous: &[PatchId],
        updated: &[PatchId],
    ) {
        for patch_id in previous {
            if let Some(issues) = self.patch_issues.get_mut(patch_id) {
                issues.retain(|existing| existing != issue_id);
                if issues.is_empty() {
                    self.patch_issues.remove(patch_id);
                }
            }
        }

        for patch_id in updated {
            let issues = self.patch_issues.entry(patch_id.clone()).or_default();
            if !issues.contains(issue_id) {
                issues.push(issue_id.clone());
            }
        }
    }

    fn index_task_for_issue(&mut self, issue_id: &IssueId, task_id: TaskId) {
        let tasks = self.issue_tasks.entry(issue_id.clone()).or_default();
        if !tasks.contains(&task_id) {
            tasks.push(task_id);
        }
    }

    fn remove_task_from_issue_index(&mut self, issue_id: &IssueId, task_id: &TaskId) {
        if let Some(tasks) = self.issue_tasks.get_mut(issue_id) {
            tasks.retain(|id| id != task_id);
            if tasks.is_empty() {
                self.issue_tasks.remove(issue_id);
            }
        }
    }

    fn validate_dependencies(&self, dependencies: &[IssueDependency]) -> Result<(), StoreError> {
        for dependency in dependencies {
            if !self.issues.contains_key(&dependency.issue_id) {
                return Err(StoreError::InvalidDependency(dependency.issue_id.clone()));
            }
        }

        Ok(())
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn add_issue(&mut self, issue: Issue) -> Result<IssueId, StoreError> {
        let id = IssueId::new();
        let new_dependencies = issue.dependencies.clone();
        let new_patches = issue.patches.clone();

        self.validate_dependencies(&new_dependencies)?;
        self.issues.insert(id.clone(), issue);

        if !new_dependencies.is_empty() {
            self.apply_issue_dependency_delta(&id, &[], &new_dependencies);
        }
        if !new_patches.is_empty() {
            self.apply_issue_patch_delta(&id, &[], &new_patches);
        }
        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Issue, StoreError> {
        self.issues
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn update_issue(&mut self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        if !self.issues.contains_key(id) {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        let previous_dependencies = self
            .issues
            .get(id)
            .map(|issue| issue.dependencies.clone())
            .unwrap_or_default();
        let previous_patches = self
            .issues
            .get(id)
            .map(|issue| issue.patches.clone())
            .unwrap_or_default();
        let updated_dependencies = issue.dependencies.clone();
        let updated_patches = issue.patches.clone();

        self.validate_dependencies(&updated_dependencies)?;
        self.issues.insert(id.clone(), issue);

        if !previous_dependencies.is_empty() || !updated_dependencies.is_empty() {
            self.apply_issue_dependency_delta(id, &previous_dependencies, &updated_dependencies);
        }
        if !previous_patches.is_empty() || !updated_patches.is_empty() {
            self.apply_issue_patch_delta(id, &previous_patches, &updated_patches);
        }
        Ok(())
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Issue)>, StoreError> {
        Ok(self
            .issues
            .iter()
            .map(|(id, issue)| (id.clone(), issue.clone()))
            .collect())
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        if filters.is_empty() {
            return Ok(HashSet::new());
        }

        let mut forward = HashMap::new();
        forward.insert(IssueDependencyType::ChildOf, self.issue_children.clone());
        forward.insert(
            IssueDependencyType::BlockedOn,
            self.issue_blocked_on.clone(),
        );

        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        for (issue_id, issue) in self.issues.iter() {
            for dependency in &issue.dependencies {
                reverse
                    .entry(dependency.dependency_type)
                    .or_default()
                    .entry(issue_id.clone())
                    .or_default()
                    .push(dependency.issue_id.clone());
            }
        }

        let context = IssueGraphContext::from_dependency_maps(
            self.issues.keys().cloned().collect(),
            forward,
            reverse,
        );
        context.apply_filters(filters)
    }

    async fn add_patch(&mut self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.patches.insert(id.clone(), patch);
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Patch, StoreError> {
        self.patches
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn update_patch(&mut self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        if !self.patches.contains_key(id) {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        self.patches.insert(id.clone(), patch);
        Ok(())
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Patch)>, StoreError> {
        Ok(self
            .patches
            .iter()
            .map(|(id, patch)| (id.clone(), patch.clone()))
            .collect())
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        match self.patches.get(patch_id) {
            Some(_) => Ok(self.patch_issues.get(patch_id).cloned().unwrap_or_default()),
            None => Err(StoreError::PatchNotFound(patch_id.clone())),
        }
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_children
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_blocked_on
                .get(issue_id)
                .cloned()
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        if !self.issues.contains_key(issue_id) {
            return Err(StoreError::IssueNotFound(issue_id.clone()));
        }

        Ok(self.issue_tasks.get(issue_id).cloned().unwrap_or_default())
    }

    async fn add_task(
        &mut self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        // Generate a unique ID for the new task
        let id = TaskId::new();
        let spawned_from = task.spawned_from.clone();

        // Add the task
        self.tasks.insert(id.clone(), task);

        // Initialize status log
        self.status_logs.insert(
            id.clone(),
            TaskStatusLog::new(Status::Pending, creation_time),
        );

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, id.clone());
        }

        Ok(id)
    }

    async fn add_task_with_id(
        &mut self,
        metis_id: TaskId,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Check if task already exists
        if self.tasks.contains_key(&metis_id) {
            return Err(StoreError::Internal(format!(
                "Task already exists: {metis_id}"
            )));
        }
        let spawned_from = task.spawned_from.clone();

        // Add the task with the specified ID
        self.tasks.insert(metis_id.clone(), task);

        // Initialize status log
        self.status_logs.insert(
            metis_id.clone(),
            TaskStatusLog::new(Status::Pending, creation_time),
        );

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, metis_id.clone());
        }

        Ok(())
    }

    async fn update_task(&mut self, metis_id: &TaskId, task: Task) -> Result<(), StoreError> {
        if !self.tasks.contains_key(metis_id) {
            return Err(StoreError::TaskNotFound(metis_id.clone()));
        }

        let previous_spawned_from = self
            .tasks
            .get(metis_id)
            .and_then(|existing| existing.spawned_from.clone());

        if let Some(previous_issue) = previous_spawned_from.as_ref() {
            if task.spawned_from.as_ref() != Some(previous_issue) {
                self.remove_task_from_issue_index(previous_issue, metis_id);
            }
        }

        // Overwrite the existing task without modifying edge structure
        self.tasks.insert(metis_id.clone(), task.clone());

        if let Some(issue_id) = task.spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, metis_id.clone());
        }
        Ok(())
    }

    async fn get_task(&self, id: &TaskId) -> Result<Task, StoreError> {
        self.tasks
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(&self) -> Result<Vec<TaskId>, StoreError> {
        Ok(self.tasks.keys().cloned().collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        Ok(self
            .status_logs
            .iter()
            .filter(|(_, status_log)| status_log.current_status() == status)
            .map(|(id, _)| id.clone())
            .collect())
    }

    async fn get_status(&self, id: &TaskId) -> Result<Status, StoreError> {
        self.status_logs
            .get(id)
            .map(|status_log| status_log.current_status())
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.status_logs
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn mark_task_running(
        &mut self,
        id: &TaskId,
        start_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        // Verify current status is Pending
        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Pending) {
            return Err(StoreError::InvalidStatusTransition);
        }

        status_log.events.push(Event::Started { at: start_time });

        Ok(())
    }

    async fn mark_task_complete(
        &mut self,
        id: &TaskId,
        result: Result<(), TaskError>,
        last_message: Option<String>,
        end_time: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        // Verify task exists
        if !self.tasks.contains_key(id) {
            return Err(StoreError::TaskNotFound(id.clone()));
        }

        let status_log = self
            .status_logs
            .get_mut(id)
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;

        if !matches!(status_log.current_status(), Status::Running) {
            return Err(StoreError::InvalidStatusTransition);
        }

        let event = match result {
            Ok(()) => Event::Completed {
                at: end_time,
                last_message,
            },
            Err(error) => Event::Failed {
                at: end_time,
                error,
            },
        };

        status_log.events.push(event);

        Ok(())
    }

    async fn add_user(&mut self, user: User) -> Result<(), StoreError> {
        if self.users.contains_key(&user.username) {
            return Err(StoreError::UserAlreadyExists(user.username.clone()));
        }

        self.users.insert(user.username.clone(), user);
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>, StoreError> {
        let mut users: Vec<User> = self.users.values().cloned().collect();
        users.sort_by(|a, b| a.username.cmp(&b.username));
        Ok(users)
    }

    async fn delete_user(&mut self, username: &Username) -> Result<(), StoreError> {
        if self.users.remove(username).is_none() {
            return Err(StoreError::UserNotFound(username.clone()));
        }

        Ok(())
    }

    async fn set_user_github_token(
        &mut self,
        username: &Username,
        github_token: String,
        github_user_id: Option<u64>,
    ) -> Result<User, StoreError> {
        let user = self
            .users
            .get_mut(username)
            .ok_or_else(|| StoreError::UserNotFound(username.clone()))?;
        user.github_token = github_token;
        if let Some(github_user_id) = github_user_id {
            user.github_user_id = Some(github_user_id);
        }
        Ok(user.clone())
    }

    async fn get_user_by_github_token(&self, github_token: &str) -> Result<User, StoreError> {
        self.users
            .values()
            .find(|user| user.github_token == github_token)
            .cloned()
            .ok_or(StoreError::UserNotFoundForToken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        issues::{
            Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
        },
        jobs::BundleSpec,
        patches::{Patch, PatchStatus},
        users::{User, Username},
    };
    use chrono::Utc;
    use metis_common::RepoName;
    use std::{collections::HashSet, str::FromStr};

    fn spawn_task() -> Task {
        Task::new(
            "0".to_string(),
            BundleSpec::None,
            None,
            Some("metis-worker:latest".to_string()),
            HashMap::new(),
        )
    }

    fn dummy_diff() -> String {
        "--- a/README.md\n+++ b/README.md\n@@\n-old\n+new\n".to_string()
    }

    fn sample_patch() -> Patch {
        Patch::new(
            "sample patch".to_string(),
            "sample patch".to_string(),
            dummy_diff(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        )
    }

    fn sample_issue(dependencies: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "issue details".to_string(),
            User::new(Username::from("creator"), String::new()),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn add_issue_rejects_missing_dependencies() {
        let mut store = MemoryStore::new();
        let missing_dependency = IssueId::new();

        let err = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                missing_dependency.clone(),
            )]))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidDependency(id) if id == missing_dependency
        ));
    }

    #[tokio::test]
    async fn update_issue_rejects_missing_dependencies() {
        let mut store = MemoryStore::new();

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let missing_dependency = IssueId::new();

        let err = store
            .update_issue(
                &issue_id,
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    missing_dependency.clone(),
                )]),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidDependency(id) if id == missing_dependency
        ));
    }

    #[tokio::test]
    async fn add_and_get_patch_assigns_id() {
        let mut store = MemoryStore::new();

        let patch = sample_patch();
        let id = store.add_patch(patch.clone()).await.unwrap();

        assert_eq!(store.get_patch(&id).await.unwrap(), patch);
    }

    #[tokio::test]
    async fn update_patch_overwrites_existing_value() {
        let mut store = MemoryStore::new();

        let id = store.add_patch(sample_patch()).await.unwrap();
        let updated = Patch::new(
            "new title".to_string(),
            "updated patch".to_string(),
            dummy_diff(),
            PatchStatus::Open,
            false,
            None,
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        );

        store.update_patch(&id, updated.clone()).await.unwrap();

        assert_eq!(store.get_patch(&id).await.unwrap(), updated);
    }

    #[tokio::test]
    async fn update_missing_patch_returns_error() {
        let mut store = MemoryStore::new();
        let missing: PatchId = "p-miss".parse().unwrap();

        let err = store
            .update_patch(
                &missing,
                Patch::new(
                    "noop patch".to_string(),
                    "noop patch".to_string(),
                    dummy_diff(),
                    PatchStatus::Open,
                    false,
                    None,
                    Vec::new(),
                    RepoName::from_str("dourolabs/sample").unwrap(),
                    None,
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::PatchNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn issue_dependency_indexes_populated_on_create() {
        let mut store = MemoryStore::new();

        let parent_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocker_id = store.add_issue(sample_issue(vec![])).await.unwrap();

        let child_id = store
            .add_issue(sample_issue(vec![
                IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
                IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone()),
            ]))
            .await
            .unwrap();

        assert_eq!(
            store.get_issue_children(&parent_id).await.unwrap(),
            vec![child_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&blocker_id).await.unwrap(),
            vec![child_id]
        );
    }

    #[tokio::test]
    async fn issue_dependency_indexes_updated_on_update_and_removal() {
        let mut store = MemoryStore::new();

        let original_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let new_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let original_blocker = store.add_issue(sample_issue(vec![])).await.unwrap();
        let new_blocker = store.add_issue(sample_issue(vec![])).await.unwrap();

        let issue_id = store
            .add_issue(sample_issue(vec![
                IssueDependency::new(IssueDependencyType::ChildOf, original_parent.clone()),
                IssueDependency::new(IssueDependencyType::BlockedOn, original_blocker.clone()),
            ]))
            .await
            .unwrap();

        assert_eq!(
            store.get_issue_children(&original_parent).await.unwrap(),
            vec![issue_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&original_blocker).await.unwrap(),
            vec![issue_id.clone()]
        );

        store
            .update_issue(
                &issue_id,
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, new_parent.clone()),
                    IssueDependency::new(IssueDependencyType::BlockedOn, new_blocker.clone()),
                ]),
            )
            .await
            .unwrap();

        assert!(
            store
                .get_issue_children(&original_parent)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .get_issue_blocked_on(&original_blocker)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.get_issue_children(&new_parent).await.unwrap(),
            vec![issue_id.clone()]
        );
        assert_eq!(
            store.get_issue_blocked_on(&new_blocker).await.unwrap(),
            vec![issue_id.clone()]
        );

        store
            .update_issue(&issue_id, sample_issue(vec![]))
            .await
            .unwrap();

        assert!(
            store
                .get_issue_children(&new_parent)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .get_issue_blocked_on(&new_blocker)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn graph_filter_returns_children() {
        let mut store = MemoryStore::new();

        let parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let child = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )]))
            .await
            .unwrap();
        let _grandchild = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                child.clone(),
            )]))
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{parent}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([child]));
    }

    #[tokio::test]
    async fn graph_filter_returns_transitive_children() {
        let mut store = MemoryStore::new();

        let parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let child = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )]))
            .await
            .unwrap();
        let grandchild = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                child.clone(),
            )]))
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("**:child-of:{parent}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([child, grandchild]));
    }

    #[tokio::test]
    async fn graph_filter_returns_ancestors_for_right_wildcards() {
        let mut store = MemoryStore::new();

        let root = store.add_issue(sample_issue(vec![])).await.unwrap();
        let child = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                root.clone(),
            )]))
            .await
            .unwrap();
        let grandchild = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                child.clone(),
            )]))
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("{grandchild}:child-of:**").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([root, child]));
    }

    #[tokio::test]
    async fn graph_filters_intersect_multiple_constraints() {
        let mut store = MemoryStore::new();

        let parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let blocker = store.add_issue(sample_issue(vec![])).await.unwrap();
        let other_parent = store.add_issue(sample_issue(vec![])).await.unwrap();
        let other_blocker = store.add_issue(sample_issue(vec![])).await.unwrap();

        let matching_issue = store
            .add_issue(sample_issue(vec![
                IssueDependency::new(IssueDependencyType::ChildOf, parent.clone()),
                IssueDependency::new(IssueDependencyType::BlockedOn, blocker.clone()),
            ]))
            .await
            .unwrap();

        let non_matching_child = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )]))
            .await
            .unwrap();
        let non_matching_blocked = store
            .add_issue(sample_issue(vec![IssueDependency::new(
                IssueDependencyType::BlockedOn,
                blocker.clone(),
            )]))
            .await
            .unwrap();
        let unrelated_issue = store
            .add_issue(sample_issue(vec![
                IssueDependency::new(IssueDependencyType::ChildOf, other_parent),
                IssueDependency::new(IssueDependencyType::BlockedOn, other_blocker),
            ]))
            .await
            .unwrap();

        let filter_a: IssueGraphFilter = format!("*:child-of:{parent}").parse().unwrap();
        let filter_b: IssueGraphFilter = format!("*:blocked-on:{blocker}").parse().unwrap();

        let matches = store
            .search_issue_graph(&[filter_a, filter_b])
            .await
            .unwrap();

        assert_eq!(matches, HashSet::from([matching_issue]));
        assert!(!matches.contains(&non_matching_child));
        assert!(!matches.contains(&non_matching_blocked));
        assert!(!matches.contains(&unrelated_issue));
    }

    #[tokio::test]
    async fn graph_filter_errors_when_literal_missing() {
        let mut store = MemoryStore::new();
        let missing = IssueId::new();

        store.add_issue(sample_issue(vec![])).await.unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{missing}").parse().unwrap();
        let result = store.search_issue_graph(&[filter]).await;

        assert!(matches!(result, Err(StoreError::IssueNotFound(id)) if id == missing));
    }

    #[tokio::test]
    async fn patch_issue_indexes_updated_on_issue_changes() {
        let mut store = MemoryStore::new();
        let patch_a = store.add_patch(sample_patch()).await.unwrap();
        let patch_b = store.add_patch(sample_patch()).await.unwrap();

        let mut issue = sample_issue(vec![]);
        issue.patches = vec![patch_a.clone()];
        let issue_id = store.add_issue(issue).await.unwrap();

        assert_eq!(
            store.get_issues_for_patch(&patch_a).await.unwrap(),
            vec![issue_id.clone()]
        );
        assert!(
            store
                .get_issues_for_patch(&patch_b)
                .await
                .unwrap()
                .is_empty()
        );

        let mut updated_issue = sample_issue(vec![]);
        updated_issue.patches = vec![patch_b.clone()];
        store.update_issue(&issue_id, updated_issue).await.unwrap();

        assert!(
            store
                .get_issues_for_patch(&patch_a)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.get_issues_for_patch(&patch_b).await.unwrap(),
            vec![issue_id]
        );
    }

    #[tokio::test]
    async fn add_and_retrieve_tasks() {
        let mut store = MemoryStore::new();

        let task = spawn_task();
        let task_id = store.add_task(task.clone(), Utc::now()).await.unwrap();

        assert_eq!(store.get_task(&task_id).await.unwrap(), task);
        assert_eq!(store.get_status(&task_id).await.unwrap(), Status::Pending);

        let tasks: HashSet<_> = store.list_tasks().await.unwrap().into_iter().collect();
        assert_eq!(tasks, HashSet::from([task_id]));
    }

    #[tokio::test]
    async fn tasks_for_issue_uses_index() {
        let mut store = MemoryStore::new();

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let other_issue = store.add_issue(sample_issue(vec![])).await.unwrap();

        let mut pending_task = spawn_task();
        pending_task.spawned_from = Some(issue_id.clone());
        let pending_id = store.add_task(pending_task, Utc::now()).await.unwrap();

        let mut running_task = spawn_task();
        running_task.spawned_from = Some(issue_id.clone());
        let running_id = store.add_task(running_task, Utc::now()).await.unwrap();
        store
            .mark_task_running(&running_id, Utc::now())
            .await
            .unwrap();

        let mut completed_task = spawn_task();
        completed_task.spawned_from = Some(issue_id.clone());
        let completed_id = store.add_task(completed_task, Utc::now()).await.unwrap();
        store
            .mark_task_running(&completed_id, Utc::now())
            .await
            .unwrap();
        store
            .mark_task_complete(&completed_id, Ok(()), None, Utc::now())
            .await
            .unwrap();

        let mut unrelated_task = spawn_task();
        unrelated_task.spawned_from = Some(other_issue.clone());
        let unrelated_id = store.add_task(unrelated_task, Utc::now()).await.unwrap();

        let tasks: HashSet<_> = store
            .get_tasks_for_issue(&issue_id)
            .await
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(tasks, HashSet::from([pending_id, running_id, completed_id]));

        let other_tasks: HashSet<_> = store
            .get_tasks_for_issue(&other_issue)
            .await
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(other_tasks, HashSet::from([unrelated_id]));
    }

    #[tokio::test]
    async fn tasks_for_missing_issue_returns_error() {
        let store = MemoryStore::new();
        let missing_issue = IssueId::new();

        let err = store.get_tasks_for_issue(&missing_issue).await.unwrap_err();

        assert!(matches!(err, StoreError::IssueNotFound(id) if id == missing_issue));
    }

    #[tokio::test]
    async fn task_starts_as_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);
    }

    #[tokio::test]
    async fn mark_task_running_transitions_from_pending() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);
    }

    #[tokio::test]
    async fn mark_task_complete_transitions_from_running() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        // First mark as running
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);

        // Then mark as complete
        store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Complete);
    }

    #[tokio::test]
    async fn mark_task_failed_transitions_from_running() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Pending);

        // First mark as running
        store.mark_task_running(&root_id, Utc::now()).await.unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Running);

        // Then mark as failed
        store
            .mark_task_complete(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test failure".to_string(),
                }),
                None,
                Utc::now(),
            )
            .await
            .unwrap();
        assert_eq!(store.get_status(&root_id).await.unwrap(), Status::Failed);
    }

    #[tokio::test]
    async fn mark_task_complete_from_pending_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as complete from pending should fail
        let err = store
            .mark_task_complete(&root_id, Ok(()), None, Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn mark_task_failed_from_pending_fails() {
        let mut store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as failed from pending should fail
        let err = store
            .mark_task_complete(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test".to_string(),
                }),
                None,
                Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn set_user_github_token_overwrites_existing_value() {
        let mut store = MemoryStore::new();
        let username = Username::from("alice");

        store
            .add_user(User {
                username: username.clone(),
                github_user_id: Some(101),
                github_token: "old-token".to_string(),
            })
            .await
            .unwrap();

        let updated = store
            .set_user_github_token(&username, "new-token".to_string(), Some(202))
            .await
            .unwrap();

        assert_eq!(updated.github_token, "new-token");
        assert_eq!(updated.github_user_id, Some(202));

        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].github_token, "new-token");
        assert_eq!(users[0].github_user_id, Some(202));
    }

    #[tokio::test]
    async fn get_user_by_github_token_returns_match() {
        let mut store = MemoryStore::new();
        let username = Username::from("user-one");

        store
            .add_user(User {
                username: username.clone(),
                github_user_id: Some(11),
                github_token: "token-abc".to_string(),
            })
            .await
            .unwrap();

        let user = store.get_user_by_github_token("token-abc").await.unwrap();

        assert_eq!(user.username, username);
    }

    #[tokio::test]
    async fn get_user_by_github_token_returns_not_found() {
        let store = MemoryStore::new();

        let err = store.get_user_by_github_token("missing").await.unwrap_err();

        assert!(matches!(err, StoreError::UserNotFoundForToken));
    }

    #[tokio::test]
    async fn delete_missing_user_returns_error() {
        let mut store = MemoryStore::new();

        let username = Username::from("missing-user");
        let err = store.delete_user(&username).await.unwrap_err();

        assert!(matches!(err, StoreError::UserNotFound(name) if name == username));
    }
}
