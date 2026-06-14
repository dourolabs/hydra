use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Enforces singleton uniqueness on agent role flags:
///
/// - At most one non-archived agent may have `is_default_conversation_agent = true`.
///
/// This is workflow-level cross-row collision check (the same shape as
/// `branch-name collisions` in `docs/architecture/domain-store-routes.md`) and
/// therefore belongs in a `Restriction` rather than the persistence layer.
#[derive(Default)]
pub struct AgentRoleUniquenessRestriction;

impl AgentRoleUniquenessRestriction {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Restriction for AgentRoleUniquenessRestriction {
    fn name(&self) -> &str {
        "agent_role_uniqueness"
    }

    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
        if !matches!(
            ctx.operation,
            Operation::CreateAgent | Operation::UpdateAgent
        ) {
            return Ok(());
        }

        let OperationPayload::Agent { name, new, .. } = ctx.payload else {
            return Ok(());
        };

        if !new.is_default_conversation_agent {
            return Ok(());
        }

        let agents = ctx.store.list_agents().await.map_err(|e| PolicyViolation {
            policy_name: self.name().to_string(),
            message: format!("Failed to list agents: {e}"),
        })?;

        let self_name = name.as_deref();

        if new.is_default_conversation_agent
            && agents.iter().any(|a| {
                a.is_default_conversation_agent && !a.archived && Some(a.name.as_str()) != self_name
            })
        {
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: "Only one default conversation agent is allowed.".to_string(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::agents::Agent;
    use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
    use crate::store::{MemoryStore, Store};

    fn sample_agent(name: &str) -> Agent {
        Agent::new(
            name.to_string(),
            format!("/agents/{name}/prompt.md"),
            None,
            3,
            i32::MAX,
            i32::MAX,
            false,
            Vec::new(),
        )
    }

    async fn evaluate(
        restriction: &AgentRoleUniquenessRestriction,
        store: &MemoryStore,
        op: Operation,
        new: Agent,
        name: Option<String>,
    ) -> Result<(), PolicyViolation> {
        let payload = OperationPayload::Agent {
            name,
            new,
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: op,
            payload: &payload,
            store,
            actor: &actor,
        };
        restriction.evaluate(&ctx).await
    }

    #[tokio::test]
    async fn rejects_second_default_conversation_agent_on_create() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        let result = evaluate(&restriction, &store, Operation::CreateAgent, chat2, None).await;
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "agent_role_uniqueness");
        assert!(violation.message.contains("default conversation agent"));
    }

    #[tokio::test]
    async fn ignores_non_agent_operations() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        store.add_agent(chat).await.unwrap();

        // CreateIssue should not be touched by this restriction even though
        // the agent role is taken.
        let payload = OperationPayload::Agent {
            name: None,
            new: {
                let mut a = sample_agent("chat2");
                a.is_default_conversation_agent = true;
                a
            },
            old: None,
        };
        let actor = ActorRef::test();
        let ctx = RestrictionContext {
            operation: Operation::CreateIssue,
            payload: &payload,
            store: &store,
            actor: &actor,
        };
        assert!(restriction.evaluate(&ctx).await.is_ok());
    }
}
