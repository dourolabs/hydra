use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};

use super::issue_graph::IssueGraphContext;
use super::{Status, Store, StoreError, Task, TaskStatusLog};
use crate::domain::{
    actors::Actor,
    issues::{Issue, IssueDependency, IssueDependencyType, IssueGraphFilter},
    patches::Patch,
    users::{User, Username},
};
use metis_common::{
    IssueId, PatchId, RepoName, TaskId, VersionNumber, Versioned, repositories::Repository,
};

/// An in-memory implementation of the Store trait.
///
/// This store keeps tasks, issues, and patches in DashMaps for fast lookups.
/// It uses internal locking to make access thread-safe.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: DashMap<TaskId, Vec<Versioned<Task>>>,
    /// Maps issue IDs to their Issue data
    issues: DashMap<IssueId, Vec<Versioned<Issue>>>,
    /// Maps patch IDs to their Patch data
    patches: DashMap<PatchId, Vec<Versioned<Patch>>>,
    /// Maps repository names to their configurations
    repositories: DashMap<RepoName, Vec<Versioned<Repository>>>,
    /// Maps parent issue IDs to their child issue IDs declared via child-of dependencies
    issue_children: DashMap<IssueId, Vec<IssueId>>,
    /// Maps blocking issue IDs to the issues that are blocked on them
    issue_blocked_on: DashMap<IssueId, Vec<IssueId>>,
    /// Maps issue IDs to tasks spawned from them
    issue_tasks: DashMap<IssueId, Vec<TaskId>>,
    /// Maps patch IDs to the issues that reference them
    patch_issues: DashMap<PatchId, Vec<IssueId>>,
    /// Maps usernames to their User data
    users: DashMap<Username, Vec<Versioned<User>>>,
    /// Maps actor names to their Actor data
    actors: DashMap<String, Vec<Versioned<Actor>>>,
}

impl MemoryStore {
    /// Creates a new empty MemoryStore.
    pub fn new() -> Self {
        Self {
            tasks: DashMap::new(),
            issues: DashMap::new(),
            patches: DashMap::new(),
            repositories: DashMap::new(),
            issue_children: DashMap::new(),
            issue_blocked_on: DashMap::new(),
            issue_tasks: DashMap::new(),
            patch_issues: DashMap::new(),
            users: DashMap::new(),
            actors: DashMap::new(),
        }
    }

    fn latest_versioned<T: Clone>(versions: &[Versioned<T>]) -> Option<Versioned<T>> {
        versions.last().cloned()
    }

    fn next_version<T>(versions: &[Versioned<T>]) -> VersionNumber {
        versions
            .last()
            .map(|entry| entry.version.saturating_add(1))
            .unwrap_or(1)
    }

    fn versioned_now<T>(item: T, version: VersionNumber) -> Versioned<T> {
        Versioned::new(item, version, Utc::now())
    }

    fn versioned_at<T>(item: T, version: VersionNumber, timestamp: DateTime<Utc>) -> Versioned<T> {
        Versioned::new(item, version, timestamp)
    }

    /// Updates issue adjacency indexes to match the provided dependency list.
    fn apply_issue_dependency_delta(
        &self,
        issue_id: &IssueId,
        previous: &[IssueDependency],
        updated: &[IssueDependency],
    ) {
        for dependency in previous {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    if let Some(mut children) = self.issue_children.get_mut(&dependency.issue_id) {
                        children.retain(|child_id| child_id != issue_id);
                        if children.is_empty() {
                            drop(children);
                            self.issue_children.remove(&dependency.issue_id);
                        }
                    }
                }
                IssueDependencyType::BlockedOn => {
                    if let Some(mut blocked) = self.issue_blocked_on.get_mut(&dependency.issue_id) {
                        blocked.retain(|blocked_id| blocked_id != issue_id);
                        if blocked.is_empty() {
                            drop(blocked);
                            self.issue_blocked_on.remove(&dependency.issue_id);
                        }
                    }
                }
            }
        }

        for dependency in updated {
            match dependency.dependency_type {
                IssueDependencyType::ChildOf => {
                    let mut children = self
                        .issue_children
                        .entry(dependency.issue_id.clone())
                        .or_default();
                    if !children.contains(issue_id) {
                        children.push(issue_id.clone());
                    }
                }
                IssueDependencyType::BlockedOn => {
                    let mut blocked = self
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
        &self,
        issue_id: &IssueId,
        previous: &[PatchId],
        updated: &[PatchId],
    ) {
        for patch_id in previous {
            if let Some(mut issues) = self.patch_issues.get_mut(patch_id) {
                issues.retain(|existing| existing != issue_id);
                if issues.is_empty() {
                    drop(issues);
                    self.patch_issues.remove(patch_id);
                }
            }
        }

        for patch_id in updated {
            let mut issues = self.patch_issues.entry(patch_id.clone()).or_default();
            if !issues.contains(issue_id) {
                issues.push(issue_id.clone());
            }
        }
    }

    fn index_task_for_issue(&self, issue_id: &IssueId, task_id: TaskId) {
        let mut tasks = self.issue_tasks.entry(issue_id.clone()).or_default();
        if !tasks.contains(&task_id) {
            tasks.push(task_id);
        }
    }

    fn remove_task_from_issue_index(&self, issue_id: &IssueId, task_id: &TaskId) {
        if let Some(mut tasks) = self.issue_tasks.get_mut(issue_id) {
            tasks.retain(|id| id != task_id);
            if tasks.is_empty() {
                drop(tasks);
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
    async fn add_repository(&self, name: RepoName, config: Repository) -> Result<(), StoreError> {
        if self.repositories.contains_key(&name) {
            return Err(StoreError::RepositoryAlreadyExists(name));
        }

        self.repositories
            .insert(name, vec![Self::versioned_now(config, 1)]);
        Ok(())
    }

    async fn get_repository(&self, name: &RepoName) -> Result<Versioned<Repository>, StoreError> {
        self.repositories
            .get(name)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
    ) -> Result<(), StoreError> {
        let mut versions = self
            .repositories
            .get_mut(&name)
            .ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        let next_version = Self::next_version(&versions);

        versions.push(Self::versioned_now(config, next_version));
        Ok(())
    }

    async fn list_repositories(
        &self,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let mut repositories: Vec<_> = self
            .repositories
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                Some((entry.key().clone(), latest))
            })
            .collect();
        repositories.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(repositories)
    }

    async fn add_issue(&self, issue: Issue) -> Result<IssueId, StoreError> {
        let id = IssueId::new();
        let new_dependencies = issue.dependencies.clone();
        let new_patches = issue.patches.clone();

        self.validate_dependencies(&new_dependencies)?;
        self.issues
            .insert(id.clone(), vec![Self::versioned_now(issue, 1)]);

        if !new_dependencies.is_empty() {
            self.apply_issue_dependency_delta(&id, &[], &new_dependencies);
        }
        if !new_patches.is_empty() {
            self.apply_issue_patch_delta(&id, &[], &new_patches);
        }
        Ok(id)
    }

    async fn get_issue(&self, id: &IssueId) -> Result<Versioned<Issue>, StoreError> {
        self.issues
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn update_issue(&self, id: &IssueId, issue: Issue) -> Result<(), StoreError> {
        let (previous_dependencies, previous_patches) = match self.issues.get(id) {
            Some(entry) => match entry.value().last() {
                Some(latest) => (
                    latest.item.dependencies.clone(),
                    latest.item.patches.clone(),
                ),
                None => return Err(StoreError::IssueNotFound(id.clone())),
            },
            None => return Err(StoreError::IssueNotFound(id.clone())),
        };
        let updated_dependencies = issue.dependencies.clone();
        let updated_patches = issue.patches.clone();

        self.validate_dependencies(&updated_dependencies)?;
        if let Some(mut versions) = self.issues.get_mut(id) {
            let next_version = Self::next_version(&versions);
            versions.push(Self::versioned_now(issue, next_version));
        }

        if !previous_dependencies.is_empty() || !updated_dependencies.is_empty() {
            self.apply_issue_dependency_delta(id, &previous_dependencies, &updated_dependencies);
        }
        if !previous_patches.is_empty() || !updated_patches.is_empty() {
            self.apply_issue_patch_delta(id, &previous_patches, &updated_patches);
        }
        Ok(())
    }

    async fn list_issues(&self) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        Ok(self
            .issues
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                Some((entry.key().clone(), latest))
            })
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
        forward.insert(
            IssueDependencyType::ChildOf,
            self.issue_children
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
        );
        forward.insert(
            IssueDependencyType::BlockedOn,
            self.issue_blocked_on
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
        );

        let mut reverse: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        for entry in self.issues.iter() {
            let issue_id = entry.key();
            if let Some(latest) = entry.value().last() {
                for dependency in &latest.item.dependencies {
                    reverse
                        .entry(dependency.dependency_type)
                        .or_default()
                        .entry(issue_id.clone())
                        .or_default()
                        .push(dependency.issue_id.clone());
                }
            }
        }

        let context = IssueGraphContext::from_dependency_maps(
            self.issues
                .iter()
                .map(|entry| entry.key().clone())
                .collect(),
            forward,
            reverse,
        );
        context.apply_filters(filters)
    }

    async fn add_patch(&self, patch: Patch) -> Result<PatchId, StoreError> {
        let id = PatchId::new();
        self.patches
            .insert(id.clone(), vec![Self::versioned_now(patch, 1)]);
        Ok(id)
    }

    async fn get_patch(&self, id: &PatchId) -> Result<Versioned<Patch>, StoreError> {
        self.patches
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn update_patch(&self, id: &PatchId, patch: Patch) -> Result<(), StoreError> {
        let mut versions = self
            .patches
            .get_mut(id)
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now(patch, next_version));
        Ok(())
    }

    async fn list_patches(&self) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        Ok(self
            .patches
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                Some((entry.key().clone(), latest))
            })
            .collect())
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        match self.patches.get(patch_id) {
            Some(_) => Ok(self
                .patch_issues
                .get(patch_id)
                .map(|entry| entry.value().clone())
                .unwrap_or_default()),
            None => Err(StoreError::PatchNotFound(patch_id.clone())),
        }
    }

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_children
                .get(issue_id)
                .map(|entry| entry.value().clone())
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        match self.issues.get(issue_id) {
            Some(_) => Ok(self
                .issue_blocked_on
                .get(issue_id)
                .map(|entry| entry.value().clone())
                .unwrap_or_default()),
            None => Err(StoreError::IssueNotFound(issue_id.clone())),
        }
    }

    async fn get_tasks_for_issue(&self, issue_id: &IssueId) -> Result<Vec<TaskId>, StoreError> {
        if !self.issues.contains_key(issue_id) {
            return Err(StoreError::IssueNotFound(issue_id.clone()));
        }

        Ok(self
            .issue_tasks
            .get(issue_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default())
    }

    async fn add_task(
        &self,
        task: Task,
        creation_time: DateTime<Utc>,
    ) -> Result<TaskId, StoreError> {
        // Generate a unique ID for the new task
        let id = TaskId::new();
        let mut task = task;
        task.status = Status::Pending;
        task.last_message = None;
        task.error = None;
        let spawned_from = task.spawned_from.clone();

        // Add the task
        self.tasks
            .insert(id.clone(), vec![Self::versioned_at(task, 1, creation_time)]);

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, id.clone());
        }

        Ok(id)
    }

    async fn add_task_with_id(
        &self,
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
        let mut task = task;
        task.status = Status::Pending;
        task.last_message = None;
        task.error = None;
        let spawned_from = task.spawned_from.clone();

        // Add the task with the specified ID
        self.tasks.insert(
            metis_id.clone(),
            vec![Self::versioned_at(task, 1, creation_time)],
        );

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, metis_id.clone());
        }

        Ok(())
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
    ) -> Result<Versioned<Task>, StoreError> {
        let previous_spawned_from = match self.tasks.get(metis_id) {
            Some(entry) => entry
                .value()
                .last()
                .and_then(|existing| existing.item.spawned_from.clone()),
            None => return Err(StoreError::TaskNotFound(metis_id.clone())),
        };

        if let Some(previous_issue) = previous_spawned_from.as_ref() {
            if task.spawned_from.as_ref() != Some(previous_issue) {
                self.remove_task_from_issue_index(previous_issue, metis_id);
            }
        }

        // Overwrite the existing task without modifying edge structure
        let updated = match self.tasks.get_mut(metis_id) {
            Some(mut versions) => {
                let next_version = Self::next_version(&versions);
                let versioned = Self::versioned_now(task.clone(), next_version);
                versions.push(versioned.clone());
                versioned
            }
            None => return Err(StoreError::TaskNotFound(metis_id.clone())),
        };

        if let Some(issue_id) = task.spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, metis_id.clone());
        }
        Ok(updated)
    }

    async fn get_task(&self, id: &TaskId) -> Result<Versioned<Task>, StoreError> {
        self.tasks
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(&self) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        Ok(self
            .tasks
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                Some((entry.key().clone(), latest))
            })
            .collect())
    }

    async fn list_tasks_with_status(&self, status: Status) -> Result<Vec<TaskId>, StoreError> {
        Ok(self
            .tasks
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .last()
                    .is_some_and(|task| task.item.status == status)
            })
            .map(|entry| entry.key().clone())
            .collect())
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.tasks
            .get(id)
            .and_then(|entry| super::task_status_log_from_versions(entry.value()))
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn add_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        if self.actors.contains_key(&name) {
            return Err(StoreError::ActorAlreadyExists(name));
        }

        self.actors
            .insert(name, vec![Self::versioned_now(actor, 1)]);
        Ok(())
    }

    async fn update_actor(&self, actor: Actor) -> Result<(), StoreError> {
        let name = actor.name();
        let mut versions = self
            .actors
            .get_mut(&name)
            .ok_or_else(|| StoreError::ActorNotFound(name.clone()))?;
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now(actor, next_version));
        Ok(())
    }

    async fn get_actor(&self, name: &str) -> Result<Versioned<Actor>, StoreError> {
        super::validate_actor_name(name)?;
        self.actors
            .get(name)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::ActorNotFound(name.to_string()))
    }

    async fn list_actors(&self) -> Result<Vec<(String, Versioned<Actor>)>, StoreError> {
        let mut actors: Vec<_> = self
            .actors
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                Some((entry.key().clone(), latest))
            })
            .collect();
        actors.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(actors)
    }

    async fn add_user(&self, user: User) -> Result<(), StoreError> {
        if self.users.contains_key(&user.username) {
            return Err(StoreError::UserAlreadyExists(user.username.clone()));
        }

        self.users
            .insert(user.username.clone(), vec![Self::versioned_now(user, 1)]);
        Ok(())
    }

    async fn update_user(&self, user: User) -> Result<Versioned<User>, StoreError> {
        let mut versions = self
            .users
            .get_mut(&user.username)
            .ok_or_else(|| StoreError::UserNotFound(user.username.clone()))?;
        let next_version = Self::next_version(&versions);
        let versioned = Self::versioned_now(user, next_version);
        versions.push(versioned.clone());
        Ok(versioned)
    }

    async fn get_user(&self, username: &Username) -> Result<Versioned<User>, StoreError> {
        self.users
            .get(username)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::UserNotFound(username.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            actors::{Actor, UserOrWorker},
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus,
                IssueType,
            },
            jobs::BundleSpec,
            patches::{Patch, PatchStatus},
            task_status::Event,
            users::{User, Username},
        },
        store::TaskError,
        test_utils::test_state_with_store,
    };
    use chrono::{Duration, Utc};
    use metis_common::{RepoName, TaskId, VersionNumber, Versioned, repositories::Repository};
    use std::{collections::HashSet, str::FromStr, sync::Arc};

    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("image:latest".to_string()),
        )
    }

    fn spawn_task() -> Task {
        Task::new(
            "0".to_string(),
            BundleSpec::None,
            None,
            Some("metis-worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
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
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
        )
    }

    fn version_numbers<T>(versions: &[Versioned<T>]) -> Vec<VersionNumber> {
        versions.iter().map(|entry| entry.version).collect()
    }

    fn assert_versioned<T: std::fmt::Debug + PartialEq>(
        actual: &Versioned<T>,
        expected_item: &T,
        expected_version: VersionNumber,
    ) {
        assert_eq!(&actual.item, expected_item);
        assert_eq!(actual.version, expected_version);
    }

    #[tokio::test]
    async fn repository_crud_round_trip() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone())
            .await
            .unwrap();

        let fetched = store.get_repository(&name).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.version, 1);

        let mut updated = config.clone();
        updated.default_branch = Some("develop".to_string());
        store
            .update_repository(name.clone(), updated.clone())
            .await
            .unwrap();

        let list = store.list_repositories().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, name);
        assert_versioned(&list[0].1, &updated, 2);

        let fetched_again = store.get_repository(&name).await.unwrap();
        assert_eq!(fetched_again.item, updated);
        assert_eq!(fetched_again.version, 2);
        assert!(fetched_again.timestamp >= fetched.timestamp);
    }

    #[tokio::test]
    async fn repository_versions_increment_and_latest_returned() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();

        let config = sample_repository_config();
        store
            .add_repository(name.clone(), config.clone())
            .await
            .unwrap();

        let mut updated = config.clone();
        updated.default_branch = Some("release".to_string());
        store
            .update_repository(name.clone(), updated.clone())
            .await
            .unwrap();

        let fetched = store.get_repository(&name).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);

        let versions = store.repositories.get(&name).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn add_repository_rejects_duplicates() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();

        store
            .add_repository(name.clone(), sample_repository_config())
            .await
            .unwrap();

        let err = store
            .add_repository(name.clone(), sample_repository_config())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::RepositoryAlreadyExists(existing) if existing == name
        ));

        let missing_name = RepoName::from_str("dourolabs/other").unwrap();
        let err = store
            .update_repository(missing_name.clone(), sample_repository_config())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(existing) if existing == missing_name
        ));
    }

    #[tokio::test]
    async fn add_issue_rejects_missing_dependencies() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();

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
    async fn issue_versions_increment_and_latest_returned() {
        let store = MemoryStore::new();

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let mut updated = sample_issue(vec![]);
        updated.description = "updated details".to_string();
        store
            .update_issue(&issue_id, updated.clone())
            .await
            .unwrap();

        let fetched = store.get_issue(&issue_id).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);

        let versions = store.issues.get(&issue_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn add_and_get_patch_assigns_id() {
        let store = MemoryStore::new();

        let patch = sample_patch();
        let id = store.add_patch(patch.clone()).await.unwrap();

        let fetched = store.get_patch(&id).await.unwrap();
        assert_eq!(fetched.item, patch);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn update_patch_overwrites_existing_value() {
        let store = MemoryStore::new();

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

        let fetched = store.get_patch(&id).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);

        let versions = store.patches.get(&id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn update_missing_patch_returns_error() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();

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
        let store = MemoryStore::new();
        let missing = IssueId::new();

        store.add_issue(sample_issue(vec![])).await.unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{missing}").parse().unwrap();
        let result = store.search_issue_graph(&[filter]).await;

        assert!(matches!(result, Err(StoreError::IssueNotFound(id)) if id == missing));
    }

    #[tokio::test]
    async fn patch_issue_indexes_updated_on_issue_changes() {
        let store = MemoryStore::new();
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
        let store = MemoryStore::new();

        let task = spawn_task();
        let task_id = store.add_task(task.clone(), Utc::now()).await.unwrap();

        let fetched = store.get_task(&task_id).await.unwrap();
        assert_versioned(&fetched, &task, 1);
        assert_eq!(
            store.get_task(&task_id).await.unwrap().item.status,
            Status::Pending
        );

        let tasks: HashSet<_> = store
            .list_tasks()
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task_id]));
    }

    #[tokio::test]
    async fn task_versions_increment_and_latest_returned() {
        let store = MemoryStore::new();

        let mut task = spawn_task();
        task.prompt = "v1".to_string();
        let task_id = store.add_task(task, Utc::now()).await.unwrap();

        let mut updated = spawn_task();
        updated.prompt = "v2".to_string();
        store.update_task(&task_id, updated.clone()).await.unwrap();

        let fetched = store.get_task(&task_id).await.unwrap();
        assert_versioned(&fetched, &updated, 2);

        let versions = store.tasks.get(&task_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn task_versions_increment_on_transitions() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;
        let task_id = store.add_task(spawn_task(), Utc::now()).await.unwrap();

        state.transition_task_to_started(&task_id).await.unwrap();
        state.transition_task_to_running(&task_id).await.unwrap();
        state
            .transition_task_to_completion(&task_id, Ok(()), None)
            .await
            .unwrap();

        let versions = store.tasks.get(&task_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2, 3, 4]);
        assert_eq!(
            store.get_task(&task_id).await.unwrap().item.status,
            Status::Complete
        );
    }

    #[tokio::test]
    async fn status_log_is_derived_from_task_versions() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;
        let created_at = Utc::now() - Duration::seconds(60);
        let mut task = spawn_task();
        task.prompt = "v1".to_string();
        let task_id = store.add_task(task.clone(), created_at).await.unwrap();

        let mut updated = task.clone();
        updated.prompt = "v2".to_string();
        store.update_task(&task_id, updated).await.unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.creation_time(), Some(created_at));
        assert_eq!(log.current_status(), Status::Pending);

        state.transition_task_to_started(&task_id).await.unwrap();
        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.current_status(), Status::Started);

        state.transition_task_to_running(&task_id).await.unwrap();
        let log = store.get_status_log(&task_id).await.unwrap();
        assert!(matches!(log.events.last(), Some(Event::Started { .. })));

        let mut running = store.get_task(&task_id).await.unwrap().item;
        running.prompt = "v3".to_string();
        store.update_task(&task_id, running).await.unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.events.len(), 3);

        state
            .transition_task_to_completion(&task_id, Ok(()), Some("done".to_string()))
            .await
            .unwrap();
        let log = store.get_status_log(&task_id).await.unwrap();
        match log.events.last() {
            Some(Event::Completed { last_message, .. }) => {
                assert_eq!(last_message.as_deref(), Some("done"))
            }
            other => panic!("expected completed event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tasks_for_issue_uses_index() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let issue_id = store.add_issue(sample_issue(vec![])).await.unwrap();
        let other_issue = store.add_issue(sample_issue(vec![])).await.unwrap();

        let mut pending_task = spawn_task();
        pending_task.spawned_from = Some(issue_id.clone());
        let pending_id = store.add_task(pending_task, Utc::now()).await.unwrap();

        let mut running_task = spawn_task();
        running_task.spawned_from = Some(issue_id.clone());
        let running_id = store.add_task(running_task, Utc::now()).await.unwrap();
        state.transition_task_to_started(&running_id).await.unwrap();
        state.transition_task_to_running(&running_id).await.unwrap();

        let mut completed_task = spawn_task();
        completed_task.spawned_from = Some(issue_id.clone());
        let completed_id = store.add_task(completed_task, Utc::now()).await.unwrap();
        state
            .transition_task_to_started(&completed_id)
            .await
            .unwrap();
        state
            .transition_task_to_running(&completed_id)
            .await
            .unwrap();
        state
            .transition_task_to_completion(&completed_id, Ok(()), None)
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
        let store = MemoryStore::new();

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Pending
        );
    }

    #[tokio::test]
    async fn transition_task_to_started_from_pending() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Pending
        );

        state.transition_task_to_started(&root_id).await.unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Started
        );
    }

    #[tokio::test]
    async fn transition_task_to_running_from_started() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        state.transition_task_to_started(&root_id).await.unwrap();
        state.transition_task_to_running(&root_id).await.unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Running
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Pending
        );

        // First mark as started then running
        state.transition_task_to_started(&root_id).await.unwrap();
        state.transition_task_to_running(&root_id).await.unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Running
        );

        // Then mark as complete
        state
            .transition_task_to_completion(&root_id, Ok(()), None)
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Complete
        );
    }

    #[tokio::test]
    async fn transition_task_to_failure_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Pending
        );

        // First mark as started then running
        state.transition_task_to_started(&root_id).await.unwrap();
        state.transition_task_to_running(&root_id).await.unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Running
        );

        // Then mark as failed
        state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test failure".to_string(),
                }),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_pending_fails() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Trying to mark as complete from pending should fail
        let err = state
            .transition_task_to_completion(&root_id, Ok(()), None)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn transition_task_to_failure_from_pending_succeeds() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let root_id = store.add_task(root_task, Utc::now()).await.unwrap();

        // Marking as failed from pending should succeed
        state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test".to_string(),
                }),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id).await.unwrap().item.status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn update_user_overwrites_existing_value() {
        let store = MemoryStore::new();
        let username = Username::from("alice");

        store
            .add_user(User {
                username: username.clone(),
                github_user_id: 101,
                github_token: "old-token".to_string(),
                github_refresh_token: "old-refresh".to_string(),
            })
            .await
            .unwrap();

        let updated = store
            .update_user(User {
                username: username.clone(),
                github_user_id: 202,
                github_token: "new-token".to_string(),
                github_refresh_token: "new-refresh".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(updated.item.github_token, "new-token");
        assert_eq!(updated.item.github_user_id, 202);
        assert_eq!(updated.item.github_refresh_token, "new-refresh");
        assert_eq!(updated.version, 2);

        let user = store.get_user(&username).await.unwrap();
        assert_eq!(user.item.github_token, "new-token");
        assert_eq!(user.item.github_user_id, 202);
        assert_eq!(user.item.github_refresh_token, "new-refresh");
        assert_eq!(user.version, 2);
    }

    #[tokio::test]
    async fn add_and_get_actor_by_name() {
        let store = MemoryStore::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            user_or_worker: UserOrWorker::Username(Username::from("ada")),
        };

        let name = actor.name();
        store.add_actor(actor.clone()).await.unwrap();

        let fetched = store.get_actor(&name).await.unwrap();
        assert_eq!(fetched.item, actor);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn add_actor_rejects_duplicate_name() {
        let store = MemoryStore::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            user_or_worker: UserOrWorker::Task(TaskId::new()),
        };
        let name = actor.name();

        store.add_actor(actor.clone()).await.unwrap();
        let err = store.add_actor(actor).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorAlreadyExists(existing) if existing == name
        ));
    }

    #[tokio::test]
    async fn update_actor_overwrites_existing_entry() {
        let store = MemoryStore::new();
        let task_id = TaskId::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            user_or_worker: UserOrWorker::Task(task_id),
        };
        let mut updated = actor.clone();
        updated.auth_token_hash = "new-hash".to_string();

        store.add_actor(actor.clone()).await.unwrap();
        store.update_actor(updated.clone()).await.unwrap();

        let fetched = store.get_actor(&updated.name()).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);
    }

    #[tokio::test]
    async fn update_actor_missing_returns_not_found() {
        let store = MemoryStore::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            user_or_worker: UserOrWorker::Username(Username::from("ada")),
        };

        let err = store.update_actor(actor).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorNotFound(name) if name == "u-ada"
        ));
    }

    #[tokio::test]
    async fn get_actor_missing_returns_not_found() {
        let store = MemoryStore::new();
        let task_id = TaskId::new();
        let name = format!("w-{task_id}");

        let err = store.get_actor(&name).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::ActorNotFound(missing) if missing == name
        ));
    }

    #[tokio::test]
    async fn get_actor_invalid_name_returns_error() {
        let store = MemoryStore::new();

        let err = store.get_actor("u-").await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::InvalidActorName(name) if name == "u-"
        ));
    }
}
