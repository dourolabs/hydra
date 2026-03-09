use crate::{
    config::AppConfig,
    domain::secrets::SecretManager,
    job_engine::JobEngine,
    store::{ReadOnlyStore, Store},
};
use std::sync::Arc;
use tokio::sync::broadcast;

use super::event_bus::{EventBus, ServerEvent, StoreWithEvents};

use super::ServiceState;

/// Shared application state and application-specific coordination such as issue lifecycle validation.
/// Type alias for the optional GitHub App client. When the `github` feature
/// is disabled, the field is always `None` (using `()` as a zero-size stand-in).
#[cfg(feature = "github")]
pub type GithubAppClient = octocrab::Octocrab;
#[cfg(not(feature = "github"))]
pub type GithubAppClient = ();

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub github_app: Option<GithubAppClient>,
    pub service_state: Arc<ServiceState>,
    pub(crate) store: Arc<StoreWithEvents>,
    pub job_engine: Arc<dyn JobEngine>,
    pub(crate) policy_engine: Arc<crate::policy::PolicyEngine>,
    pub secret_manager: Arc<SecretManager>,
}

impl AppState {
    pub fn new(
        config: Arc<AppConfig>,
        github_app: Option<GithubAppClient>,
        service_state: Arc<ServiceState>,
        store: Arc<dyn Store>,
        job_engine: Arc<dyn JobEngine>,
        secret_manager: Arc<SecretManager>,
    ) -> Self {
        let event_bus = Arc::new(EventBus::new());
        let policy_engine = Self::build_policy_engine(config.policies.as_ref());
        Self {
            config,
            github_app,
            service_state,
            store: Arc::new(StoreWithEvents::new(store, event_bus)),
            job_engine,
            policy_engine: Arc::new(policy_engine),
            secret_manager,
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
                automations: {
                    let mut automations = vec![
                        PolicyEntry::Name("cascade_issue_status".to_string()),
                        PolicyEntry::Name("kill_tasks_on_issue_failure".to_string()),
                        PolicyEntry::Name("close_merge_request_issues".to_string()),
                        PolicyEntry::Name("sync_review_request_issues".to_string()),
                        PolicyEntry::Name("patch_workflow".to_string()),
                    ];
                    #[cfg(feature = "github")]
                    automations.push(PolicyEntry::Name("github_pr_sync".to_string()));
                    automations.push(PolicyEntry::Name("notification_generation".to_string()));
                    automations.push(PolicyEntry::Name("inbox_label".to_string()));
                    automations
                },
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
