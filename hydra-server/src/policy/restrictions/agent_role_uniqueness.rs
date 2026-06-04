use async_trait::async_trait;

use crate::policy::context::{Operation, OperationPayload, RestrictionContext};
use crate::policy::{PolicyViolation, Restriction};

/// Enforces singleton uniqueness on agent role flags:
///
/// - At most one non-deleted agent may have `is_assignment_agent = true`.
/// - At most one non-deleted agent may have `is_default_conversation_agent = true`.
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

        if !new.is_assignment_agent && !new.is_default_conversation_agent {
            return Ok(());
        }

        let agents = ctx.store.list_agents().await.map_err(|e| PolicyViolation {
            policy_name: self.name().to_string(),
            message: format!("Failed to list agents: {e}"),
        })?;

        let self_name = name.as_deref();

        if new.is_assignment_agent
            && agents
                .iter()
                .any(|a| a.is_assignment_agent && !a.deleted && Some(a.name.as_str()) != self_name)
        {
            return Err(PolicyViolation {
                policy_name: self.name().to_string(),
                message: "Only one assignment agent is allowed.".to_string(),
            });
        }

        if new.is_default_conversation_agent
            && agents.iter().any(|a| {
                a.is_default_conversation_agent && !a.deleted && Some(a.name.as_str()) != self_name
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
            false,
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
    async fn allows_first_assignment_agent() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        let result = evaluate(&restriction, &store, Operation::CreateAgent, pm, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejects_second_assignment_agent_on_create() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        let result = evaluate(&restriction, &store, Operation::CreateAgent, pm2, None).await;
        let violation = result.unwrap_err();
        assert_eq!(violation.policy_name, "agent_role_uniqueness");
        assert!(violation.message.contains("assignment agent"));
    }

    #[tokio::test]
    async fn allows_self_update_keeping_assignment_flag() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm.clone()).await.unwrap();

        let result = evaluate(
            &restriction,
            &store,
            Operation::UpdateAgent,
            pm,
            Some("pm".to_string()),
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rejects_setting_assignment_flag_on_other_agent() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.add_agent(sample_agent("swe")).await.unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_assignment_agent = true;
        let result = evaluate(
            &restriction,
            &store,
            Operation::UpdateAgent,
            swe_updated,
            Some("swe".to_string()),
        )
        .await;
        assert!(result.is_err());
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
    async fn deleted_assignment_agent_does_not_block_new_one() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();
        store.delete_agent("pm").await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        let result = evaluate(&restriction, &store, Operation::CreateAgent, pm2, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ignores_non_agent_operations() {
        let restriction = AgentRoleUniquenessRestriction::new();
        let store = MemoryStore::new();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        store.add_agent(pm).await.unwrap();

        // CreateIssue should not be touched by this restriction even though
        // the agent role is taken.
        let payload = OperationPayload::Agent {
            name: None,
            new: {
                let mut a = sample_agent("pm2");
                a.is_assignment_agent = true;
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
