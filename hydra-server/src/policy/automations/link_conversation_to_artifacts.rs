use async_trait::async_trait;
use std::collections::HashSet;

use crate::app::event_bus::{EventType, ServerEvent};
use crate::domain::actors::ActorId;
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::{ObjectKind, RelationshipType};
use hydra_common::{ConversationId, HydraId, IssueId};

const AUTOMATION_NAME: &str = "link_conversation_to_artifacts";

/// When an issue, patch, or document is created or updated, link the new
/// artifact back to any conversation that should reference it.
///
/// Subscribes to `IssueCreated`/`IssueUpdated`, `PatchCreated`/`PatchUpdated`,
/// and `DocumentCreated`/`DocumentUpdated`. Computes a set of conversation IDs
/// from the event actor:
/// - Direct: if the actor is a session with a `conversation_id`, that
///   conversation is included.
/// - Transitive: if the actor is a session spawned from an issue, or the actor
///   is an issue itself, any conversation that already has a
///   `(conversation, issue, RefersTo)` row is included.
///
/// For each conversation in the resulting set, inserts a
/// `(conversation, artifact, RefersTo)` row into `object_relationships`.
/// Idempotent — the underlying insert is `INSERT OR IGNORE`.
pub struct LinkConversationToArtifactsAutomation;

impl LinkConversationToArtifactsAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }

    /// Look up conversations that already `RefersTo` the given issue.
    async fn conversations_referencing_issue(
        ctx: &AutomationContext<'_>,
        issue_id: &IssueId,
    ) -> Option<Vec<ConversationId>> {
        let issue_hid: HydraId = issue_id.clone().into();
        let rows = match ctx
            .store
            .get_relationships(None, Some(&issue_hid), Some(RelationshipType::RefersTo))
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    automation = AUTOMATION_NAME,
                    issue_id = %issue_id,
                    error = %e,
                    "failed to load RefersTo relationships; skipping transitive link"
                );
                return None;
            }
        };

        let cids = rows
            .into_iter()
            .filter(|r| r.source_kind == ObjectKind::Conversation)
            .filter_map(|r| r.source_id.as_conversation_id())
            .collect();
        Some(cids)
    }
}

#[async_trait]
impl Automation for LinkConversationToArtifactsAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![
                EventType::IssueCreated,
                EventType::IssueUpdated,
                EventType::PatchCreated,
                EventType::PatchUpdated,
                EventType::DocumentCreated,
                EventType::DocumentUpdated,
            ],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let artifact_hid: HydraId = match ctx.event {
            ServerEvent::IssueCreated { issue_id, .. }
            | ServerEvent::IssueUpdated { issue_id, .. } => issue_id.clone().into(),
            ServerEvent::PatchCreated { patch_id, .. }
            | ServerEvent::PatchUpdated { patch_id, .. } => patch_id.clone().into(),
            ServerEvent::DocumentCreated { document_id, .. }
            | ServerEvent::DocumentUpdated { document_id, .. } => document_id.clone().into(),
            _ => return Ok(()),
        };

        let actor = ctx.actor();
        let mut conversation_ids: HashSet<ConversationId> = HashSet::new();

        // Unwrap Automation/System wrappers to the underlying principal actor so
        // artifacts created by automations (e.g. the patch workflow creating a
        // review-request issue on behalf of a session) still link back to the
        // originating conversation.
        match actor.on_behalf_of() {
            Some(ActorId::Session(session_id)) => {
                let session = match ctx.store.get_session(&session_id, false).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            automation = AUTOMATION_NAME,
                            session_id = %session_id,
                            error = %e,
                            "failed to load session for actor; skipping link"
                        );
                        return Ok(());
                    }
                };

                if let Some(cid) = session.item.conversation_id() {
                    conversation_ids.insert(cid.clone());
                }

                if let Some(issue_id) = session.item.spawned_from.as_ref() {
                    if let Some(cids) = Self::conversations_referencing_issue(ctx, issue_id).await {
                        conversation_ids.extend(cids);
                    }
                }
            }
            Some(ActorId::Issue(issue_id)) => {
                if let Some(cids) = Self::conversations_referencing_issue(ctx, &issue_id).await {
                    conversation_ids.extend(cids);
                }
            }
            _ => return Ok(()),
        }

        for cid in conversation_ids {
            let cid_hid: HydraId = cid.clone().into();
            if cid_hid == artifact_hid {
                continue;
            }
            ctx.app_state
                .store
                .add_relationship_with_actor(
                    &cid_hid,
                    &artifact_hid,
                    RelationshipType::RefersTo,
                    actor.clone(),
                )
                .await
                .map_err(|e| {
                    AutomationError::Other(anyhow::anyhow!(
                        "failed to add ({cid_hid}, {artifact_hid}, {}) relationship: {e}",
                        RelationshipType::RefersTo
                    ))
                })?;

            tracing::info!(
                automation = AUTOMATION_NAME,
                conversation_id = %cid,
                artifact_id = %artifact_hid,
                "linked conversation to artifact"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::domain::patches::{Patch, PatchStatus};
    use crate::domain::sessions::{BundleSpec, Session};
    use crate::domain::users::Username;
    use crate::policy::context::AutomationContext;
    use crate::store::Status;
    use crate::test_utils;
    use chrono::Utc;
    use hydra_common::{IssueId, RepoName};
    use std::collections::HashMap;
    use std::str::FromStr;
    use std::sync::Arc;

    fn session_actor(session_id: &hydra_common::SessionId) -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Session(session_id.clone()),
        }
    }

    fn issue_actor(issue_id: &IssueId) -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Issue(issue_id.clone()),
        }
    }

    fn human_actor() -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice").into()),
        }
    }

    fn make_session(
        spawned_from: Option<IssueId>,
        conversation_id: Option<ConversationId>,
    ) -> Session {
        use crate::app::sessions::mount_spec_for_session;
        use crate::domain::sessions::{AgentConfig, SessionMode};
        let mode = match conversation_id {
            Some(cid) => SessionMode::Interactive {
                conversation_id: cid,
                idle_timeout_secs: None,
                conversation_resume_from: None,
            },
            None => SessionMode::Headless {
                prompt: "test".to_string(),
            },
        };
        Session {
            creator: Username::from("test-creator"),
            spawned_from,
            resumed_from: None,
            agent_config: AgentConfig::default(),
            mount_spec: mount_spec_for_session(&BundleSpec::None),
            context: BundleSpec::None,
            image: None,
            env_vars: HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
            secrets: None,
            mode,
            status: Status::Created,
            last_message: None,
            error: None,
            deleted: false,
            creation_time: None,
            start_time: None,
            end_time: None,
            usage: None,
        }
    }

    fn make_patch() -> Patch {
        Patch::new(
            "Test patch".to_string(),
            "Test description".to_string(),
            String::new(),
            PatchStatus::Open,
            false,
            None,
            Username::from("test-creator"),
            Vec::new(),
            RepoName::from_str("dourolabs/hydra").unwrap(),
            None,
            None,
            None,
            None,
        )
    }

    fn make_document(path: &str) -> Document {
        Document {
            title: "Test doc".to_string(),
            body_markdown: "body".to_string(),
            path: Some(path.parse().unwrap()),
            deleted: false,
        }
    }

    fn make_issue() -> crate::domain::issues::Issue {
        crate::domain::issues::Issue::new(
            crate::domain::issues::IssueType::Task,
            "Parent".to_string(),
            "desc".to_string(),
            Username::from("alice"),
            String::new(),
            crate::domain::issues::IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        )
    }

    fn patch_created_event(patch_id: hydra_common::PatchId, actor: ActorRef) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Patch {
            old: None,
            new: make_patch(),
            actor,
        });
        ServerEvent::PatchCreated {
            seq: 1,
            patch_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn patch_updated_event(patch_id: hydra_common::PatchId, actor: ActorRef) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Patch {
            old: Some(make_patch()),
            new: make_patch(),
            actor,
        });
        ServerEvent::PatchUpdated {
            seq: 2,
            patch_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn document_created_event(
        document_id: hydra_common::DocumentId,
        path: &str,
        actor: ActorRef,
    ) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Document {
            old: None,
            new: make_document(path),
            actor,
        });
        ServerEvent::DocumentCreated {
            seq: 1,
            document_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn issue_created_event(issue_id: IssueId, actor: ActorRef) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: None,
            new: make_issue(),
            actor,
        });
        ServerEvent::IssueCreated {
            seq: 1,
            issue_id,
            version: 1,
            timestamp: Utc::now(),
            payload,
        }
    }

    fn issue_updated_event(issue_id: IssueId, actor: ActorRef) -> ServerEvent {
        let payload = Arc::new(MutationPayload::Issue {
            old: Some(make_issue()),
            new: make_issue(),
            actor,
        });
        ServerEvent::IssueUpdated {
            seq: 2,
            issue_id,
            version: 2,
            timestamp: Utc::now(),
            payload,
        }
    }

    async fn add_issue(handles: &test_utils::TestStateHandles) -> IssueId {
        let (issue_id, _) = handles
            .store
            .add_issue(make_issue(), &ActorRef::test())
            .await
            .unwrap();
        issue_id
    }

    async fn add_session_to_store(
        handles: &test_utils::TestStateHandles,
        spawned_from: Option<IssueId>,
        conversation_id: Option<ConversationId>,
    ) -> hydra_common::SessionId {
        let (session_id, _) = handles
            .store
            .add_session(
                make_session(spawned_from, conversation_id),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        session_id
    }

    async fn seed_conversation_references_issue(
        handles: &test_utils::TestStateHandles,
        conversation_id: &ConversationId,
        issue_id: &IssueId,
    ) {
        let source: HydraId = conversation_id.clone().into();
        let target: HydraId = issue_id.clone().into();
        handles
            .store
            .add_relationship(&source, &target, RelationshipType::RefersTo)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn links_patch_directly_for_session_with_conversation_id() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_document_directly_for_session_with_conversation_id() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (document_id, _) = handles
            .store
            .add_document(make_document("/notes/a.md"), &session_actor(&session_id))
            .await
            .unwrap();

        let event = document_created_event(
            document_id.clone(),
            "/notes/a.md",
            session_actor(&session_id),
        );
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = document_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_issue_directly_for_session_with_conversation_id() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (issue_id, _) = handles
            .store
            .add_issue(make_issue(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = issue_created_event(issue_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = issue_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_patch_transitively_via_spawned_from() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let issue_id = add_issue(&handles).await;
        seed_conversation_references_issue(&handles, &cid, &issue_id).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone()), None).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_issue_transitively_via_issue_actor() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let parent_issue_id = add_issue(&handles).await;
        seed_conversation_references_issue(&handles, &cid, &parent_issue_id).await;

        let (new_issue_id, _) = handles
            .store
            .add_issue(make_issue(), &issue_actor(&parent_issue_id))
            .await
            .unwrap();

        let event = issue_created_event(new_issue_id.clone(), issue_actor(&parent_issue_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = new_issue_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_union_of_direct_and_transitive_conversations() {
        let handles = test_utils::test_state_handles();
        let c1 = ConversationId::new();
        let c2 = ConversationId::new();
        let issue_id = add_issue(&handles).await;
        seed_conversation_references_issue(&handles, &c2, &issue_id).await;
        let session_id =
            add_session_to_store(&handles, Some(issue_id.clone()), Some(c1.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let target: HydraId = patch_id.into();
        let c1_hid: HydraId = c1.into();
        let c2_hid: HydraId = c2.into();
        let r1 = handles
            .store
            .get_relationships(
                Some(&c1_hid),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(r1.len(), 1);
        let r2 = handles
            .store
            .get_relationships(
                Some(&c2_hid),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(r2.len(), 1);
    }

    #[tokio::test]
    async fn no_link_when_actor_is_human() {
        let handles = test_utils::test_state_handles();

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &human_actor())
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), human_actor());
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(None, Some(&target), Some(RelationshipType::RefersTo))
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn no_link_when_session_has_neither_conversation_nor_spawned_from() {
        let handles = test_utils::test_state_handles();
        let session_id = add_session_to_store(&handles, None, None).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(None, Some(&target), Some(RelationshipType::RefersTo))
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn no_link_when_spawned_from_has_no_seeded_references() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone()), None).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(None, Some(&target), Some(RelationshipType::RefersTo))
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn second_invocation_is_idempotent() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_patch_on_patch_updated() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_updated_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_patch_when_actor_is_automation_wrapping_session() {
        // Regression: automations that wrap a session actor in
        // ActorRef::Automation { triggered_by: Some(session_actor) } must
        // still have their created artifacts linked back to the session's
        // conversation.
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let wrapping_actor = ActorRef::Automation {
            automation_name: "github_pr_sync".into(),
            triggered_by: Some(Box::new(session_actor(&session_id))),
        };

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &wrapping_actor)
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), wrapping_actor);
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_issue_when_actor_is_nested_automation_wrapping_issue() {
        // Regression: traversal must work through multiple levels of
        // Automation wrappers.
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let parent_issue_id = add_issue(&handles).await;
        seed_conversation_references_issue(&handles, &cid, &parent_issue_id).await;

        let nested_actor = ActorRef::Automation {
            automation_name: "outer".into(),
            triggered_by: Some(Box::new(ActorRef::Automation {
                automation_name: "inner".into(),
                triggered_by: Some(Box::new(issue_actor(&parent_issue_id))),
            })),
        };

        let (new_issue_id, _) = handles
            .store
            .add_issue(make_issue(), &nested_actor)
            .await
            .unwrap();

        let event = issue_created_event(new_issue_id.clone(), nested_actor);
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = new_issue_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_issue_on_issue_updated() {
        let handles = test_utils::test_state_handles();
        let cid = ConversationId::new();
        let session_id = add_session_to_store(&handles, None, Some(cid.clone())).await;

        let (issue_id, _) = handles
            .store
            .add_issue(make_issue(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = issue_updated_event(issue_id.clone(), session_actor(&session_id));
        let automation = LinkConversationToArtifactsAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = cid.into();
        let target: HydraId = issue_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::RefersTo),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }
}
