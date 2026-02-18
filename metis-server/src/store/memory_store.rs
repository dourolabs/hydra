use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};

use super::issue_graph::IssueGraphContext;
use super::{ReadOnlyStore, Status, Store, StoreError, Task, TaskStatusLog};
use crate::domain::{
    actors::{Actor, ActorRef},
    documents::Document,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
    },
    patches::Patch,
    users::{User, Username},
};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::jobs::SearchJobsQuery;
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, PatchId, RepoName, TaskId, VersionNumber, Versioned,
    repositories::{Repository, SearchRepositoriesQuery},
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
    /// Maps document IDs to their Document data
    documents: DashMap<DocumentId, Vec<Versioned<Document>>>,
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
    /// Maps document paths to the document IDs that live under them
    documents_by_path: DashMap<String, HashSet<DocumentId>>,
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
            documents: DashMap::new(),
            repositories: DashMap::new(),
            issue_children: DashMap::new(),
            issue_blocked_on: DashMap::new(),
            issue_tasks: DashMap::new(),
            patch_issues: DashMap::new(),
            documents_by_path: DashMap::new(),
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

    fn versioned_now_with_actor<T>(
        item: T,
        version: VersionNumber,
        actor: &ActorRef,
    ) -> Versioned<T> {
        Versioned::with_actor(item, version, Utc::now(), actor.clone())
    }

    fn versioned_at_with_actor<T>(
        item: T,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Versioned<T> {
        Versioned::with_actor(item, version, timestamp, actor.clone())
    }

    /// Returns true if the patch matches the search term.
    fn patch_matches(search_term: Option<&str>, patch_id: &PatchId, patch: &Patch) -> bool {
        let Some(term) = search_term else {
            return true;
        };

        let lower_id = patch_id.to_string().to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        patch.title.to_lowercase().contains(term)
            || patch.description.to_lowercase().contains(term)
            || format!("{:?}", patch.status).to_lowercase().contains(term)
            || patch
                .service_repo_name
                .to_string()
                .to_lowercase()
                .contains(term)
            || patch.diff.to_lowercase().contains(term)
            || patch
                .branch_name
                .as_deref()
                .map(|value| value.to_lowercase().contains(term))
                .unwrap_or(false)
            || patch
                .github
                .as_ref()
                .map(|github| {
                    github.owner.to_lowercase().contains(term)
                        || github.repo.to_lowercase().contains(term)
                        || github.number.to_string().contains(term)
                        || github
                            .head_ref
                            .as_deref()
                            .map(|value| value.to_lowercase().contains(term))
                            .unwrap_or(false)
                        || github
                            .base_ref
                            .as_deref()
                            .map(|value| value.to_lowercase().contains(term))
                            .unwrap_or(false)
                })
                .unwrap_or(false)
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

    fn index_document_path(&self, document_id: &DocumentId, path: Option<&str>) {
        if let Some(path) = path {
            let mut entries = self.documents_by_path.entry(path.to_string()).or_default();
            entries.insert(document_id.clone());
        }
    }

    fn remove_document_path(&self, document_id: &DocumentId, path: Option<&str>) {
        if let Some(path) = path {
            if let Some(mut entries) = self.documents_by_path.get_mut(path) {
                entries.remove(document_id);
                if entries.is_empty() {
                    drop(entries);
                    self.documents_by_path.remove(path);
                }
            }
        }
    }

    fn document_ids_with_path_prefix(&self, prefix: &str) -> Vec<DocumentId> {
        if prefix.is_empty() {
            return self
                .documents
                .iter()
                .map(|entry| entry.key().clone())
                .collect();
        }

        let mut ids = Vec::new();
        for entry in self.documents_by_path.iter() {
            if entry.key().starts_with(prefix) {
                ids.extend(entry.value().iter().cloned());
            }
        }
        ids
    }

    fn document_ids_with_exact_path(&self, path: &str) -> Vec<DocumentId> {
        self.documents_by_path
            .get(path)
            .map(|entry| entry.value().iter().cloned().collect())
            .unwrap_or_default()
    }

    fn documents_from_ids(&self, ids: &[DocumentId]) -> Vec<(DocumentId, Versioned<Document>)> {
        let mut seen = HashSet::new();
        let mut documents = Vec::new();

        for id in ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(entry) = self.documents.get(id) {
                if let Some(latest) = Self::latest_versioned(entry.value()) {
                    documents.push((id.clone(), latest));
                }
            }
        }

        documents
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
impl ReadOnlyStore for MemoryStore {
    async fn get_repository(
        &self,
        name: &RepoName,
        include_deleted: bool,
    ) -> Result<Versioned<Repository>, StoreError> {
        let versioned = self
            .repositories
            .get(name)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::RepositoryNotFound(name.clone()));
        }
        Ok(versioned)
    }

    async fn list_repositories(
        &self,
        query: &SearchRepositoriesQuery,
    ) -> Result<Vec<(RepoName, Versioned<Repository>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let mut repositories: Vec<_> = self
            .repositories
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                // Skip deleted unless include_deleted
                if !include_deleted && latest.item.deleted {
                    return None;
                }
                Some((entry.key().clone(), latest))
            })
            .collect();
        repositories.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(repositories)
    }

    async fn get_issue(
        &self,
        id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        let versioned = self
            .issues
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;

        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        Ok(versioned)
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        self.issues
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let issue_type_filter: Option<IssueType> = query.issue_type.map(Into::into);
        let status_filter: Option<IssueStatus> = query.status.map(Into::into);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let assignee_filter = query
            .assignee
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());

        Ok(self
            .issues
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                if !include_deleted && latest.item.deleted {
                    return None;
                }
                let issue_id = entry.key();
                if !issue_matches(
                    issue_type_filter,
                    status_filter,
                    search_term.as_deref(),
                    assignee_filter,
                    issue_id,
                    &latest.item,
                ) {
                    return None;
                }
                Some((issue_id.clone(), latest))
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

    async fn get_patch(
        &self,
        id: &PatchId,
        include_deleted: bool,
    ) -> Result<Versioned<Patch>, StoreError> {
        let versioned = self
            .patches
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        Ok(versioned)
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        self.patches
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))
    }

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        let status_filter: Vec<crate::domain::patches::PatchStatus> =
            query.status.iter().copied().map(Into::into).collect();

        Ok(self
            .patches
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                if !include_deleted && latest.item.deleted {
                    return None;
                }
                if !status_filter.is_empty() && !status_filter.contains(&latest.item.status) {
                    return None;
                }
                if let Some(ref branch) = query.branch_name {
                    if latest.item.branch_name.as_deref() != Some(branch.as_str()) {
                        return None;
                    }
                }
                if !Self::patch_matches(search_term.as_deref(), entry.key(), &latest.item) {
                    return None;
                }
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

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let versioned = self
            .documents
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }
        Ok(versioned)
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        self.documents
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))
    }

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let mut documents: Vec<(DocumentId, Versioned<Document>)> =
            if let Some(path) = query.path_prefix.as_deref() {
                let ids = if query.path_is_exact.unwrap_or(false) {
                    self.document_ids_with_exact_path(path)
                } else {
                    self.document_ids_with_path_prefix(path)
                };
                self.documents_from_ids(&ids)
            } else {
                self.documents
                    .iter()
                    .filter_map(|entry| {
                        let latest = Self::latest_versioned(entry.value())?;
                        Some((entry.key().clone(), latest))
                    })
                    .collect()
            };

        // Filter deleted documents unless include_deleted is true
        if !query.include_deleted.unwrap_or(false) {
            documents.retain(|(_, versioned)| !versioned.item.deleted);
        }

        if let Some(created_by) = query.created_by.as_ref() {
            documents
                .retain(|(_, versioned)| versioned.item.created_by.as_ref() == Some(created_by));
        }

        if let Some(search_term) = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
        {
            documents.retain(|(_, versioned)| {
                versioned.item.title.to_lowercase().contains(&search_term)
                    || versioned
                        .item
                        .body_markdown
                        .to_lowercase()
                        .contains(&search_term)
                    || versioned
                        .item
                        .path
                        .as_deref()
                        .map(|path| path.to_lowercase().contains(&search_term))
                        .unwrap_or(false)
            });
        }

        documents.sort_by(|(left, _), (right, _)| left.cmp(right));
        Ok(documents)
    }

    async fn get_documents_by_path(
        &self,
        path_prefix: &str,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let ids = self.document_ids_with_path_prefix(path_prefix);
        let mut documents = self.documents_from_ids(&ids);
        documents.sort_by(|(left, _), (right, _)| left.cmp(right));
        Ok(documents)
    }

    async fn get_task(
        &self,
        id: &TaskId,
        include_deleted: bool,
    ) -> Result<Versioned<Task>, StoreError> {
        let versioned = self
            .tasks
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::TaskNotFound(id.clone()));
        }
        Ok(versioned)
    }

    async fn get_task_versions(&self, id: &TaskId) -> Result<Vec<Versioned<Task>>, StoreError> {
        self.tasks
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn list_tasks(
        &self,
        query: &SearchJobsQuery,
    ) -> Result<Vec<(TaskId, Versioned<Task>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        Ok(self
            .tasks
            .iter()
            .filter_map(|entry| {
                let task_id = entry.key();
                let latest = Self::latest_versioned(entry.value())?;

                // Filter by deleted status
                if !include_deleted && latest.item.deleted {
                    return None;
                }

                // Filter by spawned_from
                if let Some(expected_issue) = query.spawned_from.as_ref() {
                    if latest.item.spawned_from.as_ref() != Some(expected_issue) {
                        return None;
                    }
                }

                // Filter by status
                if let Some(expected_status) = query.status {
                    let server_status: Status = expected_status.into();
                    if latest.item.status != server_status {
                        return None;
                    }
                }

                // Filter by text search (matches task ID, prompt, status - NOT notes)
                if let Some(term) = search_term.as_deref() {
                    let matches_id = task_id.as_ref().to_lowercase().contains(term);
                    let matches_prompt = latest.item.prompt.to_lowercase().contains(term);
                    let matches_status = format!("{:?}", latest.item.status)
                        .to_lowercase()
                        .contains(term);

                    if !matches_id && !matches_prompt && !matches_status {
                        return None;
                    }
                }

                Some((task_id.clone(), latest))
            })
            .collect())
    }

    async fn get_status_log(&self, id: &TaskId) -> Result<TaskStatusLog, StoreError> {
        self.tasks
            .get(id)
            .and_then(|entry| super::task_status_log_from_versions(entry.value()))
            .ok_or_else(|| StoreError::TaskNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        ids: &[TaskId],
    ) -> Result<HashMap<TaskId, TaskStatusLog>, StoreError> {
        let mut result = HashMap::new();
        for id in ids {
            if let Some(entry) = self.tasks.get(id) {
                if let Some(log) = super::task_status_log_from_versions(entry.value()) {
                    result.insert(id.clone(), log);
                }
            }
        }
        Ok(result)
    }

    async fn count_distinct_issues(&self) -> Result<u64, StoreError> {
        Ok(self.issues.len() as u64)
    }

    async fn count_distinct_patches(&self) -> Result<u64, StoreError> {
        Ok(self.patches.len() as u64)
    }

    async fn count_distinct_documents(&self) -> Result<u64, StoreError> {
        Ok(self.documents.len() as u64)
    }

    async fn count_distinct_tasks(&self) -> Result<u64, StoreError> {
        Ok(self.tasks.len() as u64)
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

    async fn get_user(
        &self,
        username: &Username,
        include_deleted: bool,
    ) -> Result<Versioned<User>, StoreError> {
        let versioned = self
            .users
            .get(username)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::UserNotFound(username.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::UserNotFound(username.clone()));
        }
        Ok(versioned)
    }

    async fn list_users(
        &self,
        query: &SearchUsersQuery,
    ) -> Result<Vec<(Username, Versioned<User>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        let mut users = Vec::new();
        for entry in self.users.iter() {
            if let Some(versioned) = Self::latest_versioned(entry.value()) {
                // Filter deleted users by default
                if !include_deleted && versioned.item.deleted {
                    continue;
                }

                // Apply search filter
                if let Some(ref term) = search_term {
                    let username_lower = entry.key().as_str().to_lowercase();
                    if !username_lower.contains(term) {
                        continue;
                    }
                }

                users.push((entry.key().clone(), versioned));
            }
        }

        users.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
        Ok(users)
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn add_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError> {
        // Check if exists and if deleted
        if let Some(entry) = self.repositories.get(&name) {
            if let Some(latest) = Self::latest_versioned(entry.value()) {
                if latest.item.deleted {
                    // Re-create over deleted: use caller's config as-is
                    drop(entry);
                    return self.update_repository(name, config, actor).await;
                }
                return Err(StoreError::RepositoryAlreadyExists(name));
            }
        }

        self.repositories
            .insert(name, vec![Self::versioned_now_with_actor(config, 1, actor)]);
        Ok(())
    }

    async fn update_repository(
        &self,
        name: RepoName,
        config: Repository,
        actor: &ActorRef,
    ) -> Result<(), StoreError> {
        let mut versions = self
            .repositories
            .get_mut(&name)
            .ok_or_else(|| StoreError::RepositoryNotFound(name.clone()))?;
        let next_version = Self::next_version(&versions);

        versions.push(Self::versioned_now_with_actor(config, next_version, actor));
        Ok(())
    }

    async fn delete_repository(&self, name: &RepoName, actor: &ActorRef) -> Result<(), StoreError> {
        // Use include_deleted: true since we need to access the repository to mark it as deleted
        let current = self.get_repository(name, true).await?;
        let mut repo = current.item;
        repo.deleted = true;
        self.update_repository(name.clone(), repo, actor).await
    }

    async fn add_issue(
        &self,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<(IssueId, VersionNumber), StoreError> {
        let count = self.issues.len() as u64;
        let mut len = metis_common::ids::compute_random_len(count);
        let id = loop {
            let candidate =
                IssueId::generate(len).map_err(|e| StoreError::Internal(e.to_string()))?;
            if !self.issues.contains_key(&candidate) {
                break candidate;
            }
            len = (len + 1).min(metis_common::ids::MAX_RANDOM_LEN);
        };
        let new_dependencies = issue.dependencies.clone();
        let new_patches = issue.patches.clone();

        self.validate_dependencies(&new_dependencies)?;
        self.issues.insert(
            id.clone(),
            vec![Self::versioned_now_with_actor(issue, 1, actor)],
        );

        if !new_dependencies.is_empty() {
            self.apply_issue_dependency_delta(&id, &[], &new_dependencies);
        }
        if !new_patches.is_empty() {
            self.apply_issue_patch_delta(&id, &[], &new_patches);
        }
        Ok((id, 1))
    }

    async fn update_issue(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
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
        let next_version = if let Some(mut versions) = self.issues.get_mut(id) {
            let next_version = Self::next_version(&versions);
            versions.push(Self::versioned_now_with_actor(issue, next_version, actor));
            next_version
        } else {
            return Err(StoreError::IssueNotFound(id.clone()));
        };

        if !previous_dependencies.is_empty() || !updated_dependencies.is_empty() {
            self.apply_issue_dependency_delta(id, &previous_dependencies, &updated_dependencies);
        }
        if !previous_patches.is_empty() || !updated_patches.is_empty() {
            self.apply_issue_patch_delta(id, &previous_patches, &updated_patches);
        }
        Ok(next_version)
    }

    async fn delete_issue(
        &self,
        id: &IssueId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_issue(id, true).await?;
        let mut issue = current.item;
        issue.deleted = true;
        self.update_issue(id, issue, actor).await
    }

    async fn add_patch(
        &self,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<(PatchId, VersionNumber), StoreError> {
        let count = self.patches.len() as u64;
        let mut len = metis_common::ids::compute_random_len(count);
        let id = loop {
            let candidate =
                PatchId::generate(len).map_err(|e| StoreError::Internal(e.to_string()))?;
            if !self.patches.contains_key(&candidate) {
                break candidate;
            }
            len = (len + 1).min(metis_common::ids::MAX_RANDOM_LEN);
        };
        self.patches.insert(
            id.clone(),
            vec![Self::versioned_now_with_actor(patch, 1, actor)],
        );
        Ok((id, 1))
    }

    async fn update_patch(
        &self,
        id: &PatchId,
        patch: Patch,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut versions = self
            .patches
            .get_mut(id)
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now_with_actor(patch, next_version, actor));
        Ok(next_version)
    }

    async fn delete_patch(
        &self,
        id: &PatchId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_patch(id, true).await?;
        let mut patch = current.item;
        patch.deleted = true;
        self.update_patch(id, patch, actor).await
    }

    async fn add_document(
        &self,
        document: Document,
        actor: &ActorRef,
    ) -> Result<(DocumentId, VersionNumber), StoreError> {
        let count = self.documents.len() as u64;
        let mut len = metis_common::ids::compute_random_len(count);
        let id = loop {
            let candidate =
                DocumentId::generate(len).map_err(|e| StoreError::Internal(e.to_string()))?;
            if !self.documents.contains_key(&candidate) {
                break candidate;
            }
            len = (len + 1).min(metis_common::ids::MAX_RANDOM_LEN);
        };
        let path = document.path.clone();
        self.documents.insert(
            id.clone(),
            vec![Self::versioned_now_with_actor(document, 1, actor)],
        );
        self.index_document_path(&id, path.as_deref());
        Ok((id, 1))
    }

    async fn update_document(
        &self,
        id: &DocumentId,
        document: Document,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut versions = self
            .documents
            .get_mut(id)
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        let previous_path = versions
            .last()
            .and_then(|version| version.item.path.clone());
        let new_path = document.path.clone();
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now_with_actor(
            document,
            next_version,
            actor,
        ));

        if previous_path != new_path {
            self.remove_document_path(id, previous_path.as_deref());
            self.index_document_path(id, new_path.as_deref());
        }

        Ok(next_version)
    }

    async fn delete_document(
        &self,
        id: &DocumentId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_document(id, true).await?;
        let mut document = current.item;
        document.deleted = true;
        self.update_document(id, document, actor).await
    }

    async fn add_task(
        &self,
        task: Task,
        creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(TaskId, VersionNumber), StoreError> {
        let count = self.tasks.len() as u64;
        let mut len = metis_common::ids::compute_random_len(count);
        let id = loop {
            let candidate =
                TaskId::generate(len).map_err(|e| StoreError::Internal(e.to_string()))?;
            if !self.tasks.contains_key(&candidate) {
                break candidate;
            }
            len = (len + 1).min(metis_common::ids::MAX_RANDOM_LEN);
        };
        let spawned_from = task.spawned_from.clone();

        self.tasks.insert(
            id.clone(),
            vec![Self::versioned_at_with_actor(task, 1, creation_time, actor)],
        );

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, id.clone());
        }

        Ok((id, 1))
    }

    async fn update_task(
        &self,
        metis_id: &TaskId,
        task: Task,
        actor: &ActorRef,
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
                let versioned = Self::versioned_now_with_actor(task.clone(), next_version, actor);
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

    async fn delete_task(
        &self,
        id: &TaskId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_task(id, true).await?;
        let mut task = current.item;
        task.deleted = true;
        let versioned = self.update_task(id, task, actor).await?;
        Ok(versioned.version)
    }

    async fn add_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
        let name = actor.name();
        if self.actors.contains_key(&name) {
            return Err(StoreError::ActorAlreadyExists(name));
        }

        self.actors.insert(
            name,
            vec![Self::versioned_now_with_actor(actor, 1, acting_as)],
        );
        Ok(())
    }

    async fn update_actor(&self, actor: Actor, acting_as: &ActorRef) -> Result<(), StoreError> {
        let name = actor.name();
        let mut versions = self
            .actors
            .get_mut(&name)
            .ok_or_else(|| StoreError::ActorNotFound(name.clone()))?;
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now_with_actor(
            actor,
            next_version,
            acting_as,
        ));
        Ok(())
    }

    async fn add_user(&self, user: User, actor: &ActorRef) -> Result<(), StoreError> {
        if let Some(mut versions) = self.users.get_mut(&user.username) {
            // Check if the user is deleted
            if let Some(latest) = Self::latest_versioned(versions.value()) {
                if latest.item.deleted {
                    // Allow re-creation with the provided user
                    let next_version = Self::next_version(&versions);
                    let versioned = Self::versioned_now_with_actor(user, next_version, actor);
                    versions.push(versioned);
                    return Ok(());
                }
            }
            return Err(StoreError::UserAlreadyExists(user.username.clone()));
        }

        self.users.insert(
            user.username.clone(),
            vec![Self::versioned_now_with_actor(user, 1, actor)],
        );
        Ok(())
    }

    async fn update_user(
        &self,
        user: User,
        actor: &ActorRef,
    ) -> Result<Versioned<User>, StoreError> {
        let mut versions = self
            .users
            .get_mut(&user.username)
            .ok_or_else(|| StoreError::UserNotFound(user.username.clone()))?;
        let next_version = Self::next_version(&versions);
        let versioned = Self::versioned_now_with_actor(user, next_version, actor);
        versions.push(versioned.clone());
        Ok(versioned)
    }

    async fn delete_user(&self, username: &Username, actor: &ActorRef) -> Result<(), StoreError> {
        let current = self.get_user(username, true).await?;
        let mut user = current.item;
        user.deleted = true;
        self.update_user(user, actor).await?;
        Ok(())
    }
}

/// Helper function to check if an issue matches the provided filter criteria.
fn issue_matches(
    issue_type_filter: Option<IssueType>,
    status_filter: Option<IssueStatus>,
    search_term: Option<&str>,
    assignee_filter: Option<&str>,
    issue_id: &IssueId,
    issue: &Issue,
) -> bool {
    if let Some(issue_type) = issue_type_filter {
        if issue.issue_type != issue_type {
            return false;
        }
    }

    if let Some(status) = status_filter {
        if issue.status != status {
            return false;
        }
    }

    if let Some(expected_assignee) = assignee_filter {
        match issue.assignee.as_ref() {
            Some(current) if current.eq_ignore_ascii_case(expected_assignee) => {}
            _ => return false,
        }
    }

    if let Some(term) = search_term {
        let lower_id = issue_id.to_string().to_lowercase();
        if lower_id.contains(term) {
            return true;
        }

        return issue.description.to_lowercase().contains(term)
            || issue.progress.to_lowercase().contains(term)
            || issue.issue_type.as_str() == term
            || issue.status.as_str() == term
            || issue.creator.as_ref().to_lowercase().contains(term)
            || issue
                .assignee
                .as_deref()
                .map(|value| value.to_lowercase().contains(term))
                .unwrap_or(false);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{
            actors::{Actor, ActorId, ActorRef},
            issues::{
                Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus,
                IssueType,
            },
            jobs::BundleSpec,
            patches::{GithubPr, Patch, PatchStatus},
            task_status::Event,
            users::{User, Username},
        },
        store::TaskError,
        test_utils::test_state_with_store,
    };
    use chrono::{Duration, Utc};
    use metis_common::{
        IssueId, RepoName, TaskId, VersionNumber, Versioned,
        repositories::{Repository, SearchRepositoriesQuery},
    };
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
            Username::from("test-creator"),
            Some("metis-worker:latest".to_string()),
            None,
            HashMap::new(),
            None,
            None,
            None,
            Status::Created,
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
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        )
    }

    fn sample_document(path: Option<&str>, created_by: Option<TaskId>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: path.map(|p| p.parse().unwrap()),
            created_by,
            deleted: false,
        }
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
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item, config);
        assert_eq!(fetched.version, 1);

        let mut updated = config.clone();
        updated.default_branch = Some("develop".to_string());
        store
            .update_repository(name.clone(), updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, name);
        assert_versioned(&list[0].1, &updated, 2);

        let fetched_again = store.get_repository(&name, false).await.unwrap();
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
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = config.clone();
        updated.default_branch = Some("release".to_string());
        store
            .update_repository(name.clone(), updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
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
            .add_repository(name.clone(), sample_repository_config(), &ActorRef::test())
            .await
            .unwrap();

        let err = store
            .add_repository(name.clone(), sample_repository_config(), &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::RepositoryAlreadyExists(existing) if existing == name
        ));

        let missing_name = RepoName::from_str("dourolabs/other").unwrap();
        let err = store
            .update_repository(
                missing_name.clone(),
                sample_repository_config(),
                &ActorRef::test(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(existing) if existing == missing_name
        ));
    }

    #[tokio::test]
    async fn delete_repository_soft_deletes() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Delete the repository
        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        // With include_deleted=false, deleted repository returns RepositoryNotFound
        let err = store.get_repository(&name, false).await.unwrap_err();
        assert!(matches!(err, StoreError::RepositoryNotFound(_)));

        // With include_deleted=true, repository is still retrievable
        let fetched = store.get_repository(&name, true).await.unwrap();
        assert!(fetched.item.deleted);
        assert_eq!(fetched.version, 2);

        // By default, list_repositories excludes deleted
        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert!(list.is_empty());

        // With include_deleted=true, deleted repos are shown
        let query = SearchRepositoriesQuery::new(Some(true));
        let list = store.list_repositories(&query).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_recreates_over_soft_deleted_repo() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        // Create and delete
        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        // Re-create with deleted=false (caller controls the deleted field)
        let mut new_config = config.clone();
        new_config.default_branch = Some("develop".to_string());
        new_config.deleted = false;
        store
            .add_repository(name.clone(), new_config.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Repository should be active again
        let fetched = store.get_repository(&name, false).await.unwrap();
        assert!(!fetched.item.deleted);
        assert_eq!(fetched.item.default_branch, Some("develop".to_string()));
        assert_eq!(fetched.version, 3);

        // List_repositories should include it by default
        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert!(!list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_respects_caller_deleted_field() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/metis").unwrap();
        let config = sample_repository_config();

        // Create and delete
        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap();

        // Re-create with deleted=true (caller wants to keep it deleted)
        let mut new_config = config.clone();
        new_config.default_branch = Some("develop".to_string());
        new_config.deleted = true;
        store
            .add_repository(name.clone(), new_config.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Repository should still be deleted (caller's choice)
        // Use include_deleted=true to retrieve the deleted repository
        let fetched = store.get_repository(&name, true).await.unwrap();
        assert!(fetched.item.deleted);
        assert_eq!(fetched.item.default_branch, Some("develop".to_string()));
        assert_eq!(fetched.version, 3);

        // List_repositories should not include it by default
        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn delete_repository_not_found_error() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/nonexistent").unwrap();

        let err = store
            .delete_repository(&name, &ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::RepositoryNotFound(n) if n == name
        ));
    }

    #[tokio::test]
    async fn add_issue_rejects_missing_dependencies() {
        let store = MemoryStore::new();
        let missing_dependency = IssueId::new();

        let err = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    missing_dependency.clone(),
                )]),
                &ActorRef::test(),
            )
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

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let missing_dependency = IssueId::new();

        let err = store
            .update_issue(
                &issue_id,
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    missing_dependency.clone(),
                )]),
                &ActorRef::test(),
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

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = sample_issue(vec![]);
        updated.description = "updated details".to_string();
        store
            .update_issue(&issue_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);

        let versions = store.issues.get(&issue_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn issue_versions_return_ordered_entries() {
        let store = MemoryStore::new();

        let mut issue = sample_issue(vec![]);
        issue.description = "v1".to_string();
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

        let mut v2 = sample_issue(vec![]);
        v2.description = "v2".to_string();
        store
            .update_issue(&issue_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let mut v3 = sample_issue(vec![]);
        v3.description = "v3".to_string();
        store
            .update_issue(&issue_id, v3, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_issue_versions(&issue_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2, 3]);
        assert_eq!(versions[0].item.description, "v1");
        assert_eq!(versions[2].item.description, "v3");
    }

    #[tokio::test]
    async fn issue_versions_persist_actor() {
        let store = MemoryStore::new();

        let user_actor = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice").into()),
        };
        let system_actor = ActorRef::System {
            worker_name: "scheduler".into(),
            on_behalf_of: None,
        };

        let mut issue = sample_issue(vec![]);
        issue.description = "created by user".to_string();
        let (issue_id, _) = store.add_issue(issue, &user_actor).await.unwrap();

        let mut v2 = sample_issue(vec![]);
        v2.description = "updated by system".to_string();
        store
            .update_issue(&issue_id, v2, &system_actor)
            .await
            .unwrap();

        let versions = store.get_issue_versions(&issue_id).await.unwrap();
        assert_eq!(versions.len(), 2);

        // Version 1 should have the user actor
        assert_eq!(versions[0].actor.as_ref().unwrap(), &user_actor);

        // Version 2 should have the system actor
        assert_eq!(versions[1].actor.as_ref().unwrap(), &system_actor);

        // Latest version should also have actor set
        let latest = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(latest.actor.as_ref().unwrap(), &system_actor);
    }

    #[tokio::test]
    async fn add_and_get_patch_assigns_id() {
        let store = MemoryStore::new();

        let patch = sample_patch();
        let (id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&id, false).await.unwrap();
        assert_eq!(fetched.item, patch);
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn update_patch_overwrites_existing_value() {
        let store = MemoryStore::new();

        let (id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        let updated = Patch::new(
            "new title".to_string(),
            "updated patch".to_string(),
            dummy_diff(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
        );

        store
            .update_patch(&id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_patch(&id, false).await.unwrap();
        assert_eq!(fetched.item, updated);
        assert_eq!(fetched.version, 2);

        let versions = store.patches.get(&id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn patch_versions_return_ordered_entries() {
        let store = MemoryStore::new();

        let mut patch = sample_patch();
        patch.title = "v1".to_string();
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let mut v2 = sample_patch();
        v2.title = "v2".to_string();
        store
            .update_patch(&patch_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_patch_versions(&patch_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2]);
        assert_eq!(versions[0].item.title, "v1");
        assert_eq!(versions[1].item.title, "v2");
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
                    Username::from("test-creator"),
                    Vec::new(),
                    RepoName::from_str("dourolabs/sample").unwrap(),
                    None,
                ),
                &ActorRef::test(),
            )
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::PatchNotFound(id) if id == missing));
    }

    #[tokio::test]
    async fn issue_dependency_indexes_populated_on_create() {
        let store = MemoryStore::new();

        let (parent_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (blocker_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let (child_id, _) = store
            .add_issue(
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone()),
                    IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone()),
                ]),
                &ActorRef::test(),
            )
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

        let (original_parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (new_parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (original_blocker, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (new_blocker, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let (issue_id, _) = store
            .add_issue(
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, original_parent.clone()),
                    IssueDependency::new(IssueDependencyType::BlockedOn, original_blocker.clone()),
                ]),
                &ActorRef::test(),
            )
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
                &ActorRef::test(),
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
            .update_issue(&issue_id, sample_issue(vec![]), &ActorRef::test())
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

        let (parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (child, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (_grandchild, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    child.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{parent}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([child]));
    }

    #[tokio::test]
    async fn graph_filter_returns_transitive_children() {
        let store = MemoryStore::new();

        let (parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (child, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (grandchild, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    child.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("**:child-of:{parent}").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([child, grandchild]));
    }

    #[tokio::test]
    async fn graph_filter_returns_ancestors_for_right_wildcards() {
        let store = MemoryStore::new();

        let (root, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (child, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    root.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (grandchild, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    child.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("{grandchild}:child-of:**").parse().unwrap();
        let matches = store.search_issue_graph(&[filter]).await.unwrap();

        assert_eq!(matches, HashSet::from([root, child]));
    }

    #[tokio::test]
    async fn graph_filters_intersect_multiple_constraints() {
        let store = MemoryStore::new();

        let (parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (blocker, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (other_parent, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (other_blocker, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let (matching_issue, _) = store
            .add_issue(
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, parent.clone()),
                    IssueDependency::new(IssueDependencyType::BlockedOn, blocker.clone()),
                ]),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let (non_matching_child, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    parent.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (non_matching_blocked, _) = store
            .add_issue(
                sample_issue(vec![IssueDependency::new(
                    IssueDependencyType::BlockedOn,
                    blocker.clone(),
                )]),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        let (unrelated_issue, _) = store
            .add_issue(
                sample_issue(vec![
                    IssueDependency::new(IssueDependencyType::ChildOf, other_parent),
                    IssueDependency::new(IssueDependencyType::BlockedOn, other_blocker),
                ]),
                &ActorRef::test(),
            )
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

        store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let filter: IssueGraphFilter = format!("*:child-of:{missing}").parse().unwrap();
        let result = store.search_issue_graph(&[filter]).await;

        assert!(matches!(result, Err(StoreError::IssueNotFound(id)) if id == missing));
    }

    #[tokio::test]
    async fn patch_issue_indexes_updated_on_issue_changes() {
        let store = MemoryStore::new();
        let (patch_a, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();
        let (patch_b, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();

        let mut issue = sample_issue(vec![]);
        issue.patches = vec![patch_a.clone()];
        let (issue_id, _) = store.add_issue(issue, &ActorRef::test()).await.unwrap();

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
        store
            .update_issue(&issue_id, updated_issue, &ActorRef::test())
            .await
            .unwrap();

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
    async fn documents_round_trip() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(
                sample_document(Some("docs/guides/intro.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = store.get_document(&doc_id, false).await.unwrap();
        assert_eq!(fetched.item.title, "Doc");
        assert_eq!(fetched.version, 1);

        let mut updated = fetched.item.clone();
        updated.body_markdown = "Updated body".to_string();
        store
            .update_document(&doc_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_document_versions(&doc_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2]);
        assert_eq!(versions[1].item.body_markdown, "Updated body");

        let documents = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].0, doc_id);

        let by_path = store.get_documents_by_path("/docs/").await.unwrap();
        assert_eq!(by_path.len(), 1);
        assert_eq!(by_path[0].0, doc_id);
    }

    #[tokio::test]
    async fn document_filters_apply_query() {
        let store = MemoryStore::new();
        let task_id = TaskId::new();
        let other_task = TaskId::new();

        let (first, _) = store
            .add_document(
                sample_document(Some("docs/howto.md"), Some(task_id.clone())),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("notes/todo.md"), Some(other_task.clone())),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let query = SearchDocumentsQuery::new(
            Some("how".to_string()),
            Some("/docs/".to_string()),
            None,
            Some(task_id.clone()),
            None,
        );

        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, first);

        let created_by_filtered = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                None,
                None,
                Some(other_task),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(created_by_filtered.len(), 1);
    }

    #[tokio::test]
    async fn document_path_index_updates_on_change() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(
                sample_document(Some("docs/old.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let mut updated = store.get_document(&doc_id, false).await.unwrap().item;
        updated.path = Some("docs/new.md".parse().unwrap());
        store
            .update_document(&doc_id, updated, &ActorRef::test())
            .await
            .unwrap();

        assert!(
            store
                .get_documents_by_path("/docs/old")
                .await
                .unwrap()
                .is_empty()
        );
        let matches = store.get_documents_by_path("/docs/new").await.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, doc_id);
    }

    #[tokio::test]
    async fn add_and_retrieve_tasks() {
        let store = MemoryStore::new();

        let task = spawn_task();
        let (task_id, _) = store
            .add_task(task.clone(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_task(&task_id, false).await.unwrap();
        assert_versioned(&fetched, &task, 1);
        assert_eq!(
            store.get_task(&task_id, false).await.unwrap().item.status,
            Status::Created
        );

        let tasks: HashSet<_> = store
            .list_tasks(&SearchJobsQuery::default())
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
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = spawn_task();
        updated.prompt = "v2".to_string();
        store
            .update_task(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_task(&task_id, false).await.unwrap();
        assert_versioned(&fetched, &updated, 2);

        let versions = store.tasks.get(&task_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2]);
    }

    #[tokio::test]
    async fn task_versions_return_ordered_entries() {
        let store = MemoryStore::new();

        let mut task = spawn_task();
        task.prompt = "v1".to_string();
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut v2 = spawn_task();
        v2.prompt = "v2".to_string();
        store
            .update_task(&task_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_task_versions(&task_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2]);
        assert_eq!(versions[0].item.prompt, "v1");
        assert_eq!(versions[1].item.prompt, "v2");
    }

    #[tokio::test]
    async fn task_versions_increment_on_transitions() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;
        let (task_id, _) = store
            .add_task(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&task_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(&task_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap();

        let versions = store.tasks.get(&task_id).unwrap();
        assert_eq!(version_numbers(versions.value()), vec![1, 2, 3, 4]);
        assert_eq!(
            store.get_task(&task_id, false).await.unwrap().item.status,
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
        let (task_id, _) = store
            .add_task(task.clone(), created_at, &ActorRef::test())
            .await
            .unwrap();

        let mut updated = task.clone();
        updated.prompt = "v2".to_string();
        store
            .update_task(&task_id, updated, &ActorRef::test())
            .await
            .unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.events.len(), 1);
        assert_eq!(log.creation_time(), Some(created_at));
        assert_eq!(log.current_status(), Status::Created);

        state
            .transition_task_to_pending(&task_id, ActorRef::test())
            .await
            .unwrap();
        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.current_status(), Status::Pending);

        state
            .transition_task_to_running(&task_id, ActorRef::test())
            .await
            .unwrap();
        let log = store.get_status_log(&task_id).await.unwrap();
        assert!(matches!(log.events.last(), Some(Event::Started { .. })));

        let mut running = store.get_task(&task_id, false).await.unwrap().item;
        running.prompt = "v3".to_string();
        store
            .update_task(&task_id, running, &ActorRef::test())
            .await
            .unwrap();

        let log = store.get_status_log(&task_id).await.unwrap();
        assert_eq!(log.events.len(), 3);

        state
            .transition_task_to_completion(
                &task_id,
                Ok(()),
                Some("done".to_string()),
                ActorRef::test(),
            )
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

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (other_issue, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut pending_task = spawn_task();
        pending_task.spawned_from = Some(issue_id.clone());
        let (pending_id, _) = store
            .add_task(pending_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut running_task = spawn_task();
        running_task.spawned_from = Some(issue_id.clone());
        let (running_id, _) = store
            .add_task(running_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&running_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&running_id, ActorRef::test())
            .await
            .unwrap();

        let mut completed_task = spawn_task();
        completed_task.spawned_from = Some(issue_id.clone());
        let (completed_id, _) = store
            .add_task(completed_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&completed_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&completed_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(&completed_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap();

        let mut unrelated_task = spawn_task();
        unrelated_task.spawned_from = Some(other_issue.clone());
        let (unrelated_id, _) = store
            .add_task(unrelated_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

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
    async fn task_starts_as_created() {
        let store = MemoryStore::new();

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Created
        );
    }

    #[tokio::test]
    async fn transition_task_to_pending_from_created() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Created
        );

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Pending
        );
    }

    #[tokio::test]
    async fn transition_task_to_running_from_pending() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Running
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Created
        );

        // First mark as pending then running
        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Running
        );

        // Then mark as complete
        state
            .transition_task_to_completion(&root_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Complete
        );
    }

    #[tokio::test]
    async fn transition_task_to_failure_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Created
        );

        // First mark as pending then running
        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
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
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_pending_fails() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Trying to mark as complete from pending should fail
        let err = state
            .transition_task_to_completion(&root_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn transition_task_to_failure_from_pending_succeeds() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Marking as failed from pending should succeed
        state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "test".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn transition_task_idempotent_complete_to_complete() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(
                &root_id,
                Ok(()),
                Some("first message".to_string()),
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Complete
        );

        // Second Complete transition should succeed idempotently
        let result = state
            .transition_task_to_completion(
                &root_id,
                Ok(()),
                Some("second message".to_string()),
                ActorRef::test(),
            )
            .await;
        assert!(result.is_ok());

        // Original last_message should be preserved
        let task = store.get_task(&root_id, false).await.unwrap();
        assert_eq!(task.item.status, Status::Complete);
        assert_eq!(task.item.last_message.as_deref(), Some("first message"));
    }

    #[tokio::test]
    async fn transition_task_idempotent_failed_to_failed() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "first failure".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.get_task(&root_id, false).await.unwrap().item.status,
            Status::Failed
        );

        // Second Failed transition should succeed idempotently
        let result = state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "second failure".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await;
        assert!(result.is_ok());

        // Original error should be preserved
        let task = store.get_task(&root_id, false).await.unwrap();
        assert_eq!(task.item.status, Status::Failed);
        assert!(matches!(
            task.item.error,
            Some(TaskError::JobEngineError { reason }) if reason == "first failure"
        ));
    }

    #[tokio::test]
    async fn transition_task_conflict_complete_to_failed() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(&root_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap();

        // Trying to transition Complete -> Failed should fail
        let err = state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "conflict".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn transition_task_conflict_failed_to_complete() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_task(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&root_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(
                &root_id,
                Err(TaskError::JobEngineError {
                    reason: "failure".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Trying to transition Failed -> Complete should fail
        let err = state
            .transition_task_to_completion(&root_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidStatusTransition));
    }

    #[tokio::test]
    async fn update_user_overwrites_existing_value() {
        let store = MemoryStore::new();
        let username = Username::from("alice");

        store
            .add_user(
                User {
                    username: username.clone(),
                    github_user_id: 101,
                    github_token: "old-token".to_string(),
                    github_refresh_token: "old-refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let updated = store
            .update_user(
                User {
                    username: username.clone(),
                    github_user_id: 202,
                    github_token: "new-token".to_string(),
                    github_refresh_token: "new-refresh".to_string(),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        assert_eq!(updated.item.github_token, "new-token");
        assert_eq!(updated.item.github_user_id, 202);
        assert_eq!(updated.item.github_refresh_token, "new-refresh");
        assert_eq!(updated.version, 2);

        let user = store.get_user(&username, false).await.unwrap();
        assert_eq!(user.item.github_token, "new-token");
        assert_eq!(user.item.github_user_id, 202);
        assert_eq!(user.item.github_refresh_token, "new-refresh");
        assert_eq!(user.version, 2);
    }

    #[tokio::test]
    async fn get_user_filters_deleted_users() {
        let store = MemoryStore::new();
        let username = Username::from("alice");
        let user = User {
            username: username.clone(),
            github_user_id: 101,
            github_token: "token".to_string(),
            github_refresh_token: "refresh".to_string(),
            deleted: false,
        };
        store.add_user(user, &ActorRef::test()).await.unwrap();

        // User is accessible when not deleted
        let fetched = store.get_user(&username, false).await.unwrap();
        assert_eq!(fetched.item.username, username);

        // Delete the user
        store
            .delete_user(&username, &ActorRef::test())
            .await
            .unwrap();

        // User is not found with include_deleted=false
        let err = store.get_user(&username, false).await.unwrap_err();
        assert!(matches!(err, StoreError::UserNotFound(_)));

        // User is still accessible with include_deleted=true
        let fetched = store.get_user(&username, true).await.unwrap();
        assert_eq!(fetched.item.username, username);
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn add_and_get_actor_by_name() {
        let store = MemoryStore::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Username(Username::from("ada").into()),
            creator: Username::from("ada"),
        };

        let name = actor.name();
        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();

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
            actor_id: ActorId::Task(TaskId::new()),
            creator: Username::from("creator"),
        };
        let name = actor.name();

        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();
        let err = store.add_actor(actor, &ActorRef::test()).await.unwrap_err();

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
            actor_id: ActorId::Task(task_id),
            creator: Username::from("creator"),
        };
        let mut updated = actor.clone();
        updated.auth_token_hash = "new-hash".to_string();

        store
            .add_actor(actor.clone(), &ActorRef::test())
            .await
            .unwrap();
        store
            .update_actor(updated.clone(), &ActorRef::test())
            .await
            .unwrap();

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
            actor_id: ActorId::Username(Username::from("ada").into()),
            creator: Username::from("ada"),
        };

        let err = store
            .update_actor(actor, &ActorRef::test())
            .await
            .unwrap_err();

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

    #[tokio::test]
    async fn document_path_is_exact_filters_correctly() {
        let store = MemoryStore::new();

        let (exact_doc, _) = store
            .add_document(
                sample_document(Some("docs/guide.md"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("docs/guide.md.bak"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("docs/guide.md/extra"), None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // Prefix matching returns all 3
        let by_prefix = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                Some("/docs/guide.md".to_string()),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(by_prefix.len(), 3);

        // Exact matching returns only the exact path
        let by_exact = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                Some("/docs/guide.md".to_string()),
                Some(true),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(by_exact.len(), 1);
        assert_eq!(by_exact[0].0, exact_doc);

        // path_is_exact=false uses prefix matching
        let by_prefix_explicit = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                Some("/docs/guide.md".to_string()),
                Some(false),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(by_prefix_explicit.len(), 3);
    }

    #[tokio::test]
    async fn delete_issue_sets_deleted_flag_and_filters_from_list() {
        let store = MemoryStore::new();
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Issue should be visible in list initially
        let issues = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert!(!issues[0].1.item.deleted);

        // Delete the issue
        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        // Deleted issue should not appear in default list
        let issues = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert!(issues.is_empty());

        // Deleted issue should appear with include_deleted=true
        let issues = store
            .list_issues(&SearchIssuesQuery::new(
                None,
                None,
                None,
                None,
                Vec::new(),
                Some(true),
            ))
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert!(issues[0].1.item.deleted);

        // get_issue with include_deleted: true should still return the deleted issue
        let issue = store.get_issue(&issue_id, true).await.unwrap();
        assert!(issue.item.deleted);

        // get_issue with include_deleted: false should return IssueNotFound
        let err = store.get_issue(&issue_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));
    }

    #[tokio::test]
    async fn get_issue_filters_deleted_issues() {
        let store = MemoryStore::new();
        let issue = sample_issue(vec![]);
        let (issue_id, _) = store
            .add_issue(issue.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Issue is accessible when not deleted
        let fetched = store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.description, issue.description);

        // Delete the issue
        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();

        // Issue is not found with include_deleted=false
        let err = store.get_issue(&issue_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::IssueNotFound(_)));

        // Issue is still accessible with include_deleted=true
        let fetched = store.get_issue(&issue_id, true).await.unwrap();
        assert_eq!(fetched.item.description, issue.description);
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn delete_patch_sets_deleted_flag_and_filters_from_list() {
        let store = MemoryStore::new();
        let (patch_id, _) = store
            .add_patch(sample_patch(), &ActorRef::test())
            .await
            .unwrap();

        // Patch should be visible in list initially
        let patches = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(!patches[0].1.item.deleted);

        // Delete the patch
        store
            .delete_patch(&patch_id, &ActorRef::test())
            .await
            .unwrap();

        // Deleted patch should not appear in default list
        let patches = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap();
        assert!(patches.is_empty());

        // Deleted patch should appear with include_deleted=true
        let patches = store
            .list_patches(&SearchPatchesQuery::new(None, Some(true), vec![], None))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].1.item.deleted);

        // get_patch with include_deleted=true should still return the deleted patch
        let patch = store.get_patch(&patch_id, true).await.unwrap();
        assert!(patch.item.deleted);
    }

    #[tokio::test]
    async fn get_patch_filters_deleted_patches() {
        let store = MemoryStore::new();
        let patch = sample_patch();
        let (patch_id, _) = store
            .add_patch(patch.clone(), &ActorRef::test())
            .await
            .unwrap();

        // Patch is accessible when not deleted
        let fetched = store.get_patch(&patch_id, false).await.unwrap();
        assert_eq!(fetched.item.title, patch.title);

        // Delete the patch
        store
            .delete_patch(&patch_id, &ActorRef::test())
            .await
            .unwrap();

        // Patch is not found with include_deleted=false
        let err = store.get_patch(&patch_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::PatchNotFound(_)));

        // Patch is still accessible with include_deleted=true
        let fetched = store.get_patch(&patch_id, true).await.unwrap();
        assert_eq!(fetched.item.title, patch.title);
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn list_patches_filters_by_search_term() {
        let store = MemoryStore::new();

        // Create patches with different titles and descriptions
        let mut patch1 = sample_patch();
        patch1.title = "first patch".to_string();
        patch1.description = "adds the login feature".to_string();
        let (patch1_id, _) = store.add_patch(patch1, &ActorRef::test()).await.unwrap();

        let mut patch2 = sample_patch();
        patch2.title = "second patch".to_string();
        patch2.description = "fixes authentication bug".to_string();
        let (patch2_id, _) = store.add_patch(patch2, &ActorRef::test()).await.unwrap();

        let mut patch3 = sample_patch();
        patch3.title = "third update".to_string();
        patch3.description = "refactors login module".to_string();
        store.add_patch(patch3, &ActorRef::test()).await.unwrap();

        // Search by title
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("first".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);

        // Search by description
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("authentication".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch2_id);

        // Search term matching multiple patches (login)
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("login".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 2);

        // Search by patch (matches all)
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("patch".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 2); // patch1 and patch2, patch3 has "update" in title

        // No matches
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("nonexistent".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert!(patches.is_empty());

        // Empty query returns all
        let patches = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap();
        assert_eq!(patches.len(), 3);

        // Whitespace-only query returns all
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("   ".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 3);
    }

    #[tokio::test]
    async fn list_patches_filters_by_github_fields() {
        let store = MemoryStore::new();

        let mut patch1 = sample_patch();
        patch1.github = Some(GithubPr::new(
            "orgxyz".to_string(),
            "repoabc".to_string(),
            123,
            Some("feature/login".to_string()),
            Some("main".to_string()),
            None,
            None,
        ));
        let (patch1_id, _) = store.add_patch(patch1, &ActorRef::test()).await.unwrap();

        let mut patch2 = sample_patch();
        patch2.github = Some(GithubPr::new(
            "acme".to_string(),
            "project".to_string(),
            456,
            Some("bugfix/auth".to_string()),
            Some("develop".to_string()),
            None,
            None,
        ));
        let (patch2_id, _) = store.add_patch(patch2, &ActorRef::test()).await.unwrap();

        // Search by github owner
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("orgxyz".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);

        // Search by github repo (patch1 has "repoabc")
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("repoabc".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);

        // Search by github repo (patch2 has "project")
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("project".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch2_id);

        // Search by PR number
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("123".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);

        // Search by head ref
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("feature/login".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);

        // Search by base ref
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("develop".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch2_id);
    }

    #[tokio::test]
    async fn list_patches_search_is_case_insensitive() {
        let store = MemoryStore::new();

        let mut patch = sample_patch();
        patch.title = "Important Feature".to_string();
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();

        // Search with different cases
        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("important".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch_id);

        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("IMPORTANT".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch_id);

        let patches = store
            .list_patches(&SearchPatchesQuery::new(
                Some("ImPoRtAnT".to_string()),
                None,
                vec![],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch_id);
    }

    #[tokio::test]
    async fn delete_document_sets_deleted_flag_and_filters_from_list() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(sample_document(Some("test.md"), None), &ActorRef::test())
            .await
            .unwrap();

        // Document should be visible in list initially
        let docs = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(docs.len(), 1);
        assert!(!docs[0].1.item.deleted);

        // Delete the document
        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        // Deleted document should not appear in default list
        let docs = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert!(docs.is_empty());

        // Deleted document should appear with include_deleted=true
        let docs = store
            .list_documents(&SearchDocumentsQuery::new(
                None,
                None,
                None,
                None,
                Some(true),
            ))
            .await
            .unwrap();
        assert_eq!(docs.len(), 1);
        assert!(docs[0].1.item.deleted);

        // get_document with include_deleted=false should return not found
        let result = store.get_document(&doc_id, false).await;
        assert!(matches!(result, Err(StoreError::DocumentNotFound(_))));

        // get_document with include_deleted=true should return the deleted document
        let doc = store.get_document(&doc_id, true).await.unwrap();
        assert!(doc.item.deleted);
    }

    #[tokio::test]
    async fn delete_task_sets_deleted_flag_and_filters_from_list() {
        let store = MemoryStore::new();
        let task = spawn_task();
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Task should be visible in list initially
        let tasks = store.list_tasks(&SearchJobsQuery::default()).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].1.item.deleted);

        // Delete the task
        store
            .delete_task(&task_id, &ActorRef::test())
            .await
            .unwrap();

        // Deleted task should not appear in default list
        let tasks = store.list_tasks(&SearchJobsQuery::default()).await.unwrap();
        assert!(tasks.is_empty());

        // Deleted task should appear with include_deleted=true
        let tasks = store
            .list_tasks(&SearchJobsQuery::new(None, None, Some(true), None))
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].1.item.deleted);

        // get_task with include_deleted=false should return TaskNotFound for deleted task
        let err = store.get_task(&task_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::TaskNotFound(id) if id == task_id));

        // get_task with include_deleted=true should return the deleted task
        let deleted_task = store.get_task(&task_id, true).await.unwrap();
        assert!(deleted_task.item.deleted);
    }

    #[tokio::test]
    async fn delete_nonexistent_issue_returns_error() {
        let store = MemoryStore::new();
        let missing_id = IssueId::new();

        let err = store
            .delete_issue(&missing_id, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::IssueNotFound(id) if id == missing_id));
    }

    #[tokio::test]
    async fn delete_nonexistent_patch_returns_error() {
        let store = MemoryStore::new();
        let missing_id = PatchId::new();

        let err = store
            .delete_patch(&missing_id, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::PatchNotFound(id) if id == missing_id));
    }

    #[tokio::test]
    async fn delete_nonexistent_document_returns_error() {
        let store = MemoryStore::new();
        let missing_id = DocumentId::new();

        let err = store
            .delete_document(&missing_id, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::DocumentNotFound(id) if id == missing_id));
    }

    #[tokio::test]
    async fn delete_nonexistent_task_returns_error() {
        let store = MemoryStore::new();
        let missing_id = TaskId::new();

        let err = store
            .delete_task(&missing_id, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::TaskNotFound(id) if id == missing_id));
    }

    #[tokio::test]
    async fn delete_increments_version() {
        let store = MemoryStore::new();
        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let version_before = store.get_issue(&issue_id, false).await.unwrap().version;
        store
            .delete_issue(&issue_id, &ActorRef::test())
            .await
            .unwrap();
        // After deletion, we need include_deleted: true to get the issue
        let version_after = store.get_issue(&issue_id, true).await.unwrap().version;

        assert_eq!(version_after, version_before + 1);
    }

    #[tokio::test]
    async fn list_issues_filters_by_issue_type() {
        let store = MemoryStore::new();

        // Create a task issue
        let (task_issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Create a bug issue
        let mut bug_issue = sample_issue(vec![]);
        bug_issue.issue_type = IssueType::Bug;
        let (bug_issue_id, _) = store.add_issue(bug_issue, &ActorRef::test()).await.unwrap();

        // Filter by task type
        let query = SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Task),
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, task_issue_id);

        // Filter by bug type
        let query = SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Bug),
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, bug_issue_id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_status() {
        let store = MemoryStore::new();

        // Create an open issue
        let (open_issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Create a closed issue
        let mut closed_issue = sample_issue(vec![]);
        closed_issue.status = IssueStatus::Closed;
        let (closed_issue_id, _) = store
            .add_issue(closed_issue, &ActorRef::test())
            .await
            .unwrap();

        // Filter by open status
        let query = SearchIssuesQuery::new(
            None,
            Some(metis_common::api::v1::issues::IssueStatus::Open),
            None,
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, open_issue_id);

        // Filter by closed status
        let query = SearchIssuesQuery::new(
            None,
            Some(metis_common::api::v1::issues::IssueStatus::Closed),
            None,
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, closed_issue_id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_assignee() {
        let store = MemoryStore::new();

        // Create an issue with assignee
        let mut assigned_issue = sample_issue(vec![]);
        assigned_issue.assignee = Some("alice".to_string());
        let (assigned_issue_id, _) = store
            .add_issue(assigned_issue, &ActorRef::test())
            .await
            .unwrap();

        // Create an issue without assignee
        store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Filter by assignee
        let query = SearchIssuesQuery::new(
            None,
            None,
            Some("alice".to_string()),
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, assigned_issue_id);

        // Case-insensitive assignee matching
        let query = SearchIssuesQuery::new(
            None,
            None,
            Some("ALICE".to_string()),
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, assigned_issue_id);
    }

    #[tokio::test]
    async fn list_issues_filters_by_search_term() {
        let store = MemoryStore::new();

        // Create issues with different descriptions
        let mut issue1 = sample_issue(vec![]);
        issue1.description = "fix the login bug".to_string();
        let (issue1_id, _) = store.add_issue(issue1, &ActorRef::test()).await.unwrap();

        let mut issue2 = sample_issue(vec![]);
        issue2.description = "add new feature".to_string();
        store.add_issue(issue2, &ActorRef::test()).await.unwrap();

        // Search for "login"
        let query = SearchIssuesQuery::new(
            None,
            None,
            None,
            Some("login".to_string()),
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, issue1_id);
    }

    #[tokio::test]
    async fn list_issues_search_term_matches_issue_id() {
        let store = MemoryStore::new();

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Search by issue ID prefix
        let id_prefix = issue_id.to_string()[..4].to_string();
        let query = SearchIssuesQuery::new(None, None, None, Some(id_prefix), Vec::new(), None);
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, issue_id);
    }

    #[tokio::test]
    async fn list_issues_combines_multiple_filters() {
        let store = MemoryStore::new();

        // Create a bug issue assigned to alice
        let mut bug_alice = sample_issue(vec![]);
        bug_alice.issue_type = IssueType::Bug;
        bug_alice.assignee = Some("alice".to_string());
        let (bug_alice_id, _) = store.add_issue(bug_alice, &ActorRef::test()).await.unwrap();

        // Create a bug issue assigned to bob
        let mut bug_bob = sample_issue(vec![]);
        bug_bob.issue_type = IssueType::Bug;
        bug_bob.assignee = Some("bob".to_string());
        store.add_issue(bug_bob, &ActorRef::test()).await.unwrap();

        // Create a task issue assigned to alice
        let mut task_alice = sample_issue(vec![]);
        task_alice.issue_type = IssueType::Task;
        task_alice.assignee = Some("alice".to_string());
        store
            .add_issue(task_alice, &ActorRef::test())
            .await
            .unwrap();

        // Filter by bug type AND alice assignee
        let query = SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Bug),
            None,
            Some("alice".to_string()),
            None,
            Vec::new(),
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, bug_alice_id);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_spawned_from() {
        let store = MemoryStore::new();

        // Create two issues to spawn tasks from
        let (issue_a, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_b, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Create tasks spawned from different issues
        let mut task_a1 = spawn_task();
        task_a1.spawned_from = Some(issue_a.clone());
        let (task_a1_id, _) = store
            .add_task(task_a1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_a2 = spawn_task();
        task_a2.spawned_from = Some(issue_a.clone());
        let (task_a2_id, _) = store
            .add_task(task_a2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b1 = spawn_task();
        task_b1.spawned_from = Some(issue_b.clone());
        let (task_b1_id, _) = store
            .add_task(task_b1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let task_orphan = spawn_task(); // no spawned_from
        let (task_orphan_id, _) = store
            .add_task(task_orphan, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Filter by issue_a should return only tasks spawned from issue_a
        let query = SearchJobsQuery::new(None, Some(issue_a.clone()), None, None);
        let tasks: HashSet<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            tasks,
            HashSet::from([task_a1_id.clone(), task_a2_id.clone()])
        );

        // Filter by issue_b should return only tasks spawned from issue_b
        let query = SearchJobsQuery::new(None, Some(issue_b.clone()), None, None);
        let tasks: HashSet<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task_b1_id.clone()]));

        // No filter should return all tasks
        let tasks: HashSet<_> = store
            .list_tasks(&SearchJobsQuery::default())
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            tasks,
            HashSet::from([task_a1_id, task_a2_id, task_b1_id, task_orphan_id])
        );
    }

    #[tokio::test]
    async fn list_tasks_filters_by_search_term_prompt() {
        let store = MemoryStore::new();

        // Create tasks with different prompts
        let mut task1 = spawn_task();
        task1.prompt = "Fix authentication bug".to_string();
        let (task1_id, _) = store
            .add_task(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task2 = spawn_task();
        task2.prompt = "Add new feature for login".to_string();
        let (task2_id, _) = store
            .add_task(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task3 = spawn_task();
        task3.prompt = "Refactor database layer".to_string();
        let (task3_id, _) = store
            .add_task(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Search for "auth" should match task1
        let query = SearchJobsQuery::new(Some("auth".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id.clone()]);

        // Search for "login" should match task2
        let query = SearchJobsQuery::new(Some("login".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id.clone()]);

        // Search for "FIX" (case-insensitive) should match task1
        let query = SearchJobsQuery::new(Some("FIX".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id.clone()]);

        // Search for "nonexistent" should return empty
        let query = SearchJobsQuery::new(Some("nonexistent".to_string()), None, None, None);
        let tasks: Vec<_> = store.list_tasks(&query).await.unwrap();
        assert!(tasks.is_empty());

        // Empty search term should return all tasks
        let query = SearchJobsQuery::new(Some("".to_string()), None, None, None);
        let tasks: HashSet<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task1_id, task2_id, task3_id]));
    }

    #[tokio::test]
    async fn list_tasks_search_term_matches_task_id() {
        let store = MemoryStore::new();

        let task = spawn_task();
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Search by partial task ID
        let id_prefix = &task_id.as_ref()[..6]; // First 6 characters
        let query = SearchJobsQuery::new(Some(id_prefix.to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task_id]);
    }

    #[tokio::test]
    async fn batch_get_status_logs_returns_logs_for_multiple_tasks() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        // Create three tasks and transition them to different states
        let (task1_id, _) = store
            .add_task(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&task1_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&task1_id, ActorRef::test())
            .await
            .unwrap();

        let (task2_id, _) = store
            .add_task(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&task2_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&task2_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(
                &task2_id,
                Ok(()),
                Some("done".to_string()),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let (task3_id, _) = store
            .add_task(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_pending(&task3_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_running(&task3_id, ActorRef::test())
            .await
            .unwrap();
        state
            .transition_task_to_completion(
                &task3_id,
                Err(TaskError::JobEngineError {
                    reason: "test failure".to_string(),
                }),
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Batch fetch all status logs
        let logs = store
            .get_status_logs(&[task1_id.clone(), task2_id.clone(), task3_id.clone()])
            .await
            .unwrap();

        assert_eq!(logs.len(), 3);

        // Task 1: running
        let log1 = logs.get(&task1_id).unwrap();
        assert_eq!(log1.current_status(), Status::Running);

        // Task 2: complete
        let log2 = logs.get(&task2_id).unwrap();
        assert_eq!(log2.current_status(), Status::Complete);

        // Task 3: failed
        let log3 = logs.get(&task3_id).unwrap();
        assert_eq!(log3.current_status(), Status::Failed);

        // Empty batch returns empty map
        let empty = store.get_status_logs(&[]).await.unwrap();
        assert!(empty.is_empty());

        // Non-existent task is silently omitted
        let missing_id = TaskId::new();
        let partial = store
            .get_status_logs(&[task1_id.clone(), missing_id])
            .await
            .unwrap();
        assert_eq!(partial.len(), 1);
        assert!(partial.contains_key(&task1_id));
    }

    #[tokio::test]
    async fn list_tasks_search_term_matches_status() {
        let store = MemoryStore::new();

        // Create a task in Created status
        let task1 = spawn_task();
        let (task1_id, _) = store
            .add_task(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Create a task and update to Running status
        let task2 = spawn_task();
        let (task2_id, _) = store
            .add_task(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Running;
        store
            .update_task(&task2_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Create a task and update to Complete status
        let task3 = spawn_task();
        let (task3_id, _) = store
            .add_task(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Complete;
        store
            .update_task(&task3_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Search for "created" should match task1
        let query = SearchJobsQuery::new(Some("created".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id]);

        // Search for "running" should match task2
        let query = SearchJobsQuery::new(Some("running".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id]);

        // Search for "complete" should match task3
        let query = SearchJobsQuery::new(Some("complete".to_string()), None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task3_id]);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_status_field() {
        let store = MemoryStore::new();

        // Add a task with Created status (default)
        let task1 = spawn_task();
        let (task1_id, _) = store
            .add_task(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Add a task and update to Running status
        let task2 = spawn_task();
        let (task2_id, _) = store
            .add_task(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Running;
        store
            .update_task(&task2_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Add a task and update to Complete status
        let task3 = spawn_task();
        let (task3_id, _) = store
            .add_task(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Complete;
        store
            .update_task(&task3_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Filter by Created should return only task1
        let query = SearchJobsQuery::new(None, None, None, Some(Status::Created.into()));
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id]);

        // Filter by Running should return only task2
        let query = SearchJobsQuery::new(None, None, None, Some(Status::Running.into()));
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id]);

        // Filter by Complete should return only task3
        let query = SearchJobsQuery::new(None, None, None, Some(Status::Complete.into()));
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task3_id]);

        // Filter by Failed should return no tasks
        let query = SearchJobsQuery::new(None, None, None, Some(Status::Failed.into()));
        let tasks: Vec<_> = store.list_tasks(&query).await.unwrap();
        assert!(tasks.is_empty());

        // No status filter should return all tasks
        let query = SearchJobsQuery::new(None, None, None, None);
        let tasks: Vec<_> = store
            .list_tasks(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test]
    async fn list_patches_filters_by_status() {
        use metis_common::api::v1::patches::PatchStatus as ApiPatchStatus;

        let store = MemoryStore::new();

        let mut open_patch = sample_patch();
        open_patch.status = PatchStatus::Open;
        let (open_id, _) = store
            .add_patch(open_patch, &ActorRef::test())
            .await
            .unwrap();

        let mut closed_patch = sample_patch();
        closed_patch.status = PatchStatus::Closed;
        store
            .add_patch(closed_patch, &ActorRef::test())
            .await
            .unwrap();

        let mut merged_patch = sample_patch();
        merged_patch.status = PatchStatus::Merged;
        store
            .add_patch(merged_patch, &ActorRef::test())
            .await
            .unwrap();

        // Filter for Open only
        let query = SearchPatchesQuery::new(None, None, vec![ApiPatchStatus::Open], None);
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, open_id);

        // Filter for Open and Closed
        let query = SearchPatchesQuery::new(
            None,
            None,
            vec![ApiPatchStatus::Open, ApiPatchStatus::Closed],
            None,
        );
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 2);
    }

    #[tokio::test]
    async fn list_patches_filters_by_branch_name() {
        let store = MemoryStore::new();

        let mut patch1 = sample_patch();
        patch1.branch_name = Some("feature/foo".to_string());
        let (patch1_id, _) = store.add_patch(patch1, &ActorRef::test()).await.unwrap();

        let mut patch2 = sample_patch();
        patch2.branch_name = Some("feature/bar".to_string());
        store.add_patch(patch2, &ActorRef::test()).await.unwrap();

        let mut patch3 = sample_patch();
        patch3.branch_name = None;
        store.add_patch(patch3, &ActorRef::test()).await.unwrap();

        let query = SearchPatchesQuery::new(None, None, vec![], Some("feature/foo".to_string()));
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, patch1_id);
    }

    #[tokio::test]
    async fn list_patches_combines_status_and_branch_name_filters() {
        use metis_common::api::v1::patches::PatchStatus as ApiPatchStatus;

        let store = MemoryStore::new();

        let mut open_foo = sample_patch();
        open_foo.status = PatchStatus::Open;
        open_foo.branch_name = Some("feature/foo".to_string());
        let (open_foo_id, _) = store.add_patch(open_foo, &ActorRef::test()).await.unwrap();

        let mut closed_foo = sample_patch();
        closed_foo.status = PatchStatus::Closed;
        closed_foo.branch_name = Some("feature/foo".to_string());
        store
            .add_patch(closed_foo, &ActorRef::test())
            .await
            .unwrap();

        let mut open_bar = sample_patch();
        open_bar.status = PatchStatus::Open;
        open_bar.branch_name = Some("feature/bar".to_string());
        store.add_patch(open_bar, &ActorRef::test()).await.unwrap();

        // Open patches with branch "feature/foo"
        let query = SearchPatchesQuery::new(
            None,
            None,
            vec![ApiPatchStatus::Open],
            Some("feature/foo".to_string()),
        );
        let patches = store.list_patches(&query).await.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0, open_foo_id);
    }

    #[tokio::test]
    async fn list_patches_branch_name_filter_no_match() {
        let store = MemoryStore::new();

        let mut patch = sample_patch();
        patch.branch_name = Some("feature/foo".to_string());
        store.add_patch(patch, &ActorRef::test()).await.unwrap();

        let query =
            SearchPatchesQuery::new(None, None, vec![], Some("feature/nonexistent".to_string()));
        let patches = store.list_patches(&query).await.unwrap();
        assert!(patches.is_empty());
    }

    #[tokio::test]
    async fn list_patches_deleted_patch_excluded_from_branch_filter() {
        let store = MemoryStore::new();

        let mut patch = sample_patch();
        patch.branch_name = Some("feature/foo".to_string());
        let (patch_id, _) = store.add_patch(patch, &ActorRef::test()).await.unwrap();
        store
            .delete_patch(&patch_id, &ActorRef::test())
            .await
            .unwrap();

        let query = SearchPatchesQuery::new(None, None, vec![], Some("feature/foo".to_string()));
        let patches = store.list_patches(&query).await.unwrap();
        assert!(patches.is_empty());
    }
}
