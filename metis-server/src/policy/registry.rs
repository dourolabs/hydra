use super::config::PolicyConfig;
use super::{Automation, PolicyEngine, Restriction};
use std::collections::HashMap;

/// Factory function type for creating restrictions from optional TOML params.
pub type RestrictionFactory =
    Box<dyn Fn(Option<&toml::Value>) -> Result<Box<dyn Restriction>, String> + Send + Sync>;

/// Factory function type for creating automations from optional TOML params.
pub type AutomationFactory =
    Box<dyn Fn(Option<&toml::Value>) -> Result<Box<dyn Automation>, String> + Send + Sync>;

/// Registry that maps policy names to factory functions and builds a
/// `PolicyEngine` from a `PolicyConfig`.
pub struct PolicyRegistry {
    restriction_factories: HashMap<String, RestrictionFactory>,
    automation_factories: HashMap<String, AutomationFactory>,
}

impl PolicyRegistry {
    pub fn new() -> Self {
        Self {
            restriction_factories: HashMap::new(),
            automation_factories: HashMap::new(),
        }
    }

    /// Register a restriction factory under the given name.
    pub fn register_restriction<F>(&mut self, name: &str, factory: F)
    where
        F: Fn(Option<&toml::Value>) -> Result<Box<dyn Restriction>, String> + Send + Sync + 'static,
    {
        self.restriction_factories
            .insert(name.to_string(), Box::new(factory));
    }

    /// Register an automation factory under the given name.
    pub fn register_automation<F>(&mut self, name: &str, factory: F)
    where
        F: Fn(Option<&toml::Value>) -> Result<Box<dyn Automation>, String> + Send + Sync + 'static,
    {
        self.automation_factories
            .insert(name.to_string(), Box::new(factory));
    }

    /// Build a `PolicyEngine` from the global policy list in the given config.
    ///
    /// Returns an error if any referenced policy name is not registered.
    pub fn build(&self, config: &PolicyConfig) -> Result<PolicyEngine, String> {
        let mut restrictions: Vec<Box<dyn Restriction>> = Vec::new();
        let mut automations: Vec<Box<dyn Automation>> = Vec::new();

        for entry in &config.global.restrictions {
            let name = entry.name();
            let factory = self
                .restriction_factories
                .get(name)
                .ok_or_else(|| format!("unknown restriction policy: '{name}'"))?;
            let restriction = factory(entry.params())?;
            restrictions.push(restriction);
        }

        for entry in &config.global.automations {
            let name = entry.name();
            let factory = self
                .automation_factories
                .get(name)
                .ok_or_else(|| format!("unknown automation policy: '{name}'"))?;
            let automation = factory(entry.params())?;
            automations.push(automation);
        }

        Ok(PolicyEngine::new(restrictions, automations))
    }
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `PolicyRegistry` pre-loaded with all built-in policies
/// (restrictions and automations).
pub fn build_default_registry() -> PolicyRegistry {
    use super::automations::*;
    use super::integrations::GithubPrSyncAutomation;
    use super::restrictions::*;

    let mut registry = PolicyRegistry::new();

    // Restrictions
    registry.register_restriction("issue_lifecycle_validation", |_params| {
        Ok(Box::new(IssueLifecycleRestriction::new()))
    });
    registry.register_restriction("task_state_machine", |_params| {
        Ok(Box::new(TaskStateMachineRestriction::new()))
    });
    registry.register_restriction("duplicate_branch_name", |_params| {
        Ok(Box::new(DuplicateBranchRestriction::new()))
    });
    registry.register_restriction("hidden_document_path", |_params| {
        Ok(Box::new(HiddenDocumentPathRestriction::new()))
    });
    registry.register_restriction("running_job_validation", |_params| {
        Ok(Box::new(RunningJobValidationRestriction::new()))
    });
    registry.register_restriction("require_creator", |_params| {
        Ok(Box::new(RequireCreatorRestriction::new()))
    });

    // Automations (order matters: cascade must run before kill_tasks)
    registry.register_automation("cascade_issue_status", |params| {
        Ok(Box::new(CascadeIssueStatusAutomation::new(params)?))
    });
    registry.register_automation("kill_tasks_on_issue_failure", |params| {
        Ok(Box::new(KillTasksOnFailureAutomation::new(params)?))
    });
    registry.register_automation("close_merge_request_issues", |params| {
        Ok(Box::new(CloseMergeRequestIssuesAutomation::new(params)?))
    });
    registry.register_automation("create_merge_request_issue", |params| {
        Ok(Box::new(CreateMergeRequestIssueAutomation::new(params)?))
    });
    registry.register_automation("inherit_creator_from_parent", |params| {
        Ok(Box::new(InheritCreatorAutomation::new(params)?))
    });
    registry.register_automation("github_pr_sync", |params| {
        Ok(Box::new(GithubPrSyncAutomation::new(params)?))
    });

    registry
}
