use crate::{
    background::AgentQueue,
    config::AppConfig,
    job_engine::JobEngine,
    store::{ReadOnlyStore, Store},
};
use octocrab::Octocrab;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::broadcast;

use super::event_bus::{EventBus, ServerEvent, StoreWithEvents};

use super::ServiceState;

/// Shared application state and application-specific coordination such as issue lifecycle validation.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub github_app: Option<Octocrab>,
    pub service_state: Arc<ServiceState>,
    pub(crate) store: Arc<StoreWithEvents>,
    pub job_engine: Arc<dyn JobEngine>,
    pub(crate) agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    pub(crate) policy_engine: Arc<crate::policy::PolicyEngine>,
}

impl AppState {
    pub fn new(
        config: Arc<AppConfig>,
        github_app: Option<Octocrab>,
        service_state: Arc<ServiceState>,
        store: Arc<dyn Store>,
        job_engine: Arc<dyn JobEngine>,
        agents: Arc<RwLock<Vec<Arc<AgentQueue>>>>,
    ) -> Self {
        let event_bus = Arc::new(EventBus::new());
        let policy_engine = Self::build_policy_engine(config.policies.as_ref());
        Self {
            config,
            github_app,
            service_state,
            store: Arc::new(StoreWithEvents::new(store, event_bus)),
            job_engine,
            agents,
            policy_engine: Arc::new(policy_engine),
        }
    }

    /// Build the policy engine from config, or fall back to all built-in
    /// policies with default params when no `[policies]` section is present.
    pub(crate) fn build_policy_engine(
        policy_config: Option<&crate::policy::config::PolicyConfig>,
    ) -> crate::policy::PolicyEngine {
        use crate::policy::config::{PolicyConfig, PolicyEntry, PolicyList};
        use crate::policy::registry::build_default_registry;

        let default_config = PolicyConfig {
            global: PolicyList {
                restrictions: vec![
                    PolicyEntry::Name("issue_lifecycle_validation".to_string()),
                    PolicyEntry::Name("task_state_machine".to_string()),
                    PolicyEntry::Name("duplicate_branch_name".to_string()),
                    PolicyEntry::Name("running_job_validation".to_string()),
                    PolicyEntry::Name("require_creator".to_string()),
                ],
                automations: vec![
                    PolicyEntry::Name("cascade_issue_status".to_string()),
                    PolicyEntry::Name("kill_tasks_on_issue_failure".to_string()),
                    PolicyEntry::Name("close_merge_request_issues".to_string()),
                    PolicyEntry::Name("patch_workflow".to_string()),
                    PolicyEntry::Name("github_pr_sync".to_string()),
                ],
            },
        };

        let config = policy_config.unwrap_or(&default_config);
        let registry = build_default_registry();
        registry
            .build(config)
            .expect("policy configuration should be valid")
    }

    /// Create an AppState with a custom policy engine (useful for testing).
    #[cfg(any(test, feature = "test-utils"))]
    pub fn with_policy_engine(mut self, engine: crate::policy::PolicyEngine) -> Self {
        self.policy_engine = Arc::new(engine);
        self
    }

    /// Returns a new broadcast receiver for server events.
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.store.event_bus().subscribe()
    }

    /// Returns a reference to the event bus.
    pub fn event_bus(&self) -> &EventBus {
        self.store.event_bus()
    }

    /// Returns a reference to the policy engine.
    pub fn policy_engine(&self) -> &crate::policy::PolicyEngine {
        &self.policy_engine
    }

    /// Returns a reference to the underlying store (as a read-only trait object).
    pub fn store(&self) -> &dyn ReadOnlyStore {
        self.store.as_ref()
    }
}
