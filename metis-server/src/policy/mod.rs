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

/// A single-scope engine holding restrictions and automations for one scope
/// (global or a specific repo).
pub(crate) struct ScopedEngine {
    pub(crate) restrictions: Vec<Box<dyn Restriction>>,
    pub(crate) automations: Vec<Box<dyn Automation>>,
}

/// The core policy engine that holds all active restrictions and automations.
///
/// Supports per-repo overrides: when a repo name matches a per-repo entry,
/// that repo's restrictions and automations are used instead of the global
/// defaults. If no per-repo entry exists, the global engine is used.
pub struct PolicyEngine {
    global: ScopedEngine,
    repo_engines: std::collections::HashMap<String, ScopedEngine>,
}

impl PolicyEngine {
    /// Create a new policy engine with the given restrictions and automations
    /// (global scope only, no per-repo overrides).
    pub fn new(
        restrictions: Vec<Box<dyn Restriction>>,
        automations: Vec<Box<dyn Automation>>,
    ) -> Self {
        Self {
            global: ScopedEngine {
                restrictions,
                automations,
            },
            repo_engines: std::collections::HashMap::new(),
        }
    }

    /// Create a policy engine with global restrictions/automations and
    /// per-repo override engines.
    pub(crate) fn with_repo_engines(
        restrictions: Vec<Box<dyn Restriction>>,
        automations: Vec<Box<dyn Automation>>,
        repo_engines: std::collections::HashMap<String, PolicyEngine>,
    ) -> Self {
        let converted = repo_engines
            .into_iter()
            .map(|(name, engine)| (name, engine.global))
            .collect();
        Self {
            global: ScopedEngine {
                restrictions,
                automations,
            },
            repo_engines: converted,
        }
    }

    /// Create an empty policy engine with no restrictions or automations.
    pub fn empty() -> Self {
        Self {
            global: ScopedEngine {
                restrictions: Vec::new(),
                automations: Vec::new(),
            },
            repo_engines: std::collections::HashMap::new(),
        }
    }

    /// Returns the scoped engine for a given repo name, falling back to
    /// the global engine if no per-repo override exists.
    #[allow(dead_code)]
    pub(crate) fn engine_for_repo(&self, repo_name: &str) -> &ScopedEngine {
        self.repo_engines.get(repo_name).unwrap_or(&self.global)
    }

    /// Evaluate all restrictions for a proposed operation (global scope).
    /// Returns the first violation encountered, if any.
    pub async fn check_restrictions(
        &self,
        ctx: &RestrictionContext<'_>,
    ) -> Result<(), PolicyViolation> {
        for restriction in &self.global.restrictions {
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
        for automation in &self.global.automations {
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

    /// Returns the number of registered restrictions (global scope).
    pub fn restriction_count(&self) -> usize {
        self.global.restrictions.len()
    }

    /// Returns the number of registered automations (global scope).
    pub fn automation_count(&self) -> usize {
        self.global.automations.len()
    }

    /// Returns the number of per-repo engine overrides.
    pub fn repo_engine_count(&self) -> usize {
        self.repo_engines.len()
    }

    // ----- Shortcut methods for each mutation type -----

    /// Check restrictions for creating an issue.
    pub async fn check_create_issue(
        &self,
        new: &crate::domain::issues::Issue,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating an issue.
    pub async fn check_update_issue(
        &self,
        issue_id: &metis_common::IssueId,
        new: &crate::domain::issues::Issue,
        old: Option<&crate::domain::issues::Issue>,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a patch.
    pub async fn check_create_patch(
        &self,
        new: &crate::domain::patches::Patch,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for creating a document.
    pub async fn check_create_document(
        &self,
        new: &crate::domain::documents::Document,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a document.
    pub async fn check_update_document(
        &self,
        document_id: &metis_common::DocumentId,
        new: &crate::domain::documents::Document,
        old: Option<&crate::domain::documents::Document>,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }

    /// Check restrictions for updating a job/task status.
    pub async fn check_update_job(
        &self,
        task_id: &metis_common::TaskId,
        new: &crate::store::Task,
        old: Option<&crate::store::Task>,
        store: &dyn crate::store::ReadOnlyStore,
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
        };
        self.check_restrictions(&ctx).await
    }
}

#[cfg(test)]
mod tests;
