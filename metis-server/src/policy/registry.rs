use super::config::{PolicyConfig, PolicyList, RepoOverride};
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

    /// Build a `PolicyEngine` from a single `PolicyList`.
    fn build_engine_from_list(&self, list: &PolicyList) -> Result<PolicyEngine, String> {
        let mut restrictions: Vec<Box<dyn Restriction>> = Vec::new();
        let mut automations: Vec<Box<dyn Automation>> = Vec::new();

        for entry in &list.restrictions {
            let name = entry.name();
            let factory = self
                .restriction_factories
                .get(name)
                .ok_or_else(|| format!("unknown restriction policy: '{name}'"))?;
            let restriction = factory(entry.params())?;
            restrictions.push(restriction);
        }

        for entry in &list.automations {
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

    /// Build a `PolicyEngine` from a `RepoOverride` (restrictions only).
    fn build_engine_from_repo_override(
        &self,
        repo_override: &RepoOverride,
    ) -> Result<PolicyEngine, String> {
        let mut restrictions: Vec<Box<dyn Restriction>> = Vec::new();

        for entry in &repo_override.restrictions {
            let name = entry.name();
            let factory = self
                .restriction_factories
                .get(name)
                .ok_or_else(|| format!("unknown restriction policy: '{name}'"))?;
            let restriction = factory(entry.params())?;
            restrictions.push(restriction);
        }

        Ok(PolicyEngine::new(restrictions, Vec::new()))
    }

    /// Build a `PolicyEngine` from a `PolicyConfig`, including per-repo overrides.
    ///
    /// Returns an error if any referenced policy name is not registered or
    /// if any policy parameters are invalid.
    pub fn build(&self, config: &PolicyConfig) -> Result<PolicyEngine, String> {
        let global_engine = self.build_engine_from_list(&config.global)?;

        let mut repo_overrides: HashMap<String, PolicyEngine> = HashMap::new();
        for (repo_name, repo_override) in &config.repos {
            let repo_engine = self.build_engine_from_repo_override(repo_override)?;
            repo_overrides.insert(repo_name.clone(), repo_engine);
        }

        Ok(global_engine.set_repo_overrides(repo_overrides))
    }

    /// Validate a `PolicyConfig` without building a full engine.
    ///
    /// Returns an error on unknown policy names or invalid params.
    pub fn validate_config(&self, config: &PolicyConfig) -> Result<(), anyhow::Error> {
        self.validate_list(&config.global, "global")?;
        for (repo_name, repo_override) in &config.repos {
            self.validate_repo_override(repo_override, &format!("repos.\"{repo_name}\""))?;
        }
        Ok(())
    }

    fn validate_list(&self, list: &PolicyList, scope: &str) -> Result<(), anyhow::Error> {
        for entry in &list.restrictions {
            let name = entry.name();
            if !self.restriction_factories.contains_key(name) {
                anyhow::bail!("unknown restriction policy '{name}' in {scope}");
            }
            let factory = &self.restriction_factories[name];
            factory(entry.params()).map_err(|e| {
                anyhow::anyhow!("invalid params for restriction '{name}' in {scope}: {e}")
            })?;
        }
        for entry in &list.automations {
            let name = entry.name();
            if !self.automation_factories.contains_key(name) {
                anyhow::bail!("unknown automation policy '{name}' in {scope}");
            }
            let factory = &self.automation_factories[name];
            factory(entry.params()).map_err(|e| {
                anyhow::anyhow!("invalid params for automation '{name}' in {scope}: {e}")
            })?;
        }
        Ok(())
    }

    fn validate_repo_override(
        &self,
        repo_override: &RepoOverride,
        scope: &str,
    ) -> Result<(), anyhow::Error> {
        for entry in &repo_override.restrictions {
            let name = entry.name();
            if !self.restriction_factories.contains_key(name) {
                anyhow::bail!("unknown restriction policy '{name}' in {scope}");
            }
            let factory = &self.restriction_factories[name];
            factory(entry.params()).map_err(|e| {
                anyhow::anyhow!("invalid params for restriction '{name}' in {scope}: {e}")
            })?;
        }
        Ok(())
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
        Ok(Box::new(
            super::integrations::github_pr_sync::GithubPrSyncAutomation::new(params)?,
        ))
    });

    registry
}
