use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    ConversationEventSummary, ReadOnlyStore, Session, SessionEvent, SessionEventSummary, Status,
    Store, StoreError, TaskStatusLog,
};
use crate::domain::conversations::{Conversation, ConversationEvent};
use crate::domain::{
    actors::{Actor, ActorRef},
    agents::Agent,
    documents::Document,
    issues::{Issue, IssueDependency, IssueStatus, IssueType},
    labels::Label,
    patches::Patch,
    secrets::SecretRef,
    users::{User, Username},
};
use hydra_common::api::v1::conversations::SearchConversationsQuery;
use hydra_common::api::v1::documents::SearchDocumentsQuery;
use hydra_common::api::v1::issues::SearchIssuesQuery;
use hydra_common::api::v1::pagination::{DecodedCursor, MAX_LIMIT as PAGINATION_MAX_LIMIT};
use hydra_common::api::v1::patches::SearchPatchesQuery;
use hydra_common::api::v1::sessions::SearchSessionsQuery;
use hydra_common::api::v1::users::SearchUsersQuery;
use hydra_common::{
    ConversationEventId, ConversationId, DocumentId, HydraId, IssueId, LabelId, PatchId, RepoName,
    SessionId, VersionNumber, Versioned,
    api::v1::labels::{LabelSummary, SearchLabelsQuery},
    ids::random_len_for_count,
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
    /// Maps agent names to their Agent data (non-versioned)
    agents: DashMap<String, Agent>,
    /// Maps label IDs to their Label data (non-versioned)
    labels: DashMap<LabelId, Label>,
    /// Maps object IDs to associated label IDs
    object_labels: DashMap<HydraId, HashSet<LabelId>>,
    /// Maps label IDs to associated object IDs
    label_objects: DashMap<LabelId, HashSet<HydraId>>,
    /// Maps actor_name to list of token hashes
    auth_tokens: DashMap<String, Vec<String>>,
    /// Maps (username, secret_name, internal) to encrypted_value
    user_secrets: DashMap<(Username, String, bool), Vec<u8>>,
    /// Stores object relationships as (source_id, rel_type, target_id) -> ObjectRelationship
    object_relationships:
        DashMap<(HydraId, super::RelationshipType, HydraId), super::ObjectRelationship>,
    /// Maps conversation IDs to their versioned Conversation data
    conversations: DashMap<ConversationId, Vec<Versioned<Conversation>>>,
    /// Maps conversation IDs to their versioned events
    conversation_events: DashMap<ConversationId, Vec<Versioned<ConversationEvent>>>,
    /// Maps session IDs to their versioned session events. Each entry pairs
    /// the event with the monotonic `next_session_event_seq` value assigned
    /// at append time, which serves as the global ordering primitive used by
    /// the conversation read path fan-out merge (see design doc §3.4.1).
    session_events: DashMap<SessionId, Vec<(u64, Versioned<SessionEvent>)>>,
    /// Maps session IDs to their opaque session-state blobs.
    session_state: DashMap<SessionId, Vec<u8>>,
    /// Monotonic counter stamped on every appended session event, providing
    /// a process-wide insertion order across sessions. Mirrors the postgres
    /// BIGSERIAL / sqlite ROWID used by the SQL backends for the same purpose.
    next_session_event_seq: AtomicU64,
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
            agents: DashMap::new(),
            labels: DashMap::new(),
            object_labels: DashMap::new(),
            label_objects: DashMap::new(),
            auth_tokens: DashMap::new(),
            user_secrets: DashMap::new(),
            object_relationships: DashMap::new(),
            conversations: DashMap::new(),
            conversation_events: DashMap::new(),
            session_events: DashMap::new(),
            session_state: DashMap::new(),
            next_session_event_seq: AtomicU64::new(1),
        }
    }

    fn next_issue_id(&self) -> IssueId {
        let len = random_len_for_count(self.issues.len() as u64);
        IssueId::generate(len).expect("length within bounds")
    }

    fn next_patch_id(&self) -> PatchId {
        let len = random_len_for_count(self.patches.len() as u64);
        PatchId::generate(len).expect("length within bounds")
    }

    fn next_document_id(&self) -> DocumentId {
        let len = random_len_for_count(self.documents.len() as u64);
        DocumentId::generate(len).expect("length within bounds")
    }

    fn next_session_id(&self) -> SessionId {
        let len = random_len_for_count(self.tasks.len() as u64);
        SessionId::generate(len).expect("length within bounds")
    }

    fn next_label_id(&self) -> LabelId {
        let count = self
            .labels
            .iter()
            .filter(|entry| !entry.value().deleted)
            .count() as u64;
        let len = random_len_for_count(count);
        LabelId::generate(len).expect("length within bounds")
    }

    fn next_conversation_id(&self) -> ConversationId {
        let len = random_len_for_count(self.conversations.len() as u64);
        ConversationId::generate(len).expect("length within bounds")
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
        let ids_filter = &query.ids;
        let issue_type_filter: Option<IssueType> = query.issue_type.map(Into::into);
        let status_filter: Vec<IssueStatus> =
            query.status.iter().copied().map(Into::into).collect();
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
        let creator_filter = query
            .creator
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());

        self.issues.iter().filter_map(move |entry| {
            let latest = Self::latest_versioned(entry.value())?;
            if !include_deleted && latest.item.deleted {
                return None;
            }
            let issue_id = entry.key();

            // When `ids` is provided, filter by ID (intersected with other filters).
            if !ids_filter.is_empty() && !ids_filter.contains(issue_id) {
                return None;
            }

            if let Some(expected_creator) = creator_filter {
                if !latest
                    .item
                    .creator
                    .as_ref()
                    .eq_ignore_ascii_case(expected_creator)
                {
                    return None;
                }
            }

            if !issue_matches(
                issue_type_filter,
                &status_filter,
                search_term.as_deref(),
                assignee_filter,
                issue_id,
                &latest.item,
            ) {
                return None;
            }
            if !query.label_ids.is_empty() {
                let object_id = HydraId::from(issue_id.clone());
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
            if !query.ids.is_empty() && !query.ids.contains(entry.key()) {
                return None;
            }
            if !include_deleted && latest.item.deleted {
                return None;
            }
            if !status_filter.is_empty() && !status_filter.contains(&latest.item.status) {
                return None;
            }
            if let Some(branch) = query
                .branch_name
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                if latest.item.branch_name.as_deref() != Some(branch) {
                    return None;
                }
            }
            if let Some(repo_name) = query
                .repo_name
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                if latest.item.service_repo_name.as_str() != repo_name {
                    return None;
                }
            }
            if let Some(creator) = query
                .creator
                .as_ref()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
            {
                if latest.item.creator.as_str().to_lowercase() != creator.to_lowercase() {
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

        // When `ids` is provided, filter by ID (intersected with other filters).
        if !query.ids.is_empty() {
            documents.retain(|(id, _)| query.ids.contains(id));
        }

        if !query.include_deleted.unwrap_or(false) {
            documents.retain(|(_, versioned)| !versioned.item.deleted);
        }

        if let Some(has_path) = query.has_path {
            documents.retain(|(_, versioned)| versioned.item.path.is_some() == has_path);
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

            if !query.spawned_from_ids.is_empty() {
                match latest.item.spawned_from.as_ref() {
                    Some(spawned) if query.spawned_from_ids.contains(spawned) => {}
                    _ => return None,
                }
            }

            if let Some(expected_creator) = query.creator.as_deref() {
                if latest.item.creator.as_ref() != expected_creator {
                    return None;
                }
            }

            if let Some(expected_conversation) = query.conversation_id.as_ref() {
                if latest.item.conversation_id() != Some(expected_conversation) {
                    return None;
                }
            }

            if !query.status.is_empty() {
                let status_filter: Vec<Status> = query
                    .status
                    .iter()
                    .copied()
                    .filter_map(|s| s.try_into().ok())
                    .collect();
                if !status_filter.contains(&latest.item.status) {
                    return None;
                }
            }

            if let Some(term) = search_term.as_deref() {
                let matches_id = task_id.as_ref().to_lowercase().contains(term);
                let matches_prompt = latest
                    .item
                    .mode
                    .prompt_for_legacy_wire()
                    .to_lowercase()
                    .contains(term);
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
        let source_id = HydraId::from(issue_id.clone());

        // Remove only the rel_types managed by this function. Other
        // rel_types (e.g. has-document) are owned by other code paths and
        // must not be stomped by issue updates.
        let managed = |rel: super::RelationshipType| {
            matches!(
                rel,
                super::RelationshipType::ChildOf
                    | super::RelationshipType::BlockedOn
                    | super::RelationshipType::HasPatch
            )
        };
        let keys_to_remove: Vec<_> = self
            .object_relationships
            .iter()
            .filter(|entry| entry.key().0 == source_id && managed(entry.key().1))
            .map(|entry| entry.key().clone())
            .collect();
        for key in keys_to_remove {
            self.object_relationships.remove(&key);
        }

        // Insert dependency relationships
        for dep in &issue.dependencies {
            let target_id = HydraId::from(dep.issue_id.clone());
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
            let target_id = HydraId::from(patch_id.clone());
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
        target_id: &HydraId,
        rel_type: super::RelationshipType,
    ) -> Vec<HydraId> {
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
        let normalized_needle = query
            .remote_url
            .as_deref()
            .map(Repository::normalize_remote_url);
        let mut repositories: Vec<_> = self
            .repositories
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                // Skip deleted unless include_deleted
                if !include_deleted && latest.item.deleted {
                    return None;
                }
                if let Some(needle) = normalized_needle.as_deref()
                    && Repository::normalize_remote_url(&latest.item.remote_url) != needle
                {
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

    async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        if !self.issues.contains_key(issue_id) {
            return Err(StoreError::IssueNotFound(issue_id.clone()));
        }
        let target_id = HydraId::from(issue_id.clone());
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
        let target_id = HydraId::from(issue_id.clone());
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
        let target_id = HydraId::from(patch_id.clone());
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

    async fn find_non_deleted_document_by_exact_path(
        &self,
        path: &str,
    ) -> Result<Option<DocumentId>, StoreError> {
        let ids = self.document_ids_with_exact_path(path);
        for id in ids {
            if let Some(entry) = self.documents.get(&id) {
                if let Some(latest) = Self::latest_versioned(entry.value()) {
                    if !latest.item.deleted {
                        return Ok(Some(id));
                    }
                }
            }
        }
        Ok(None)
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

    async fn get_documents_by_paths(
        &self,
        paths: &[String],
    ) -> Result<Vec<(String, DocumentId, String)>, StoreError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: std::collections::HashSet<&str> = paths.iter().map(|p| p.as_str()).collect();
        let mut results: Vec<(String, DocumentId, String)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for entry in self.documents.iter() {
            let versions = entry.value();
            let Some(latest) = Self::latest_versioned(versions) else {
                continue;
            };
            if latest.item.deleted {
                continue;
            }
            let Some(ref path) = latest.item.path else {
                continue;
            };
            let path_str: &str = path.as_ref();
            if !wanted.contains(path_str) {
                continue;
            }
            if !seen.insert(path_str.to_string()) {
                continue;
            }
            results.push((
                path_str.to_string(),
                entry.key().clone(),
                latest.item.title.clone(),
            ));
        }
        Ok(results)
    }

    async fn list_document_path_children(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, String, u64, bool)>, StoreError> {
        let prefix = if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{prefix}/")
        };

        let mut segment_counts: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        let mut segment_is_doc: std::collections::BTreeMap<String, bool> =
            std::collections::BTreeMap::new();

        for entry in self.documents.iter() {
            let versions = entry.value();
            if let Some(latest) = Self::latest_versioned(versions) {
                if latest.item.deleted {
                    continue;
                }
                if let Some(ref path) = latest.item.path {
                    let path_str: &str = path.as_ref();
                    if let Some(rest) = path_str.strip_prefix(prefix.as_str()) {
                        if rest.is_empty() {
                            continue;
                        }
                        let segment = match rest.find('/') {
                            Some(idx) => &rest[..idx],
                            None => rest,
                        };
                        *segment_counts.entry(segment.to_string()).or_insert(0) += 1;
                        let full_path = format!("{prefix}{segment}");
                        if path_str == full_path {
                            segment_is_doc.insert(segment.to_string(), true);
                        }
                    }
                }
            }
        }

        Ok(segment_counts
            .into_iter()
            .map(|(segment, count)| {
                let full_path = format!("{prefix}{segment}");
                let is_document = segment_is_doc.get(&segment).copied().unwrap_or(false);
                (segment, full_path, count, is_document)
            })
            .collect())
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
        object_id: &HydraId,
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
        object_ids: &[HydraId],
    ) -> Result<HashMap<HydraId, Vec<LabelSummary>>, StoreError> {
        let mut result: HashMap<HydraId, Vec<LabelSummary>> = HashMap::new();
        for object_id in object_ids {
            let labels = self.get_labels_for_object(object_id).await?;
            if !labels.is_empty() {
                result.insert(object_id.clone(), labels);
            }
        }
        Ok(result)
    }

    async fn get_objects_for_label(&self, label_id: &LabelId) -> Result<Vec<HydraId>, StoreError> {
        match self.label_objects.get(label_id) {
            Some(ids) => Ok(ids.iter().cloned().collect()),
            None => Ok(Vec::new()),
        }
    }

    // ---- Object relationships (read-only) ----

    async fn get_relationships(
        &self,
        source_id: Option<&HydraId>,
        target_id: Option<&HydraId>,
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
        source_ids: Option<&[HydraId]>,
        target_ids: Option<&[HydraId]>,
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
        ids: &[HydraId],
        direction: super::TransitiveDirection,
        rel_type: super::RelationshipType,
    ) -> Result<Vec<super::ObjectRelationship>, StoreError> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();

        for start in ids {
            if visited.insert(start.clone()) {
                queue.push_back(start.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            for entry in self.object_relationships.iter() {
                let rel = entry.value();
                if rel.rel_type != rel_type {
                    continue;
                }
                let (match_field, next_field) = match direction {
                    super::TransitiveDirection::Forward => (&rel.source_id, &rel.target_id),
                    super::TransitiveDirection::Backward => (&rel.target_id, &rel.source_id),
                };
                if *match_field == current {
                    result.push(rel.clone());
                    if visited.insert(next_field.clone()) {
                        queue.push_back(next_field.clone());
                    }
                }
            }
        }

        Ok(result)
    }

    // ---- Auth tokens (read-only) ----

    async fn get_auth_token_hashes(&self, actor_name: &str) -> Result<Vec<String>, StoreError> {
        Ok(self
            .auth_tokens
            .get(actor_name)
            .map(|v| v.value().clone())
            .unwrap_or_default())
    }

    // ---- User secrets (read-only) ----

    async fn get_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        // Prefer external (internal=false) over internal (internal=true)
        let external_key = (username.clone(), secret_name.to_string(), false);
        if let Some(v) = self.user_secrets.get(&external_key) {
            return Ok(Some(v.value().clone()));
        }
        let internal_key = (username.clone(), secret_name.to_string(), true);
        Ok(self
            .user_secrets
            .get(&internal_key)
            .map(|v| v.value().clone()))
    }

    async fn list_user_secret_names(
        &self,
        username: &Username,
    ) -> Result<Vec<SecretRef>, StoreError> {
        use std::collections::HashMap;
        // Deduplicate: if both internal and external exist, report internal=false
        let mut map: HashMap<String, bool> = HashMap::new();
        for entry in self.user_secrets.iter() {
            if &entry.key().0 == username {
                let name = entry.key().1.clone();
                let internal = entry.key().2;
                let existing = map.entry(name).or_insert(internal);
                // false (external) wins over true (internal)
                if !internal {
                    *existing = false;
                }
            }
        }
        let mut refs: Vec<SecretRef> = map
            .into_iter()
            .map(|(name, internal)| SecretRef { name, internal })
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(refs)
    }

    // ---- Conversation (read-only) ----

    async fn get_conversation(
        &self,
        id: &ConversationId,
        include_deleted: bool,
    ) -> Result<Versioned<Conversation>, StoreError> {
        let versions = self
            .conversations
            .get(id)
            .ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        let versioned = Self::latest_versioned(versions.value())
            .ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;

        if !include_deleted && versioned.item.deleted {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }

        Ok(versioned)
    }

    async fn list_conversations(
        &self,
        query: &SearchConversationsQuery,
    ) -> Result<Vec<(ConversationId, Versioned<Conversation>)>, StoreError> {
        let include_deleted = query.include_deleted.unwrap_or(false);
        let search_term = query
            .q
            .as_ref()
            .map(|v| v.trim().to_lowercase())
            .filter(|v| !v.is_empty());
        let creator_filter = query
            .creator
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let status_filter = query
            .status
            .map(crate::domain::conversations::ConversationStatus::from);

        let items: Vec<(ConversationId, Versioned<Conversation>)> = self
            .conversations
            .iter()
            .filter_map(|entry| {
                let id = entry.key().clone();
                let versions = entry.value();
                let versioned = Self::latest_versioned(versions)?;
                let conv = &versioned.item;

                if !include_deleted && conv.deleted {
                    return None;
                }

                if let Some(status) = status_filter {
                    if conv.status != status {
                        return None;
                    }
                }

                if let Some(creator) = &creator_filter {
                    if !conv.creator.as_ref().eq_ignore_ascii_case(creator) {
                        return None;
                    }
                }

                if let Some(ref term) = search_term {
                    let matches_id = id.as_ref().to_lowercase().contains(term);
                    let matches_title = conv
                        .title
                        .as_deref()
                        .map(|t| t.to_lowercase().contains(term))
                        .unwrap_or(false);
                    let matches_agent = conv
                        .agent_name
                        .as_deref()
                        .map(|a| a.to_lowercase().contains(term))
                        .unwrap_or(false);
                    if !matches_id && !matches_title && !matches_agent {
                        return None;
                    }
                }

                Some((id, versioned))
            })
            .collect();

        let paginated = apply_memory_pagination(
            items,
            |(_id, v)| v.timestamp,
            |(id, _v)| id.as_ref(),
            &query.cursor,
            query.limit,
        )?;

        Ok(paginated)
    }

    async fn get_conversation_events(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<Versioned<ConversationEvent>>, StoreError> {
        if !self.conversations.contains_key(id) {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }
        Ok(self
            .conversation_events
            .get(id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default())
    }

    async fn get_conversation_versions(
        &self,
        id: &ConversationId,
    ) -> Result<Vec<Versioned<Conversation>>, StoreError> {
        let snapshot = self.get_conversation(id, false).await?;
        let events = self.get_conversation_events(id).await?;
        Ok(crate::store::fold_conversation_versions(
            id, &snapshot, &events,
        ))
    }

    async fn get_conversation_event_summaries(
        &self,
        ids: &[ConversationId],
    ) -> Result<HashMap<ConversationId, ConversationEventSummary>, StoreError> {
        let mut result = HashMap::new();
        for id in ids {
            // Walk every session linked to this conversation and sum the
            // chat-text events (UserMessage / AssistantMessage). ToolUse,
            // Suspending, Resumed, and Closed are excluded — they are
            // bookkeeping rather than messages the user sees in the column.
            // ConversationEvents are lifecycle-only post Phase E step 18 and
            // do not contribute to the count.
            //
            // While we're walking sessions, also pick the most recent
            // chat-text event for `last_event_preview`. Sessions are ordered
            // newest-last by `list_session_ids_by_conversation_id`; walking
            // them in reverse lets us short-circuit the preview lookup as
            // soon as we find one.
            let session_ids = self.list_session_ids_by_conversation_id(id).await?;
            let mut event_count: usize = 0;
            let mut last_event_preview = None;
            for sid in session_ids.iter().rev() {
                if let Some(events_entry) = self.session_events.get(sid) {
                    let events = events_entry.value();
                    event_count += events
                        .iter()
                        .filter(|(_seq, v)| {
                            matches!(
                                v.item,
                                SessionEvent::UserMessage { .. }
                                    | SessionEvent::AssistantMessage { .. }
                            )
                        })
                        .count();
                    if last_event_preview.is_none() {
                        last_event_preview =
                            events.iter().rev().find_map(|(_seq, v)| match v.item {
                                SessionEvent::UserMessage { .. }
                                | SessionEvent::AssistantMessage { .. } => Some(v.item.preview()),
                                _ => None,
                            });
                    }
                }
            }

            if event_count > 0 || last_event_preview.is_some() {
                result.insert(
                    id.clone(),
                    ConversationEventSummary {
                        event_count,
                        last_event_preview,
                    },
                );
            }
        }
        Ok(result)
    }

    async fn get_session_events(
        &self,
        id: &SessionId,
    ) -> Result<Vec<Versioned<SessionEvent>>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        Ok(self
            .session_events
            .get(id)
            .map(|entry| entry.value().iter().map(|(_seq, v)| v.clone()).collect())
            .unwrap_or_default())
    }

    async fn list_session_ids_by_conversation_id(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<SessionId>, StoreError> {
        let mut matches: Vec<(SessionId, DateTime<Utc>)> = self
            .tasks
            .iter()
            .filter_map(|entry| {
                let latest = Self::latest_versioned(entry.value())?;
                if latest.item.deleted {
                    return None;
                }
                let linked = latest
                    .item
                    .conversation_id()
                    .map(|cid| cid == conversation_id)
                    .unwrap_or(false);
                if !linked {
                    return None;
                }
                let creation_time = latest.item.creation_time.unwrap_or(latest.creation_time);
                Some((entry.key().clone(), creation_time))
            })
            .collect();
        matches.sort_by(|(a_id, a_time), (b_id, b_time)| {
            a_time
                .cmp(b_time)
                .then_with(|| a_id.as_ref().cmp(b_id.as_ref()))
        });
        Ok(matches.into_iter().map(|(id, _)| id).collect())
    }

    async fn get_session_event_summaries(
        &self,
        ids: &[SessionId],
    ) -> Result<HashMap<SessionId, SessionEventSummary>, StoreError> {
        let mut result = HashMap::new();
        for id in ids {
            if let Some(entry) = self.session_events.get(id) {
                let events = entry.value();
                if !events.is_empty() {
                    result.insert(
                        id.clone(),
                        SessionEventSummary {
                            event_count: events.len(),
                            last_event_preview: events.last().map(|(_seq, v)| v.item.preview()),
                        },
                    );
                }
            }
        }
        Ok(result)
    }

    async fn get_session_state(&self, id: &SessionId) -> Result<Option<Vec<u8>>, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        Ok(self
            .session_state
            .get(id)
            .map(|entry| entry.value().clone()))
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
        let id = self.next_issue_id();

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
        let id = self.next_patch_id();
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
        // Check path uniqueness for non-deleted documents
        if let Some(ref path) = document.path {
            if self
                .find_non_deleted_document_by_exact_path(path.as_ref())
                .await?
                .is_some()
            {
                return Err(StoreError::DocumentPathConflict);
            }
        }

        let id = self.next_document_id();
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
        // Read previous path without holding a long-lived guard
        let previous_path = {
            let versions = self
                .documents
                .get(id)
                .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
            versions.last().and_then(|v| v.item.path.clone())
        };
        let new_path = document.path.clone();

        // Check path uniqueness when the path is changing and the document is not being deleted
        if !document.deleted && new_path != previous_path {
            if let Some(ref path) = new_path {
                let ids = self.document_ids_with_exact_path(path.as_ref());
                for other_id in ids {
                    if &other_id == id {
                        continue;
                    }
                    if let Some(entry) = self.documents.get(&other_id) {
                        if let Some(latest) = Self::latest_versioned(entry.value()) {
                            if !latest.item.deleted {
                                return Err(StoreError::DocumentPathConflict);
                            }
                        }
                    }
                }
            }
        }

        // Now get mutable access to push the new version
        let mut versions = self
            .documents
            .get_mut(id)
            .ok_or_else(|| StoreError::DocumentNotFound(id.clone()))?;
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
        let id = self.next_session_id();
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
        hydra_id: &SessionId,
        session: Session,
        actor: &ActorRef,
    ) -> Result<Versioned<Session>, StoreError> {
        let previous_spawned_from = match self.tasks.get(hydra_id) {
            Some(entry) => entry
                .value()
                .last()
                .and_then(|existing| existing.item.spawned_from.clone()),
            None => return Err(StoreError::SessionNotFound(hydra_id.clone())),
        };

        if let Some(previous_issue) = previous_spawned_from.as_ref() {
            if session.spawned_from.as_ref() != Some(previous_issue) {
                self.remove_task_from_issue_index(previous_issue, hydra_id);
            }
        }

        // Overwrite the existing session without modifying edge structure
        let updated = match self.tasks.get_mut(hydra_id) {
            Some(mut versions) => {
                let next_version = Self::next_version(&versions);
                let versioned =
                    Self::versioned_now_with_actor(session.clone(), next_version, actor);
                versions.push(versioned.clone());
                versioned
            }
            None => return Err(StoreError::SessionNotFound(hydra_id.clone())),
        };

        if let Some(issue_id) = session.spawned_from.as_ref() {
            self.index_task_for_issue(issue_id, hydra_id.clone());
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

        // Validate default-conversation-agent uniqueness.
        if agent.is_default_conversation_agent {
            let has_default = self
                .agents
                .iter()
                .any(|e| e.value().is_default_conversation_agent && !e.value().deleted);
            if has_default {
                return Err(StoreError::ConversationAgentAlreadyExists);
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

        // Validate default-conversation-agent uniqueness (exclude self).
        if agent.is_default_conversation_agent {
            let conflict = self.agents.iter().any(|e| {
                e.value().is_default_conversation_agent
                    && !e.value().deleted
                    && e.key() != &agent.name
            });
            if conflict {
                return Err(StoreError::ConversationAgentAlreadyExists);
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

        let id = self.next_label_id();

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
        object_id: &HydraId,
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
        object_id: &HydraId,
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
        source_id: &HydraId,
        target_id: &HydraId,
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
        source_id: &HydraId,
        target_id: &HydraId,
        rel_type: super::RelationshipType,
    ) -> Result<bool, StoreError> {
        let key = (source_id.clone(), rel_type, target_id.clone());
        Ok(self.object_relationships.remove(&key).is_some())
    }

    // ---- Auth token mutations ----

    async fn add_auth_token(&self, actor_name: &str, token_hash: &str) -> Result<(), StoreError> {
        let mut entry = self.auth_tokens.entry(actor_name.to_string()).or_default();
        let hash = token_hash.to_string();
        if !entry.contains(&hash) {
            entry.push(hash);
        }
        Ok(())
    }

    async fn delete_auth_tokens_for_actor(&self, actor_name: &str) -> Result<(), StoreError> {
        self.auth_tokens.remove(actor_name);
        Ok(())
    }

    // ---- User secret mutations ----

    async fn set_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
        encrypted_value: &[u8],
        internal: bool,
    ) -> Result<(), StoreError> {
        let key = (username.clone(), secret_name.to_string(), internal);
        self.user_secrets.insert(key, encrypted_value.to_vec());
        Ok(())
    }

    async fn delete_user_secret(
        &self,
        username: &Username,
        secret_name: &str,
    ) -> Result<(), StoreError> {
        // Only delete the external version
        let key = (username.clone(), secret_name.to_string(), false);
        self.user_secrets.remove(&key);
        Ok(())
    }

    // ---- Conversation mutations ----

    async fn add_conversation(
        &self,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<(ConversationId, VersionNumber), StoreError> {
        let id = self.next_conversation_id();
        let now = Utc::now();
        let versioned = Versioned::with_actor(conversation, 1, now, actor.clone(), now);
        self.conversations.insert(id.clone(), vec![versioned]);
        Ok((id, 1))
    }

    async fn update_conversation(
        &self,
        id: &ConversationId,
        conversation: Conversation,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        let mut entry = self
            .conversations
            .get_mut(id)
            .ok_or_else(|| StoreError::ConversationNotFound(id.clone()))?;
        let versions = entry.value_mut();
        let next_version = Self::next_version(versions);
        let creation_time = versions
            .first()
            .map(|v| v.creation_time)
            .unwrap_or_else(Utc::now);
        let now = Utc::now();
        versions.push(Versioned::with_actor(
            conversation,
            next_version,
            now,
            actor.clone(),
            creation_time,
        ));
        Ok(next_version)
    }

    async fn append_conversation_event(
        &self,
        id: &ConversationId,
        event: ConversationEvent,
        actor: &ActorRef,
    ) -> Result<ConversationEventId, StoreError> {
        if !self.conversations.contains_key(id) {
            return Err(StoreError::ConversationNotFound(id.clone()));
        }
        let mut events = self.conversation_events.entry(id.clone()).or_default();
        let event_index = events.len();
        let next_version = Self::next_version(&events);
        let now = Utc::now();
        events.push(Versioned::with_actor(
            event,
            next_version,
            now,
            actor.clone(),
            now,
        ));
        Ok(ConversationEventId {
            conversation_id: id.clone(),
            event_index,
        })
    }

    async fn append_session_event(
        &self,
        id: &SessionId,
        event: SessionEvent,
        actor: &ActorRef,
    ) -> Result<VersionNumber, StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        let mut events = self.session_events.entry(id.clone()).or_default();
        let next_version = events
            .last()
            .map(|(_seq, v)| v.version.saturating_add(1))
            .unwrap_or(1);
        let global_seq = self.next_session_event_seq.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        events.push((
            global_seq,
            Versioned::with_actor(event, next_version, now, actor.clone(), now),
        ));
        Ok(next_version)
    }

    async fn store_session_state(
        &self,
        id: &SessionId,
        data: Vec<u8>,
        _actor: &ActorRef,
    ) -> Result<(), StoreError> {
        if !self.tasks.contains_key(id) {
            return Err(StoreError::SessionNotFound(id.clone()));
        }
        self.session_state.insert(id.clone(), data);
        Ok(())
    }
}

/// Helper function to check if an issue matches the provided filter criteria.
fn issue_matches(
    issue_type_filter: Option<IssueType>,
    status_filter: &[IssueStatus],
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

    if !status_filter.is_empty() && !status_filter.contains(&issue.status) {
        return false;
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
            issues::{Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType},
            patches::{GithubPr, Patch, PatchStatus},
            sessions::BundleSpec,
            task_status::Event,
            users::{User, Username},
        },
        store::TaskError,
        test_utils::test_state_with_store,
    };
    use chrono::{Duration, Utc};
    use hydra_common::{
        IssueId, RepoName, SessionId, VersionNumber, Versioned,
        api::v1::repositories::{DynamicRef, MergePolicy, MergerRule, Principal, ReviewerGroup},
        api::v1::users::Username as ApiUsername,
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

    fn spawn_task() -> Session {
        spawn_task_with_prompt("0")
    }

    fn spawn_task_with_prompt(prompt: &str) -> Session {
        use crate::app::sessions::mount_spec_for_session;
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            AgentConfig::default(),
            mount_spec_for_session(&BundleSpec::None),
            Some("hydra-worker:latest".to_string()),
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless {
                prompt: prompt.to_string(),
            },
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
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/sample").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn sample_document(path: Option<&str>) -> Document {
        Document {
            title: "Doc".to_string(),
            body_markdown: "Body".to_string(),
            path: path.map(|p| p.parse().unwrap()),
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
            None,
            None,
            None,
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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
    async fn repository_persists_merge_policy_round_trip() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
        let mut config = sample_repository_config();
        config.merge_policy = Some(MergePolicy {
            reviewers: vec![ReviewerGroup {
                label: Some("code-review".to_string()),
                any_of: vec![Principal::User(ApiUsername::from("reviewer"))],
                count: 1,
                exclude_author: true,
            }],
            mergers: Some(MergerRule {
                any_of: vec![Principal::Dynamic(DynamicRef::PatchAuthor)],
            }),
        });

        store
            .add_repository(name.clone(), config.clone(), &ActorRef::test())
            .await
            .unwrap();

        let fetched = store.get_repository(&name, false).await.unwrap();
        assert_eq!(fetched.item.merge_policy, config.merge_policy);
        assert_eq!(fetched.item, config);
    }

    #[tokio::test]
    async fn repository_versions_increment_and_latest_returned() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/hydra").unwrap();

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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();

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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
        let query = SearchRepositoriesQuery::new(Some(true), None);
        let list = store.list_repositories(&query).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].1.item.deleted);
    }

    #[tokio::test]
    async fn add_repository_recreates_over_soft_deleted_repo() {
        let store = MemoryStore::new();
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
        let name = RepoName::from_str("dourolabs/hydra").unwrap();
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
    async fn list_repositories_filters_by_remote_url() {
        let store = MemoryStore::new();
        let alpha = RepoName::from_str("dourolabs/alpha").unwrap();
        let beta = RepoName::from_str("dourolabs/beta").unwrap();
        let gamma = RepoName::from_str("dourolabs/gamma").unwrap();

        // alpha and gamma share the *same* canonical remote (different surface
        // forms: SSH vs HTTPS with trailing `.git`). beta is a distinct repo.
        store
            .add_repository(
                alpha.clone(),
                Repository::new(
                    "https://github.com/dourolabs/alpha.git".to_string(),
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_repository(
                beta.clone(),
                Repository::new(
                    "https://github.com/dourolabs/beta.git".to_string(),
                    None,
                    None,
                ),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_repository(
                gamma.clone(),
                Repository::new("git@github.com:dourolabs/alpha".to_string(), None, None),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        // Different-but-equivalent surface form for alpha → exactly two matches
        // (alpha + gamma) because their normalized URLs collide.
        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://GitHub.com/dourolabs/alpha/".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert_eq!(list.len(), 2);
        let names: Vec<_> = list.iter().map(|(n, _)| n.clone()).collect();
        assert!(names.contains(&alpha));
        assert!(names.contains(&gamma));

        // Distinct remote → one match.
        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://github.com/dourolabs/beta".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, beta);

        // No registered repository matches → empty list.
        let q = SearchRepositoriesQuery::new(
            None,
            Some("https://github.com/dourolabs/missing".to_string()),
        );
        let list = store.list_repositories(&q).await.unwrap();
        assert!(list.is_empty());

        // No filter → all repos returned.
        let list = store
            .list_repositories(&SearchRepositoriesQuery::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 3);
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
                sample_document(Some("docs/guides/intro.md")),
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

        let (first, _) = store
            .add_document(sample_document(Some("docs/howto.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("notes/todo.md")), &ActorRef::test())
            .await
            .unwrap();

        let query = SearchDocumentsQuery::new(
            Some("how".to_string()),
            Some("/docs/".to_string()),
            None,
            None,
        );

        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, first);
    }

    #[tokio::test]
    async fn list_documents_filters_by_ids() {
        let store = MemoryStore::new();

        let (a, _) = store
            .add_document(sample_document(Some("docs/a.md")), &ActorRef::test())
            .await
            .unwrap();
        let (b, _) = store
            .add_document(sample_document(Some("docs/b.md")), &ActorRef::test())
            .await
            .unwrap();
        let (_c, _) = store
            .add_document(sample_document(Some("notes/c.md")), &ActorRef::test())
            .await
            .unwrap();

        // (a) exact id match returns only those documents.
        let mut query = SearchDocumentsQuery::default();
        query.ids = vec![a.clone(), b.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        let mut found_ids: Vec<DocumentId> = filtered.iter().map(|d| d.0.clone()).collect();
        found_ids.sort();
        let mut expected = vec![a.clone(), b.clone()];
        expected.sort();
        assert_eq!(found_ids, expected);

        // (b) ids intersect with other filters (path_prefix).
        let mut query = SearchDocumentsQuery::new(None, Some("/docs/".to_string()), None, None);
        query.ids = vec![a.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, a);

        // ids that don't intersect with the path filter return no rows.
        let mut query = SearchDocumentsQuery::new(None, Some("/notes/".to_string()), None, None);
        query.ids = vec![a.clone()];
        let filtered = store.list_documents(&query).await.unwrap();
        assert!(filtered.is_empty());

        // (c) empty ids vec behaves like the field is absent (returns all).
        let all = store
            .list_documents(&SearchDocumentsQuery::default())
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn document_path_index_updates_on_change() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/old.md")), &ActorRef::test())
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

        let task = spawn_task_with_prompt("v1");
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let updated = spawn_task_with_prompt("v2");
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

        let task = spawn_task_with_prompt("v1");
        let (task_id, _) = store
            .add_session(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let v2 = spawn_task_with_prompt("v2");
        store
            .update_session(&task_id, v2, &ActorRef::test())
            .await
            .unwrap();

        let versions = store.get_session_versions(&task_id).await.unwrap();
        assert_eq!(version_numbers(&versions), vec![1, 2]);
        assert_eq!(versions[0].item.mode.prompt_for_legacy_wire(), "v1");
        assert_eq!(versions[1].item.mode.prompt_for_legacy_wire(), "v2");
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
            .transition_task_to_completion(&task_id, Ok(()), None, None, ActorRef::test())
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
        let task = spawn_task_with_prompt("v1");
        let (task_id, _) = store
            .add_session(task.clone(), created_at, &ActorRef::test())
            .await
            .unwrap();

        let mut updated = task.clone();
        updated.mode = crate::domain::sessions::SessionMode::Headless {
            prompt: "v2".to_string(),
        };
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
        running.mode = crate::domain::sessions::SessionMode::Headless {
            prompt: "v3".to_string(),
        };
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
                None,
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
            .transition_task_to_completion(&completed_id, Ok(()), None, None, ActorRef::test())
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
            .transition_task_to_completion(&root_id, Ok(()), None, None, ActorRef::test())
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
            .transition_task_to_completion(&root_id, Ok(()), None, None, ActorRef::test())
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
            Status::Complete
        );

        // Second Complete transition should succeed idempotently
        let result = state
            .transition_task_to_completion(
                &root_id,
                Ok(()),
                Some("second message".to_string()),
                None,
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
            .transition_task_to_completion(&root_id, Ok(()), None, None, ActorRef::test())
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
                None,
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Trying to transition Failed -> Complete should fail
        let err = state
            .transition_task_to_completion(&root_id, Ok(()), None, None, ActorRef::test())
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
    async fn auth_tokens_add_and_get() {
        let store = MemoryStore::new();
        store.add_auth_token("u-alice", "hash1").await.unwrap();
        store.add_auth_token("u-alice", "hash2").await.unwrap();

        let hashes = store.get_auth_token_hashes("u-alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string(), "hash2".to_string()]);
    }

    #[tokio::test]
    async fn auth_tokens_get_empty() {
        let store = MemoryStore::new();
        let hashes = store.get_auth_token_hashes("u-nobody").await.unwrap();
        assert!(hashes.is_empty());
    }

    #[tokio::test]
    async fn auth_tokens_delete_for_actor() {
        let store = MemoryStore::new();
        store.add_auth_token("u-alice", "hash1").await.unwrap();
        store.add_auth_token("u-alice", "hash2").await.unwrap();
        store.delete_auth_tokens_for_actor("u-alice").await.unwrap();

        let hashes = store.get_auth_token_hashes("u-alice").await.unwrap();
        assert!(hashes.is_empty());
    }

    #[tokio::test]
    async fn auth_tokens_duplicate_insert_is_idempotent() {
        let store = MemoryStore::new();
        store.add_auth_token("u-alice", "hash1").await.unwrap();
        store.add_auth_token("u-alice", "hash1").await.unwrap();

        let hashes = store.get_auth_token_hashes("u-alice").await.unwrap();
        assert_eq!(hashes, vec!["hash1".to_string()]);
    }

    #[tokio::test]
    async fn document_path_is_exact_filters_correctly() {
        let store = MemoryStore::new();

        let (exact_doc, _) = store
            .add_document(sample_document(Some("docs/guide.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("docs/guide.md.bak")),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_document(
                sample_document(Some("docs/guide.md/extra")),
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
                vec![],
                None,
                None,
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
            .add_document(sample_document(Some("test.md")), &ActorRef::test())
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
            .list_documents(&SearchDocumentsQuery::new(None, None, None, Some(true)))
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
            Some(hydra_common::api::v1::issues::IssueType::Task),
            vec![],
            None,
            None,
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, task_issue_id);

        // Filter by bug type
        let query = SearchIssuesQuery::new(
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            None,
            None,
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
            vec![hydra_common::api::v1::issues::IssueStatus::Open],
            None,
            None,
            None,
        );
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, open_issue_id);

        // Filter by closed status
        let query = SearchIssuesQuery::new(
            None,
            vec![hydra_common::api::v1::issues::IssueStatus::Closed],
            None,
            None,
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
        let query = SearchIssuesQuery::new(None, vec![], Some("alice".to_string()), None, None);
        let issues = store.list_issues(&query).await.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, assigned_issue_id);

        // Case-insensitive assignee matching
        let query = SearchIssuesQuery::new(None, vec![], Some("ALICE".to_string()), None, None);
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
        let query = SearchIssuesQuery::new(None, vec![], None, Some("login".to_string()), None);
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
        let query = SearchIssuesQuery::new(None, vec![], None, Some(id_prefix), None);
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
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            Some("alice".to_string()),
            None,
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
    async fn list_tasks_filters_by_spawned_from_ids() {
        let store = MemoryStore::new();

        let (issue_a, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_b, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (issue_c, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        let mut task_a = spawn_task();
        task_a.spawned_from = Some(issue_a.clone());
        let (task_a_id, _) = store
            .add_session(task_a, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b = spawn_task();
        task_b.spawned_from = Some(issue_b.clone());
        let (task_b_id, _) = store
            .add_session(task_b, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_c = spawn_task();
        task_c.spawned_from = Some(issue_c.clone());
        store
            .add_session(task_c, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Filter by spawned_from_ids should return matching tasks
        let mut query = SearchSessionsQuery::default();
        query.spawned_from_ids = vec![issue_a.clone(), issue_b.clone()];
        let tasks: HashSet<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task_a_id, task_b_id]));
    }

    #[tokio::test]
    async fn list_tasks_filters_by_creator() {
        let store = MemoryStore::new();

        let mut task_alice = spawn_task();
        task_alice.creator = Username::from("alice");
        let (alice_id, _) = store
            .add_session(task_alice, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_bob = spawn_task();
        task_bob.creator = Username::from("bob");
        store
            .add_session(task_bob, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.creator = Some("alice".to_string());
        let tasks: HashSet<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([alice_id]));
    }

    #[tokio::test]
    async fn list_tasks_filters_by_conversation_id() {
        let store = MemoryStore::new();

        let conv_a = ConversationId::new();
        let conv_b = ConversationId::new();

        let mut task_a = spawn_task();
        task_a.mode = crate::domain::sessions::SessionMode::Interactive {
            conversation_id: conv_a.clone(),
            idle_timeout_secs: None,
            conversation_resume_from: None,
        };
        let (task_a_id, _) = store
            .add_session(task_a, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut task_b = spawn_task();
        task_b.mode = crate::domain::sessions::SessionMode::Interactive {
            conversation_id: conv_b.clone(),
            idle_timeout_secs: None,
            conversation_resume_from: None,
        };
        store
            .add_session(task_b, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        // Non-interactive session (no `interactive`, so no conversation link).
        store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(conv_a.clone());
        let tasks: HashSet<_> = store
            .list_sessions(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(tasks, HashSet::from([task_a_id]));

        // An unrelated conversation returns nothing.
        let mut query = SearchSessionsQuery::default();
        query.conversation_id = Some(ConversationId::new());
        let tasks: Vec<_> = store.list_sessions(&query).await.unwrap();
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn list_issues_filters_by_ids() {
        let store = MemoryStore::new();

        let (id_a, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (id_b, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();
        let (_id_c, _) = store
            .add_issue(sample_issue(vec![]), &ActorRef::test())
            .await
            .unwrap();

        // Batch-fetch by ids
        let mut query = SearchIssuesQuery::default();
        query.ids = vec![id_a.clone(), id_b.clone()];
        let issues: HashSet<_> = store
            .list_issues(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(issues, HashSet::from([id_a, id_b]));
    }

    #[tokio::test]
    async fn list_issues_filters_by_creator() {
        let store = MemoryStore::new();

        let mut issue_alice = sample_issue(vec![]);
        issue_alice.creator = Username::from("alice");
        let (alice_id, _) = store
            .add_issue(issue_alice, &ActorRef::test())
            .await
            .unwrap();

        let mut issue_bob = sample_issue(vec![]);
        issue_bob.creator = Username::from("bob");
        store.add_issue(issue_bob, &ActorRef::test()).await.unwrap();

        // Filter by creator
        let mut query = SearchIssuesQuery::default();
        query.creator = Some("alice".to_string());
        let issues: Vec<_> = store
            .list_issues(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0], alice_id);
    }

    #[tokio::test]
    async fn list_tasks_filters_by_search_term_prompt() {
        let store = MemoryStore::new();

        // Create tasks with different prompts
        let task1 = spawn_task_with_prompt("Fix authentication bug");
        let (task1_id, _) = store
            .add_session(task1, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let task2 = spawn_task_with_prompt("Add new feature for login");
        let (task2_id, _) = store
            .add_session(task2, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let task3 = spawn_task_with_prompt("Refactor database layer");
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
                None,
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
        use hydra_common::api::v1::patches::PatchStatus as ApiPatchStatus;

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
        use hydra_common::api::v1::patches::PatchStatus as ApiPatchStatus;

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
    async fn list_patches_filters_by_repo_name() {
        use hydra_common::api::v1::patches::PatchStatus as ApiPatchStatus;

        let store = MemoryStore::new();

        let mut patch_a = sample_patch();
        patch_a.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_a.status = PatchStatus::Open;
        let (patch_a_id, _) = store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_b.status = PatchStatus::Closed;
        let (patch_b_id, _) = store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let mut patch_c = sample_patch();
        patch_c.service_repo_name = RepoName::from_str("dourolabs/other").unwrap();
        store.add_patch(patch_c, &ActorRef::test()).await.unwrap();

        // (a) exact repo_name match returns only those patches.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/hydra".to_string());
        let mut filtered: Vec<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        filtered.sort();
        let mut expected = vec![patch_a_id.clone(), patch_b_id.clone()];
        expected.sort();
        assert_eq!(filtered, expected);

        // (b) repo_name AND-intersects with status.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/hydra".to_string());
        query.status = vec![ApiPatchStatus::Open];
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, patch_a_id);

        // (c) non-matching repo_name returns nothing.
        let mut query = SearchPatchesQuery::default();
        query.repo_name = Some("dourolabs/missing".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn list_patches_filters_by_creator() {
        use hydra_common::api::v1::patches::PatchStatus as ApiPatchStatus;

        let store = MemoryStore::new();

        let mut patch_a = sample_patch();
        patch_a.creator = Username::from("Alice");
        patch_a.status = PatchStatus::Open;
        let (patch_a_id, _) = store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.creator = Username::from("alice");
        patch_b.status = PatchStatus::Closed;
        let (patch_b_id, _) = store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let mut patch_c = sample_patch();
        patch_c.creator = Username::from("bob");
        store.add_patch(patch_c, &ActorRef::test()).await.unwrap();

        // (a) case-insensitive creator match returns both alice patches.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("ALICE".to_string());
        let mut filtered: Vec<PatchId> = store
            .list_patches(&query)
            .await
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        filtered.sort();
        let mut expected = vec![patch_a_id.clone(), patch_b_id.clone()];
        expected.sort();
        assert_eq!(filtered, expected);

        // (b) creator AND-intersects with status.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("alice".to_string());
        query.status = vec![ApiPatchStatus::Open];
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, patch_a_id);

        // (c) non-matching creator returns nothing.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("carol".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn list_patches_empty_string_filter_is_noop() {
        let store = MemoryStore::new();

        let mut patch_a = sample_patch();
        patch_a.creator = Username::from("alice");
        patch_a.service_repo_name = RepoName::from_str("dourolabs/hydra").unwrap();
        patch_a.branch_name = Some("feature/foo".to_string());
        store.add_patch(patch_a, &ActorRef::test()).await.unwrap();

        let mut patch_b = sample_patch();
        patch_b.creator = Username::from("bob");
        patch_b.service_repo_name = RepoName::from_str("dourolabs/other").unwrap();
        patch_b.branch_name = Some("feature/bar".to_string());
        store.add_patch(patch_b, &ActorRef::test()).await.unwrap();

        let baseline = store
            .list_patches(&SearchPatchesQuery::default())
            .await
            .unwrap()
            .len();
        assert_eq!(baseline, 2);

        // Each empty-string filter individually is a no-op.
        for field in ["creator", "repo_name", "branch_name"] {
            let mut query = SearchPatchesQuery::default();
            match field {
                "creator" => query.creator = Some(String::new()),
                "repo_name" => query.repo_name = Some(String::new()),
                "branch_name" => query.branch_name = Some(String::new()),
                _ => unreachable!(),
            }
            let filtered = store.list_patches(&query).await.unwrap();
            assert_eq!(filtered.len(), baseline, "empty {field} should be a no-op");
        }

        // All three empty filters combined is also a no-op.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some(String::new());
        query.repo_name = Some(String::new());
        query.branch_name = Some(String::new());
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), baseline);

        // Whitespace-only values are likewise a no-op after trim.
        let mut query = SearchPatchesQuery::default();
        query.creator = Some("   ".to_string());
        query.repo_name = Some("   ".to_string());
        query.branch_name = Some("   ".to_string());
        let filtered = store.list_patches(&query).await.unwrap();
        assert_eq!(filtered.len(), baseline);
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

        let cursor = hydra_common::api::v1::pagination::DecodedCursor {
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

        let cursor2 = hydra_common::api::v1::pagination::DecodedCursor {
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

        let cursor = hydra_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchPatchesQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_patches(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = hydra_common::api::v1::pagination::DecodedCursor {
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
                .add_document(sample_document(Some(&format!("doc_{i}.md"))), &actor)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut query = SearchDocumentsQuery::default();
        query.limit = Some(2);
        let page1 = store.list_documents(&query).await.unwrap();
        assert_eq!(page1.len(), 3);

        let cursor = hydra_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchDocumentsQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_documents(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = hydra_common::api::v1::pagination::DecodedCursor {
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

        let cursor = hydra_common::api::v1::pagination::DecodedCursor {
            timestamp: page1[1].1.timestamp,
            id: page1[1].0.to_string(),
        }
        .encode();

        let mut query2 = SearchSessionsQuery::default();
        query2.limit = Some(2);
        query2.cursor = Some(cursor);
        let page2 = store.list_sessions(&query2).await.unwrap();
        assert_eq!(page2.len(), 3);

        let cursor2 = hydra_common::api::v1::pagination::DecodedCursor {
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
        let cursor = hydra_common::api::v1::pagination::DecodedCursor {
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

        let cursor2 = hydra_common::api::v1::pagination::DecodedCursor {
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
            None,
            3,
            i32::MAX,
            false,
            false,
            Vec::new(),
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
        assert!(!fetched.is_default_conversation_agent);
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
    async fn default_conversation_agent_uniqueness_on_add() {
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        let err = store.add_agent(chat2).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationAgentAlreadyExists));
    }

    #[tokio::test]
    async fn default_conversation_agent_uniqueness_on_update() {
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_default_conversation_agent = true;
        let err = store.update_agent(swe_updated).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationAgentAlreadyExists));
    }

    #[tokio::test]
    async fn default_conversation_agent_can_update_itself() {
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let mut chat_updated = sample_agent("chat");
        chat_updated.is_default_conversation_agent = true;
        chat_updated.max_tries = 10;
        store.update_agent(chat_updated).await.unwrap();

        let fetched = store.get_agent("chat").await.unwrap();
        assert_eq!(fetched.max_tries, 10);
        assert!(fetched.is_default_conversation_agent);
    }

    #[tokio::test]
    async fn deleted_default_conversation_agent_allows_new_one() {
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();
        store.delete_agent("chat").await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        store.add_agent(chat2).await.unwrap();

        let fetched = store.get_agent("chat2").await.unwrap();
        assert!(fetched.is_default_conversation_agent);
    }

    #[tokio::test]
    async fn assignment_and_default_conversation_flags_independent() {
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        // Adding a separate agent with only is_default_conversation_agent should succeed.
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let pm = store.get_agent("pm").await.unwrap();
        assert!(pm.is_assignment_agent);
        assert!(!pm.is_default_conversation_agent);
        let chat = store.get_agent("chat").await.unwrap();
        assert!(!chat.is_assignment_agent);
        assert!(chat.is_default_conversation_agent);
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
            None,
            None,
            None,
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
            None,
            None,
            None,
        );
        store.add_issue(closed, &actor).await.unwrap();

        // Count all issues
        let query =
            hydra_common::api::v1::issues::SearchIssuesQuery::new(None, vec![], None, None, None);
        assert_eq!(store.count_issues(&query).await.unwrap(), 5);

        // Count only bugs
        let query = hydra_common::api::v1::issues::SearchIssuesQuery::new(
            Some(hydra_common::api::v1::issues::IssueType::Bug),
            vec![],
            None,
            None,
            None,
        );
        assert_eq!(store.count_issues(&query).await.unwrap(), 1);

        // Count only closed
        let query = hydra_common::api::v1::issues::SearchIssuesQuery::new(
            None,
            vec![hydra_common::api::v1::issues::IssueStatus::Closed],
            None,
            None,
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
            hydra_common::api::v1::patches::SearchPatchesQuery::new(None, None, Vec::new(), None);
        assert_eq!(store.count_patches(&query).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn count_documents_returns_total_matching() {
        let store = MemoryStore::new();
        let actor = ActorRef::test();

        store
            .add_document(sample_document(Some("docs/a.md")), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("docs/b.md")), &actor)
            .await
            .unwrap();
        store
            .add_document(sample_document(Some("other/c.md")), &actor)
            .await
            .unwrap();

        // Count all
        let query =
            hydra_common::api::v1::documents::SearchDocumentsQuery::new(None, None, None, None);
        assert_eq!(store.count_documents(&query).await.unwrap(), 3);

        // Count with path prefix filter
        let query = hydra_common::api::v1::documents::SearchDocumentsQuery::new(
            Some("docs/".to_string()),
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
            hydra_common::api::v1::sessions::SearchSessionsQuery::new(None, None, None, vec![]);
        assert_eq!(store.count_sessions(&query).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn count_labels_returns_total_matching() {
        use crate::domain::labels::Label as DomainLabel;
        use hydra_common::Rgb;

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
        let query = hydra_common::api::v1::labels::SearchLabelsQuery::default();
        assert_eq!(store.count_labels(&query).await.unwrap(), 3);

        // Count with search filter
        let mut query = hydra_common::api::v1::labels::SearchLabelsQuery::default();
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
        let mut query =
            hydra_common::api::v1::issues::SearchIssuesQuery::new(None, vec![], None, None, None);
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
    async fn refers_to_relationship_round_trip_conversation_to_issue() {
        use crate::store::{ObjectKind, RelationshipType};

        let store = MemoryStore::new();
        let actor_ref = ActorRef::test();

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let conversation_id = hydra_common::ConversationId::new();

        let source = HydraId::from(conversation_id.clone());
        let target = HydraId::from(issue_id.clone());

        store
            .add_relationship(&source, &target, RelationshipType::RefersTo)
            .await
            .unwrap();

        let rels = store
            .get_relationships(Some(&source), None, Some(RelationshipType::RefersTo))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].source_id, source);
        assert_eq!(rels[0].source_kind, ObjectKind::Conversation);
        assert_eq!(rels[0].target_id, target);
        assert_eq!(rels[0].target_kind, ObjectKind::Issue);
        assert_eq!(rels[0].rel_type, RelationshipType::RefersTo);
    }

    #[tokio::test]
    async fn update_issue_preserves_has_document_relationships() {
        use crate::store::RelationshipType;

        let store = MemoryStore::new();
        let actor_ref = ActorRef::test();

        let (issue_id, _) = store
            .add_issue(sample_issue(vec![]), &actor_ref)
            .await
            .unwrap();
        let (doc_id, _) = store
            .add_document(sample_document(None), &actor_ref)
            .await
            .unwrap();

        let source = HydraId::from(issue_id.clone());
        let target = HydraId::from(doc_id.clone());

        // Seed an externally-owned (issue, document, has-document) row.
        store
            .add_relationship(&source, &target, RelationshipType::HasDocument)
            .await
            .unwrap();

        // Update the issue with no document changes; the unmanaged row must survive.
        let mut updated = sample_issue(vec![]);
        updated.progress = "halfway".to_string();
        store
            .update_issue(&issue_id, updated, &actor_ref)
            .await
            .unwrap();

        let rels = store
            .get_relationships(Some(&source), None, Some(RelationshipType::HasDocument))
            .await
            .unwrap();
        assert_eq!(rels.len(), 1, "has-document row must survive issue update");
        assert_eq!(rels[0].target_id, target);
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

        let sid1 = HydraId::from(id1.clone());
        let sid2 = HydraId::from(id2.clone());
        let sid3 = HydraId::from(id3.clone());
        let tpid = HydraId::from(pid.clone());

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
        use crate::store::{RelationshipType, TransitiveDirection};

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

        let a = HydraId::from(id_a.clone());
        let b = HydraId::from(id_b.clone());
        let c = HydraId::from(id_c.clone());
        let p = HydraId::from(pid.clone());

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
            .get_relationships_transitive(
                &[a.clone()],
                TransitiveDirection::Forward,
                RelationshipType::ChildOf,
            )
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
            .get_relationships_transitive(
                &[c.clone()],
                TransitiveDirection::Backward,
                RelationshipType::ChildOf,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Transitive has-patch from B should only find B->P
        let results = store
            .get_relationships_transitive(
                &[b.clone()],
                TransitiveDirection::Forward,
                RelationshipType::HasPatch,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].target_id, p);
    }

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_existing() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/unique.md")), &ActorRef::test())
            .await
            .unwrap();

        let found = store
            .find_non_deleted_document_by_exact_path("/docs/unique.md")
            .await
            .unwrap();
        assert_eq!(found, Some(doc_id));
    }

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_none_for_deleted() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/deleted.md")), &ActorRef::test())
            .await
            .unwrap();
        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        let found = store
            .find_non_deleted_document_by_exact_path("/docs/deleted.md")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn find_non_deleted_document_by_exact_path_returns_none_for_missing() {
        let store = MemoryStore::new();
        let found = store
            .find_non_deleted_document_by_exact_path("/docs/nonexistent.md")
            .await
            .unwrap();
        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn add_document_duplicate_path_fails() {
        let store = MemoryStore::new();
        store
            .add_document(sample_document(Some("docs/conflict.md")), &ActorRef::test())
            .await
            .unwrap();

        let result = store
            .add_document(sample_document(Some("docs/conflict.md")), &ActorRef::test())
            .await;
        assert!(
            matches!(result, Err(StoreError::DocumentPathConflict)),
            "expected DocumentPathConflict, got {result:?}"
        );
    }

    #[tokio::test]
    async fn add_document_path_reusable_after_deletion() {
        let store = MemoryStore::new();
        let (doc_id, _) = store
            .add_document(sample_document(Some("docs/reuse.md")), &ActorRef::test())
            .await
            .unwrap();

        store
            .delete_document(&doc_id, &ActorRef::test())
            .await
            .unwrap();

        // Should succeed since the original document is deleted
        let result = store
            .add_document(sample_document(Some("docs/reuse.md")), &ActorRef::test())
            .await;
        assert!(
            result.is_ok(),
            "expected success after deletion, got {result:?}"
        );
    }

    #[tokio::test]
    async fn add_document_null_paths_not_constrained() {
        let store = MemoryStore::new();
        store
            .add_document(sample_document(None), &ActorRef::test())
            .await
            .unwrap();

        // Adding another document with no path should succeed
        let result = store
            .add_document(sample_document(None), &ActorRef::test())
            .await;
        assert!(
            result.is_ok(),
            "expected success for NULL paths, got {result:?}"
        );
    }

    #[tokio::test]
    async fn update_document_path_conflict_fails() {
        let store = MemoryStore::new();
        let (doc1_id, _) = store
            .add_document(sample_document(Some("docs/first.md")), &ActorRef::test())
            .await
            .unwrap();

        let (_doc2_id, _) = store
            .add_document(sample_document(Some("docs/second.md")), &ActorRef::test())
            .await
            .unwrap();

        // Try to update doc1's path to conflict with doc2
        let mut updated = sample_document(Some("docs/second.md"));
        updated.title = "Updated".to_string();
        let result = store
            .update_document(&doc1_id, updated, &ActorRef::test())
            .await;
        assert!(
            matches!(result, Err(StoreError::DocumentPathConflict)),
            "expected DocumentPathConflict, got {result:?}"
        );
    }

    // ---- Conversation tests ----

    fn sample_conversation() -> Conversation {
        use crate::domain::conversations::ConversationStatus;
        Conversation {
            title: Some("Test conversation".to_string()),
            agent_name: Some("test-agent".to_string()),
            status: ConversationStatus::Active,
            creator: Username::from("alice"),
            session_settings: Default::default(),
            deleted: false,
        }
    }

    fn test_actor() -> ActorRef {
        ActorRef::test()
    }

    #[tokio::test]
    async fn create_and_get_conversation() {
        let store = MemoryStore::new();
        let (id, version) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        assert_eq!(version, 1);

        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_eq!(fetched.item.title.as_deref(), Some("Test conversation"));
        assert_eq!(fetched.item.agent_name.as_deref(), Some("test-agent"));
        assert_eq!(fetched.version, 1);
    }

    #[tokio::test]
    async fn get_conversation_not_found() {
        let store = MemoryStore::new();
        let id = hydra_common::ConversationId::new();
        let result = store.get_conversation(&id, false).await;
        assert!(matches!(result, Err(StoreError::ConversationNotFound(_))));
    }

    #[tokio::test]
    async fn update_conversation_fields() {
        use crate::domain::conversations::ConversationStatus;

        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        let mut updated_conv = sample_conversation();
        updated_conv.status = ConversationStatus::Idle;
        updated_conv.title = Some("New title".to_string());

        let version = store
            .update_conversation(&id, updated_conv, &test_actor())
            .await
            .unwrap();
        assert_eq!(version, 2);

        // Verify persistence
        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_eq!(fetched.item.status, ConversationStatus::Idle);
        assert_eq!(fetched.item.title.as_deref(), Some("New title"));
        assert_eq!(fetched.version, 2);
    }

    #[tokio::test]
    async fn update_conversation_not_found() {
        let store = MemoryStore::new();
        let id = hydra_common::ConversationId::new();
        let result = store
            .update_conversation(&id, sample_conversation(), &test_actor())
            .await;
        assert!(matches!(result, Err(StoreError::ConversationNotFound(_))));
    }

    #[tokio::test]
    async fn append_and_get_conversation_events() {
        use crate::domain::conversations::ConversationEvent;

        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // Initially no events
        let events = store.get_conversation_events(&id).await.unwrap();
        assert!(events.is_empty());

        let event = ConversationEvent::Suspending {
            reason: "idle".to_string(),
            timestamp: Utc::now(),
        };
        let v1 = store
            .append_conversation_event(&id, event, &test_actor())
            .await
            .unwrap();
        assert_eq!(v1.conversation_id, id);
        assert_eq!(v1.event_index, 0);

        let event2 = ConversationEvent::Closed {
            timestamp: Utc::now(),
        };
        let v2 = store
            .append_conversation_event(&id, event2, &test_actor())
            .await
            .unwrap();
        assert_eq!(v2.conversation_id, id);
        assert_eq!(v2.event_index, 1);

        // Verify persistence
        let events = store.get_conversation_events(&id).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].version, 1);
        assert_eq!(events[1].version, 2);
    }

    #[tokio::test]
    async fn get_conversation_versions_folds_events_into_snapshots() {
        use crate::domain::conversations::{ConversationEvent, ConversationStatus};

        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // No events yet -> empty result.
        let versions = store.get_conversation_versions(&id).await.unwrap();
        assert!(versions.is_empty());

        let ts1 = Utc::now();
        store
            .append_conversation_event(
                &id,
                ConversationEvent::Suspending {
                    reason: "idle".to_string(),
                    timestamp: ts1,
                },
                &test_actor(),
            )
            .await
            .unwrap();
        let ts2 = Utc::now();
        store
            .append_conversation_event(
                &id,
                ConversationEvent::Closed { timestamp: ts2 },
                &test_actor(),
            )
            .await
            .unwrap();

        let events = store.get_conversation_events(&id).await.unwrap();
        let versions = store.get_conversation_versions(&id).await.unwrap();

        // One snapshot per event, with versions / timestamps lifted from the events.
        assert_eq!(versions.len(), events.len());
        for (v, e) in versions.iter().zip(events.iter()) {
            assert_eq!(v.version, e.version);
            assert_eq!(v.timestamp, e.timestamp);
            assert_eq!(v.actor, e.actor);
            assert_eq!(v.creation_time, e.creation_time);
        }

        // Final snapshot reflects the Closed event.
        assert_eq!(
            versions.last().unwrap().item.status,
            ConversationStatus::Closed
        );
        // Mid-stream snapshot for the Suspending event is Idle.
        assert_eq!(versions[0].item.status, ConversationStatus::Idle);
    }

    #[tokio::test]
    async fn get_conversation_versions_not_found_for_missing_conversation() {
        let store = MemoryStore::new();
        let id = hydra_common::ConversationId::new();
        let err = store.get_conversation_versions(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationNotFound(_)));
    }

    #[tokio::test]
    async fn get_conversation_versions_rejects_deleted_conversation() {
        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let mut deleted = sample_conversation();
        deleted.deleted = true;
        store
            .update_conversation(&id, deleted, &test_actor())
            .await
            .unwrap();
        let err = store.get_conversation_versions(&id).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationNotFound(_)));
    }

    #[tokio::test]
    async fn list_conversations_basic() {
        use hydra_common::api::v1::conversations::SearchConversationsQuery;

        let store = MemoryStore::new();
        store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        let results = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn list_conversations_filter_by_status() {
        use crate::domain::conversations::ConversationStatus;
        use hydra_common::api::v1::conversations::SearchConversationsQuery;

        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let mut closed_conv = sample_conversation();
        closed_conv.status = ConversationStatus::Closed;
        store
            .update_conversation(&id, closed_conv, &test_actor())
            .await
            .unwrap();
        store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap(); // Active

        let results = store
            .list_conversations(&SearchConversationsQuery {
                status: Some(hydra_common::api::v1::conversations::ConversationStatus::Active),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.item.status, ConversationStatus::Active);
    }

    #[tokio::test]
    async fn list_conversations_excludes_deleted() {
        use hydra_common::api::v1::conversations::SearchConversationsQuery;

        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // Conversation should be visible in list initially
        let results = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1.item.deleted);

        // Soft-delete the conversation
        let mut deleted_conv = sample_conversation();
        deleted_conv.deleted = true;
        store
            .update_conversation(&id, deleted_conv, &test_actor())
            .await
            .unwrap();

        // Deleted conversation should not appear in default list
        let results = store
            .list_conversations(&SearchConversationsQuery::default())
            .await
            .unwrap();
        assert!(results.is_empty());

        // Deleted conversation should appear with include_deleted=true
        let results = store
            .list_conversations(&SearchConversationsQuery {
                include_deleted: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.item.deleted);
    }

    #[tokio::test]
    async fn get_conversation_filters_deleted() {
        let store = MemoryStore::new();
        let (id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // Conversation is accessible when not deleted
        let fetched = store.get_conversation(&id, false).await.unwrap();
        assert_eq!(fetched.item.title.as_deref(), Some("Test conversation"));

        // Soft-delete the conversation
        let mut deleted_conv = sample_conversation();
        deleted_conv.deleted = true;
        store
            .update_conversation(&id, deleted_conv, &test_actor())
            .await
            .unwrap();

        // get_conversation with include_deleted=false should return ConversationNotFound
        let err = store.get_conversation(&id, false).await.unwrap_err();
        assert!(matches!(err, StoreError::ConversationNotFound(_)));

        // get_conversation with include_deleted=true should return the deleted conversation
        let fetched = store.get_conversation(&id, true).await.unwrap();
        assert!(fetched.item.deleted);
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_ignores_lifecycle_conversation_events() {
        let store = MemoryStore::new();
        let (id1, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (id2, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (id3, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // Conversation 1: 2 lifecycle ConversationEvents and no sessions.
        // event_count counts chat-text SessionEvents only, so lifecycle
        // ConversationEvents (Suspending / Closed) do not contribute. With no
        // sessions, the result is omitted from the map.
        store
            .append_conversation_event(
                &id1,
                ConversationEvent::Suspending {
                    reason: "idle".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_conversation_event(
                &id1,
                ConversationEvent::Closed {
                    timestamp: chrono::Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        // Conversation 2: 1 lifecycle ConversationEvent, no sessions.
        store
            .append_conversation_event(
                &id2,
                ConversationEvent::Suspending {
                    reason: "sigterm".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        // Conversation 3: nothing at all.

        let summaries = store
            .get_conversation_event_summaries(&[id1.clone(), id2.clone(), id3.clone()])
            .await
            .unwrap();

        // None of the three conversations have any chat-text SessionEvents,
        // so none of them appear in the summary map.
        assert!(!summaries.contains_key(&id1));
        assert!(!summaries.contains_key(&id2));
        assert!(!summaries.contains_key(&id3));
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_previews_chat_text_from_sessions() {
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (sid, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid,
                SessionEvent::UserMessage {
                    content: "hello world".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // One chat-text SessionEvent on the linked session.
        assert_eq!(s.event_count, 1);
        assert_eq!(s.last_event_preview.as_deref(), Some("User: hello world"));
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_prefers_latest_chat_text_within_session() {
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (sid, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid,
                SessionEvent::UserMessage {
                    content: "hi".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid,
                SessionEvent::AssistantMessage {
                    content: "hey there".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // Two chat-text events appended to the same session.
        assert_eq!(s.event_count, 2);
        assert_eq!(
            s.last_event_preview.as_deref(),
            Some("Assistant: hey there")
        );
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_skips_non_chat_session_events() {
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        // Older session has a chat-text event…
        let (older, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now() - chrono::Duration::seconds(10),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &older,
                SessionEvent::UserMessage {
                    content: "early greeting".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        // …and a newer session has only lifecycle/tool-use events.
        let (newer, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &newer,
                SessionEvent::ToolUse {
                    tool_name: "bash".to_string(),
                    payload: serde_json::json!({}),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &newer,
                SessionEvent::Suspending {
                    reason: "idle".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &newer,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // Only the older session has a chat-text event; the newer session's
        // ToolUse / Suspending / Closed lifecycle events don't count.
        assert_eq!(s.event_count, 1);
        assert_eq!(
            s.last_event_preview.as_deref(),
            Some("User: early greeting")
        );
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_chat_text_overrides_lifecycle_conversation_event() {
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (sid, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &sid,
                SessionEvent::UserMessage {
                    content: "first".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_conversation_event(
                &conv,
                ConversationEvent::Closed {
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        assert_eq!(s.event_count, 1);
        assert_eq!(s.last_event_preview.as_deref(), Some("User: first"));
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_latest_session_wins_over_older() {
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        // Older session, with a chat-text event timestamped *later* than the
        // event in the newer session — to verify that ordering is by session,
        // not by per-event wall-clock timestamp.
        let (older, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now() - chrono::Duration::seconds(10),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &older,
                SessionEvent::UserMessage {
                    content: "from older session, written later".to_string(),
                    timestamp: Utc::now() + chrono::Duration::seconds(60),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        let (newer, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &newer,
                SessionEvent::AssistantMessage {
                    content: "from newer session".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // Both sessions contribute one chat-text event each.
        assert_eq!(s.event_count, 2);
        assert_eq!(
            s.last_event_preview.as_deref(),
            Some("Assistant: from newer session")
        );
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_sums_chat_text_across_sessions() {
        // Regression test for the chat-list "Messages" column: when a
        // conversation has multiple sessions (close → resume), the count must
        // sum chat-text events across every session, not just the most recent.
        let store = MemoryStore::new();
        let (conv, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // First (closed) session — two messages.
        let (s_old, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now() - chrono::Duration::seconds(30),
                &test_actor(),
            )
            .await
            .unwrap();
        for content in ["one", "two"] {
            store
                .append_session_event(
                    &s_old,
                    SessionEvent::UserMessage {
                        content: content.to_string(),
                        timestamp: Utc::now(),
                    },
                    &test_actor(),
                )
                .await
                .unwrap();
        }
        // Lifecycle event on the old session — must not contribute.
        store
            .append_session_event(
                &s_old,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        // Second (current) session — three messages plus a ToolUse.
        let (s_new, _) = store
            .add_session(
                interactive_session(Some(conv.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s_new,
                SessionEvent::UserMessage {
                    content: "three".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s_new,
                SessionEvent::ToolUse {
                    tool_name: "bash".to_string(),
                    payload: serde_json::json!({}),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s_new,
                SessionEvent::AssistantMessage {
                    content: "four".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s_new,
                SessionEvent::UserMessage {
                    content: "five".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_conversation_event_summaries(&[conv.clone()])
            .await
            .unwrap();
        let s = summaries.get(&conv).expect("summary present");
        // 2 chat-text events on the old session + 3 on the new session = 5.
        // ToolUse and Closed are excluded.
        assert_eq!(s.event_count, 5);
        // Preview comes from the most recent chat-text event in the newest
        // session.
        assert_eq!(s.last_event_preview.as_deref(), Some("User: five"));
    }

    #[tokio::test]
    async fn get_conversation_event_summaries_empty_ids() {
        let store = MemoryStore::new();
        let summaries = store.get_conversation_event_summaries(&[]).await.unwrap();
        assert!(summaries.is_empty());
    }

    fn interactive_session(conversation_id: Option<ConversationId>) -> Session {
        let mut session = spawn_task();
        match conversation_id {
            Some(conv_id) => {
                session.mode = crate::domain::sessions::SessionMode::Interactive {
                    conversation_id: conv_id,
                    idle_timeout_secs: None,
                    conversation_resume_from: None,
                };
            }
            None => {
                session.mode = crate::domain::sessions::SessionMode::Headless {
                    prompt: String::new(),
                };
            }
        }
        session
    }

    #[tokio::test]
    async fn append_and_get_session_events_returns_in_append_order() {
        let store = MemoryStore::new();
        let (sid, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();

        // No events yet.
        let events = store.get_session_events(&sid).await.unwrap();
        assert!(events.is_empty());

        let v1 = store
            .append_session_event(
                &sid,
                SessionEvent::UserMessage {
                    content: "hi".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        assert_eq!(v1, 1);
        let v2 = store
            .append_session_event(
                &sid,
                SessionEvent::AssistantMessage {
                    content: "hello".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        assert_eq!(v2, 2);

        let events = store.get_session_events(&sid).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].version, 1);
        assert_eq!(events[1].version, 2);
        assert!(matches!(events[0].item, SessionEvent::UserMessage { .. }));
        assert!(matches!(
            events[1].item,
            SessionEvent::AssistantMessage { .. }
        ));
    }

    #[tokio::test]
    async fn append_session_event_not_found_for_missing_session() {
        let store = MemoryStore::new();
        let missing = SessionId::generate(6).unwrap();
        let err = store
            .append_session_event(
                &missing,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
        let err = store.get_session_events(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn session_event_global_counter_is_monotonic_across_sessions() {
        let store = MemoryStore::new();
        let (s1, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();
        let (s2, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();

        let start = store.next_session_event_seq.load(Ordering::Relaxed);
        store
            .append_session_event(
                &s1,
                SessionEvent::UserMessage {
                    content: "a".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s2,
                SessionEvent::UserMessage {
                    content: "b".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s1,
                SessionEvent::AssistantMessage {
                    content: "c".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        // Counter has advanced by exactly 3 across the two sessions.
        let end = store.next_session_event_seq.load(Ordering::Relaxed);
        assert_eq!(end - start, 3);

        // The per-event seqs were assigned strictly increasing across sessions:
        // s1's first event got `start`, s2's only event got `start + 1`,
        // s1's second event got `start + 2`.
        let s1_seqs: Vec<u64> = store
            .session_events
            .get(&s1)
            .unwrap()
            .value()
            .iter()
            .map(|(seq, _)| *seq)
            .collect();
        let s2_seqs: Vec<u64> = store
            .session_events
            .get(&s2)
            .unwrap()
            .value()
            .iter()
            .map(|(seq, _)| *seq)
            .collect();
        assert_eq!(s1_seqs, vec![start, start + 2]);
        assert_eq!(s2_seqs, vec![start + 1]);
    }

    #[tokio::test]
    async fn store_and_get_session_state_blob() {
        let store = MemoryStore::new();
        let (sid, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();

        let state = store.get_session_state(&sid).await.unwrap();
        assert!(state.is_none());

        let data = vec![1u8, 2, 3, 4, 5];
        store
            .store_session_state(&sid, data.clone(), &test_actor())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data));

        // Overwrite.
        let data2 = vec![9u8, 8, 7];
        store
            .store_session_state(&sid, data2.clone(), &test_actor())
            .await
            .unwrap();
        let state = store.get_session_state(&sid).await.unwrap();
        assert_eq!(state, Some(data2));
    }

    #[tokio::test]
    async fn session_state_not_found_for_missing_session() {
        let store = MemoryStore::new();
        let missing = SessionId::generate(6).unwrap();
        let err = store.get_session_state(&missing).await.unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
        let err = store
            .store_session_state(&missing, vec![1], &test_actor())
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SessionNotFound(_)));
    }

    #[tokio::test]
    async fn list_session_ids_by_conversation_id_finds_linked_sessions() {
        let store = MemoryStore::new();
        let (conv_id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (other_conv_id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();

        // Session A: linked to conv_id, created earliest.
        let t1 = Utc::now();
        let (sid_a, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                t1,
                &test_actor(),
            )
            .await
            .unwrap();
        // Session B: linked to a different conversation.
        let (_sid_b, _) = store
            .add_session(
                interactive_session(Some(other_conv_id.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        // Session C: linked to conv_id, created later than A.
        let t3 = Utc::now() + chrono::Duration::seconds(1);
        let (sid_c, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                t3,
                &test_actor(),
            )
            .await
            .unwrap();
        // Session D: not interactive at all.
        let (_sid_d, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();

        let ids = store
            .list_session_ids_by_conversation_id(&conv_id)
            .await
            .unwrap();
        assert_eq!(ids, vec![sid_a.clone(), sid_c.clone()]);

        // Unrelated conversation returns no sessions.
        let unrelated = hydra_common::ConversationId::new();
        let ids = store
            .list_session_ids_by_conversation_id(&unrelated)
            .await
            .unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn list_session_ids_by_conversation_id_excludes_deleted_sessions() {
        let store = MemoryStore::new();
        let (conv_id, _) = store
            .add_conversation(sample_conversation(), &test_actor())
            .await
            .unwrap();
        let (sid, _) = store
            .add_session(
                interactive_session(Some(conv_id.clone())),
                Utc::now(),
                &test_actor(),
            )
            .await
            .unwrap();
        store.delete_session(&sid, &test_actor()).await.unwrap();
        let ids = store
            .list_session_ids_by_conversation_id(&conv_id)
            .await
            .unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn get_session_event_summaries_returns_counts_and_previews() {
        let store = MemoryStore::new();
        let (s1, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();
        let (s2, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();
        let (s3, _) = store
            .add_session(spawn_task(), Utc::now(), &test_actor())
            .await
            .unwrap();

        store
            .append_session_event(
                &s1,
                SessionEvent::UserMessage {
                    content: "first".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s1,
                SessionEvent::AssistantMessage {
                    content: "second".to_string(),
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();
        store
            .append_session_event(
                &s2,
                SessionEvent::Closed {
                    timestamp: Utc::now(),
                },
                &test_actor(),
            )
            .await
            .unwrap();

        let summaries = store
            .get_session_event_summaries(&[s1.clone(), s2.clone(), s3.clone()])
            .await
            .unwrap();

        let s1_summary = summaries.get(&s1).expect("s1 summary");
        assert_eq!(s1_summary.event_count, 2);
        assert_eq!(
            s1_summary.last_event_preview.as_deref(),
            Some("Assistant: second")
        );

        let s2_summary = summaries.get(&s2).expect("s2 summary");
        assert_eq!(s2_summary.event_count, 1);
        assert_eq!(s2_summary.last_event_preview.as_deref(), Some("Closed"));

        // s3 has no events and is omitted from the result.
        assert!(!summaries.contains_key(&s3));

        // Empty input → empty output.
        let summaries = store.get_session_event_summaries(&[]).await.unwrap();
        assert!(summaries.is_empty());
    }

    fn insert_dummy_sessions(store: &MemoryStore, count: usize) {
        for _ in 0..count {
            // Generate at MAX_RANDOM_LEN so collisions across a 677-row test
            // are vanishingly unlikely without falling back to retry.
            let id = SessionId::generate(12).unwrap();
            store.tasks.insert(
                id,
                vec![Versioned::with_actor(
                    spawn_task(),
                    1,
                    Utc::now(),
                    ActorRef::test(),
                    Utc::now(),
                )],
            );
        }
    }

    #[tokio::test]
    async fn add_session_grows_id_suffix_with_table_size() {
        let store = MemoryStore::new();

        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "fresh store should use default suffix length"
        );

        insert_dummy_sessions(&store, 26); // total = 27
        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            6,
            "27 rows should still use default suffix"
        );

        insert_dummy_sessions(&store, 649); // total = 677
        let (id, _) = store
            .add_session(spawn_task(), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        assert_eq!(
            id.as_ref().len() - SessionId::prefix().len(),
            7,
            "677 rows should bump suffix length to 7"
        );
    }

    fn insert_dummy_undeleted_labels(store: &MemoryStore, count: usize) -> Vec<LabelId> {
        let now = Utc::now();
        let mut ids = Vec::with_capacity(count);
        for i in 0..count {
            // MAX_RANDOM_LEN keeps collisions across 677 rows vanishingly
            // unlikely without falling back to retry.
            let id = LabelId::generate(12).unwrap();
            store.labels.insert(
                id.clone(),
                Label {
                    name: format!("dummy-{i}"),
                    color: "#000000".parse().unwrap(),
                    deleted: false,
                    recurse: false,
                    hidden: false,
                    created_at: now,
                    updated_at: now,
                },
            );
            ids.push(id);
        }
        ids
    }

    #[tokio::test]
    async fn delete_label_keeps_next_label_id_default() {
        let store = MemoryStore::new();

        // Seed 677 live labels so next_label_id sees a count past the 6 → 7
        // threshold.
        let dummies = insert_dummy_undeleted_labels(&store, 677);
        let pre = store
            .add_label(sample_label("live-pre", "#ffffff"))
            .await
            .unwrap();
        assert_eq!(
            pre.as_ref().len() - LabelId::prefix().len(),
            7,
            "677 live labels should bump suffix length to 7"
        );

        // Soft-delete every label; the live count should drop back to zero
        // and the next id should fall back to the default 6-char suffix.
        for id in &dummies {
            store.delete_label(id).await.unwrap();
        }
        store.delete_label(&pre).await.unwrap();

        let post = store
            .add_label(sample_label("live-post", "#ffffff"))
            .await
            .unwrap();
        assert_eq!(
            post.as_ref().len() - LabelId::prefix().len(),
            6,
            "soft-deleted labels must not inflate the suffix length"
        );
    }
}
