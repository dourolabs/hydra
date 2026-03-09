use async_trait::async_trait;
use metis_common::TaskId;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};
use crate::store::Status;

/// Validates that when `created_by` is a job ID, the job must be in Running status.
#[derive(Default)]
pub struct RunningJobValidationRestriction;

impl RunningJobValidationRestriction {
    pub fn new() -> Self {
        Self
    }
}

impl RunningJobValidationRestriction {
    fn extract_job_id<'a>(&self, ctx: &'a RestrictionContext<'_>) -> Option<&'a TaskId> {
        match (ctx.operation, ctx.payload) {
            (Operation::CreatePatch, OperationPayload::Patch { new, .. }) => {
                new.created_by.as_ref()
            }
            (Operation::CreateDocument, OperationPayload::Document { new, .. }) => {
                new.created_by.as_ref()
            }
            _ => None,
        }
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

#[async_trait]
impl Restriction for RunningJobValidationRestriction {
    fn name(&self) -> &str {
        "running_job_validation"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        let Some(job_id) = self.extract_job_id(ctx) else {
            return Ok(());
        };

        let task = ctx
            .store
            .get_task(job_id, false)
            .await
            .map_err(|e| PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!("Failed to look up job {job_id}: {e}"),
            })?;

        if task.item.status != Status::Running {
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!(
                    "Job {job_id} is not in Running status (current: {}). \
                     Only running jobs can create/modify resources.",
                    status_str(task.item.status)
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::domain::jobs::{BundleSpec, Task};
    use crate::domain::task_status::Status;
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, ReadOnlyStore, Store};
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_task() -> Task {
        Task::new(
            "test".to_string(),
            BundleSpec::None,
            None,
            Username::from("test-creator"),
            None,
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

    fn make_doc(created_by: Option<TaskId>) -> Document {
        Document {
            title: String::new(),
            body_markdown: String::new(),
            path: None,
            created_by,
            deleted: false,
        }
    }

    #[tokio::test]
    async fn allows_when_no_job_id() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();
        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(None),
            old: None,
        };
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
    async fn allows_running_job() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let task = make_task();
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();
        // Transition to running
        let mut t = store.get_task(&task_id, false).await.unwrap().item;
        t.status = Status::Pending;
        store
            .update_task(&task_id, t, &ActorRef::test())
            .await
            .unwrap();
        let mut t = store.get_task(&task_id, false).await.unwrap().item;
        t.status = Status::Running;
        store
            .update_task(&task_id, t, &ActorRef::test())
            .await
            .unwrap();

        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(Some(task_id)),
            old: None,
        };
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
    async fn rejects_non_running_job() {
        let restriction = RunningJobValidationRestriction::new();
        let store = MemoryStore::new();

        let task = make_task();
        let (task_id, _) = store
            .add_task(task, Utc::now(), &ActorRef::test())
            .await
            .unwrap();

        let payload = OperationPayload::Document {
            document_id: None,
            new: make_doc(Some(task_id.clone())),
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateDocument,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "running_job_validation");
        assert!(violation.message.contains("not in Running status"));
        assert!(violation.message.contains(&task_id.to_string()));
    }
}
