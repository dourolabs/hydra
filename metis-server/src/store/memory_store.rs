use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};

use super::issue_graph::IssueGraphContext;
use super::{ReadOnlyStore, Session, Status, Store, StoreError, TaskStatusLog};
use crate::domain::{
    actors::{Actor, ActorId, ActorRef},
    agents::Agent,
    documents::Document,
    issues::{
        Issue, IssueDependency, IssueDependencyType, IssueGraphFilter, IssueStatus, IssueType,
    },
    labels::Label,
    messages::Message,
    notifications::Notification,
    patches::Patch,
    secrets::SecretRef,
    users::{User, Username},
};
use metis_common::api::v1::documents::SearchDocumentsQuery;
use metis_common::api::v1::issues::SearchIssuesQuery;
use metis_common::api::v1::messages::SearchMessagesQuery;
use metis_common::api::v1::pagination::{DecodedCursor, MAX_LIMIT as PAGINATION_MAX_LIMIT};
use metis_common::api::v1::patches::SearchPatchesQuery;
use metis_common::api::v1::sessions::SearchSessionsQuery;
use metis_common::api::v1::users::SearchUsersQuery;
use metis_common::{
    DocumentId, IssueId, LabelId, MessageId, MetisId, NotificationId, PatchId, RepoName, SessionId,
    VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    api::v1::notifications::ListNotificationsQuery,
    repositories::{Repository, SearchRepositoriesQuery},
};

/// An in-memory implementation of the Store trait.
///
/// This store keeps tasks, issues, and patches in DashMaps for fast lookups.
/// It uses internal locking to make access thread-safe.
pub struct MemoryStore {
    /// Maps task IDs to their Task data
    tasks: DashMap<SessionId, Vec<Versioned<Session>>>,
    /// Maps issue IDs to their Issue data
    issues: DashMap<IssueId, Vec<Versioned<Issue>>>,
    /// Maps patch IDs to their Patch data
    patches: DashMap<PatchId, Vec<Versioned<Patch>>>,
    /// Maps document IDs to their Document data
    documents: DashMap<DocumentId, Vec<Versioned<Document>>>,
    /// Maps repository names to their configurations
    repositories: DashMap<RepoName, Vec<Versioned<Repository>>>,
    /// Maps issue IDs to tasks spawned from them
    issue_tasks: DashMap<IssueId, Vec<SessionId>>,
    /// Maps document paths to the document IDs that live under them
    documents_by_path: DashMap<String, HashSet<DocumentId>>,
    /// Maps usernames to their User data
    users: DashMap<Username, Vec<Versioned<User>>>,
    /// Maps actor names to their Actor data
    actors: DashMap<String, Vec<Versioned<Actor>>>,
    /// Maps message IDs to their versioned Message data
    messages: DashMap<MessageId, Vec<Versioned<Message>>>,
    /// Maps notification IDs to their Notification data (non-versioned)
    notifications: DashMap<NotificationId, Notification>,
    /// Maps agent names to their Agent data (non-versioned)
    agents: DashMap<String, Agent>,
    /// Maps label IDs to their Label data (non-versioned)
    labels: DashMap<LabelId, Label>,
    /// Maps object IDs to associated label IDs
    object_labels: DashMap<MetisId, HashSet<LabelId>>,
    /// Maps label IDs to associated object IDs
    label_objects: DashMap<LabelId, HashSet<MetisId>>,
    /// Maps (username, secret_name) to (encrypted_value, internal)
    user_secrets: DashMap<(Username, String), (Vec<u8>, bool)>,
    /// Stores object relationships as (source_id, rel_type, target_id) -> ObjectRelationship
    object_relationships:
        DashMap<(MetisId, super::RelationshipType, MetisId), super::ObjectRelationship>,
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
            issue_tasks: DashMap::new(),
            documents_by_path: DashMap::new(),
            users: DashMap::new(),
            actors: DashMap::new(),
            messages: DashMap::new(),
            notifications: DashMap::new(),
            agents: DashMap::new(),
            labels: DashMap::new(),
            object_labels: DashMap::new(),
            label_objects: DashMap::new(),
            user_secrets: DashMap::new(),
            object_relationships: DashMap::new(),
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
        let now = Utc::now();
        Versioned::with_actor(item, version, now, actor.clone(), now)
    }

    fn versioned_at_with_actor<T>(
        item: T,
        version: VersionNumber,
        timestamp: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Versioned<T> {
        Versioned::with_actor(item, version, timestamp, actor.clone(), timestamp)
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

    /// Returns an iterator over issues matching the query filters (without pagination).
    fn filter_issues<'a>(
        &'a self,
        query: &'a SearchIssuesQuery,
    ) -> impl Iterator<Item = (IssueId, Versioned<Issue>)> + 'a {
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

        self.issues.iter().filter_map(move |entry| {
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
            if !query.label_ids.is_empty() {
                let object_id = MetisId::from(issue_id.clone());
                let has_all_labels = if let Some(issue_labels) = self.object_labels.get(&object_id)
                {
                    query.label_ids.iter().all(|lid| issue_labels.contains(lid))
                } else {
                    false
                };
                if !has_all_labels {
                    return None;
                }
            }
            Some((issue_id.clone(), latest))
        })
    }

    /// Returns an iterator over patches matching the query filters (without pagination).
    fn filter_patches<'a>(
        &'a self,
        query: &'a SearchPatchesQuery,
    ) -> impl Iterator<Item = (PatchId, Versioned<Patch>)> + 'a {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let status_filter: Vec<crate::domain::patches::PatchStatus> =
            query.status.iter().copied().map(Into::into).collect();

        self.patches.iter().filter_map(move |entry| {
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
    }

    /// Returns filtered documents matching the query (without pagination).
    fn filter_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Vec<(DocumentId, Versioned<Document>)> {
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
                        let mut latest = Self::latest_versioned(entry.value())?;
                        latest.creation_time = entry.value()[0].timestamp;
                        Some((entry.key().clone(), latest))
                    })
                    .collect()
            };

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

        documents
    }

    /// Returns an iterator over tasks matching the query filters (without pagination).
    fn filter_tasks<'a>(
        &'a self,
        query: &'a SearchSessionsQuery,
    ) -> impl Iterator<Item = (SessionId, Versioned<Session>)> + 'a {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        self.tasks.iter().filter_map(move |entry| {
            let task_id = entry.key();
            let latest = Self::latest_versioned(entry.value())?;

            if !include_deleted && latest.item.deleted {
                return None;
            }

            if let Some(expected_issue) = query.spawned_from.as_ref() {
                if latest.item.spawned_from.as_ref() != Some(expected_issue) {
                    return None;
                }
            }

            if !query.status.is_empty() {
                let status_filter: Vec<Status> =
                    query.status.iter().copied().map(Into::into).collect();
                if !status_filter.contains(&latest.item.status) {
                    return None;
                }
            }

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
    }

    /// Returns filtered labels matching the query (without pagination).
    fn filter_labels(&self, query: &SearchLabelsQuery) -> Vec<(LabelId, Label)> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let mut results: Vec<(LabelId, Label)> = Vec::new();

        for entry in self.labels.iter() {
            let label = entry.value();

            if !include_deleted && label.deleted {
                continue;
            }

            if let Some(ref q) = query.q {
                let q_lower = q.to_lowercase();
                if !label.name.to_lowercase().contains(&q_lower) {
                    continue;
                }
            }

            results.push((entry.key().clone(), label.clone()));
        }

        results
    }

    /// Updates issue adjacency indexes to match the provided dependency list.
    /// Syncs the object_relationships store for the given issue (delete old + insert new).
    fn sync_issue_relationships(&self, issue_id: &IssueId, issue: &Issue) {
        let source_id = MetisId::from(issue_id.clone());

        // Remove all existing relationships where this issue is the source
        let keys_to_remove: Vec<_> = self
            .object_relationships
            .iter()
            .filter(|entry| entry.key().0 == source_id)
            .map(|entry| entry.key().clone())
            .collect();
        for key in keys_to_remove {
            self.object_relationships.remove(&key);
        }

        // Insert dependency relationships
        for dep in &issue.dependencies {
            let target_id = MetisId::from(dep.issue_id.clone());
            let rel_type = super::RelationshipType::from(dep.dependency_type);
            let key = (source_id.clone(), rel_type, target_id.clone());
            self.object_relationships.insert(
                key,
                super::ObjectRelationship {
                    source_id: source_id.clone(),
                    source_kind: super::ObjectKind::Issue,
                    target_id,
                    target_kind: super::ObjectKind::Issue,
                    rel_type,
                },
            );
        }

        // Insert patch relationships
        for patch_id in &issue.patches {
            let target_id = MetisId::from(patch_id.clone());
            let rel_type = super::RelationshipType::HasPatch;
            let key = (source_id.clone(), rel_type, target_id.clone());
            self.object_relationships.insert(
                key,
                super::ObjectRelationship {
                    source_id: source_id.clone(),
                    source_kind: super::ObjectKind::Issue,
                    target_id,
                    target_kind: super::ObjectKind::Patch,
                    rel_type,
                },
            );
        }
    }

    /// Returns source IDs for relationships where target_id matches and rel_type matches.
    /// This is a "reverse" lookup: e.g., find all issues that have a ChildOf relationship
    /// pointing at `target_id` (i.e., children of `target_id`).
    fn get_sources_by_target_and_type(
        &self,
        target_id: &MetisId,
        rel_type: super::RelationshipType,
    ) -> Vec<MetisId> {
        self.object_relationships
            .iter()
            .filter(|entry| entry.key().2 == *target_id && entry.key().1 == rel_type)
            .map(|entry| entry.key().0.clone())
            .collect()
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
                if let Some(mut latest) = Self::latest_versioned(entry.value()) {
                    latest.creation_time = entry.value()[0].timestamp;
                    documents.push((id.clone(), latest));
                }
            }
        }

        documents
    }

    fn index_task_for_issue(&self, issue_id: &IssueId, task_id: SessionId) {
        let mut tasks = self.issue_tasks.entry(issue_id.clone()).or_default();
        if !tasks.contains(&task_id) {
            tasks.push(task_id);
        }
    }

    fn remove_task_from_issue_index(&self, issue_id: &IssueId, task_id: &SessionId) {
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
        let entry = self
            .issues
            .get(id)
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;
        let mut versioned = Self::latest_versioned(entry.value())
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;

        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        versioned.creation_time = entry.value()[0].timestamp;
        Ok(versioned)
    }

    async fn get_issue_versions(&self, id: &IssueId) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let entry = self
            .issues
            .get(id)
            .ok_or_else(|| StoreError::IssueNotFound(id.clone()))?;
        let creation_time = entry.value()[0].timestamp;
        let mut versions = entry.value().clone();
        for v in &mut versions {
            v.creation_time = creation_time;
        }
        Ok(versions)
    }

    async fn list_issues(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let items: Vec<_> = self
            .filter_issues(query)
            .map(|(issue_id, mut latest)| {
                latest.creation_time = self
                    .issues
                    .get(&issue_id)
                    .map(|e| e.value()[0].timestamp)
                    .unwrap_or(latest.timestamp);
                (issue_id, latest)
            })
            .collect();
        apply_memory_pagination(
            items,
            |(_id, v)| v.timestamp,
            |(id, _v)| id.as_ref(),
            &query.cursor,
            query.limit,
        )
    }

    async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        Ok(self.filter_issues(query).count() as u64)
    }

    async fn search_issue_graph(
        &self,
        filters: &[IssueGraphFilter],
    ) -> Result<HashSet<IssueId>, StoreError> {
        if filters.is_empty() {
            return Ok(HashSet::new());
        }

        // Build forward maps from object_relationships (target -> [sources])
        let mut forward: HashMap<IssueDependencyType, HashMap<IssueId, Vec<IssueId>>> =
            HashMap::new();
        for dep_type in [IssueDependencyType::ChildOf, IssueDependencyType::BlockedOn] {
            let rel_type = super::RelationshipType::from(dep_type);
            let mut map: HashMap<IssueId, Vec<IssueId>> = HashMap::new();
            for entry in self.object_relationships.iter() {
                if entry.key().1 == rel_type {
                    if let (Ok(target_id), Ok(source_id)) = (
                        IssueId::try_from(entry.key().2.clone()),
                        IssueId::try_from(entry.key().0.clone()),
                    ) {
                        map.entry(target_id).or_default().push(source_id);
                    }
                }
            }
            forward.insert(dep_type, map);
        }

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
        if !self.issues.contains_key(issue_id) {
            return Err(StoreError::IssueNotFound(issue_id.clone()));
        }
        let target_id = MetisId::from(issue_id.clone());
        let children: Vec<IssueId> = self
            .get_sources_by_target_and_type(&target_id, super::RelationshipType::ChildOf)
            .into_iter()
            .filter_map(|id| IssueId::try_from(id).ok())
            .collect();
        Ok(children)
    }

    async fn get_issue_blocked_on(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        if !self.issues.contains_key(issue_id) {
            return Err(StoreError::IssueNotFound(issue_id.clone()));
        }
        let target_id = MetisId::from(issue_id.clone());
        let blocked: Vec<IssueId> = self
            .get_sources_by_target_and_type(&target_id, super::RelationshipType::BlockedOn)
            .into_iter()
            .filter_map(|id| IssueId::try_from(id).ok())
            .collect();
        Ok(blocked)
    }

    async fn get_sessions_for_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<SessionId>, StoreError> {
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
        let entry = self
            .patches
            .get(id)
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let mut versioned = Self::latest_versioned(entry.value())
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::PatchNotFound(id.clone()));
        }
        versioned.creation_time = entry.value()[0].timestamp;
        Ok(versioned)
    }

    async fn get_patch_versions(&self, id: &PatchId) -> Result<Vec<Versioned<Patch>>, StoreError> {
        let entry = self
            .patches
            .get(id)
            .ok_or_else(|| StoreError::PatchNotFound(id.clone()))?;
        let creation_time = entry.value()[0].timestamp;
        let mut versions = entry.value().clone();
        for v in &mut versions {
            v.creation_time = creation_time;
        }
        Ok(versions)
    }

    async fn list_patches(
        &self,
        query: &SearchPatchesQuery,
    ) -> Result<Vec<(PatchId, Versioned<Patch>)>, StoreError> {
        let items: Vec<_> = self
            .filter_patches(query)
            .map(|(patch_id, mut latest)| {
                latest.creation_time = self
                    .patches
                    .get(&patch_id)
                    .map(|e| e.value()[0].timestamp)
                    .unwrap_or(latest.timestamp);
                (patch_id, latest)
            })
            .collect();
        apply_memory_pagination(
            items,
            |(_id, v)| v.timestamp,
            |(id, _v)| id.as_ref(),
            &query.cursor,
            query.limit,
        )
    }

    async fn count_patches(&self, query: &SearchPatchesQuery) -> Result<u64, StoreError> {
        Ok(self.filter_patches(query).count() as u64)
    }

    async fn get_issues_for_patch(&self, patch_id: &PatchId) -> Result<Vec<IssueId>, StoreError> {
        if !self.patches.contains_key(patch_id) {
            return Err(StoreError::PatchNotFound(patch_id.clone()));
        }
        let target_id = MetisId::from(patch_id.clone());
        let issues: Vec<IssueId> = self
            .get_sources_by_target_and_type(&target_id, super::RelationshipType::HasPatch)
            .into_iter()
            .filter_map(|id| IssueId::try_from(id).ok())
            .collect();
        Ok(issues)
    }

    async fn get_document(
        &self,
        id: &DocumentId,
        include_deleted: bool,
    ) -> Result<Versioned<Document>, StoreError> {
        let entry = self
            .documents
            .get(id)
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        let mut versioned = Self::latest_versioned(entry.value())
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::DocumentNotFound(id.clone()));
        }
        versioned.creation_time = entry.value()[0].timestamp;
        Ok(versioned)
    }

    async fn get_document_versions(
        &self,
        id: &DocumentId,
    ) -> Result<Vec<Versioned<Document>>, StoreError> {
        let entry = self
            .documents
            .get(id)
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
        let creation_time = entry.value()[0].timestamp;
        let mut versions = entry.value().clone();
        for v in &mut versions {
            v.creation_time = creation_time;
        }
        Ok(versions)
    }

    async fn list_documents(
        &self,
        query: &SearchDocumentsQuery,
    ) -> Result<Vec<(DocumentId, Versioned<Document>)>, StoreError> {
        let documents = self.filter_documents(query);
        apply_memory_pagination(
            documents,
            |(_id, v)| v.timestamp,
            |(id, _v)| id.as_ref(),
            &query.cursor,
            query.limit,
        )
    }

    async fn count_documents(&self, query: &SearchDocumentsQuery) -> Result<u64, StoreError> {
        Ok(self.filter_documents(query).len() as u64)
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

    async fn get_session(
        &self,
        id: &SessionId,
        include_deleted: bool,
    ) -> Result<Versioned<Session>, StoreError> {
        let versioned = self
            .tasks
            .get(id)
            .and_then(|entry| Self::latest_versioned(entry.value()))
            .ok_or_else(|| StoreError::SessionNotFound(id.clone()))?;
        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        Ok(versioned)
    }

    async fn get_session_versions(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<Session>>, StoreError> {
        self.tasks
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::SessionNotFound(id.clone()))
    }

    async fn list_sessions(
        &self,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<(SessionId, Versioned<Session>)>, StoreError> {
        let items: Vec<_> = self.filter_tasks(query).collect();
        apply_memory_pagination(
            items,
            |(_id, v)| v.timestamp,
            |(id, _v)| id.as_ref(),
            &query.cursor,
            query.limit,
        )
    }

    async fn count_sessions(&self, query: &SearchSessionsQuery) -> Result<u64, StoreError> {
        Ok(self.filter_tasks(query).count() as u64)
    }

    async fn get_status_log(&self, id: &SessionId) -> Result<TaskStatusLog, StoreError> {
        self.tasks
            .get(id)
            .and_then(|entry| super::session_status_log_from_versions(entry.value()))
            .ok_or_else(|| StoreError::SessionNotFound(id.clone()))
    }

    async fn get_status_logs(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, TaskStatusLog>, StoreError> {
        let mut result = HashMap::new();
        for id in ids {
            if let Some(entry) = self.tasks.get(id) {
                if let Some(log) = super::session_status_log_from_versions(entry.value()) {
                    result.insert(id.clone(), log);
                }
            }
        }
        Ok(result)
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

    // ---- Message (read-only) ----

    async fn get_message(&self, id: &MessageId) -> Result<Versioned<Message>, StoreError> {
        let versions = self
            .messages
            .get(id)
            .ok_or_else(|| StoreError::MessageNotFound(id.clone()))?;
        Self::latest_versioned(versions.value())
            .ok_or_else(|| StoreError::MessageNotFound(id.clone()))
    }

    async fn list_messages(
        &self,
        query: &SearchMessagesQuery,
    ) -> Result<Vec<(MessageId, Versioned<Message>)>, StoreError> {
        let limit = query.limit.unwrap_or(50) as usize;
        let include_deleted = query.include_deleted.unwrap_or(false);

        // Collect all messages with their latest versions
        let mut all_messages: Vec<(MessageId, Versioned<Message>)> = Vec::new();
        for entry in self.messages.iter() {
            let msg_id = entry.key().clone();
            let versions = entry.value();
            if let Some(latest) = Self::latest_versioned(versions) {
                let msg = &latest.item;

                // Filter by deleted
                if msg.deleted && !include_deleted {
                    continue;
                }

                // Filter by sender
                if let Some(ref sender_filter) = query.sender {
                    match &msg.sender {
                        Some(sender) if sender.to_string() == *sender_filter => {}
                        _ => continue,
                    }
                }

                // Filter by recipient
                if let Some(ref recipient_filter) = query.recipient {
                    if msg.recipient.to_string() != *recipient_filter {
                        continue;
                    }
                }

                // Filter by after timestamp
                if let Some(ref after_ts) = query.after {
                    if latest.timestamp <= *after_ts {
                        continue;
                    }
                }

                // Filter by before timestamp
                if let Some(ref before_ts) = query.before {
                    if latest.timestamp >= *before_ts {
                        continue;
                    }
                }

                // Filter by is_read
                if let Some(is_read_filter) = query.is_read {
                    if msg.is_read != is_read_filter {
                        continue;
                    }
                }

                all_messages.push((msg_id, latest));
            }
        }

        // Sort by timestamp descending (newest first)
        all_messages.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp));
        all_messages.truncate(limit);

        Ok(all_messages)
    }

    // ---- Notification (read-only) ----

    async fn get_notification(&self, id: &NotificationId) -> Result<Notification, StoreError> {
        self.notifications
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::NotificationNotFound(id.clone()))
    }

    async fn list_notifications(
        &self,
        query: &ListNotificationsQuery,
    ) -> Result<Vec<(NotificationId, Notification)>, StoreError> {
        let limit = query.limit.unwrap_or(50) as usize;

        let mut results: Vec<(NotificationId, Notification)> = Vec::new();
        for entry in self.notifications.iter() {
            let notif = entry.value();

            // Filter by recipient
            if let Some(ref recipient_filter) = query.recipient {
                if notif.recipient.to_string() != *recipient_filter {
                    continue;
                }
            }

            // Filter by is_read
            if let Some(is_read_filter) = query.is_read {
                if notif.is_read != is_read_filter {
                    continue;
                }
            }

            // Filter by before timestamp
            if let Some(ref before_ts) = query.before {
                if notif.created_at >= *before_ts {
                    continue;
                }
            }

            // Filter by after timestamp
            if let Some(ref after_ts) = query.after {
                if notif.created_at <= *after_ts {
                    continue;
                }
            }

            results.push((entry.key().clone(), notif.clone()));
        }

        // Sort by created_at descending (newest first)
        results.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
        results.truncate(limit);

        Ok(results)
    }

    async fn count_unread_notifications(&self, recipient: &ActorId) -> Result<u64, StoreError> {
        let recipient_str = recipient.to_string();
        let count = self
            .notifications
            .iter()
            .filter(|entry| {
                let n = entry.value();
                !n.is_read && n.recipient.to_string() == recipient_str
            })
            .count();
        Ok(count as u64)
    }

    // ---- Agent (read-only) ----

    async fn get_agent(&self, name: &str) -> Result<Agent, StoreError> {
        let agent = self
            .agents
            .get(name)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::AgentNotFound(name.to_string()))?;
        if agent.deleted {
            return Err(StoreError::AgentNotFound(name.to_string()));
        }
        Ok(agent)
    }

    async fn list_agents(&self) -> Result<Vec<Agent>, StoreError> {
        let mut results: Vec<Agent> = self
            .agents
            .iter()
            .filter(|entry| !entry.value().deleted)
            .map(|entry| entry.value().clone())
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(results)
    }

    // ---- Label (read-only) ----

    async fn get_label(&self, id: &LabelId) -> Result<Label, StoreError> {
        let label = self
            .labels
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| StoreError::LabelNotFound(id.clone()))?;
        if label.deleted {
            return Err(StoreError::LabelNotFound(id.clone()));
        }
        Ok(label)
    }

    async fn list_labels(
        &self,
        query: &SearchLabelsQuery,
    ) -> Result<Vec<(LabelId, Label)>, StoreError> {
        let mut results = self.filter_labels(query);

        if query.limit.is_some() || query.cursor.is_some() {
            apply_memory_pagination(
                results,
                |(_id, label)| label.updated_at,
                |(id, _label)| id.as_ref(),
                &query.cursor,
                query.limit,
            )
        } else {
            results.sort_by(|(_, a), (_, b)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            Ok(results)
        }
    }

    async fn count_labels(&self, query: &SearchLabelsQuery) -> Result<u64, StoreError> {
        Ok(self.filter_labels(query).len() as u64)
    }

    async fn get_label_by_name(&self, name: &str) -> Result<Option<(LabelId, Label)>, StoreError> {
        let name_lower = name.to_lowercase();
        for entry in self.labels.iter() {
            let label = entry.value();
            if !label.deleted && label.name == name_lower {
                return Ok(Some((entry.key().clone(), label.clone())));
            }
        }
        Ok(None)
    }

    async fn get_labels_for_object(
        &self,
        object_id: &MetisId,
    ) -> Result<Vec<LabelSummary>, StoreError> {
        let label_ids = match self.object_labels.get(object_id) {
            Some(ids) => ids.clone(),
            None => return Ok(Vec::new()),
        };

        let mut result: Vec<LabelSummary> = label_ids
            .iter()
            .filter_map(|label_id| {
                let label = self.labels.get(label_id)?;
                if label.deleted {
                    return None;
                }
                Some(LabelSummary::new(
                    label_id.clone(),
                    label.name.clone(),
                    label.color.clone(),
                    label.recurse,
                    label.hidden,
                ))
            })
            .collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    async fn get_labels_for_objects(
        &self,
        object_ids: &[MetisId],
    ) -> Result<HashMap<MetisId, Vec<LabelSummary>>, StoreError> {
        let mut result: HashMap<MetisId, Vec<LabelSummary>> = HashMap::new();
        for object_id in object_ids {
            let labels = self.get_labels_for_object(object_id).await?;
            if !labels.is_empty() {
                result.insert(object_id.clone(), labels);
            }
        }
        Ok(result)
    }

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<MetisId>, StoreError> {
        match self.label_objects.get(label_id) {
            Some(ids) => Ok(ids.iter().cloned().collect()),
            None => Ok(Vec::new()),
        }
    }

    // ---- Object relationships (read-only) ----

    async fn get_relationships(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: Option<super::RelationshipType>,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let results: Vec<super::ObjectRelationship> = self
            .object_relationships
            .iter()
            .filter(|entry| {
                let rel = entry.value();
                if let Some(sid) = source_id {
                    if &rel.source_id != sid {
                        return false;
                    }
                }
                if let Some(tid) = target_id {
                    if &rel.target_id != tid {
                        return false;
                    }
                }
                if let Some(rt) = rel_type {
                    if rel.rel_type != rt {
                        return false;
                    }
                }
                true
            })
            .map(|entry| entry.value().clone())
            .collect();
        Ok(results)
    }

    async fn get_relationships_batch(
        &self,
        source_ids: Option<&[MetisId]>,
        target_ids: Option<&[MetisId]>,
        rel_type: Option<super::RelationshipType>,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let results: Vec<super::ObjectRelationship> = self
            .object_relationships
            .iter()
            .filter(|entry| {
                let rel = entry.value();
                if let Some(sids) = source_ids {
                    if !sids.contains(&rel.source_id) {
                        return false;
                    }
                }
                if let Some(tids) = target_ids {
                    if !tids.contains(&rel.target_id) {
                        return false;
                    }
                }
                if let Some(rt) = rel_type {
                    if rel.rel_type != rt {
                        return false;
                    }
                }
                true
            })
            .map(|entry| entry.value().clone())
            .collect();
        Ok(results)
    }

    async fn get_relationships_transitive(
        &self,
        source_id: Option<&MetisId>,
        target_id: Option<&MetisId>,
        rel_type: super::RelationshipType,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();

        if let Some(start) = source_id {
            // Forward traversal: follow source -> target edges
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(start.clone());
            visited.insert(start.clone());

            while let Some(current) = queue.pop_front() {
                for entry in self.object_relationships.iter() {
                    let rel = entry.value();
                    if rel.rel_type == rel_type && rel.source_id == current {
                        result.push(rel.clone());
                        if visited.insert(rel.target_id.clone()) {
                            queue.push_back(rel.target_id.clone());
                        }
                    }
                }
            }
        } else if let Some(start) = target_id {
            // Backward traversal: follow target -> source edges
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(start.clone());
            visited.insert(start.clone());

            while let Some(current) = queue.pop_front() {
                for entry in self.object_relationships.iter() {
                    let rel = entry.value();
                    if rel.rel_type == rel_type && rel.target_id == current {
                        result.push(rel.clone());
                        if visited.insert(rel.source_id.clone()) {
                            queue.push_back(rel.source_id.clone());
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    // ---- User secrets (read-only) ----

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let key = (username.clone(), secret_name.to_string());
        Ok(self.user_secrets.get(&key).map(|v| v.value().0.clone()))
    }

    async fn list_user_secret_names(
        &self,
        username: &Username,
    ) -> Result<Vec<SecretRef>, StoreError> {
        let mut refs: Vec<SecretRef> = self
            .user_secrets
            .iter()
            .filter(|entry| &entry.key().0 == username)
            .map(|entry| SecretRef {
                name: entry.key().1.clone(),
                internal: entry.value().1,
            })
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(refs)
    }

    async fn is_secret_internal(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<bool, StoreError> {
        let key = (username.clone(), secret_name.to_string());
        Ok(self.user_secrets.get(&key).map_or(false, |v| v.value().1))
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
        let id = IssueId::new();

        self.validate_dependencies(&issue.dependencies)?;

        // Sync object_relationships
        self.sync_issue_relationships(&id, &issue);

        self.issues.insert(
            id.clone(),
            vec![Self::versioned_now_with_actor(issue, 1, actor)],
        );

        Ok((id, 1))
    }

    async fn update_issue(
        &self,
        id: &IssueId,
        issue: Issue,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        if !self.issues.contains_key(id) {
            return Err(StoreError::IssueNotFound(id.clone()));
        }

        self.validate_dependencies(&issue.dependencies)?;

        // Sync object_relationships
        self.sync_issue_relationships(id, &issue);

        let next_version = if let Some(mut versions) = self.issues.get_mut(id) {
            let next_version = Self::next_version(&versions);
            versions.push(Self::versioned_now_with_actor(issue, next_version, actor));
            next_version
        } else {
            return Err(StoreError::IssueNotFound(id.clone()));
        };

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
        let id = PatchId::new();
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
        let id = DocumentId::new();
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

    async fn add_session(
        &self,
        mut session: Session,
        creation_time: DateTime<Utc>,
        actor: &ActorRef,
    ) -> Result<(SessionId, VersionNumber), StoreError> {
        let id = SessionId::new();
        let spawned_from = session.spawned_from.clone();

        session.creation_time = Some(creation_time);
        self.tasks.insert(
            id.clone(),
            vec![Self::versioned_at_with_actor(
                session,
                1,
                creation_time,
                actor,
            )],
        );

        if let Some(issue_id) = spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, id.clone());
        }

        Ok((id, 1))
    }

    async fn update_session(
        &self,
        metis_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let previous_spawned_from = match self.tasks.get(metis_id) {
            Some(entry) => entry
                .value()
                .last()
                .and_then(|existing| existing.item.spawned_from.clone()),
            None => return Err(StoreError::SessionNotFound(metis_id.clone())),
        };

        if let Some(previous_issue) = previous_spawned_from.as_ref() {
            if session.spawned_from.as_ref() != Some(previous_issue) {
                self.remove_task_from_issue_index(previous_issue, metis_id);
            }
        }

        // Overwrite the existing session without modifying edge structure
        let updated = match self.tasks.get_mut(metis_id) {
            Some(mut versions) => {
                let next_version = Self::next_version(&versions);
                let versioned =
                    Self::versioned_now_with_actor(session.clone(), next_version, actor);
                versions.push(versioned.clone());
                versioned
            }
            None => return Err(StoreError::SessionNotFound(metis_id.clone())),
        };

        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, metis_id.clone());
        }
        Ok(updated)
    }

    async fn delete_session(
        &self,
        id: &SessionId,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let current = self.get_session(id, true).await?;
        let mut task = current.item;
        task.deleted = true;
        let versioned = self.update_session(id, task, actor).await?;
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

    // ---- Notification mutations ----

    async fn insert_notification(
        &self,
        notification: Notification,
    ) -> Result<NotificationId, StoreError> {
        let id = NotificationId::new();

        self.notifications.insert(id.clone(), notification);
        Ok(id)
    }

    async fn mark_notification_read(&self, id: &NotificationId) -> Result<(), StoreError> {
        let mut entry = self
            .notifications
            .get_mut(id)
            .ok_or_else(|| StoreError::NotificationNotFound(id.clone()))?;
        entry.is_read = true;
        Ok(())
    }

    async fn mark_all_notifications_read(
        &self,
        recipient: &ActorId,
        before: Option<DateTime<Utc>>,
    ) -> Result<u64, StoreError> {
        let recipient_str = recipient.to_string();
        let mut count = 0u64;
        for mut entry in self.notifications.iter_mut() {
            let notif = entry.value_mut();
            if notif.is_read {
                continue;
            }
            if notif.recipient.to_string() != recipient_str {
                continue;
            }
            if let Some(before_ts) = before {
                if notif.created_at >= before_ts {
                    continue;
                }
            }
            notif.is_read = true;
            count += 1;
        }
        Ok(count)
    }

    // ---- Message mutations ----

    async fn add_message(
        &self,
        message: Message,
        actor: &ActorRef,
    ) -> Result<(MessageId, VersionNumber), StoreError> {
        let id = MessageId::new();

        self.messages.insert(
            id.clone(),
            vec![Self::versioned_now_with_actor(message, 1, actor)],
        );

        Ok((id, 1))
    }

    async fn update_message(
        &self,
        id: &MessageId,
        message: Message,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut versions = self
            .messages
            .get_mut(id)
            .ok_or_else(|| StoreError::MessageNotFound(id.clone()))?;
        let next_version = Self::next_version(&versions);
        versions.push(Self::versioned_now_with_actor(message, next_version, actor));
        Ok(next_version)
    }

    // ---- Agent mutations ----

    async fn add_agent(&self, agent: Agent) -> Result<(), StoreError> {
        // Check if a non-deleted agent with this name already exists.
        if let Some(entry) = self.agents.get(&agent.name) {
            if !entry.value().deleted {
                return Err(StoreError::AgentAlreadyExists(agent.name));
            }
        }

        // Validate assignment agent uniqueness.
        if agent.is_assignment_agent {
            let has_assignment = self
                .agents
                .iter()
                .any(|e| e.value().is_assignment_agent && !e.value().deleted);
            if has_assignment {
                return Err(StoreError::AssignmentAgentAlreadyExists);
            }
        }

        self.agents.insert(agent.name.clone(), agent);
        Ok(())
    }

    async fn update_agent(&self, agent: Agent) -> Result<(), StoreError> {
        // Check the agent exists and is not deleted.
        let _ = self.get_agent(&agent.name).await?;

        // Validate assignment agent uniqueness (exclude self).
        if agent.is_assignment_agent {
            let conflict = self.agents.iter().any(|e| {
                e.value().is_assignment_agent && !e.value().deleted && e.key() != &agent.name
            });
            if conflict {
                return Err(StoreError::AssignmentAgentAlreadyExists);
            }
        }

        self.agents.insert(agent.name.clone(), agent);
        Ok(())
    }

    async fn delete_agent(&self, name: &str) -> Result<(), StoreError> {
        let mut agent = self.get_agent(name).await?;
        agent.deleted = true;
        agent.updated_at = Utc::now();
        self.agents.insert(name.to_string(), agent);
        Ok(())
    }

    // ---- Label mutations ----

    async fn add_label(&self, label: Label) -> Result<LabelId, StoreError> {
        // Check uniqueness by name
        if self.get_label_by_name(&label.name).await?.is_some() {
            return Err(StoreError::LabelAlreadyExists(label.name.clone()));
        }

        let id = LabelId::new();

        self.labels.insert(id.clone(), label);
        Ok(id)
    }

    async fn update_label(&self, id: &LabelId, label: Label) -> Result<(), StoreError> {
        // Check the label exists
        if !self.labels.contains_key(id) {
            return Err(StoreError::LabelNotFound(id.clone()));
        }

        // Check name uniqueness (exclude self)
        if let Some((existing_id, _)) = self.get_label_by_name(&label.name).await? {
            if existing_id != *id {
                return Err(StoreError::LabelAlreadyExists(label.name.clone()));
            }
        }

        self.labels.insert(id.clone(), label);
        Ok(())
    }

    async fn delete_label(&self, id: &LabelId) -> Result<(), StoreError> {
        let mut label = self.get_label(id).await?;
        label.deleted = true;
        label.updated_at = Utc::now();
        self.labels.insert(id.clone(), label);
        Ok(())
    }

    async fn add_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let newly_added = self
            .object_labels
            .entry(object_id.clone())
            .or_default()
            .insert(label_id.clone());
        self.label_objects
            .entry(label_id.clone())
            .or_default()
            .insert(object_id.clone());
        Ok(newly_added)
    }

    async fn remove_label_association(
        &self,
        label_id: &LabelId,
        object_id: &MetisId,
    ) -> Result<bool, StoreError> {
        let removed = self
            .object_labels
            .get_mut(object_id)
            .map(|mut label_ids| label_ids.remove(label_id))
            .unwrap_or(false);
        if let Some(mut object_ids) = self.label_objects.get_mut(label_id) {
            object_ids.remove(object_id);
        }
        Ok(removed)
    }

    // ---- Object relationship mutations ----

    async fn add_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: super::RelationshipType,
    ) -> Result<bool, StoreError> {
        let source_kind = super::object_kind_from_id(source_id)?;
        let target_kind = super::object_kind_from_id(target_id)?;
        let key = (source_id.clone(), rel_type, target_id.clone());
        if self.object_relationships.contains_key(&key) {
            return Ok(false);
        }
        self.object_relationships.insert(
            key,
            super::ObjectRelationship {
                source_id: source_id.clone(),
                source_kind,
                target_id: target_id.clone(),
                target_kind,
                rel_type,
            },
        );
        Ok(true)
    }

    async fn remove_relationship(
        &self,
        source_id: &MetisId,
        target_id: &MetisId,
        rel_type: super::RelationshipType,
    ) -> Result<bool, StoreError> {
        let key = (source_id.clone(), rel_type, target_id.clone());
        Ok(self.object_relationships.remove(&key).is_some())
    }

    // ---- User secret mutations ----

    async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
        internal: bool,
    ) -> Result<(), StoreError> {
        let key = (username.clone(), secret_name.to_string());
        self.user_secrets
            .insert(key, (encrypted_value.to_vec(), internal));
        Ok(())
    }

    async fn delete_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<(), StoreError> {
        let key = (username.clone(), secret_name.to_string());
        self.user_secrets.remove(&key);
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

        return issue.title.to_lowercase().contains(term)
            || issue.description.to_lowercase().contains(term)
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

/// Applies in-memory cursor-based pagination to a list of items.
///
/// Sorts by (timestamp DESC, id DESC), applies cursor filter, then limits results.
/// Fetches limit+1 items so callers can detect whether a next page exists.
fn apply_memory_pagination<T>(
    mut items: Vec<T>,
    get_timestamp: impl Fn(&T) -> DateTime<Utc>,
    get_id: impl Fn(&T) -> &str,
    cursor: &Option<String>,
    limit: Option<u32>,
) -> Result<Vec<T>, StoreError> {
    // Sort by timestamp DESC, id DESC
    items.sort_by(|a, b| {
        get_timestamp(b)
            .cmp(&get_timestamp(a))
            .then_with(|| get_id(b).cmp(get_id(a)))
    });

    // Apply cursor filter (keep only items that come after cursor in DESC order)
    if let Some(cursor_str) = cursor {
        let decoded = DecodedCursor::decode(cursor_str)
            .map_err(|e| StoreError::Internal(format!("invalid cursor: {e}")))?;
        items.retain(|item| {
            let ts = get_timestamp(item);
            let id = get_id(item);
            (ts, id) < (decoded.timestamp, decoded.id.as_str())
        });
    }

    // Apply limit (take one extra for next_cursor detection)
    if let Some(limit) = limit {
        let effective = (limit.min(PAGINATION_MAX_LIMIT) + 1) as usize;
        items.truncate(effective);
    }

    Ok(items)
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
            patches::{GithubPr, Patch, PatchStatus},
            sessions::BundleSpec,
            task_status::Event,
            users::{User, Username},
        },
        store::TaskError,
        test_utils::test_state_with_store,
    };
    use chrono::{Duration, Utc};
    use metis_common::{
        IssueId, RepoName, SessionId, VersionNumber, Versioned,
        repositories::{Repository, SearchRepositoriesQuery},
    };
    use std::{collections::HashSet, str::FromStr, sync::Arc};

    fn sample_repository_config() -> Repository {
        Repository::new(
            "https://example.com/repo.git".to_string(),
            Some("main".to_string()),
            Some("image:latest".to_string()),
            None,
        )
    }

    fn spawn_task() -> Session {
        Session::new(
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
            None,
            None,
            None,
        )
    }

    fn sample_document(path: Option<&str>, created_by: Option<SessionId>) -> Document {
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
            "Test Title".to_string(),
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
            None,
            None,
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
                    None,
                    None,
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
        let task_id = SessionId::new();
        let other_task = SessionId::new();

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
        let now = Utc::now();
        let (task_id, _) = store
            .add_session(task.clone(), now, &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
        // add_task sets creation_time on the stored task
        let mut expected = task.clone();
        expected.creation_time = Some(now);
        assert_versioned(&fetched, &expected, 1);
        assert_eq!(
            store
                .get_session(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Created
        );

        let tasks: HashSet<_> = store
            .list_sessions(&SearchSessionsQuery::default())
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
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut updated = spawn_task();
        updated.prompt = "v2".to_string();
        store
            .update_session(&task_id, updated.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_session(&task_id, false).await.unwrap();
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
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut v2 = spawn_task();
        v2.prompt = "v2".to_string();
        store
            .update_session(&task_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_session_versions(&task_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2]);
        assert_eq!(versions[0].item.prompt, "v1");
        assert_eq!(versions[1].item.prompt, "v2");
    }

    #[tokio::test]
    async fn task_versions_increment_on_transitions() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;
        let (task_id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
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
            store
                .get_session(&task_id, false)
                .await
                .unwrap()
                .item
                .status,
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
            .add_session(task.clone(), created_at, &ActorRef::test())
            .await
            .unwrap();

        let mut updated = task.clone();
        updated.prompt = "v2".to_string();
        store
            .update_session(&task_id, updated, &ActorRef::test())
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

        let mut running = store.get_session(&task_id, false).await.unwrap().item;
        running.prompt = "v3".to_string();
        store
            .update_session(&task_id, running, &ActorRef::test())
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
            .add_session(pending_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut running_task = spawn_task();
        running_task.spawned_from = Some(issue_id.clone());
        let (running_id, _) = store
            .add_session(running_task, Utc::now(), &ActorRef::test())
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
            .add_session(completed_task, Utc::now(), &ActorRef::test())
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
            .add_session(unrelated_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let tasks: HashSet<_> = store
            .get_sessions_for_issue(&issue_id)
            .await
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(tasks, HashSet::from([pending_id, running_id, completed_id]));

        let other_tasks: HashSet<_> = store
            .get_sessions_for_issue(&other_issue)
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

        let err = store
            .get_sessions_for_issue(&missing_issue)
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::IssueNotFound(id) if id == missing_issue));
    }

    #[tokio::test]
    async fn task_starts_as_created() {
        let store = MemoryStore::new();

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Created
        );
    }

    #[tokio::test]
    async fn transition_task_to_pending_from_created() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Created
        );

        state
            .transition_task_to_pending(&root_id, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Pending
        );
    }

    #[tokio::test]
    async fn transition_task_to_running_from_pending() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Running
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Running
        );

        // Then mark as complete
        state
            .transition_task_to_completion(&root_id, Ok(()), None, ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Complete
        );
    }

    #[tokio::test]
    async fn transition_task_to_failure_from_running() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        assert_eq!(
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn transition_task_to_completion_from_pending_fails() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
            Status::Failed
        );
    }

    #[tokio::test]
    async fn transition_task_idempotent_complete_to_complete() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
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
        let task = store.get_session(&root_id, false).await.unwrap();
        assert_eq!(task.item.status, Status::Complete);
        assert_eq!(task.item.last_message.as_deref(), Some("first message"));
    }

    #[tokio::test]
    async fn transition_task_idempotent_failed_to_failed() {
        let store = Arc::new(MemoryStore::new());
        let state = test_state_with_store(store.clone()).state;

        let root_task = spawn_task();
        let (root_id, _) = store
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            store
                .get_session(&root_id, false)
                .await
                .unwrap()
                .item
                .status,
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
        let task = store.get_session(&root_id, false).await.unwrap();
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
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
            .add_session(root_task, Utc::now(), &ActorRef::test())
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
                    github_user_id: Some(101),
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
                    github_user_id: Some(202),
                    deleted: false,
                },
                &ActorRef::test(),
            )
            .await
            .unwrap();

        assert_eq!(updated.item.github_user_id, Some(202));
        assert_eq!(updated.version, 2);

        let user = store.get_user(&username, false).await.unwrap();
        assert_eq!(user.item.github_user_id, Some(202));
        assert_eq!(user.version, 2);
    }

    #[tokio::test]
    async fn get_user_filters_deleted_users() {
        let store = MemoryStore::new();
        let username = Username::from("alice");
        let user = User {
            username: username.clone(),
            github_user_id: Some(101),
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
            actor_id: ActorId::Session(SessionId::new()),
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
        let task_id = SessionId::new();
        let actor = Actor {
            auth_token_hash: "hash".to_string(),
            auth_token_salt: "salt".to_string(),
            actor_id: ActorId::Session(task_id),
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
        let task_id = SessionId::new();
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
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Task should be visible in list initially
        let tasks = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(!tasks[0].1.item.deleted);

        // Delete the task
        store
            .delete_session(&task_id, &ActorRef::test())
            .await
            .unwrap();

        // Deleted task should not appear in default list
        let tasks = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .unwrap();
        assert!(tasks.is_empty());

        // Deleted task should appear with include_deleted=true
        let tasks = store
            .list_sessions(&SearchSessionsQuery::new(None, None, Some(true), vec![]))
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].1.item.deleted);

        // get_task with include_deleted=false should return TaskNotFound for deleted task
        let err = store.get_session(&task_id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(id) if id == task_id));

        // get_task with include_deleted=true should return the deleted task
        let deleted_task = store.get_session(&task_id, true).await.unwrap();
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
        let missing_id = SessionId::new();

        let err = store
            .delete_session(&missing_id, &ActorRef::test())
            .await
            .unwrap_err();

        assert!(matches!(err, StoreError::SessionNotFound(id) if id == missing_id));
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
            .add_session(task_a1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_a2 = spawn_task();
        task_a2.spawned_from = Some(issue_a.clone());
        let (task_a2_id, _) = store
            .add_session(task_a2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b1 = spawn_task();
        task_b1.spawned_from = Some(issue_b.clone());
        let (task_b1_id, _) = store
            .add_session(task_b1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let task_orphan = spawn_task(); // no spawned_from
        let (task_orphan_id, _) = store
            .add_session(task_orphan, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Filter by issue_a should return only tasks spawned from issue_a
        let query = SearchSessionsQuery::new(None, Some(issue_a.clone()), None, vec![]);
        let tasks: HashSet<_> = store
            .list_sessions(&query)
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
        let query = SearchSessionsQuery::new(None, Some(issue_b.clone()), None, vec![]);
        let tasks: HashSet<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task_b1_id.clone()]));

        // No filter should return all tasks
        let tasks: HashSet<_> = store
            .list_sessions(&SearchSessionsQuery::default())
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
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task2 = spawn_task();
        task2.prompt = "Add new feature for login".to_string();
        let (task2_id, _) = store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task3 = spawn_task();
        task3.prompt = "Refactor database layer".to_string();
        let (task3_id, _) = store
            .add_session(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Search for "auth" should match task1
        let query = SearchSessionsQuery::new(Some("auth".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id.clone()]);

        // Search for "login" should match task2
        let query = SearchSessionsQuery::new(Some("login".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id.clone()]);

        // Search for "FIX" (case-insensitive) should match task1
        let query = SearchSessionsQuery::new(Some("FIX".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id.clone()]);

        // Search for "nonexistent" should return empty
        let query = SearchSessionsQuery::new(Some("nonexistent".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store.list_sessions(&query).await.unwrap();
        assert!(tasks.is_empty());

        // Empty search term should return all tasks
        let query = SearchSessionsQuery::new(Some("".to_string()), None, None, vec![]);
        let tasks: HashSet<_> = store
            .list_sessions(&query)
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
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Search by partial task ID
        let id_prefix = &task_id.as_ref()[..6]; // First 6 characters
        let query = SearchSessionsQuery::new(Some(id_prefix.to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
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
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
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
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
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
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
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
        let missing_id = SessionId::new();
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
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Create a task and update to Running status
        let task2 = spawn_task();
        let (task2_id, _) = store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Running;
        store
            .update_session(&task2_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Create a task and update to Complete status
        let task3 = spawn_task();
        let (task3_id, _) = store
            .add_session(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Complete;
        store
            .update_session(&task3_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Search for "created" should match task1
        let query = SearchSessionsQuery::new(Some("created".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id]);

        // Search for "running" should match task2
        let query = SearchSessionsQuery::new(Some("running".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id]);

        // Search for "complete" should match task3
        let query = SearchSessionsQuery::new(Some("complete".to_string()), None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
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
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Add a task and update to Running status
        let task2 = spawn_task();
        let (task2_id, _) = store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Running;
        store
            .update_session(&task2_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Add a task and update to Complete status
        let task3 = spawn_task();
        let (task3_id, _) = store
            .add_session(task3, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        let mut updated = spawn_task();
        updated.status = Status::Complete;
        store
            .update_session(&task3_id, updated, &ActorRef::test())
            .await
            .unwrap();

        // Filter by Created should return only task1
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Created.into()]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task1_id]);

        // Filter by Running should return only task2
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Running.into()]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task2_id]);

        // Filter by Complete should return only task3
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Complete.into()]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, vec![task3_id]);

        // Filter by Failed should return no tasks
        let query = SearchSessionsQuery::new(None, None, None, vec![Status::Failed.into()]);
        let tasks: Vec<_> = store.list_sessions(&query).await.unwrap();
        assert!(tasks.is_empty());

        // No status filter should return all tasks
        let query = SearchSessionsQuery::new(None, None, None, vec![]);
        let tasks: Vec<_> = store
            .list_sessions(&query)
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

    #[tokio::test]
    async fn message_filter_by_is_read() {
        let store = MemoryStore::new();

        let sender = ActorId::Username(Username::from("alice").into());
        let recipient = ActorId::Username(Username::from("bob").into());

        // Create an unread message (default)
        let msg_unread = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "unread message".into(),
        );
        let (_id_unread, _) = store
            .add_message(msg_unread, &ActorRef::test())
            .await
            .unwrap();

        // Create a message and mark it as read
        let msg = Message::new(
            Some(sender.clone()),
            recipient.clone(),
            "read message".into(),
        );
        let (id_read, _) = store.add_message(msg, &ActorRef::test()).await.unwrap();

        let read_msg = Message {
            sender: Some(sender.clone()),
            recipient: recipient.clone(),
            body: "read message".into(),
            deleted: false,
            is_read: true,
        };
        store
            .update_message(&id_read, read_msg, &ActorRef::test())
            .await
            .unwrap();

        // Filter for read messages only
        let mut query = SearchMessagesQuery::default();
        query.is_read = Some(true);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id_read);
        assert_eq!(results[0].1.item.body, "read message");
        assert!(results[0].1.item.is_read);

        // Filter for unread messages only
        let mut query = SearchMessagesQuery::default();
        query.is_read = Some(false);
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.body, "unread message");
        assert!(!results[0].1.item.is_read);

        // No filter returns all messages
        let query = SearchMessagesQuery::default();
        let results = store.list_messages(&query).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    // ---- Notification tests ----

    fn sample_notification(recipient: ActorId) -> Notification {
        Notification::new(
            recipient,
            Some(ActorId::Username(Username::from("alice").into())),
            "issue".to_string(),
            IssueId::new().into(),
            1,
            "updated".to_string(),
            "Issue status changed".to_string(),
            None,
            "walk_up".to_string(),
        )
    }

    fn make_notif_query(recipient: Option<String>) -> ListNotificationsQuery {
        let mut q = ListNotificationsQuery::default();
        q.recipient = recipient;
        q
    }

    #[tokio::test]
    async fn insert_notification_returns_id() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient);

        let id = store.insert_notification(notif).await.unwrap();
        assert!(
            id.as_ref().starts_with("nf-"),
            "notification id should start with nf-, got: {id}"
        );
    }

    #[tokio::test]
    async fn list_notifications_returns_inserted() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient.clone());

        let id = store.insert_notification(notif).await.unwrap();

        let query = make_notif_query(Some(recipient.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
        assert!(!results[0].1.is_read);
    }

    #[tokio::test]
    async fn list_notifications_filters_by_is_read() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        // Insert two notifications
        let notif1 = sample_notification(recipient.clone());
        let id1 = store.insert_notification(notif1).await.unwrap();
        let notif2 = sample_notification(recipient.clone());
        let _id2 = store.insert_notification(notif2).await.unwrap();

        // Mark one as read
        store.mark_notification_read(&id1).await.unwrap();

        // List unread only
        let mut query = make_notif_query(Some(recipient.to_string()));
        query.is_read = Some(false);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1.is_read);

        // List read only
        let mut query = make_notif_query(Some(recipient.to_string()));
        query.is_read = Some(true);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_read);
    }

    #[tokio::test]
    async fn list_notifications_filters_by_recipient() {
        let store = MemoryStore::new();
        let alice = ActorId::Username(Username::from("alice").into());
        let bob = ActorId::Username(Username::from("bob").into());

        store
            .insert_notification(sample_notification(alice.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(bob.clone()))
            .await
            .unwrap();

        let query = make_notif_query(Some(alice.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.recipient, alice);
    }

    #[tokio::test]
    async fn count_unread_notifications_returns_correct_count() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        // Initially 0
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );

        // Insert 3 notifications
        let id1 = store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            3
        );

        // Mark one as read
        store.mark_notification_read(&id1).await.unwrap();
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn mark_notification_read_updates_notification() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());
        let notif = sample_notification(recipient.clone());
        let id = store.insert_notification(notif).await.unwrap();

        store.mark_notification_read(&id).await.unwrap();

        let query = make_notif_query(Some(recipient.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_read);
    }

    #[tokio::test]
    async fn mark_notification_read_fails_for_unknown_id() {
        let store = MemoryStore::new();
        let unknown_id: NotificationId = "nf-abcdef".parse().unwrap();
        let result = store.mark_notification_read(&unknown_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mark_all_notifications_read_marks_all() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();
        store
            .insert_notification(sample_notification(recipient.clone()))
            .await
            .unwrap();

        let marked = store
            .mark_all_notifications_read(&recipient, None)
            .await
            .unwrap();
        assert_eq!(marked, 3);

        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            0
        );

        // Calling again returns 0 (all already read)
        let marked = store
            .mark_all_notifications_read(&recipient, None)
            .await
            .unwrap();
        assert_eq!(marked, 0);
    }

    #[tokio::test]
    async fn mark_all_notifications_read_with_before_filter() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        // Insert notification with known created_at
        let mut notif1 = sample_notification(recipient.clone());
        notif1.created_at = Utc::now() - Duration::hours(2);
        store.insert_notification(notif1).await.unwrap();

        let mut notif2 = sample_notification(recipient.clone());
        notif2.created_at = Utc::now() - Duration::hours(1);
        store.insert_notification(notif2).await.unwrap();

        let notif3 = sample_notification(recipient.clone());
        store.insert_notification(notif3).await.unwrap();

        // Mark only notifications before 30 minutes ago
        let cutoff = Utc::now() - Duration::minutes(30);
        let marked = store
            .mark_all_notifications_read(&recipient, Some(cutoff))
            .await
            .unwrap();
        assert_eq!(marked, 2);

        // One still unread (the most recent)
        assert_eq!(
            store.count_unread_notifications(&recipient).await.unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn list_notifications_respects_limit() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        for _ in 0..5 {
            store
                .insert_notification(sample_notification(recipient.clone()))
                .await
                .unwrap();
        }

        let mut query = make_notif_query(Some(recipient.to_string()));
        query.limit = Some(3);
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn list_notifications_sorted_by_created_at_desc() {
        let store = MemoryStore::new();
        let recipient = ActorId::Username(Username::from("bob").into());

        let mut notif1 = sample_notification(recipient.clone());
        notif1.created_at = Utc::now() - Duration::hours(2);
        notif1.summary = "oldest".to_string();
        store.insert_notification(notif1).await.unwrap();

        let mut notif2 = sample_notification(recipient.clone());
        notif2.created_at = Utc::now() - Duration::hours(1);
        notif2.summary = "middle".to_string();
        store.insert_notification(notif2).await.unwrap();

        let mut notif3 = sample_notification(recipient.clone());
        notif3.created_at = Utc::now();
        notif3.summary = "newest".to_string();
        store.insert_notification(notif3).await.unwrap();

        let query = make_notif_query(Some(recipient.to_string()));
        let results = store.list_notifications(&query).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].1.summary, "newest");
        assert_eq!(results[1].1.summary, "middle");
        assert_eq!(results[2].1.summary, "oldest");
    }

    // ---- Label tests ----

    fn sample_label(name: &str, color: &str) -> Label {
        Label::new(name.to_string(), color.parse().unwrap(), true, false)
    }

    #[tokio::test]
    async fn label_crud_round_trip() {
        let store = MemoryStore::new();

        // CREATE
        let label = sample_label("bug", "#e74c3c");
        let label_id = store.add_label(label.clone()).await.unwrap();

        // READ
        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "bug");
        assert_eq!(fetched.color.as_ref(), "#e74c3c");
        assert!(!fetched.deleted);

        // LIST
        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, label_id);
        assert_eq!(results[0].1.name, "bug");

        // GET BY NAME
        let found = store.get_label_by_name("bug").await.unwrap();
        assert!(found.is_some());
        let (found_id, found_label) = found.unwrap();
        assert_eq!(found_id, label_id);
        assert_eq!(found_label.name, "bug");
    }

    #[tokio::test]
    async fn add_label_rejects_duplicates() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let err = store
            .add_label(sample_label("bug", "#3498db"))
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StoreError::LabelAlreadyExists(name) if name == "bug"
        ));
    }

    #[tokio::test]
    async fn delete_label_soft_deletes() {
        let store = MemoryStore::new();

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        // Delete (soft delete)
        store.delete_label(&label_id).await.unwrap();

        // get_label returns not found for soft-deleted labels
        let err = store.get_label(&label_id).await.unwrap_err();
        assert!(matches!(err, StoreError::LabelNotFound(_)));

        // list_labels excludes deleted labels by default
        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert!(results.is_empty());

        // list_labels with include_deleted returns soft-deleted labels
        let mut query = SearchLabelsQuery::default();
        query.include_deleted = Some(true);
        let results = store.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.deleted);
    }

    #[tokio::test]
    async fn update_label_changes_name_and_color() {
        let store = MemoryStore::new();

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        let mut updated = store.get_label(&label_id).await.unwrap();
        updated.name = "defect".to_string();
        updated.color = "#3498db".parse().unwrap();
        updated.updated_at = Utc::now();
        store.update_label(&label_id, updated).await.unwrap();

        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "defect");
        assert_eq!(fetched.color.as_ref(), "#3498db");
    }

    #[tokio::test]
    async fn update_label_rejects_name_collision() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();
        let feature_id = store
            .add_label(sample_label("feature", "#3498db"))
            .await
            .unwrap();

        // Try to rename "feature" to "bug" — should fail
        let mut updated = store.get_label(&feature_id).await.unwrap();
        updated.name = "bug".to_string();
        let err = store.update_label(&feature_id, updated).await.unwrap_err();

        assert!(matches!(
            err,
            StoreError::LabelAlreadyExists(name) if name == "bug"
        ));
    }

    #[tokio::test]
    async fn update_label_allows_same_name() {
        let store = MemoryStore::new();

        let label_id = store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        // Update with same name but different color — should succeed
        let mut updated = store.get_label(&label_id).await.unwrap();
        updated.color = "#3498db".parse().unwrap();
        store.update_label(&label_id, updated).await.unwrap();

        let fetched = store.get_label(&label_id).await.unwrap();
        assert_eq!(fetched.name, "bug");
        assert_eq!(fetched.color.as_ref(), "#3498db");
    }

    #[tokio::test]
    async fn get_label_by_name_case_insensitive() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();

        // Search with different casing
        let found = store.get_label_by_name("BUG").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().1.name, "bug");
    }

    #[tokio::test]
    async fn list_labels_filters_by_query() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("bug", "#e74c3c"))
            .await
            .unwrap();
        store
            .add_label(sample_label("feature", "#3498db"))
            .await
            .unwrap();
        store
            .add_label(sample_label("bugfix", "#2ecc71"))
            .await
            .unwrap();

        let mut query = SearchLabelsQuery::default();
        query.q = Some("bug".to_string());
        let results = store.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        // Results sorted by name (no pagination params)
        assert_eq!(results[0].1.name, "bug");
        assert_eq!(results[1].1.name, "bugfix");
    }

    #[tokio::test]
    async fn list_labels_sorted_by_name() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("zebra", "#000000"))
            .await
            .unwrap();
        store
            .add_label(sample_label("alpha", "#111111"))
            .await
            .unwrap();
        store
            .add_label(sample_label("middle", "#222222"))
            .await
            .unwrap();

        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        // Without pagination params, sorted alphabetically by name
        assert_eq!(results[0].1.name, "alpha");
        assert_eq!(results[1].1.name, "middle");
        assert_eq!(results[2].1.name, "zebra");
    }

    // ---- Pagination integration tests ----

    #[tokio::test]
    async fn issues_pagination_returns_correct_pages() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        // Create 5 issues with delays for distinct timestamps
        for _ in 0..5 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Page 1: limit=2, store returns limit+1 items
        let mut query = SearchIssuesQuery::default();
        query.limit = Some(2);
        let page1 = store.list_issues(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        let cursor = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        // Page 2: use cursor, limit=2
        let mut query2 = SearchIssuesQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_issues(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page2[1].1.timestamp,
            id: page2[1].0.to_string(),
        }
        .encode();

        // Page 3: only 1 item remaining (no extra = last page)
        let mut query3 = SearchIssuesQuery::default();
        query3.limit = Some(2);
        query3.cursor = Some(cursor2);
        let page3 = store.list_issues(&query3).await.unwrap();
        assert_eq!(page3.len(), 1);

        // No overlap: collect all IDs across pages
        let all_ids: Vec<_> = page1[..2]
            .iter()
            .chain(page2[..2].iter())
            .chain(page3.iter())
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(all_ids.len(), 5);
        let unique: HashSet<_> = all_ids.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn issues_pagination_without_limit_returns_all() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..3 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        }

        let results = store
            .list_issues(&SearchIssuesQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn patches_pagination_returns_correct_pages() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..5 {
            store.add_patch(sample_patch(), &actor).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut query = SearchPatchesQuery::default();
        query.limit = Some(2);
        let page1 = store.list_patches(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        let cursor = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchPatchesQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_patches(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page2[1].1.timestamp,
            id: page2[1].0.to_string(),
        }
        .encode();

        let mut query3 = SearchPatchesQuery::default();
        query3.limit = Some(2);
        query3.cursor = Some(cursor2);
        let page3 = store.list_patches(&query3).await.unwrap();
        assert_eq!(page3.len(), 1);

        let all_ids: Vec<_> = page1[..2]
            .iter()
            .chain(page2[..2].iter())
            .chain(page3.iter())
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(all_ids.len(), 5);
        let unique: HashSet<_> = all_ids.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn documents_pagination_returns_correct_pages() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for i in 0..5 {
            store
                .add_document(sample_document(Some(&format!("doc_{i}.md")), None), &actor)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut query = SearchDocumentsQuery::default();
        query.limit = Some(2);
        let page1 = store.list_documents(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        let cursor = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchDocumentsQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_documents(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page2[1].1.timestamp,
            id: page2[1].0.to_string(),
        }
        .encode();

        let mut query3 = SearchDocumentsQuery::default();
        query3.limit = Some(2);
        query3.cursor = Some(cursor2);
        let page3 = store.list_documents(&query3).await.unwrap();
        assert_eq!(page3.len(), 1);

        let all_ids: Vec<_> = page1[..2]
            .iter()
            .chain(page2[..2].iter())
            .chain(page3.iter())
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(all_ids.len(), 5);
        let unique: HashSet<_> = all_ids.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn jobs_pagination_returns_correct_pages() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..5 {
            store
                .add_session(spawn_task(), Utc::now(), &actor)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut query = SearchSessionsQuery::default();
        query.limit = Some(2);
        let page1 = store.list_sessions(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        let cursor = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchSessionsQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_sessions(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page2[1].1.timestamp,
            id: page2[1].0.to_string(),
        }
        .encode();

        let mut query3 = SearchSessionsQuery::default();
        query3.limit = Some(2);
        query3.cursor = Some(cursor2);
        let page3 = store.list_sessions(&query3).await.unwrap();
        assert_eq!(page3.len(), 1);

        let all_ids: Vec<_> = page1[..2]
            .iter()
            .chain(page2[..2].iter())
            .chain(page3.iter())
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(all_ids.len(), 5);
        let unique: HashSet<_> = all_ids.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn labels_pagination_returns_correct_pages() {
        let store = MemoryStore::new();

        // Create 5 labels with delays to ensure distinct millisecond timestamps
        let names = ["alpha", "bravo", "charlie", "delta", "echo"];
        for name in &names {
            store
                .add_label(sample_label(name, "#000000"))
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Page 1: limit=2, should get 2+1 items (limit+1 pattern)
        let mut query = SearchLabelsQuery::default();
        query.limit = Some(2);
        let page1 = store.list_labels(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        // Simulate what the route handler does: truncate to limit and encode cursor
        let cursor = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.updated_at,
            id: page1[1].0.to_string(),
        }
        .encode();

        // Page 2: use cursor, limit=2
        let mut query2 = SearchLabelsQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_labels(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = metis_common::api::v1::pagination::DecodedCursor {
            timestamp: page2[1].1.updated_at,
            id: page2[1].0.to_string(),
        }
        .encode();

        // Page 3: use cursor2, limit=2
        let mut query3 = SearchLabelsQuery::default();
        query3.limit = Some(2);
        query3.cursor = Some(cursor2);
        let page3 = store.list_labels(&query3).await.unwrap();
        // Only 1 item remaining (no extra item = last page)
        assert_eq!(page3.len(), 1);

        // Verify no overlap: collect all names across pages
        let all_names: Vec<String> = page1[..2]
            .iter()
            .chain(page2[..2].iter())
            .chain(page3.iter())
            .map(|(_, l)| l.name.clone())
            .collect();
        assert_eq!(all_names.len(), 5);
        let unique: std::collections::HashSet<_> = all_names.iter().collect();
        assert_eq!(unique.len(), 5);
    }

    #[tokio::test]
    async fn labels_pagination_without_limit_returns_all() {
        let store = MemoryStore::new();

        for name in &["alpha", "bravo", "charlie"] {
            store
                .add_label(sample_label(name, "#000000"))
                .await
                .unwrap();
        }

        // No limit = all results, backward compat
        let results = store
            .list_labels(&SearchLabelsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        // Sorted alphabetically by name when no pagination
        assert_eq!(results[0].1.name, "alpha");
        assert_eq!(results[1].1.name, "bravo");
        assert_eq!(results[2].1.name, "charlie");
    }

    #[tokio::test]
    async fn labels_pagination_with_limit_sorts_by_updated_at() {
        let store = MemoryStore::new();

        store
            .add_label(sample_label("zebra", "#000000"))
            .await
            .unwrap();
        store
            .add_label(sample_label("alpha", "#111111"))
            .await
            .unwrap();

        let mut query = SearchLabelsQuery::default();
        query.limit = Some(10);
        let results = store.list_labels(&query).await.unwrap();
        assert_eq!(results.len(), 2);
        // With pagination, sorted by updated_at DESC (most recently created first)
        assert_eq!(results[0].1.name, "alpha");
        assert_eq!(results[1].1.name, "zebra");
    }

    // ---- Agent tests ----

    fn sample_agent(name: &str) -> Agent {
        Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            3,
            i32::MAX,
            false,
        )
    }

    #[tokio::test]
    async fn add_and_get_agent() {
        let store = MemoryStore::new();
        let agent = sample_agent("swe");

        store.add_agent(agent.clone()).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.name, "swe");
        assert_eq!(fetched.prompt_path, "/agents/swe/prompt.md");
        assert_eq!(fetched.max_tries, 3);
        assert!(!fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn add_agent_duplicate_returns_error() {
        let store = MemoryStore::new();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let err = store.add_agent(sample_agent("swe")).await.unwrap_err();
        assert!(matches!(err, StoreError::AgentAlreadyExists(_)));
    }

    #[tokio::test]
    async fn list_agents_excludes_deleted() {
        let store = MemoryStore::new();
        store.add_agent(sample_agent("alpha")).await.unwrap();
        store.add_agent(sample_agent("beta")).await.unwrap();
        store.delete_agent("alpha").await.unwrap();

        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "beta");
    }

    #[tokio::test]
    async fn list_agents_sorted_by_name() {
        let store = MemoryStore::new();
        store.add_agent(sample_agent("zebra")).await.unwrap();
        store.add_agent(sample_agent("alpha")).await.unwrap();

        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents[0].name, "alpha");
        assert_eq!(agents[1].name, "zebra");
    }

    #[tokio::test]
    async fn update_agent_changes_fields() {
        let store = MemoryStore::new();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut updated = sample_agent("swe");
        updated.max_tries = 5;
        updated.prompt_path = "/agents/swe/v2.md".to_string();
        store.update_agent(updated).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.max_tries, 5);
        assert_eq!(fetched.prompt_path, "/agents/swe/v2.md");
    }

    #[tokio::test]
    async fn update_nonexistent_agent_returns_error() {
        let store = MemoryStore::new();
        let err = store
            .update_agent(sample_agent("missing"))
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn delete_agent_soft_deletes() {
        let store = MemoryStore::new();
        store.add_agent(sample_agent("swe")).await.unwrap();
        store.delete_agent("swe").await.unwrap();

        let err = store.get_agent("swe").await.unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn delete_nonexistent_agent_returns_error() {
        let store = MemoryStore::new();
        let err = store.delete_agent("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn assignment_agent_uniqueness_on_add() {
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        let err = store.add_agent(pm2).await.unwrap_err();
        assert!(matches!(err, StoreError::AssignmentAgentAlreadyExists));
    }

    #[tokio::test]
    async fn assignment_agent_uniqueness_on_update() {
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_assignment_agent = true;
        let err = store.update_agent(swe_updated).await.unwrap_err();
        assert!(matches!(err, StoreError::AssignmentAgentAlreadyExists));
    }

    #[tokio::test]
    async fn assignment_agent_can_update_itself() {
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        let mut pm_updated = sample_agent("pm");
        pm_updated.is_assignment_agent = true;
        pm_updated.max_tries = 10;
        store.update_agent(pm_updated).await.unwrap();

        let fetched = store.get_agent("pm").await.unwrap();
        assert_eq!(fetched.max_tries, 10);
        assert!(fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn deleted_assignment_agent_allows_new_one() {
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.delete_agent("pm").await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        store.add_agent(pm2).await.unwrap();

        let fetched = store.get_agent("pm2").await.unwrap();
        assert!(fetched.is_assignment_agent);
    }

    #[tokio::test]
    async fn add_agent_after_soft_deletion_same_name() {
        let store = MemoryStore::new();
        let agent = sample_agent("swe");
        store.add_agent(agent).await.unwrap();
        store.delete_agent("swe").await.unwrap();

        // Re-creating with the same name should succeed.
        let mut agent2 = sample_agent("swe");
        agent2.prompt_path = "new/path".to_string();
        store.add_agent(agent2).await.unwrap();

        let fetched = store.get_agent("swe").await.unwrap();
        assert_eq!(fetched.prompt_path, "new/path");
        assert!(!fetched.deleted);
    }

    // --- count_* method tests ---

    #[tokio::test]
    async fn count_issues_returns_total_matching() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        // Create 5 issues: 3 open tasks, 1 open bug, 1 closed task
        for _ in 0..3 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
        }
        let bug = Issue::new(
            IssueType::Bug,
            "Bug Title".to_string(),
            "a bug".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        store.add_issue(bug, &actor).await.unwrap();

        let closed = Issue::new(
            IssueType::Task,
            "Closed".to_string(),
            "closed task".to_string(),
            Username::from("creator"),
            String::new(),
            IssueStatus::Closed,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        store.add_issue(closed, &actor).await.unwrap();

        // Count all issues
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);

        // Count only bugs
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            Some(metis_common::api::v1::issues::IssueType::Bug),
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);

        // Count only closed
        let query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            Some(metis_common::api::v1::issues::IssueStatus::Closed),
            None,
            None,
            Vec::new(),
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn count_patches_returns_total_matching() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..3 {
            store.add_patch(sample_patch(), &actor).await.unwrap();
        }

        let query =
            metis_common::api::v1::patches::SearchPatchesQuery::new(None, None, Vec::new(), None);
        assert_eq!(store.count_patches(&query).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn count_documents_returns_total_matching() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        store
            .add_document(sample_document(Some("docs/a.md"), None), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("docs/b.md"), None), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("other/c.md"), None), &actor)
            .await
            .unwrap();

        // Count all
        let query = metis_common::api::v1::documents::SearchDocumentsQuery::new(
            None, None, None, None, None,
        );
        assert_eq!(store.count_documents(&query).await.unwrap(), 3);

        // Count with path prefix filter
        let query = metis_common::api::v1::documents::SearchDocumentsQuery::new(
            Some("docs/".to_string()),
            None,
            None,
            None,
            None,
        );
        assert_eq!(store.count_documents(&query).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn count_tasks_returns_total_matching() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..4 {
            store
                .add_session(spawn_task(), Utc::now(), &actor)
                .await
                .unwrap();
        }

        let query =
            metis_common::api::v1::sessions::SearchSessionsQuery::new(None, None, None, vec![]);
        assert_eq!(store.count_sessions(&query).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn count_labels_returns_total_matching() {
        use crate::domain::labels::Label as DomainLabel;
        use metis_common::Rgb;

        let store = MemoryStore::new();
        let default_color: Rgb = "#000000".parse().unwrap();

        store
            .add_label(DomainLabel::new(
                "bug".to_string(),
                default_color.clone(),
                true,
                false,
            ))
            .await
            .unwrap();
        store
            .add_label(DomainLabel::new(
                "feature".to_string(),
                default_color.clone(),
                true,
                false,
            ))
            .await
            .unwrap();
        store
            .add_label(DomainLabel::new(
                "bugfix".to_string(),
                default_color,
                true,
                false,
            ))
            .await
            .unwrap();

        // Count all
        let query = metis_common::api::v1::labels::SearchLabelsQuery::default();
        assert_eq!(store.count_labels(&query).await.unwrap(), 3);

        // Count with search filter
        let mut query = metis_common::api::v1::labels::SearchLabelsQuery::default();
        query.q = Some("bug".to_string());
        assert_eq!(store.count_labels(&query).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn count_issues_ignores_pagination() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        for _ in 0..5 {
            store.add_issue(sample_issue(vec![]), &actor).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Count should return 5 even when limit is set
        let mut query = metis_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
        );
        query.limit = Some(2);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);
    }

    #[tokio::test]
    async fn has_document_relationship_type_round_trips() {
        use crate::store::RelationshipType;
        use std::str::FromStr;

        let rt = RelationshipType::HasDocument;
        assert_eq!(rt.as_str(), "has-document");
        assert_eq!(rt.to_string(), "has-document");

        assert_eq!(
            RelationshipType::from_str("has-document").unwrap(),
            RelationshipType::HasDocument
        );
        assert_eq!(
            RelationshipType::from_str("has_document").unwrap(),
            RelationshipType::HasDocument
        );
        assert_eq!(
            RelationshipType::from_str("hasDocument").unwrap(),
            RelationshipType::HasDocument
        );
    }

    #[tokio::test]
    async fn get_relationships_batch_filters_by_multiple_sources() {
        use crate::store::RelationshipType;

        let store = MemoryStore::new();
        let actor_ref = ActorRef::test();

        let (id1, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (id2, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (id3, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor_ref).await.unwrap();

        let sid1 = MetisId::from(id1.clone());
        let sid2 = MetisId::from(id2.clone());
        let sid3 = MetisId::from(id3.clone());
        let tpid = MetisId::from(pid.clone());

        store
            .add_relationship(&sid1, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();
        store
            .add_relationship(&sid2, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();
        store
            .add_relationship(&sid3, &tpid, RelationshipType::HasPatch)
            .await
            .unwrap();

        // Batch query for id1 and id2 only
        let results = store
            .get_relationships_batch(
                Some(&[sid1.clone(), sid2.clone()]),
                None,
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Empty source_ids returns empty
        let results = store
            .get_relationships_batch(Some(&[]), None, None)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn get_relationships_transitive_follows_same_type_only() {
        use crate::store::RelationshipType;

        let store = MemoryStore::new();
        let actor_ref = ActorRef::test();

        // Create 3 issues: A -> B -> C (child-of chain)
        // Also B -> patch (has-patch, should NOT be followed)
        let (id_a, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (id_b, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (id_c, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (pid, _) = store.add_patch(sample_patch(), &actor_ref).await.unwrap();

        let a = MetisId::from(id_a.clone());
        let b = MetisId::from(id_b.clone());
        let c = MetisId::from(id_c.clone());
        let p = MetisId::from(pid.clone());

        // A is child-of B, B is child-of C
        store
            .add_relationship(&a, &b, RelationshipType::ChildOf)
            .await
            .unwrap();
        store
            .add_relationship(&b, &c, RelationshipType::ChildOf)
            .await
            .unwrap();
        // B has-patch P (different rel_type)
        store
            .add_relationship(&b, &p, RelationshipType::HasPatch)
            .await
            .unwrap();

        // Forward transitive from A following child-of
        let results = store
            .get_relationships_transitive(Some(&a), None, RelationshipType::ChildOf)
            .await
            .unwrap();
        // Should find A->B and B->C, but NOT B->P
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| r.rel_type == RelationshipType::ChildOf)
        );

        // Backward transitive from C following child-of
        let results = store
            .get_relationships_transitive(None, Some(&c), RelationshipType::ChildOf)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Transitive has-patch from B should only find B->P
        let results = store
            .get_relationships_transitive(Some(&b), None, RelationshipType::HasPatch)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, p);
    }
}
