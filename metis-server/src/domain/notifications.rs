use super::actors::ActorId;
use super::issues::IssueDependencyType;
use crate::app::event_bus::{MutationPayload, ServerEvent};
use crate::store::{ReadOnlyStore, StoreError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use metis_common::{IssueId, MetisId, VersionNumber};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The server-side domain notification type.
///
/// Notifications are non-versioned: the only mutation after creation is marking as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub recipient: ActorId,
    pub source_actor: Option<ActorId>,
    pub object_kind: String,
    pub object_id: MetisId,
    pub object_version: VersionNumber,
    pub event_type: String,
    pub summary: String,
    pub source_issue_id: Option<IssueId>,
    pub policy: String,
    pub is_read: bool,
    pub created_at: DateTime<Utc>,
}

impl Notification {
    pub fn new(
        recipient: ActorId,
        source_actor: Option<ActorId>,
        object_kind: String,
        object_id: MetisId,
        object_version: VersionNumber,
        event_type: String,
        summary: String,
        source_issue_id: Option<IssueId>,
        policy: String,
    ) -> Self {
        Self {
            recipient,
            source_actor,
            object_kind,
            object_id,
            object_version,
            event_type,
            summary,
            source_issue_id,
            policy,
            is_read: false,
            created_at: Utc::now(),
        }
    }
}

// Conversions between domain and API wire types.
use metis_common::api::v1 as api;

impl From<api::notifications::Notification> for Notification {
    fn from(value: api::notifications::Notification) -> Self {
        Self {
            recipient: value.recipient,
            source_actor: value.source_actor,
            object_kind: value.object_kind,
            object_id: value.object_id,
            object_version: value.object_version,
            event_type: value.event_type,
            summary: value.summary,
            source_issue_id: value.source_issue_id,
            policy: value.policy,
            is_read: value.is_read,
            created_at: value.created_at,
        }
    }
}

impl From<Notification> for api::notifications::Notification {
    fn from(value: Notification) -> Self {
        let mut notif = api::notifications::Notification::new(
            value.recipient,
            value.source_actor,
            value.object_kind,
            value.object_id,
            value.object_version,
            value.event_type,
            value.summary,
            value.source_issue_id,
            value.policy,
        );
        notif.is_read = value.is_read;
        notif.created_at = value.created_at;
        notif
    }
}

/// A policy that determines who should be notified about a given event.
///
/// The `NotificationAutomation` holds a `Vec<Box<dyn NotificationPolicy>>` and runs all
/// policies for each event, deduplicating recipients across policies. Each policy is
/// independently testable and produces a set of `ActorId`s that should receive a
/// notification for the given mutation.
///
/// The `policy` column in the notifications table records which policy generated each
/// row, enabling debugging and future per-policy management.
#[async_trait]
pub trait NotificationPolicy: Send + Sync {
    /// A short identifier for this policy, stored in the `policy` column.
    fn name(&self) -> &str;

    /// Given a server event and read-only access to the store, return the set of
    /// `ActorId`s that should receive a notification.
    ///
    /// The caller is responsible for deduplicating and filtering (e.g., removing the
    /// actor who caused the change).
    async fn resolve_recipients(
        &self,
        event: &ServerEvent,
        store: &dyn ReadOnlyStore,
    ) -> Result<Vec<ActorId>, StoreError>;
}

/// Notification routing policy that walks up the `child-of` dependency tree.
///
/// For each event, identifies the source issue(s) and walks up the ancestor chain.
/// At each level, adds the issue itself (as `ActorId::Issue`), its creator, and its
/// assignee as notification recipients.
pub struct WalkUpPolicy;

impl WalkUpPolicy {
    /// Collect recipients for a given issue and all its ancestors via `child-of`.
    async fn collect_recipients_for_issue(
        &self,
        issue_id: &IssueId,
        store: &dyn ReadOnlyStore,
    ) -> Result<Vec<ActorId>, StoreError> {
        let mut recipients = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(issue_id.clone());

        while let Some(current_id) = queue.pop_front() {
            if !visited.insert(current_id.clone()) {
                continue;
            }

            let issue = match store.get_issue(&current_id, false).await {
                Ok(versioned) => versioned.item,
                Err(StoreError::IssueNotFound(_)) => continue,
                Err(e) => return Err(e),
            };

            // Add the issue itself as a recipient
            recipients.push(ActorId::Issue(current_id.clone()));

            // Add the creator
            recipients.push(ActorId::Username(issue.creator.clone().into()));

            // Add the assignee if present
            if let Some(ref assignee) = issue.assignee {
                let username = metis_common::api::v1::users::Username::from(assignee.clone());
                recipients.push(ActorId::Username(username));
            }

            // Walk up: find parent issues via child-of dependencies
            for dep in &issue.dependencies {
                if dep.dependency_type == IssueDependencyType::ChildOf {
                    queue.push_back(dep.issue_id.clone());
                }
            }
        }

        Ok(recipients)
    }
}

#[async_trait]
impl NotificationPolicy for WalkUpPolicy {
    fn name(&self) -> &str {
        "walk_up"
    }

    async fn resolve_recipients(
        &self,
        event: &ServerEvent,
        store: &dyn ReadOnlyStore,
    ) -> Result<Vec<ActorId>, StoreError> {
        let payload = event.payload();
        let source_issue_ids = match payload.as_ref() {
            MutationPayload::Issue { .. } => {
                // The source issue is the issue itself
                match event {
                    ServerEvent::IssueCreated { issue_id, .. }
                    | ServerEvent::IssueUpdated { issue_id, .. }
                    | ServerEvent::IssueDeleted { issue_id, .. } => {
                        vec![issue_id.clone()]
                    }
                    _ => vec![],
                }
            }
            MutationPayload::Session { new, .. } => {
                // Source issue = spawned_from
                new.spawned_from.iter().cloned().collect()
            }
            MutationPayload::Patch { .. } => {
                // Source issues = issues referencing this patch
                match event {
                    ServerEvent::PatchCreated { patch_id, .. }
                    | ServerEvent::PatchUpdated { patch_id, .. }
                    | ServerEvent::PatchDeleted { patch_id, .. } => {
                        match store.get_issues_for_patch(patch_id).await {
                            Ok(ids) => ids,
                            Err(e) => {
                                tracing::warn!(
                                    patch_id = %patch_id,
                                    error = %e,
                                    "failed to get issues for patch in WalkUpPolicy; \
                                     skipping patch notification recipients"
                                );
                                vec![]
                            }
                        }
                    }
                    _ => vec![],
                }
            }
            MutationPayload::Document { .. }
            | MutationPayload::Label { .. }
            | MutationPayload::Message { .. }
            | MutationPayload::Notification { .. } => {
                // No source issue for documents, labels, messages, or notifications
                vec![]
            }
        };

        let mut all_recipients = Vec::new();
        for issue_id in &source_issue_ids {
            let recipients = self.collect_recipients_for_issue(issue_id, store).await?;
            all_recipients.extend(recipients);
        }

        Ok(all_recipients)
    }
}

/// Generate a human-readable summary for a server event.
///
/// Only called for mutation events that carry a `MutationPayload`.
pub fn generate_summary(event: &ServerEvent) -> String {
    let payload = event.payload();
    match payload.as_ref() {
        MutationPayload::Issue { old, new, .. } => {
            let id = match event {
                ServerEvent::IssueCreated { issue_id, .. } => issue_id.to_string(),
                ServerEvent::IssueUpdated { issue_id, .. } => issue_id.to_string(),
                ServerEvent::IssueDeleted { issue_id, .. } => issue_id.to_string(),
                _ => "unknown".to_string(),
            };
            if old.is_none() {
                return format!("Issue {id} was created");
            }
            if new.deleted {
                return format!("Issue {id} was deleted");
            }
            if let Some(old) = old {
                if old.status != new.status {
                    let old_status = old.status;
                    let new_status = new.status;
                    return format!("Issue {id} status changed from {old_status} to {new_status}");
                }
                if old.progress != new.progress {
                    return format!("Issue {id} progress was updated");
                }
                if old.description != new.description {
                    return format!("Issue {id} description was updated");
                }
                if old.assignee != new.assignee {
                    return format!("Issue {id} assignee was updated");
                }
            }
            format!("Issue {id} was updated")
        }
        MutationPayload::Patch { old, new, .. } => {
            let id = match event {
                ServerEvent::PatchCreated { patch_id, .. } => patch_id.to_string(),
                ServerEvent::PatchUpdated { patch_id, .. } => patch_id.to_string(),
                ServerEvent::PatchDeleted { patch_id, .. } => patch_id.to_string(),
                _ => "unknown".to_string(),
            };
            if old.is_none() {
                return format!("Patch {id} was created");
            }
            if new.deleted {
                return format!("Patch {id} was deleted");
            }
            if let Some(old) = old {
                if old.status != new.status {
                    let old_status = old.status;
                    let new_status = new.status;
                    return format!("Patch {id} status changed from {old_status} to {new_status}");
                }
            }
            format!("Patch {id} was updated")
        }
        MutationPayload::Session { old, new, .. } => {
            let id = match event {
                ServerEvent::SessionCreated { session_id, .. } => session_id.to_string(),
                ServerEvent::SessionUpdated { session_id, .. } => session_id.to_string(),
                _ => "unknown".to_string(),
            };
            if old.is_none() {
                return format!("Session {id} was created");
            }
            if let Some(old) = old {
                if old.status != new.status {
                    let old_status = old.status;
                    let new_status = new.status;
                    return format!(
                        "Session {id} status changed from {old_status} to {new_status}"
                    );
                }
            }
            format!("Session {id} was updated")
        }
        MutationPayload::Document { old, new, .. } => {
            let id = match event {
                ServerEvent::DocumentCreated { document_id, .. } => document_id.to_string(),
                ServerEvent::DocumentUpdated { document_id, .. } => document_id.to_string(),
                ServerEvent::DocumentDeleted { document_id, .. } => document_id.to_string(),
                _ => "unknown".to_string(),
            };
            if old.is_none() {
                return format!("Document {id} was created");
            }
            if new.deleted {
                return format!("Document {id} was deleted");
            }
            format!("Document {id} was updated")
        }
        MutationPayload::Message { old, .. } => {
            let id = match event {
                ServerEvent::MessageCreated { message_id, .. } => message_id.to_string(),
                ServerEvent::MessageUpdated { message_id, .. } => message_id.to_string(),
                _ => "unknown".to_string(),
            };
            if old.is_none() {
                return format!("Message {id} was created");
            }
            format!("Message {id} was updated")
        }
        MutationPayload::Label { .. } => {
            // Label events are excluded from notification generation via the event filter;
            // this arm should never be reached.
            unreachable!("label events should be filtered out before notification generation")
        }
        MutationPayload::Notification { new, .. } => {
            let id = match event {
                ServerEvent::NotificationCreated {
                    notification_id, ..
                } => notification_id.to_string(),
                _ => "unknown".to_string(),
            };
            format!("Notification {id} was created: {}", new.summary)
        }
    }
}

/// Extract the object kind string from a server event.
pub fn event_object_kind(event: &ServerEvent) -> &'static str {
    match event {
        ServerEvent::IssueCreated { .. }
        | ServerEvent::IssueUpdated { .. }
        | ServerEvent::IssueDeleted { .. } => "issue",
        ServerEvent::PatchCreated { .. }
        | ServerEvent::PatchUpdated { .. }
        | ServerEvent::PatchDeleted { .. } => "patch",
        ServerEvent::SessionCreated { .. } | ServerEvent::SessionUpdated { .. } => "session",
        ServerEvent::DocumentCreated { .. }
        | ServerEvent::DocumentUpdated { .. }
        | ServerEvent::DocumentDeleted { .. } => "document",
        ServerEvent::LabelCreated { .. }
        | ServerEvent::LabelUpdated { .. }
        | ServerEvent::LabelDeleted { .. } => "label",
        ServerEvent::MessageCreated { .. } | ServerEvent::MessageUpdated { .. } => "message",
        ServerEvent::NotificationCreated { .. } => "notification",
    }
}

/// Extract the object ID as a MetisId from a server event.
pub fn event_object_id(event: &ServerEvent) -> MetisId {
    match event {
        ServerEvent::IssueCreated { issue_id, .. }
        | ServerEvent::IssueUpdated { issue_id, .. }
        | ServerEvent::IssueDeleted { issue_id, .. } => issue_id.clone().into(),
        ServerEvent::PatchCreated { patch_id, .. }
        | ServerEvent::PatchUpdated { patch_id, .. }
        | ServerEvent::PatchDeleted { patch_id, .. } => patch_id.clone().into(),
        ServerEvent::SessionCreated { session_id, .. }
        | ServerEvent::SessionUpdated { session_id, .. } => session_id.clone().into(),
        ServerEvent::DocumentCreated { document_id, .. }
        | ServerEvent::DocumentUpdated { document_id, .. }
        | ServerEvent::DocumentDeleted { document_id, .. } => document_id.clone().into(),
        ServerEvent::LabelCreated { label_id, .. }
        | ServerEvent::LabelUpdated { label_id, .. }
        | ServerEvent::LabelDeleted { label_id, .. } => label_id.clone().into(),
        ServerEvent::MessageCreated { message_id, .. }
        | ServerEvent::MessageUpdated { message_id, .. } => message_id.clone().into(),
        ServerEvent::NotificationCreated {
            notification_id, ..
        } => notification_id.clone().into(),
    }
}

/// Extract the version number from a server event.
pub fn event_version(event: &ServerEvent) -> VersionNumber {
    match event {
        ServerEvent::IssueCreated { version, .. }
        | ServerEvent::IssueUpdated { version, .. }
        | ServerEvent::IssueDeleted { version, .. }
        | ServerEvent::PatchCreated { version, .. }
        | ServerEvent::PatchUpdated { version, .. }
        | ServerEvent::PatchDeleted { version, .. }
        | ServerEvent::SessionCreated { version, .. }
        | ServerEvent::SessionUpdated { version, .. }
        | ServerEvent::DocumentCreated { version, .. }
        | ServerEvent::DocumentUpdated { version, .. }
        | ServerEvent::DocumentDeleted { version, .. }
        | ServerEvent::LabelCreated { version, .. }
        | ServerEvent::LabelUpdated { version, .. }
        | ServerEvent::LabelDeleted { version, .. }
        | ServerEvent::MessageCreated { version, .. }
        | ServerEvent::MessageUpdated { version, .. }
        | ServerEvent::NotificationCreated { version, .. } => *version,
    }
}

/// Extract the event type string ("created", "updated", or "deleted").
pub fn event_type_str(event: &ServerEvent) -> &'static str {
    match event {
        ServerEvent::IssueCreated { .. }
        | ServerEvent::PatchCreated { .. }
        | ServerEvent::SessionCreated { .. }
        | ServerEvent::DocumentCreated { .. }
        | ServerEvent::LabelCreated { .. }
        | ServerEvent::MessageCreated { .. } => "created",
        ServerEvent::IssueUpdated { .. }
        | ServerEvent::PatchUpdated { .. }
        | ServerEvent::SessionUpdated { .. }
        | ServerEvent::DocumentUpdated { .. }
        | ServerEvent::LabelUpdated { .. }
        | ServerEvent::MessageUpdated { .. } => "updated",
        ServerEvent::IssueDeleted { .. }
        | ServerEvent::PatchDeleted { .. }
        | ServerEvent::DocumentDeleted { .. }
        | ServerEvent::LabelDeleted { .. } => "deleted",
        ServerEvent::NotificationCreated { .. } => "created",
    }
}

/// Extract the source issue ID(s) from a server event, if any.
///
/// Only called for mutation events that carry a `MutationPayload`.
pub fn event_source_issue_id(event: &ServerEvent) -> Option<IssueId> {
    let payload = event.payload();
    match payload.as_ref() {
        MutationPayload::Issue { .. } => match event {
            ServerEvent::IssueCreated { issue_id, .. }
            | ServerEvent::IssueUpdated { issue_id, .. }
            | ServerEvent::IssueDeleted { issue_id, .. } => Some(issue_id.clone()),
            _ => None,
        },
        MutationPayload::Session { new, .. } => new.spawned_from.clone(),
        MutationPayload::Patch { .. }
        | MutationPayload::Document { .. }
        | MutationPayload::Label { .. }
        | MutationPayload::Message { .. }
        | MutationPayload::Notification { .. } => None,
    }
}

/// Extract the source actor's `ActorId` from an `ActorRef`, if available.
pub fn actor_ref_to_actor_id(actor_ref: &crate::domain::actors::ActorRef) -> Option<ActorId> {
    match actor_ref {
        crate::domain::actors::ActorRef::Authenticated { actor_id } => Some(actor_id.clone()),
        crate::domain::actors::ActorRef::System { on_behalf_of, .. } => on_behalf_of.clone(),
        crate::domain::actors::ActorRef::Automation { triggered_by, .. } => {
            triggered_by.as_ref().and_then(|t| actor_ref_to_actor_id(t))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::issues::{Issue, IssueDependency, IssueStatus, IssueType};
    use crate::domain::users::Username;
    use crate::store::{MemoryStore, Store};
    use metis_common::IssueId;
    use std::sync::Arc;

    #[test]
    fn notification_domain_roundtrip() {
        let notif = Notification::new(
            ActorId::Username(Username::from("alice").into()),
            Some(ActorId::Issue("i-abcdef".parse::<IssueId>().unwrap())),
            "issue".to_string(),
            "i-abcdef".parse::<IssueId>().unwrap().into(),
            1,
            "updated".to_string(),
            "Issue status changed".to_string(),
            None,
            "walk_up".to_string(),
        );

        let json = serde_json::to_string(&notif).expect("serialize");
        let decoded: Notification = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, notif);
        assert!(!decoded.is_read);
    }

    #[test]
    fn notification_api_domain_roundtrip() {
        let api_notif = api::notifications::Notification::new(
            ActorId::Username(Username::from("alice").into()),
            None,
            "patch".to_string(),
            metis_common::PatchId::new().into(),
            1,
            "created".to_string(),
            "Patch created".to_string(),
            None,
            "walk_up".to_string(),
        );

        let domain_notif: Notification = api_notif.clone().into();
        let back: api::notifications::Notification = domain_notif.into();
        assert_eq!(back, api_notif);
    }

    fn make_issue(creator: &str, assignee: Option<&str>, deps: Vec<IssueDependency>) -> Issue {
        Issue::new(
            IssueType::Task,
            "Test Title".to_string(),
            "test issue".to_string(),
            Username::from(creator),
            String::new(),
            IssueStatus::Open,
            assignee.map(|s| s.to_string()),
            None,
            vec![],
            deps,
            vec![],
        )
    }

    fn test_actor() -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Username(metis_common::api::v1::users::Username::from("test-actor")),
        }
    }

    #[tokio::test]
    async fn walk_up_policy_issue_created_includes_self_and_parents() {
        let store = Arc::new(MemoryStore::new());
        let policy = WalkUpPolicy;

        // Create a parent issue
        let parent = make_issue("alice", Some("bob"), vec![]);
        let (parent_id, _) = store
            .add_issue(parent.clone(), &test_actor())
            .await
            .unwrap();

        // Create a child issue that is child-of parent
        let child = make_issue(
            "charlie",
            None,
            vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )],
        );
        let (child_id, child_version) =
            store.add_issue(child.clone(), &test_actor()).await.unwrap();

        // Simulate an IssueCreated event for the child
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: child,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: child_id.clone(),
            version: child_version,
            timestamp: Utc::now(),
            payload,
        };

        let recipients = policy
            .resolve_recipients(&event, store.as_ref())
            .await
            .unwrap();

        // Should include: child issue, child creator (charlie),
        // parent issue, parent creator (alice), parent assignee (bob)
        assert!(recipients.contains(&ActorId::Issue(child_id)));
        assert!(recipients.contains(&ActorId::Username(
            metis_common::api::v1::users::Username::from("charlie")
        )));
        assert!(recipients.contains(&ActorId::Issue(parent_id)));
        assert!(recipients.contains(&ActorId::Username(
            metis_common::api::v1::users::Username::from("alice")
        )));
        assert!(recipients.contains(&ActorId::Username(
            metis_common::api::v1::users::Username::from("bob")
        )));
    }

    #[tokio::test]
    async fn walk_up_policy_job_created_uses_spawned_from() {
        let store = Arc::new(MemoryStore::new());
        let policy = WalkUpPolicy;

        // Create the source issue
        let issue = make_issue("alice", None, vec![]);
        let (issue_id, _) = store.add_issue(issue.clone(), &test_actor()).await.unwrap();

        // Create a Task with spawned_from pointing to the issue
        let task = crate::store::Session {
            prompt: "test".to_string(),
            context: crate::domain::sessions::BundleSpec::default(),
            spawned_from: Some(issue_id.clone()),
            creator: Username::from("alice"),
            image: None,
            model: None,
            env_vars: Default::default(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            status: crate::store::Status::Created,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
        };

        let payload = Arc::new(MutationPayload::Session {
            old: None,
            new: task.clone(),
            actor: test_actor(),
        });
        let session_id = metis_common::SessionId::new();
        let event = ServerEvent::SessionCreated {
            seq: 1,
            session_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let recipients = policy
            .resolve_recipients(&event, store.as_ref())
            .await
            .unwrap();

        assert!(recipients.contains(&ActorId::Issue(issue_id)));
        assert!(recipients.contains(&ActorId::Username(
            metis_common::api::v1::users::Username::from("alice")
        )));
    }

    #[tokio::test]
    async fn walk_up_policy_document_returns_empty() {
        let store = Arc::new(MemoryStore::new());
        let policy = WalkUpPolicy;

        let doc = crate::domain::documents::Document {
            title: "test".to_string(),
            body_markdown: "hello".to_string(),
            path: None,
            created_by: None,
            deleted: false,
        };

        let payload = Arc::new(MutationPayload::Document {
            old: None,
            new: doc,
            actor: test_actor(),
        });
        let event = ServerEvent::DocumentCreated {
            seq: 1,
            document_id: metis_common::DocumentId::new(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        let recipients = policy
            .resolve_recipients(&event, store.as_ref())
            .await
            .unwrap();

        assert!(recipients.is_empty());
    }

    #[test]
    fn summary_issue_created() {
        let issue_id: IssueId = "i-abcdef".parse().unwrap();
        let issue = make_issue("alice", None, vec![]);
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(generate_summary(&event), "Issue i-abcdef was created");
    }

    #[test]
    fn summary_issue_status_change() {
        let issue_id: IssueId = "i-abcdef".parse().unwrap();
        let old_issue = make_issue("alice", None, vec![]);
        let mut new_issue = old_issue.clone();
        new_issue.status = IssueStatus::InProgress;

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(
            generate_summary(&event),
            "Issue i-abcdef status changed from open to in-progress"
        );
    }

    #[test]
    fn summary_issue_deleted() {
        let issue_id: IssueId = "i-abcdef".parse().unwrap();
        let old_issue = make_issue("alice", None, vec![]);
        let mut new_issue = old_issue.clone();
        new_issue.deleted = true;

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueDeleted {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(generate_summary(&event), "Issue i-abcdef was deleted");
    }

    #[test]
    fn summary_issue_progress_update() {
        let issue_id: IssueId = "i-abcdef".parse().unwrap();
        let old_issue = make_issue("alice", None, vec![]);
        let mut new_issue = old_issue.clone();
        new_issue.progress = "50% done".to_string();

        let payload = Arc::new(MutationPayload::Issue {
            old: Some(old_issue),
            new: new_issue,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueUpdated {
            seq: 1,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(
            generate_summary(&event),
            "Issue i-abcdef progress was updated"
        );
    }

    #[test]
    fn summary_session_created() {
        let task = crate::store::Session {
            prompt: "test".to_string(),
            context: crate::domain::sessions::BundleSpec::default(),
            spawned_from: None,
            creator: Username::from("alice"),
            image: None,
            model: None,
            env_vars: Default::default(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            status: crate::store::Status::Created,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
        };

        let payload = Arc::new(MutationPayload::Session {
            old: None,
            new: task,
            actor: test_actor(),
        });
        let session_id: metis_common::SessionId = "t-abcdef".parse().unwrap();
        let event = ServerEvent::SessionCreated {
            seq: 1,
            session_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(generate_summary(&event), "Session t-abcdef was created");
    }

    #[test]
    fn event_helpers_extract_correct_info() {
        let issue_id: IssueId = "i-abcdef".parse().unwrap();
        let issue = make_issue("alice", None, vec![]);
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: issue,
            actor: test_actor(),
        });
        let event = ServerEvent::IssueCreated {
            seq: 1,
            issue_id: issue_id.clone(),
            version: 3,
            timestamp: Utc::now(),
            payload,
        };

        assert_eq!(event_object_kind(&event), "issue");
        assert_eq!(event_object_id(&event), MetisId::from(issue_id.clone()));
        assert_eq!(event_version(&event), 3);
        assert_eq!(event_type_str(&event), "created");
        assert_eq!(event_source_issue_id(&event), Some(issue_id));
    }
}
