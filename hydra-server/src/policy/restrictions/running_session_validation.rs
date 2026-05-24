use async_trait::async_trait;
use hydra_common::{ActorId, IssueId, SessionId, api::v1::sessions::SearchSessionsQuery};

use crate::policy::context::{Operation, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};
use crate::store::{ReadOnlyStore, Status, StoreError};

/// Validates that the actor performing a Create{Patch,Document} operation
/// is backed by a running job/session.
///
/// Identity is derived from `RestrictionContext.actor`:
/// - `ActorId::Session(s)` validates session `s` directly.
/// - `ActorId::Issue(i)` validates the currently-running session spawned from
///   issue `i` (if any).
/// - Other actor variants (user, service, no principal) bypass the check.
#[derive(Default)]
pub struct RunningJobValidationRestriction;

impl RunningJobValidationRestriction {
    pub fn new() -> Self {
        Self
    }
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Created => "created",
        Status::Pending => "pending",
        Status::Running => "running",
        Status::Complete => "complete",
        Status::Failed => "failed",
    }
}

impl RunningJobValidationRestriction {
    fn lookup_err(&self, msg: String) -> PolicyViolation {
        PolicyViolation {
            policy_name: self.name().to_string(),
            message: msg,
        }
    }

    async fn running_session_for_issue(
        &self,
        store: &dyn ReadOnlyStore,
        issue_id: &IssueId,
    ) -> Result<Option<SessionId>, StoreError> {
        let query = SearchSessionsQuery::new(
            None,
            Some(issue_id.clone()),
            None,
            vec![Status::Running.into()],
        );
        let mut sessions = store.list_sessions(&query).await?;
        Ok(sessions.pop().map(|(id, _)| id))
    }
}

#[async_trait]
impl Restriction for RunningJobValidationRestriction {
    fn name(&self) -> &str {
        "running_job_validation"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        match ctx.operation {
            Operation::CreatePatch | Operation::CreateDocument => {}
            _ => return Ok(()),
        }

        let session_id = match ctx.actor.on_behalf_of() {
            Some(ActorId::Session(s)) => s,
            Some(ActorId::Issue(issue_id)) => {
                match self
                    .running_session_for_issue(ctx.store, &issue_id)
                    .await
                    .map_err(|e| {
                        self.lookup_err(format!(
                            "Failed to look up running session for issue {issue_id}: {e}"
                        ))
                    })? {
                    Some(s) => s,
                    None => {
                        return Err(self.lookup_err(format!(
                            "Issue {issue_id} has no running session. \
                             Only running jobs can create/modify resources."
                        )));
                    }
                }
            }
            _ => return Ok(()),
        };

        let task = ctx
            .store
            .get_session(&session_id, false)
            .await
            .map_err(|e| self.lookup_err(format!("Failed to look up job {session_id}: {e}")))?;

        if task.item.status != Status::Running {
            return Err(self.lookup_err(format!(
                "Job {session_id} is not in Running status (current: {}). \
                 Only running jobs can create/modify resources.",
                status_str(task.item.status)
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::domain::sessions::{BundleSpec, Session};
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, ReadOnlyStore, Store};
    use chrono::Utc;
    use hydra_common::ActorId;
    use std::collections::HashMap;
    use std::str::FromStr;

    fn make_session(spawned_from: Option<IssueId>) -> Session {
        use crate::app::sessions::mount_spec_for_session;
        use crate::domain::sessions::{AgentConfig, SessionMode};
        Session::new(
            Username::from("test-creator"),
            spawned_from,
            None,
            AgentConfig::default(),
            mount_spec_for_session(&BundleSpec::None),
            None,
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless {
                prompt: "test".to_string(),
            },
            Status::Created,
            None,
            None,
        )
    }

    fn empty_doc() -> Document {
        Document {
            title: String::new(),
            body_markdown: String::new(),
            path: None,
            deleted: false,
        }
    }

    fn document_payload() -> OperationPayload {
        OperationPayload::Document {
            document_id: None,
            new: empty_doc(),
            old: None,
        }
    }

    fn session_actor(session_id: SessionId) -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Session(session_id),
        }
    }

    fn issue_actor(issue_id: IssueId) -> ActorRef {
        ActorRef::Authenticated {
            actor_id: ActorId::Issue(issue_id),
        }
    }

    /// Transition a session to Running by walking the status state machine.
    async fn make_session_running(store: &MemoryStore, session_id: &SessionId) {
        let mut t = store.get_session(session_id, false).await.unwrap().item;
        t.status = Status::Pending;
        store
            .update_session(session_id, t, &ActorRef::test())
            .await
            .unwrap();
        let mut t = store.get_session(session_id, false).await.unwrap().item;
        t.status = Status::Running;
        store
            .update_session(session_id, t, &ActorRef::test())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn allows_when_actor_is_neither_session_nor_issue() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();
        let payload = document_payload();
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_session_actor_when_session_is_running() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let (session_id, _) = store
            .add_session(make_session(None), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        make_session_running(&store, &session_id).await;

        let payload = document_payload();
        let actor = session_actor(session_id);
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_session_actor_when_session_is_not_running() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let (session_id, _) = store
            .add_session(make_session(None), Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let payload = document_payload();
        let actor = session_actor(session_id.clone());
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let violation = restriction.evaluate(&ctx).await.unwrap_err();
        assert_eq!(violation.policy_name, "running_job_validation");
        assert!(violation.message.contains("not in Running status"));
        assert!(violation.message.contains(&session_id.to_string()));
    }

    #[tokio::test]
    async fn allows_issue_actor_when_running_session_exists() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let (session_id, _) = store
            .add_session(
                make_session(Some(issue_id.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();
        make_session_running(&store, &session_id).await;

        let payload = document_payload();
        let actor = issue_actor(issue_id);
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_issue_actor_when_no_running_session() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        // Add a session spawned from the issue but leave it non-running.
        let (_session_id, _) = store
            .add_session(
                make_session(Some(issue_id.clone())),
                Utc::now(),
                &ActorRef::test(),
            )
            .await
            .unwrap();

        let payload = document_payload();
        let actor = issue_actor(issue_id.clone());
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let violation = restriction.evaluate(&ctx).await.unwrap_err();
        assert_eq!(violation.policy_name, "running_job_validation");
        assert!(violation.message.contains("no running session"));
        assert!(violation.message.contains(&issue_id.to_string()));
    }

    #[tokio::test]
    async fn ignores_non_create_operations() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let (session_id, _) = store
            .add_session(make_session(None), Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        // Session is not running, but update operations must skip the check.

        let payload = document_payload();
        let actor = session_actor(session_id);
        let ctx = RestrictionContext {
            operation: Operation::UpdateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
