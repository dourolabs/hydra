pub mod automations;
pub mod config;
pub mod context;
pub mod integrations;
pub mod registry;
pub mod restrictions;
pub mod runner;

use crate::app::event_bus::{EventType, ServerEvent};
use async_trait::async_trait;
use context::{AutomationContext, RestrictionContext};
use futures::FutureExt;
use std::any::Any;
use std::fmt;
use std::panic::AssertUnwindSafe;

/// Extract a human-readable message from a `catch_unwind` panic payload.
pub fn panic_message(payload: &Box<dyn Any + Send>) -> &str {
    payload
        .downcast_ref::<String>()
        .map(|s| s.as_str())
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic")
}

/// A structured error returned when a restriction rejects a proposed mutation.
///
/// The `message` field must be descriptive and actionable — agents rely on it
/// to determine how to resolve the problem (e.g., "Cannot close issue i-abc123:
/// 2 child issues are still open (i-def456, i-ghi789). Close or drop all
/// children first.").
#[derive(Debug, Clone)]
pub struct PolicyViolation {
    pub policy_name: String,
    pub message: String,
}

impl fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.policy_name, self.message)
    }
}

impl std::error::Error for PolicyViolation {}

/// Error type returned by automations.
#[derive(Debug, thiserror::Error)]
pub enum AutomationError {
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Describes which events an automation subscribes to.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Which event types to match. Empty means match all.
    pub event_types: Vec<EventType>,
    /// Event types to explicitly exclude, checked after `event_types`.
    pub exclude_event_types: Vec<EventType>,
}

impl EventFilter {
    /// Returns `true` if this filter matches the given event.
    pub fn matches(&self, event: &ServerEvent) -> bool {
        let et = event.event_type();
        if !self.event_types.is_empty() && !self.event_types.contains(&et) {
            return false;
        }
        if self.exclude_event_types.contains(&et) {
            return false;
        }
        true
    }
}

/// A policy that validates a proposed mutation before it is persisted.
/// Returning `Err` rejects the mutation with a descriptive violation.
///
/// # Adding a new restriction
///
/// Registering a restriction takes three steps. Skipping any of them leaves
/// it silently inactive in production deployments:
///
/// 1. **Implement `Restriction`** on a struct, typically in
///    `hydra-server/src/policy/restrictions/`. Pick a stable `name()` —
///    it is the key in YAML config and in logs.
/// 2. **Register a factory** under that name in
///    [`crate::policy::registry::build_default_registry`]. The factory
///    receives the optional YAML `params` and produces a `Box<dyn Restriction>`.
/// 3. **Add the name to the active list** in
///    [`crate::app::default_policy_config`] if it should run by
///    default, OR document it as opt-in (the operator must add it to their
///    `policies.restrictions` config). Anything registered in step 2 but
///    absent from both the default list and the operator's config is
///    inert — the registry is just a name → factory map; activation comes
///    from the `PolicyList`.
///
/// ```ignore
/// // Step 1 (restrictions/my_restriction.rs):
/// pub struct MyRestriction;
/// #[async_trait]
/// impl Restriction for MyRestriction {
///     fn name(&self) -> &str { "my_restriction" }
///     async fn evaluate(&self, _ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation> {
///         Ok(())
///     }
/// }
///
/// // Step 2 (in build_default_registry):
/// registry.register_restriction("my_restriction", |_params| Ok(Box::new(MyRestriction)));
///
/// // Step 3 (in default_policy_config, optional):
/// PolicyEntry::Name("my_restriction".to_string()),
/// ```
#[async_trait]
pub trait Restriction: Send + Sync {
    /// A unique name for this restriction (used in config and logging).
    fn name(&self) -> &str;

    /// Evaluate the restriction against a proposed mutation.
    /// Return `Ok(())` to allow or `Err(PolicyViolation)` to reject.
    async fn evaluate(&self, ctx: &RestrictionContext<'_>) -> Result<(), PolicyViolation>;
}

/// A policy that reacts to a successfully persisted event by performing
/// side effects.
///
/// # Adding a new automation
///
/// Registering an automation takes three steps — the same shape as
/// [`Restriction`]:
///
/// 1. **Implement `Automation`** on a struct in
///    `hydra-server/src/policy/automations/`. Pick a stable `name()`.
/// 2. **Register a factory** under that name in
///    [`crate::policy::registry::build_default_registry`].
/// 3. **Add the name to the active list** in
///    [`crate::app::default_policy_config`] if it should run by
///    default. Registration alone does **not** activate the automation; it
///    must also appear in the active `PolicyList` returned by
///    `default_policy_config()` or in the operator's `policies.automations`
///    config. This is the most common debugging gotcha — see the
///    `default_policy_config` docs for the activation-vs-registration split.
///
/// # Event filtering and silent paths
///
/// [`Self::event_filter`] selects which [`ServerEvent`] variants trigger
/// the automation; the runner skips any event that does not match. Inside
/// [`Self::execute`], **every silent early-return path should emit a
/// `tracing::warn!` with structured fields** (`automation`, plus any
/// relevant entity id) so operators can see why an automation no-op'd
/// instead of acting. Hard misconfigurations that prevent the automation
/// from doing its job (e.g. missing default agent, missing prompt
/// document) should return [`AutomationError`] so the runner surfaces them
/// at `error!` level via its `"automation failed"` log — these are far
/// more visible than a `warn!` in noisy production logs.
///
/// ```ignore
/// // Step 1 (automations/my_automation.rs):
/// pub struct MyAutomation;
/// #[async_trait]
/// impl Automation for MyAutomation {
///     fn name(&self) -> &str { "my_automation" }
///     fn event_filter(&self) -> EventFilter {
///         EventFilter { event_types: vec![EventType::IssueCreated], ..Default::default() }
///     }
///     async fn execute(&self, _ctx: &AutomationContext<'_>) -> Result<(), AutomationError> {
///         Ok(())
///     }
/// }
///
/// // Step 2 (in build_default_registry):
/// registry.register_automation("my_automation", |params| Ok(Box::new(MyAutomation::new(params)?)));
///
/// // Step 3 (in default_policy_config, optional):
/// PolicyEntry::Name("my_automation".to_string()),
/// ```
#[async_trait]
pub trait Automation: Send + Sync {
    /// A unique name for this automation (used in config and logging).
    fn name(&self) -> &str;

    /// Which events this automation subscribes to.
    fn event_filter(&self) -> EventFilter;

    /// Execute the automation's side effects.
    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError>;
}

/// The core policy engine that holds all active restrictions and automations.
pub struct PolicyEngine {
    restrictions: Vec<Box<dyn Restriction>>,
    automations: Vec<Box<dyn Automation>>,
}

impl PolicyEngine {
    /// Create a new policy engine with the given restrictions and automations.
    pub fn new(
        restrictions: Vec<Box<dyn Restriction>>,
        automations: Vec<Box<dyn Automation>>,
    ) -> Self {
        Self {
            restrictions,
            automations,
        }
    }

    /// Create an empty policy engine with no restrictions or automations.
    pub fn empty() -> Self {
        Self {
            restrictions: Vec::new(),
            automations: Vec::new(),
        }
    }

    /// Evaluate all restrictions for a proposed operation.
    /// Returns the first violation encountered, if any.
    pub async fn check_restrictions(
        &self,
        ctx: &RestrictionContext<'_>,
    ) -> Result<(), PolicyViolation> {
        for restriction in &self.restrictions {
            restriction.evaluate(ctx).await?;
        }
        Ok(())
    }

    /// Run all automations whose event filter matches the given event.
    /// Errors are logged but do not fail the original operation.
    pub async fn run_automations(&self, ctx: &AutomationContext<'_>) {
        // Automations always run from the global engine (not per-repo)
        // because automations react to events and the event bus doesn't
        // have a per-repo scope.
        for automation in &self.automations {
            if automation.event_filter().matches(ctx.event) {
                let name = automation.name().to_owned();
                match AssertUnwindSafe(automation.execute(ctx))
                    .catch_unwind()
                    .await
                {
                    Ok(Err(e)) => {
                        tracing::error!(
                            automation = %name,
                            error = %e,
                            "automation failed"
                        );
                    }
                    Err(panic_payload) => {
                        let msg = panic_message(&panic_payload);
                        tracing::error!(
                            automation = %name,
                            panic = %msg,
                            "automation panicked"
                        );
                    }
                    Ok(Ok(())) => {}
                }
            }
        }
    }

    /// Returns the number of registered restrictions.
    pub fn restriction_count(&self) -> usize {
        self.restrictions.len()
    }

    /// Returns the number of registered automations.
    pub fn automation_count(&self) -> usize {
        self.automations.len()
    }

    // ----- Shortcut methods for each mutation type -----

    /// Check restrictions for creating an issue.
    pub async fn check_create_issue(
        &self,
        new: &crate::domain::issues::Issue,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Issue {
            issue_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreateIssue,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating an issue.
    pub async fn check_update_issue(
        &self,
        issue_id: &hydra_common::IssueId,
        new: &crate::domain::issues::Issue,
        old: Option<&crate::domain::issues::Issue>,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Issue {
            issue_id: Some(issue_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateIssue,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a patch.
    pub async fn check_create_patch(
        &self,
        new: &crate::domain::patches::Patch,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Patch {
            patch_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreatePatch,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a document.
    pub async fn check_create_document(
        &self,
        new: &crate::domain::documents::Document,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Document {
            document_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreateDocument,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a document.
    pub async fn check_update_document(
        &self,
        document_id: &hydra_common::DocumentId,
        new: &crate::domain::documents::Document,
        old: Option<&crate::domain::documents::Document>,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Document {
            document_id: Some(document_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateDocument,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating an agent.
    pub async fn check_create_agent(
        &self,
        new: &crate::domain::agents::Agent,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Agent {
            name: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreateAgent,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating an agent.
    pub async fn check_update_agent(
        &self,
        name: &str,
        new: &crate::domain::agents::Agent,
        old: Option<&crate::domain::agents::Agent>,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Agent {
            name: Some(name.to_string()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateAgent,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a job/task status.
    pub async fn check_update_job(
        &self,
        task_id: &hydra_common::SessionId,
        new: &crate::store::Session,
        old: Option<&crate::store::Session>,
        store: &dyn crate::store::ReadOnlyStore,
        actor: &crate::domain::actors::ActorRef,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Job {
            task_id: Some(task_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateJob,
            payload: &payload,
            store,
            actor,
        };
        self.check_restrictions(&ctx).await
    }
}

#[cfg(test)]
mod tests;
