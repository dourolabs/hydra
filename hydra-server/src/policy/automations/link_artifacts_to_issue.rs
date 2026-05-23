use async_trait::async_trait;

use crate::app::event_bus::{EventType, ServerEvent};
use crate::domain::actors::{ActorId, ActorRef};
use crate::policy::context::AutomationContext;
use crate::policy::{Automation, AutomationError, EventFilter};
use crate::store::RelationshipType;
use hydra_common::HydraId;

const AUTOMATION_NAME: &str = "link_artifacts_to_issue";

/// When a session or issue actor creates or updates a patch or document, link
/// the artifact back to the relevant issue.
///
/// Subscribes to `PatchCreated`/`PatchUpdated` and `DocumentCreated`/`DocumentUpdated`.
/// Resolves the source issue from the event actor:
/// - `ActorId::Session(sid)`: loads the session and uses its `spawned_from`
///   issue (no link if the session has no `spawned_from`).
/// - `ActorId::Issue(iid)`: uses the issue id directly.
///
/// In either case, inserts a `(issue, artifact, has-patch | has-document)` row
/// into `object_relationships`. Idempotent — the underlying insert is
/// `INSERT OR IGNORE`.
///
/// Human and service actors are intentionally ignored; callers that want a
/// link should use `POST /v1/relations` directly.
pub struct LinkArtifactsToIssueAutomation;

impl LinkArtifactsToIssueAutomation {
    pub fn new(_params: Option<&serde_yaml_ng::Value>) -> Result<Self, String> {
        Ok(Self)
    }
}

#[async_trait]
impl Automation for LinkArtifactsToIssueAutomation {
    fn name(&self) -> &str {
        AUTOMATION_NAME
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            event_types: vec![
                EventType::PatchCreated,
                EventType::PatchUpdated,
                EventType::DocumentCreated,
                EventType::DocumentUpdated,
            ],
            ..Default::default()
        }
    }

    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
        let actor = ctx.actor();
        let issue_id = match actor {
            ActorRef::Authenticated {
                actor_id: ActorId::Session(sid),
            } => {
                let session = match ctx.store.get_session(sid, false).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            automation = AUTOMATION_NAME,
                            session_id = %sid,
                            error = %e,
                            "failed to load session for actor; skipping link"
                        );
                        return Ok(());
                    }
                };
                match session.item.spawned_from {
                    Some(id) => id,
                    None => return Ok(()),
                }
            }
            ActorRef::Authenticated {
                actor_id: ActorId::Issue(iid),
            } => iid.clone(),
            _ => return Ok(()),
        };

        let (target_id, rel_type): (HydraId, RelationshipType) = match ctx.event {
            ServerEvent::PatchCreated { patch_id, .. }
            | ServerEvent::PatchUpdated { patch_id, .. } => {
                (patch_id.clone().into(), RelationshipType::HasPatch)
            }
            ServerEvent::DocumentCreated { document_id, .. }
            | ServerEvent::DocumentUpdated { document_id, .. } => {
                (document_id.clone().into(), RelationshipType::HasDocument)
            }
            _ => return Ok(()),
        };

        let source_id: HydraId = issue_id.clone().into();
        ctx.app_state
            .store
            .add_relationship_with_actor(&source_id, &target_id, rel_type, actor.clone())
            .await
            .map_err(|e| {
                AutomationError::Other(anyhow::anyhow!(
                    "failed to add ({source_id}, {target_id}, {rel_type}) relationship: {e}"
                ))
            })?;

        tracing::info!(
            automation = AUTOMATION_NAME,
            issue_id = %issue_id,
            target_id = %target_id,
            rel_type = %rel_type,
            "linked artifact to issue"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::event_bus::MutationPayload;
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

    fn make_session(spawned_from: Option<IssueId>) -> Session {
        use crate::app::sessions::mount_spec_for_session;
        use crate::domain::sessions::{AgentConfig, SessionMode};
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
            mode: SessionMode::Headless {
                prompt: "test".to_string(),
            },
            conversation_resume_from: None,
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
            created_by: None,
            deleted: false,
        }
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

    async fn add_issue(handles: &test_utils::TestStateHandles) -> IssueId {
        let issue = crate::domain::issues::Issue::new(
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
        );
        let (issue_id, _) = handles
            .store
            .add_issue(issue, &ActorRef::test())
            .await
            .unwrap();
        issue_id
    }

    async fn add_session_to_store(
        handles: &test_utils::TestStateHandles,
        spawned_from: Option<IssueId>,
    ) -> hydra_common::SessionId {
        let (session_id, _) = handles
            .store
            .add_session(make_session(spawned_from), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        session_id
    }

    #[tokio::test]
    async fn links_patch_to_issue_for_session_actor_with_spawned_from() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_document_to_issue_for_session_actor_with_spawned_from() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone())).await;

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
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = document_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasDocument),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn no_link_when_actor_is_human() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &human_actor())
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), human_actor());
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn no_link_when_session_has_no_spawned_from() {
        let handles = test_utils::test_state_handles();
        let session_id = add_session_to_store(&handles, None).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(None, Some(&target), Some(RelationshipType::HasPatch))
            .await
            .unwrap();
        assert!(relations.is_empty());
    }

    #[tokio::test]
    async fn second_invocation_is_idempotent() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), session_actor(&session_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_patch_to_issue_for_issue_actor() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &issue_actor(&issue_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), issue_actor(&issue_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_document_to_issue_for_issue_actor() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;

        let (document_id, _) = handles
            .store
            .add_document(make_document("/notes/a.md"), &issue_actor(&issue_id))
            .await
            .unwrap();

        let event =
            document_created_event(document_id.clone(), "/notes/a.md", issue_actor(&issue_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = document_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasDocument),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn second_invocation_is_idempotent_for_issue_actor() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &issue_actor(&issue_id))
            .await
            .unwrap();

        let event = patch_created_event(patch_id.clone(), issue_actor(&issue_id));
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();
        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }

    #[tokio::test]
    async fn links_patch_to_issue_on_patch_updated() {
        let handles = test_utils::test_state_handles();
        let issue_id = add_issue(&handles).await;
        let session_id = add_session_to_store(&handles, Some(issue_id.clone())).await;

        let (patch_id, _) = handles
            .store
            .add_patch(make_patch(), &session_actor(&session_id))
            .await
            .unwrap();

        let payload = Arc::new(MutationPayload::Patch {
            old: Some(make_patch()),
            new: make_patch(),
            actor: session_actor(&session_id),
        });
        let event = ServerEvent::PatchUpdated {
            seq: 2,
            patch_id: patch_id.clone(),
            version: 2,
            timestamp: Utc::now(),
            payload,
        };
        let automation = LinkArtifactsToIssueAutomation::new(None).unwrap();
        let ctx = AutomationContext {
            event: &event,
            app_state: &handles.state,
            store: handles.store.as_ref(),
        };

        automation.execute(&ctx).await.unwrap();

        let source: HydraId = issue_id.into();
        let target: HydraId = patch_id.into();
        let relations = handles
            .store
            .get_relationships(
                Some(&source),
                Some(&target),
                Some(RelationshipType::HasPatch),
            )
            .await
            .unwrap();
        assert_eq!(relations.len(), 1);
    }
}
