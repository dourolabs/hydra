use crate::{background::AgentQueue, domain::agents::Agent, store::ReadOnlyStore};
use std::sync::Arc;
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent '{name}' already exists")]
    AlreadyExists { name: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
    #[error("only one assignment agent is allowed")]
    AssignmentAgentConflict,
    #[error("store error: {0}")]
    Store(#[from] crate::store::StoreError),
}

impl AppState {
    /// Reload the in-memory agent cache from the database.
    pub async fn refresh_agents_from_db(&self) -> Result<(), AgentError> {
        let agents = self.store.list_agents().await?;
        let queues: Vec<Arc<AgentQueue>> = agents
            .iter()
            .map(|agent| Arc::new(AgentQueue::from_record(agent)))
            .collect();
        let mut guard = self.agents.write().await;
        *guard = queues;
        Ok(())
    }

    pub async fn list_agents_from_db(&self) -> Result<Vec<Agent>, AgentError> {
        Ok(self.store.list_agents().await?)
    }

    pub async fn get_agent_from_db(&self, name: &str) -> Result<Agent, AgentError> {
        self.store.get_agent(name).await.map_err(|e| match e {
            crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
            other => AgentError::Store(other),
        })
    }

    pub async fn agent_queues(&self) -> Vec<Arc<AgentQueue>> {
        self.agents.read().await.clone()
    }

    pub async fn create_agent(&self, agent: Agent) -> Result<Agent, AgentError> {
        self.store
            .add_agent(agent.clone())
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentAlreadyExists(name) => {
                    AgentError::AlreadyExists { name }
                }
                crate::store::StoreError::AssignmentAgentAlreadyExists => {
                    AgentError::AssignmentAgentConflict
                }
                other => AgentError::Store(other),
            })?;

        self.refresh_agents_from_db().await?;

        info!(agent = %agent.name, "agent created");
        Ok(agent)
    }

    pub async fn update_agent(
        &self,
        agent_name: &str,
        updated: Agent,
    ) -> Result<Agent, AgentError> {
        if updated.name != agent_name {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        }

        self.store
            .update_agent(updated.clone())
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
                crate::store::StoreError::AssignmentAgentAlreadyExists => {
                    AgentError::AssignmentAgentConflict
                }
                other => AgentError::Store(other),
            })?;

        self.refresh_agents_from_db().await?;

        info!(agent = %agent_name, "agent updated");
        Ok(updated)
    }

    pub async fn delete_agent(&self, agent_name: &str) -> Result<Agent, AgentError> {
        let agent = self.get_agent_from_db(agent_name).await?;

        self.store
            .delete_agent(agent_name)
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
                other => AgentError::Store(other),
            })?;

        self.refresh_agents_from_db().await?;

        info!(agent = %agent_name, "agent deleted");
        Ok(agent)
    }
}
