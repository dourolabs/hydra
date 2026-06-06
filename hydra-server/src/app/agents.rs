use crate::{
    domain::{actors::ActorRef, agents::Agent},
    store::ReadOnlyStore,
};
use hydra_common::api::v1::documents::SearchDocumentsQuery;
use hydra_common::api::v1::projects::{Project, StatusDefinition};
use hydra_common::api::v1::sessions::McpConfig;
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};

use super::app_state::AppState;

/// Doc-store path for the shared system-prompt slice that every named-agent
/// session inherits. Designed by [[d-rzreslz]] §4: a singleton document edited
/// the same way as today's agent prompts; missing or empty content
/// contributes an empty slice rather than failing the spawn.
pub const SYSTEM_PROMPT_PATH: &str = "/agents/system_prompt.md";

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("agent '{name}' already exists")]
    AlreadyExists { name: String },
    #[error("agent '{name}' not found")]
    NotFound { name: String },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
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

    /// Resolves the agent that should be applied to a conversation session.
    ///
    /// - If `agent_name` is `Some`, fetches that agent by name; returns
    ///   `AgentError::NotFound` if the agent does not exist.
    /// - If `agent_name` is `None`, scans the agent list for the single
    ///   non-deleted agent flagged `is_default_conversation_agent`. Returns
    ///   `Ok(None)` if no such agent exists.
    pub async fn resolve_conversation_agent(
        &self,
        agent_name: Option<&str>,
    ) -> Result<Option<Agent>, AgentError> {
        if let Some(name) = agent_name {
            return self.get_agent(name).await.map(Some);
        }

        let agents = self.list_agents().await?;
        Ok(agents
            .into_iter()
            .find(|agent| agent.is_default_conversation_agent && !agent.deleted))
    }

    pub async fn create_agent(&self, agent: Agent, actor: ActorRef) -> Result<Agent, AgentError> {
        let store = self.store.as_ref();
        self.policy_engine
            .check_create_agent(&agent, store, &actor)
            .await?;

        self.store
            .add_agent(agent.clone())
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentAlreadyExists(name) => {
                    AgentError::AlreadyExists { name }
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
        actor: ActorRef,
    ) -> Result<Agent, AgentError> {
        if updated.name != agent_name {
            return Err(AgentError::NotFound {
                name: agent_name.to_string(),
            });
        }

        let store = self.store.as_ref();
        let old = self.store.get_agent(agent_name).await.ok();
        self.policy_engine
            .check_update_agent(agent_name, &updated, old.as_ref(), store, &actor)
            .await?;

        self.store
            .update_agent(updated.clone())
            .await
            .map_err(|e| match e {
                crate::store::StoreError::AgentNotFound(name) => AgentError::NotFound { name },
                other => AgentError::Store(other),
            })?;

        info!(agent = %agent_name, "agent updated");
        Ok(updated)
    }

    /// Concatenate the four prompt layers — system, agent, project, status —
    /// into the `system_prompt` for a named-agent session.
    ///
    /// The agent layer keeps today's hard-fail semantics: a missing or empty
    /// `agent.prompt_path` is an error. The other three layers tolerate
    /// `None` paths or missing documents and contribute an empty slice
    /// instead (logged at `info`). The final string is the non-empty slices
    /// joined by `\n\n`, so a layer that resolves to empty does not leave a
    /// dangling separator.
    pub async fn resolve_session_system_prompt(
        &self,
        agent: &Agent,
        project: &Project,
        status: &StatusDefinition,
    ) -> anyhow::Result<String> {
        let system = self.resolve_optional_prompt(Some(SYSTEM_PROMPT_PATH)).await;
        let agent_slice = self.resolve_agent_prompt(&agent.prompt_path).await?;
        let project_slice = self
            .resolve_optional_prompt(project.prompt_path.as_deref())
            .await;
        let status_slice = self
            .resolve_optional_prompt(status.prompt_path.as_deref())
            .await;

        let joined = [system, agent_slice, project_slice, status_slice]
            .into_iter()
            .filter(|slice| !slice.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(joined)
    }

    /// None-tolerant variant of [`Self::resolve_agent_prompt`] used for the
    /// system / project / status slices. A `None` path, an empty path, a
    /// missing document, or a store error all collapse to an empty slice
    /// (logged at `info`) — these layers are augmentations and must never
    /// hard-fail a session spawn.
    async fn resolve_optional_prompt(&self, prompt_path: Option<&str>) -> String {
        let Some(path) = prompt_path else {
            return String::new();
        };
        if path.is_empty() {
            return String::new();
        }
        let query = SearchDocumentsQuery::new(None, Some(path.to_string()), Some(true), None);
        let documents = match self.list_documents(&query).await {
            Ok(docs) => docs,
            Err(err) => {
                info!(path = %path, error = %err, "prompt layer document query failed; treating as empty slice");
                return String::new();
            }
        };
        let Some((_, versioned)) = documents.into_iter().next() else {
            info!(path = %path, "prompt layer document not found; treating as empty slice");
            return String::new();
        };
        versioned.item.body_markdown.trim_end().to_string()
    }

    /// Fetch the prompt text for an agent from the document store.
    ///
    /// Returns an error if `prompt_path` is empty or the document is not found.
    pub async fn resolve_agent_prompt(&self, prompt_path: &str) -> anyhow::Result<String> {
        if prompt_path.is_empty() {
            anyhow::bail!("prompt_path is empty");
        }

        let query =
            SearchDocumentsQuery::new(None, Some(prompt_path.to_string()), Some(true), None);

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

    /// Batch-fetch prompt texts for multiple agents with a single document store query.
    ///
    /// Returns a map from agent name to prompt text.
    pub async fn resolve_agent_prompts(&self, agents: &[Agent]) -> HashMap<String, String> {
        self.resolve_batch_from_documents(agents, |agent| Some(agent.prompt_path.as_str()))
            .await
    }

    /// Batch-fetch MCP config content for multiple agents with a single document store query.
    ///
    /// Returns a map from agent name to raw MCP config text.
    pub async fn resolve_mcp_configs_batch(&self, agents: &[Agent]) -> HashMap<String, String> {
        self.resolve_batch_from_documents(agents, |agent| agent.mcp_config_path.as_deref())
            .await
    }

    /// Fetch raw MCP config content for a single agent from the document store.
    pub async fn resolve_mcp_config_content(&self, agent: &Agent) -> Option<String> {
        let mcp_config_path = agent.mcp_config_path.as_deref()?;
        let query =
            SearchDocumentsQuery::new(None, Some(mcp_config_path.to_string()), Some(true), None);
        let documents = self.list_documents(&query).await.ok()?;
        let (_, versioned) = documents.into_iter().next()?;
        Some(versioned.item.body_markdown.trim_end().to_string())
    }

    /// Shared helper that batch-fetches document content for agents.
    ///
    /// Queries documents with the `/agents/` prefix, builds a path-to-body map,
    /// then matches each agent using the provided `path_extractor`.
    async fn resolve_batch_from_documents(
        &self,
        agents: &[Agent],
        path_extractor: impl Fn(&Agent) -> Option<&str>,
    ) -> HashMap<String, String> {
        let query = SearchDocumentsQuery::new(None, Some("/agents/".into()), None, None);

        let documents = match self.list_documents(&query).await {
            Ok(docs) => docs,
            Err(_) => return HashMap::new(),
        };

        let path_to_body: HashMap<String, String> = documents
            .into_iter()
            .filter_map(|(_, versioned)| {
                let path = versioned.item.path.as_ref()?.to_string();
                Some((path, versioned.item.body_markdown.trim_end().to_string()))
            })
            .collect();

        agents
            .iter()
            .filter_map(|agent| {
                let path = path_extractor(agent)?;
                let body = path_to_body.get(path)?;
                Some((agent.name.clone(), body.clone()))
            })
            .collect()
    }

    /// Fetch and parse MCP config for an agent from the document store.
    ///
    /// Returns `None` if the document is not found (logs a warning).
    /// Returns an error only on unexpected failures (e.g. network/parse errors).
    pub async fn resolve_agent_mcp_config(
        &self,
        mcp_config_path: &str,
    ) -> anyhow::Result<Option<McpConfig>> {
        let query =
            SearchDocumentsQuery::new(None, Some(mcp_config_path.to_string()), Some(true), None);

        let documents = self
            .list_documents(&query)
            .await
            .map_err(|e| anyhow::anyhow!("failed to query document store for MCP config: {e}"))?;

        let Some((_, versioned)) = documents.into_iter().next() else {
            warn!(path = %mcp_config_path, "MCP config document not found; leaving mcp_config as None");
            return Ok(None);
        };

        let mcp_config: McpConfig = serde_json::from_str(versioned.item.body_markdown.trim_end())
            .map_err(|e| {
            anyhow::anyhow!("failed to parse MCP config at '{mcp_config_path}' as JSON: {e}")
        })?;

        Ok(Some(mcp_config))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::domain::documents::Document;
    use crate::test_utils::test_state;
    use hydra_common::api::v1::projects::{
        IconKey, Project, ProjectKey, StatusDefinition, StatusKey,
    };
    use hydra_common::api::v1::users::Username;

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

    fn status_def_with_prompt(key: &str, prompt_path: Option<&str>) -> StatusDefinition {
        let mut def = StatusDefinition::new(
            StatusKey::try_new(key).unwrap(),
            key.to_string(),
            IconKey::try_new("circle").unwrap(),
            "#abcdef".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        def.prompt_path = prompt_path.map(str::to_string);
        def
    }

    fn project_with_prompt(prompt_path: Option<&str>) -> Project {
        let mut proj = Project::new(
            ProjectKey::try_new("eng").unwrap(),
            "Engineering".to_string(),
            vec![status_def_with_prompt("open", None)],
            StatusKey::try_new("open").unwrap(),
            Username::try_new("system").unwrap(),
            false,
        );
        proj.prompt_path = prompt_path.map(str::to_string);
        proj
    }

    async fn seed_document(state: &crate::app::AppState, path: &str, body: &str) {
        let doc = Document {
            title: path.to_string(),
            body_markdown: body.to_string(),
            path: Some(path.parse().unwrap()),
            deleted: false,
        };
        state
            .store
            .add_document_with_actor(doc, ActorRef::test())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resolve_session_system_prompt_joins_all_four_layers_in_order() {
        let state = test_state();
        let agent = sample_agent("swe");
        state
            .create_agent(agent.clone(), ActorRef::test())
            .await
            .unwrap();
        seed_document(&state, SYSTEM_PROMPT_PATH, "SYSTEM").await;
        seed_document(&state, &agent.prompt_path, "AGENT").await;
        seed_document(&state, "/projects/eng/prompt.md", "PROJECT").await;
        seed_document(&state, "/projects/eng/statuses/open.md", "STATUS").await;

        let project = project_with_prompt(Some("/projects/eng/prompt.md"));
        let status = status_def_with_prompt("open", Some("/projects/eng/statuses/open.md"));

        let joined = state
            .resolve_session_system_prompt(&agent, &project, &status)
            .await
            .unwrap();
        assert_eq!(joined, "SYSTEM\n\nAGENT\n\nPROJECT\n\nSTATUS");
    }

    #[tokio::test]
    async fn resolve_session_system_prompt_skips_none_layers_without_dangling_separators() {
        let state = test_state();
        let agent = sample_agent("swe");
        state
            .create_agent(agent.clone(), ActorRef::test())
            .await
            .unwrap();
        // No system / project / status docs seeded; the only document present
        // is the agent prompt, so the resolver must emit just that slice with
        // no leading or trailing blank-line gaps.
        seed_document(&state, &agent.prompt_path, "AGENT").await;

        let project = project_with_prompt(None);
        let status = status_def_with_prompt("open", None);

        let joined = state
            .resolve_session_system_prompt(&agent, &project, &status)
            .await
            .unwrap();
        assert_eq!(joined, "AGENT");
    }

    #[tokio::test]
    async fn resolve_session_system_prompt_treats_missing_optional_doc_as_empty_slice() {
        let state = test_state();
        let agent = sample_agent("swe");
        state
            .create_agent(agent.clone(), ActorRef::test())
            .await
            .unwrap();
        seed_document(&state, &agent.prompt_path, "AGENT").await;
        // Project & status point at paths that don't exist in the doc store.
        // The resolver must silently degrade to empty slices.
        let project = project_with_prompt(Some("/projects/eng/prompt.md"));
        let status = status_def_with_prompt("open", Some("/projects/eng/statuses/open.md"));

        let joined = state
            .resolve_session_system_prompt(&agent, &project, &status)
            .await
            .unwrap();
        assert_eq!(joined, "AGENT");
    }

    #[tokio::test]
    async fn resolve_session_system_prompt_hard_fails_on_missing_agent_prompt() {
        let state = test_state();
        let agent = sample_agent("swe");
        state
            .create_agent(agent.clone(), ActorRef::test())
            .await
            .unwrap();
        // Intentionally do not seed the agent's prompt document.
        let project = project_with_prompt(None);
        let status = status_def_with_prompt("open", None);

        let err = state
            .resolve_session_system_prompt(&agent, &project, &status)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no document found at path"),
            "expected missing-doc error from agent layer; got: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_session_system_prompt_hard_fails_on_empty_agent_prompt_path() {
        let state = test_state();
        let mut agent = sample_agent("swe");
        agent.prompt_path = String::new();
        let project = project_with_prompt(None);
        let status = status_def_with_prompt("open", None);

        let err = state
            .resolve_session_system_prompt(&agent, &project, &status)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("prompt_path is empty"),
            "expected empty-prompt-path error from agent layer; got: {err}"
        );
    }

    #[tokio::test]
    async fn create_agent_assignment_uniqueness_blocked_by_restriction() {
        let state = test_state();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        state.create_agent(pm, ActorRef::test()).await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        let err = state.create_agent(pm2, ActorRef::test()).await.unwrap_err();
        assert!(matches!(err, AgentError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn update_agent_assignment_uniqueness_blocked_by_restriction() {
        let state = test_state();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        state.create_agent(pm, ActorRef::test()).await.unwrap();
        state
            .create_agent(sample_agent("swe"), ActorRef::test())
            .await
            .unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_assignment_agent = true;
        let err = state
            .update_agent("swe", swe_updated, ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn create_agent_default_conversation_uniqueness_blocked_by_restriction() {
        let state = test_state();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        state.create_agent(chat, ActorRef::test()).await.unwrap();

        let mut chat2 = sample_agent("chat2");
        chat2.is_default_conversation_agent = true;
        let err = state
            .create_agent(chat2, ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn update_agent_default_conversation_uniqueness_blocked_by_restriction() {
        let state = test_state();
        let mut chat = sample_agent("chat");
        chat.is_default_conversation_agent = true;
        state.create_agent(chat, ActorRef::test()).await.unwrap();
        state
            .create_agent(sample_agent("swe"), ActorRef::test())
            .await
            .unwrap();

        let mut swe_updated = sample_agent("swe");
        swe_updated.is_default_conversation_agent = true;
        let err = state
            .update_agent("swe", swe_updated, ActorRef::test())
            .await
            .unwrap_err();
        assert!(matches!(err, AgentError::PolicyViolation(_)));
    }

    #[tokio::test]
    async fn deleted_assignment_agent_allows_new_one() {
        let state = test_state();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        state.create_agent(pm, ActorRef::test()).await.unwrap();
        state.delete_agent("pm").await.unwrap();

        let mut pm2 = sample_agent("pm2");
        pm2.is_assignment_agent = true;
        state.create_agent(pm2, ActorRef::test()).await.unwrap();
    }

    #[tokio::test]
    async fn assignment_agent_can_update_itself() {
        let state = test_state();
        let mut pm = sample_agent("pm");
        pm.is_assignment_agent = true;
        state.create_agent(pm, ActorRef::test()).await.unwrap();

        let mut pm_updated = sample_agent("pm");
        pm_updated.is_assignment_agent = true;
        pm_updated.max_tries = 10;
        state
            .update_agent("pm", pm_updated, ActorRef::test())
            .await
            .unwrap();

        let fetched = state.get_agent("pm").await.unwrap();
        assert_eq!(fetched.max_tries, 10);
        assert!(fetched.is_assignment_agent);
    }
}
