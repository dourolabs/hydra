use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};
use crate::store::Status;

/// Validates task status transitions follow the state machine:
/// Created -> Pending -> Running -> Complete/Failed
/// with idempotent self-transitions for terminal states.
#[derive(Default)]
pub struct TaskStateMachineRestriction;

impl TaskStateMachineRestriction {
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

fn valid_transitions(from: Status) -> &'static [Status] {
    match from {
        Status::Created => &[Status::Pending, Status::Failed],
        Status::Pending => &[Status::Running, Status::Complete, Status::Failed],
        Status::Running => &[Status::Complete, Status::Failed],
        // Terminal states: only self-transitions are valid (idempotent)
        Status::Complete => &[Status::Complete],
        Status::Failed => &[Status::Failed],
    }
}

#[async_trait]
impl Restriction for TaskStateMachineRestriction {
    fn name(&self) -> &str {
        "task_state_machine"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        if ctx.operation != Operation::UpdateJob {
            return Ok(());
        }

        let OperationPayload::Job { new, old, .. } = ctx.payload else {
            return Ok(());
        };

        let Some(old) = old else {
            return Ok(());
        };

        let current = old.status;
        let target = new.status;

        // Check if this transition is valid
        let allowed = valid_transitions(current);
        if !allowed.contains(&target) {
            let valid_str = allowed
                .iter()
                .map(|s| status_str(*s))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: format!(
                    "Cannot transition task from {current} to {target}. \
                     Valid transitions from {current} are: {valid_str}.",
                    current = status_str(current),
                    target = status_str(target),
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
    use crate::domain::jobs::{BundleSpec, Task};
    use crate::domain::users::Username;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::MemoryStore;
    use std::collections::HashMap;

    fn make_task_with_status(status: Status) -> Task {
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
            status,
            None,
            None,
        )
    }

    #[tokio::test]
    async fn allows_valid_transition_pending_to_running() {
        let restriction = TaskStateMachineRestriction::new();
        let store = MemoryStore::new();
        let old = make_task_with_status(Status::Pending);
        let new = make_task_with_status(Status::Running);
        let payload = OperationPayload::Job {
            task_id: None,
            new,
            old: Some(old),
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::UpdateJob,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn allows_idempotent_terminal_transition() {
        let restriction = TaskStateMachineRestriction::new();
        let store = MemoryStore::new();
        let old = make_task_with_status(Status::Complete);
        let new = make_task_with_status(Status::Complete);
        let payload = OperationPayload::Job {
            task_id: None,
            new,
            old: Some(old),
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::UpdateJob,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_invalid_transition() {
        let restriction = TaskStateMachineRestriction::new();
        let store = MemoryStore::new();
        let old = make_task_with_status(Status::Complete);
        let new = make_task_with_status(Status::Running);
        let payload = OperationPayload::Job {
            task_id: None,
            new,
            old: Some(old),
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::UpdateJob,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        let result = restriction.evaluate(&ctx).await;
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "task_state_machine");
        assert!(violation.message.contains("Cannot transition task"));
        assert!(violation.message.contains("complete"));
        assert!(violation.message.contains("running"));
    }

    #[tokio::test]
    async fn ignores_non_update_operations() {
        let restriction = TaskStateMachineRestriction::new();
        let store = MemoryStore::new();
        let new = make_task_with_status(Status::Running);
        let payload = OperationPayload::Job {
            task_id: None,
            new,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateJob,
            actor: &actor,
            payload: &payload,
            store: &store,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
