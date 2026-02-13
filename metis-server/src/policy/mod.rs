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
use std::fmt;

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
}

impl EventFilter {
    /// Returns `true` if this filter matches the given event.
    pub fn matches(&self, event: &ServerEvent) -> bool {
        if self.event_types.is_empty() {
            return true;
        }
        self.event_types.contains(&event.event_type())
    }
}

/// A policy that validates a proposed mutation before it is persisted.
/// Returning `Err` rejects the mutation with a descriptive violation.
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
#[async_trait]
pub trait Automation: Send + Sync {
    /// A unique name for this automation (used in config and logging).
    fn name(&self) -> &str;

    /// Which events this automation subscribes to.
    fn event_filter(&self) -> EventFilter;

    /// Execute the automation's side effects.
    async fn execute(&self, ctx: &AutomationContext<'_>) -> Result<(), AutomationError>;
}

/// The core policy engine that holds all active restrictions and automations,
/// with optional per-repo overrides.
pub struct PolicyEngine {
    restrictions: Vec<Box<dyn Restriction>>,
    automations: Vec<Box<dyn Automation>>,
    /// Per-repo policy overrides. When a mutation is associated with a repo
    /// listed here, that repo's engine is used instead of the global one.
    repo_overrides: std::collections::HashMap<String, PolicyEngine>,
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
            repo_overrides: std::collections::HashMap::new(),
        }
    }

    /// Create a new policy engine with per-repo overrides.
    pub fn with_repo_overrides(
        restrictions: Vec<Box<dyn Restriction>>,
        automations: Vec<Box<dyn Automation>>,
        repo_overrides: std::collections::HashMap<String, PolicyEngine>,
    ) -> Self {
        Self {
            restrictions,
            automations,
            repo_overrides,
        }
    }

    /// Set per-repo overrides on an existing engine, consuming and returning it.
    pub fn set_repo_overrides(
        mut self,
        repo_overrides: std::collections::HashMap<String, PolicyEngine>,
    ) -> Self {
        self.repo_overrides = repo_overrides;
        self
    }

    /// Create an empty policy engine with no restrictions or automations.
    pub fn empty() -> Self {
        Self {
            restrictions: Vec::new(),
            automations: Vec::new(),
            repo_overrides: std::collections::HashMap::new(),
        }
    }

    /// Resolve which engine to use for the given repo context.
    /// If a per-repo override exists, use it; otherwise use self (global).
    fn resolve_for_repo(&self, repo: Option<&metis_common::RepoName>) -> &PolicyEngine {
        if let Some(repo_name) = repo {
            let key = repo_name.to_string();
            if let Some(override_engine) = self.repo_overrides.get(&key) {
                return override_engine;
            }
        }
        self
    }

    /// Evaluate all restrictions for a proposed operation.
    /// If the context has a repo with a per-repo override, uses that override.
    /// Returns the first violation encountered, if any.
    pub async fn check_restrictions(
        &self,
        ctx: &RestrictionContext<'_>,
    ) -> Result<(), PolicyViolation> {
        let engine = self.resolve_for_repo(ctx.repo);
        for restriction in &engine.restrictions {
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
                if let Err(e) = automation.execute(ctx).await {
                    tracing::error!(
                        automation = automation.name(),
                        error = %e,
                        "automation failed"
                    );
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

    /// Returns the number of per-repo overrides.
    pub fn repo_override_count(&self) -> usize {
        self.repo_overrides.len()
    }

    // ----- Shortcut methods for each mutation type -----

    /// Check restrictions for creating an issue.
    pub async fn check_create_issue(
        &self,
        new: &crate::domain::issues::Issue,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Issue {
            issue_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreateIssue,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating an issue.
    pub async fn check_update_issue(
        &self,
        issue_id: &metis_common::IssueId,
        new: &crate::domain::issues::Issue,
        old: Option<&crate::domain::issues::Issue>,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Issue {
            issue_id: Some(issue_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateIssue,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a patch.
    pub async fn check_create_patch(
        &self,
        new: &crate::domain::patches::Patch,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Patch {
            patch_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreatePatch,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a document.
    pub async fn check_create_document(
        &self,
        new: &crate::domain::documents::Document,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Document {
            document_id: None,
            new: new.clone(),
            old: None,
        };
        let ctx = RestrictionContext {
            operation: context::Operation::CreateDocument,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a document.
    pub async fn check_update_document(
        &self,
        document_id: &metis_common::DocumentId,
        new: &crate::domain::documents::Document,
        old: Option<&crate::domain::documents::Document>,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Document {
            document_id: Some(document_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateDocument,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a job/task status.
    pub async fn check_update_job(
        &self,
        task_id: &metis_common::TaskId,
        new: &crate::store::Task,
        old: Option<&crate::store::Task>,
        store: &dyn crate::store::Store,
    ) -> Result<(), PolicyViolation> {
        let payload = context::OperationPayload::Job {
            task_id: Some(task_id.clone()),
            new: new.clone(),
            old: old.cloned(),
        };
        let ctx = RestrictionContext {
            operation: context::Operation::UpdateJob,
            repo: None,
            payload: &payload,
            store,
        };
        self.check_restrictions(&ctx).await
    }
}

#[cfg(test)]
mod tests;
