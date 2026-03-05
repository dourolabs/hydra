use crate::{domain::agents::Agent, store::ReadOnlyStore};
use metis_common::api::v1::documents::SearchDocumentsQuery;
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
    pub async fn list_agents(&self) -> Result<Vec<Agent>, AgentError> {
        Ok(self.store.list_agents().await?)
    }

    pub async fn get_agent(&self, name: &str) -> Result<Agent, AgentError> {
        self.store.get_agent(name).await.map_err(|e| match e {
            crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
            other => AgentError::Store(other),
        })
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

        info!(agent = %agent_name, "agent updated");
        Ok(updated)
    }

    /// Fetch the prompt text for an agent from the document store.
    ///
    /// Returns an error if `prompt_path` is empty or the document is not found.
    pub async fn resolve_agent_prompt(&self, prompt_path: &str) -> anyhow::Result<String> {
        if prompt_path.is_empty() {
            anyhow::bail!("prompt_path is empty");
        }

        let query =
            SearchDocumentsQuery::new(None, Some(prompt_path.to_string()), Some(true), None, None);

        let documents = self
            .list_documents(&query)
            .await
            .map_err(|e| anyhow::anyhow!("failed to query document store for agent prompt: {e}"))?;

        let (_, versioned) = documents
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no document found at path '{prompt_path}'"))?;

        Ok(versioned.item.body_markdown.trim_end().to_string())
    }

    pub async fn delete_agent(&self, agent_name: &str) -> Result<Agent, AgentError> {
        let agent = self.get_agent(agent_name).await?;

        self.store
            .delete_agent(agent_name)
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
                other => AgentError::Store(other),
            })?;

        info!(agent = %agent_name, "agent deleted");
        Ok(agent)
    }
}
