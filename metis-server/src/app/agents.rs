use crate::{background::AgentQueue, config::AgentQueueConfig};
use std::sync::Arc;
use thiserror::Error;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent '{name}' already exists")]
    AlreadyExists { name: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
}

impl AppState {
    pub async fn list_agent_configs(&self) -> Vec<AgentQueueConfig> {
        self.agents
            .read()
            .await
            .iter()
            .map(|agent| agent.as_config())
            .collect()
    }

    pub async fn get_agent_config(&self, name: &str) -> Option<AgentQueueConfig> {
        self.agents
            .read()
            .await
            .iter()
            .find(|agent| agent.name == name)
            .map(|agent| agent.as_config())
    }

    pub async fn agent_queues(&self) -> Vec<Arc<AgentQueue>> {
        self.agents.read().await.clone()
    }

    pub async fn create_agent(
        &self,
        agent: AgentQueueConfig,
    ) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;
        if agents.iter().any(|existing| existing.name == agent.name) {
            return Err(AgentError::AlreadyExists {
                name: agent.name.clone(),
            });
        }

        let created = Arc::new(AgentQueue::from_config(&agent));
        agents.push(created.clone());

        Ok(created.as_config())
    }

    pub async fn update_agent(
        &self,
        agent_name: &str,
        updated: AgentQueueConfig,
    ) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;

        if updated.name != agent_name && agents.iter().any(|existing| existing.name == updated.name)
        {
            return Err(AgentError::AlreadyExists {
                name: updated.name.clone(),
            });
        }

        let Some(index) = agents.iter().position(|agent| agent.name == agent_name) else {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        };

        let replacement = Arc::new(AgentQueue::from_config(&updated));
        agents[index] = replacement.clone();

        Ok(replacement.as_config())
    }

    pub async fn delete_agent(&self, agent_name: &str) -> Result<AgentQueueConfig, AgentError> {
        let mut agents = self.agents.write().await;

        let Some(index) = agents.iter().position(|agent| agent.name == agent_name) else {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        };

        let removed = agents.remove(index);
        Ok(removed.as_config())
    }
}
