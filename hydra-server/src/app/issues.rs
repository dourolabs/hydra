use crate::{
    domain::actors::ActorRef,
    domain::issues::{Issue, IssueDependencyType},
    store::{ReadOnlyStore, Status, StoreError},
};
use chrono::Utc;
use hydra_common::{
    SessionId, VersionNumber, Versioned,
    api::v1 as api,
    api::v1::form::{Effect, FormResponse, Input},
    api::v1::issues::SearchIssuesQuery,
    issues::IssueId,
    principal::Principal,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use thiserror::Error;
use tracing::info;

use super::app_state::AppState;

#[derive(Debug, Error)]
pub enum UpsertIssueError {
    #[error("job_id may only be provided when creating an issue")]
    JobIdProvidedForUpdate,
    #[error("issue creator must be set")]
    MissingCreator,
    /// `issue.assignee` referenced a User or Agent that does not exist in
    /// the store. Returned as HTTP 400 by `routes::issues`. External
    /// principals are not validated (no DB lookup is meaningful), so this
    /// variant never wraps a `Principal::External`.
    #[error("unknown actor '{principal}'")]
    UnknownAssignee { principal: Principal },
    #[error("failed to validate assignee existence")]
    AssigneeLookup {
        #[source]
        source: StoreError,
        principal: Principal,
    },
    #[error("issue dependency '{dependency_id}' not found")]
    MissingDependency {
        #[source]
        source: StoreError,
        dependency_id: IssueId,
    },
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: Option<IssueId>,
    },
    #[error("job '{job_id}' not found")]
    JobNotFound {
        #[source]
        source: StoreError,
        job_id: SessionId,
    },
    #[error("failed to validate job status for '{job_id}'")]
    JobStatusLookup {
        #[source]
        source: StoreError,
        job_id: SessionId,
    },
    #[error("job_id must reference a running job")]
    JobNotRunning {
        job_id: SessionId,
        status: Option<Status>,
    },
    #[error("failed to read tasks for dropped issue '{issue_id}'")]
    TaskLookup {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("failed to kill task '{job_id}' for dropped issue '{issue_id}'")]
    KillTask {
        #[source]
        source: crate::job_engine::JobEngineError,
        issue_id: IssueId,
        job_id: SessionId,
    },
    #[error("{0}")]
    PolicyViolation(#[from] crate::policy::PolicyViolation),
    #[error("invalid form: {message}")]
    InvalidForm { message: String },
}

#[derive(Debug, Error)]
pub enum SubmitFormActionError {
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue has no form or action not found")]
    ActionNotFound { issue_id: IssueId },
    #[error("validation failed")]
    ValidationFailed {
        field_errors: HashMap<String, String>,
    },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("actor '{actor_name}' cannot submit form actions")]
    UnsupportedActor { actor_name: String },
}

impl AppState {
    pub async fn get_issue(
        &self,
        issue_id: &IssueId,
        include_deleted: bool,
    ) -> Result<Versioned<Issue>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue(issue_id, include_deleted).await
    }

    pub async fn get_issue_versions(
        &self,
        issue_id: &IssueId,
    ) -> Result<Vec<Versioned<Issue>>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue_versions(issue_id).await
    }

    pub async fn list_issues(&self) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_issues(&SearchIssuesQuery::default()).await
    }

    pub async fn list_issues_with_query(
        &self,
        query: &SearchIssuesQuery,
    ) -> Result<Vec<(IssueId, Versioned<Issue>)>, StoreError> {
        let store = self.store.as_ref();
        store.list_issues(query).await
    }

    pub async fn count_issues(&self, query: &SearchIssuesQuery) -> Result<u64, StoreError> {
        let store = self.store.as_ref();
        store.count_issues(query).await
    }

    pub async fn upsert_issue(
        &self,
        issue_id: Option<IssueId>,
        request: api::issues::UpsertIssueRequest,
        actor: ActorRef,
    ) -> Result<(IssueId, VersionNumber), UpsertIssueError> {
        let api::issues::UpsertIssueRequest {
            issue,
            session_id: job_id,
            label_ids,
            label_names,
            ..
        } = request;
        let issue: Issue = issue.into();
        // Validate that the typed assignee actually exists. Unknown
        // User / Agent principals are rejected with 400. External
        // principals are not validated (no DB lookup is meaningful).
        if let Some(ref principal) = issue.assignee {
            let exists = self
                .store
                .as_ref()
                .principal_exists(principal)
                .await
                .map_err(|source| UpsertIssueError::AssigneeLookup {
                    source,
                    principal: principal.clone(),
                })?;
            if !exists {
                return Err(UpsertIssueError::UnknownAssignee {
                    principal: principal.clone(),
                });
            }
        }
        if let Some(ref form) = issue.form {
            form.validate_field_keys()
                .map_err(|message| UpsertIssueError::InvalidForm { message })?;
        }
        // Validate that `status` is one of the resolved project's status
        // keys; reject unknown keys with a policy violation. Reads the
        // project's declared statuses from the store.
        match self.resolve_status(&issue).await {
            Ok(_) => {}
            Err(crate::app::projects::ResolveStatusError::ProjectNotFound(project_id)) => {
                return Err(UpsertIssueError::PolicyViolation(
                    crate::policy::PolicyViolation {
                        policy_name: "project_status_validation".to_string(),
                        message: format!("project '{project_id}' not found"),
                    },
                ));
            }
            Err(
                crate::app::projects::ResolveStatusError::UnknownStatus(_)
                | crate::app::projects::ResolveStatusError::InvalidKey(_),
            ) => {
                return Err(UpsertIssueError::PolicyViolation(
                    crate::policy::PolicyViolation {
                        policy_name: "project_status_validation".to_string(),
                        message: format!(
                            "status '{}' is not declared in the resolved project",
                            issue.status
                        ),
                    },
                ));
            }
            Err(crate::app::projects::ResolveStatusError::Store(source)) => {
                return Err(UpsertIssueError::Store {
                    source,
                    issue_id: issue_id.clone(),
                });
            }
        }
        let is_create = issue_id.is_none();
        let dependencies = issue.dependencies.clone();

        let store = self.store.as_ref();
        let label_actor = actor.clone();

        let (issue_id, version) = match issue_id {
            Some(id) => {
                if job_id.is_some() {
                    return Err(UpsertIssueError::JobIdProvidedForUpdate);
                }

                let updated_issue = issue.clone();

                // Run restriction policies (require_creator)
                {
                    self.policy_engine
                        .check_update_issue(&id, &updated_issue, None, store, &actor)
                        .await?;
                }

                match self
                    .store
                    .update_issue_with_actor(&id, updated_issue, actor)
                    .await
                {
                    Ok(version) => (id, version),
                    Err(source @ StoreError::IssueNotFound(_)) => {
                        return Err(UpsertIssueError::IssueNotFound {
                            issue_id: id.clone(),
                            source,
                        });
                    }
                    Err(StoreError::InvalidDependency(dependency_id)) => {
                        return Err(UpsertIssueError::MissingDependency {
                            dependency_id: dependency_id.clone(),
                            source: StoreError::InvalidDependency(dependency_id),
                        });
                    }
                    Err(source) => {
                        return Err(UpsertIssueError::Store {
                            source,
                            issue_id: Some(id),
                        });
                    }
                }
            }
            None => {
                if let Some(ref job_id) = job_id {
                    let status = store
                        .get_session(job_id, false)
                        .await
                        .map_err(|source| match source {
                            StoreError::SessionNotFound(_) => UpsertIssueError::JobNotFound {
                                job_id: job_id.clone(),
                                source,
                            },
                            other => UpsertIssueError::JobStatusLookup {
                                job_id: job_id.clone(),
                                source: other,
                            },
                        })?
                        .item
                        .status;

                    if status != Status::Running {
                        return Err(UpsertIssueError::JobNotRunning {
                            job_id: job_id.clone(),
                            status: Some(status),
                        });
                    }
                }

                // Run restriction policies (require_creator)
                {
                    self.policy_engine
                        .check_create_issue(&issue, store, &actor)
                        .await?;
                }

                let (id, version) = self
                    .store
                    .add_issue_with_actor(issue, actor)
                    .await
                    .map_err(|source| match source {
                        StoreError::InvalidDependency(dependency_id) => {
                            UpsertIssueError::MissingDependency {
                                dependency_id: dependency_id.clone(),
                                source: StoreError::InvalidDependency(dependency_id),
                            }
                        }
                        other => UpsertIssueError::Store {
                            source: other,
                            issue_id: None,
                        },
                    })?;
                (id, version)
            }
        };

        info!(issue_id = %issue_id, "issue stored successfully");

        // Sync label associations if requested
        if label_ids.is_some() || label_names.is_some() {
            let resolved = self
                .resolve_label_ids(label_ids, label_names, label_actor)
                .await
                .map_err(|e| UpsertIssueError::Store {
                    source: match e {
                        super::CreateLabelError::Store { source } => source,
                        other => StoreError::Internal(other.to_string()),
                    },
                    issue_id: Some(issue_id.clone()),
                })?;

            let object_id = hydra_common::HydraId::from(issue_id.clone());

            // Get current labels and compute diff
            let current_labels =
                self.get_labels_for_object(&object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;

            let current_ids: HashSet<hydra_common::LabelId> =
                current_labels.iter().map(|l| l.label_id.clone()).collect();
            let desired_ids: HashSet<hydra_common::LabelId> = resolved.into_iter().collect();

            // Remove labels that are no longer desired
            for old_id in current_ids.difference(&desired_ids) {
                self.remove_label_association(old_id, &object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;
            }

            // Add newly desired labels
            for new_id in desired_ids.difference(&current_ids) {
                self.add_label_association(new_id, &object_id)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;
            }
        }

        // Inherit labels from parent issues when creating a child issue
        if is_create {
            let parent_ids: Vec<IssueId> = dependencies
                .iter()
                .filter(|d| d.dependency_type == IssueDependencyType::ChildOf)
                .map(|d| d.issue_id.clone())
                .collect();

            if !parent_ids.is_empty() {
                let parent_hydra_ids: Vec<hydra_common::HydraId> = parent_ids
                    .iter()
                    .map(|id| hydra_common::HydraId::from(id.clone()))
                    .collect();
                let parent_labels = self
                    .get_labels_for_objects(&parent_hydra_ids)
                    .await
                    .map_err(|source| UpsertIssueError::Store {
                        source,
                        issue_id: Some(issue_id.clone()),
                    })?;

                let child_object_id = hydra_common::HydraId::from(issue_id.clone());
                let mut inherited = HashSet::new();
                for labels in parent_labels.values() {
                    for label in labels {
                        if !label.recurse {
                            continue;
                        }
                        if inherited.insert(label.label_id.clone()) {
                            self.add_label_association(&label.label_id, &child_object_id)
                                .await
                                .map_err(|source| UpsertIssueError::Store {
                                    source,
                                    issue_id: Some(issue_id.clone()),
                                })?;
                        }
                    }
                }
            }
        }

        Ok((issue_id, version))
    }

    pub async fn archive_issue(
        &self,
        issue_id: &IssueId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store.delete_issue_with_actor(issue_id, actor).await?;
        Ok(())
    }

    /// Unified readiness rule:
    ///
    /// ```text
    /// ready ⇔
    ///     every blocked_on dep    has resolve_status(dep).unblocks_dependents = true
    ///   ∧ every direct child      has resolve_status(child).unblocks_parents  = true
    ///   ∧ resolve_status(issue).suppress_sessions = false
    /// ```
    ///
    /// The dependency clauses are about ancestors/descendants; the
    /// issue's own status appears only via `suppress_sessions`, which is
    /// an explicit opt-out. Issues parked in a status flagged
    /// `suppress_sessions = true` are intentionally kept out of the
    /// spawn dispatcher even when otherwise ready — the typical use is
    /// a custom "parked" or "waiting on human" status where the user
    /// wants the issue to retain its assignee (so dispatch is not
    /// suppressed via the `clear_assignee` path) but to stop spawning
    /// new sessions until a status transition reactivates it.
    ///
    /// Status resolution failures (unknown project, malformed key) on
    /// the issue's own status, blockers, or children are treated as
    /// "not ready" and logged — upstream validation should have
    /// prevented these, so the warn flags a real misconfiguration
    /// rather than a routine signal.
    pub async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let store = self.store.as_ref();
        let issue = store.get_issue(issue_id, false).await?.item;

        match self.resolve_status(&issue).await {
            Ok(def) => {
                if def.suppress_sessions {
                    return Ok(false);
                }
            }
            Err(err) => {
                tracing::warn!(
                    issue_id = %issue_id,
                    issue_status = %issue.status,
                    error = %err,
                    "is_issue_ready: failed to resolve own status; treating as not ready"
                );
                return Ok(false);
            }
        }

        for dependency in &issue.dependencies {
            if dependency.dependency_type != IssueDependencyType::BlockedOn {
                continue;
            }
            let blocker = store.get_issue(&dependency.issue_id, false).await?.item;
            let blocker_resolved = match self.resolve_status(&blocker).await {
                Ok(def) => def,
                Err(err) => {
                    tracing::warn!(
                        issue_id = %issue_id,
                        blocker_id = %dependency.issue_id,
                        blocker_status = %blocker.status,
                        error = %err,
                        "is_issue_ready: failed to resolve blocker status; treating as not ready"
                    );
                    return Ok(false);
                }
            };
            if !blocker_resolved.unblocks_dependents {
                return Ok(false);
            }
        }

        for child_id in store.get_issue_children(issue_id).await? {
            let child = store.get_issue(&child_id, false).await?.item;
            let child_resolved = match self.resolve_status(&child).await {
                Ok(def) => def,
                Err(err) => {
                    tracing::warn!(
                        issue_id = %issue_id,
                        child_id = %child_id,
                        child_status = %child.status,
                        error = %err,
                        "is_issue_ready: failed to resolve child status; treating as not ready"
                    );
                    return Ok(false);
                }
            };
            if !child_resolved.unblocks_parents {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub async fn get_issue_children(&self, issue_id: &IssueId) -> Result<Vec<IssueId>, StoreError> {
        let store = self.store.as_ref();
        store.get_issue_children(issue_id).await
    }

    pub async fn submit_form_action(
        &self,
        issue_id: IssueId,
        action_id: String,
        values: HashMap<String, Value>,
        actor: ActorRef,
    ) -> Result<(VersionNumber, FormResponse), SubmitFormActionError> {
        let store = self.store.as_ref();
        let versioned = store.get_issue(&issue_id, false).await.map_err(|source| {
            SubmitFormActionError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;
        let mut issue = versioned.item;

        // Find the form and action
        let form = issue
            .form
            .as_ref()
            .ok_or_else(|| SubmitFormActionError::ActionNotFound {
                issue_id: issue_id.clone(),
            })?;

        let action = form
            .actions
            .iter()
            .find(|a| a.id == action_id)
            .ok_or_else(|| SubmitFormActionError::ActionNotFound {
                issue_id: issue_id.clone(),
            })?;

        // Build a map of field key -> &Field for lookup
        let field_map: HashMap<&str, _> = form.fields.iter().map(|f| (f.key.as_str(), f)).collect();

        // Check for unknown keys
        let mut field_errors: HashMap<String, String> = HashMap::new();
        for key in values.keys() {
            if !field_map.contains_key(key.as_str()) {
                field_errors.insert(key.clone(), "unknown field".to_string());
            }
        }
        if !field_errors.is_empty() {
            return Err(SubmitFormActionError::ValidationFailed { field_errors });
        }

        // Validate required fields and type-check all provided values
        let required_keys: HashSet<&str> = action.requires.iter().map(|s| s.as_str()).collect();

        for key in &action.requires {
            match values.get(key) {
                None | Some(Value::Null) => {
                    field_errors.insert(key.clone(), "required".to_string());
                }
                Some(val) => {
                    if let Some(field) = field_map.get(key.as_str()) {
                        if let Some(err) = validate_field_value(&field.input, val) {
                            field_errors.insert(key.clone(), err);
                        }
                    }
                }
            }
        }

        // Validate non-required fields if present
        for (key, val) in &values {
            if required_keys.contains(key.as_str()) {
                continue; // already validated above
            }
            if val.is_null() {
                continue; // absent/null non-required fields are fine
            }
            if let Some(field) = field_map.get(key.as_str()) {
                if let Some(err) = validate_field_value(&field.input, val) {
                    field_errors.insert(key.clone(), err);
                }
            }
        }

        if !field_errors.is_empty() {
            return Err(SubmitFormActionError::ValidationFailed { field_errors });
        }

        // Extract actor_id from ActorRef for the FormResponse. System
        // workers without an `on_behalf_of` and bare Automation triggers
        // don't have a Principal-eligible identity to attribute; reject
        // those rather than papering over them with a synthetic name.
        let actor_id = match &actor {
            ActorRef::Authenticated { actor_id, .. } => actor_id.clone(),
            ActorRef::System {
                on_behalf_of: Some(id),
                ..
            } => id.clone(),
            ActorRef::System { worker_name, .. } => {
                return Err(SubmitFormActionError::UnsupportedActor {
                    actor_name: worker_name.clone(),
                });
            }
            ActorRef::Automation {
                automation_name, ..
            } => {
                return Err(SubmitFormActionError::UnsupportedActor {
                    actor_name: automation_name.clone(),
                });
            }
            ActorRef::Trigger {
                on_behalf_of: Some(id),
                ..
            } => id.clone(),
            ActorRef::Trigger { trigger_id, .. } => {
                return Err(SubmitFormActionError::UnsupportedActor {
                    actor_name: trigger_id.to_string(),
                });
            }
        };

        // Build the FormResponse
        let form_response = FormResponse {
            action_id: action_id.clone(),
            actor: actor_id,
            values: values.clone(),
            submitted_at: Utc::now(),
        };

        // Apply effects
        let effect = action.effect.clone();
        issue.form_response = Some(form_response.clone());

        // Resolve the optional comment body before any writes so we can
        // sequence `add_comment` (which validates the issue exists and
        // allocates a per-issue sequence) before `update_issue`, giving us
        // a rollback-on-comment-failure guarantee: if the comment insert
        // fails, the status transition is never applied. Absent, null, or
        // whitespace-only values are treated as "no comment".
        let comment_body = match &effect {
            Effect::UpdateIssue {
                add_comment_from: Some(field_key),
                ..
            } => match form_response.values.get(field_key) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => {
                    let trimmed = s.trim();
                    (!trimmed.is_empty()).then(|| s.clone())
                }
                Some(other) => {
                    let coerced = other.to_string();
                    (!coerced.trim().is_empty()).then_some(coerced)
                }
            },
            _ => None,
        };

        match effect {
            Effect::UpdateIssue { status, .. } => {
                issue.status = status;
            }
            Effect::RecordOnly => {}
            _ => {}
        }

        if let Some(body) = comment_body {
            self.store
                .add_comment(&issue_id, body, &actor)
                .await
                .map_err(|source| SubmitFormActionError::Store {
                    source,
                    issue_id: issue_id.clone(),
                })?;
        }

        let version = self
            .store
            .update_issue_with_actor(&issue_id, issue, actor)
            .await
            .map_err(|source| SubmitFormActionError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        info!(issue_id = %issue_id, action_id, "form action submitted");
        Ok((version, form_response))
    }
}

static REGEX_CACHE: LazyLock<Mutex<HashMap<String, regex::Regex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Validates a single field value against its input type definition.
/// Returns `None` if valid, or `Some(error_message)` if invalid.
fn validate_field_value(input: &Input, value: &Value) -> Option<String> {
    match input {
        Input::Text {
            min_length,
            max_length,
            pattern,
            ..
        } => {
            let Some(s) = value.as_str() else {
                return Some("must be a string".to_string());
            };
            if let Some(min) = min_length {
                if s.len() < *min {
                    return Some(format!("must be at least {min} characters"));
                }
            }
            if let Some(max) = max_length {
                if s.len() > *max {
                    return Some(format!("must be at most {max} characters"));
                }
            }
            if let Some(pat) = pattern {
                let re = {
                    let mut cache = REGEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(re) = cache.get(pat) {
                        re.clone()
                    } else {
                        match regex::Regex::new(pat) {
                            Ok(re) => {
                                cache.insert(pat.clone(), re.clone());
                                re
                            }
                            Err(_) => {
                                return Some(format!("invalid pattern '{pat}'"));
                            }
                        }
                    }
                };
                if !re.is_match(s) {
                    return Some(format!("must match pattern '{pat}'"));
                }
            }
            None
        }
        Input::Textarea {
            min_length,
            max_length,
            ..
        } => {
            let Some(s) = value.as_str() else {
                return Some("must be a string".to_string());
            };
            if let Some(min) = min_length {
                if s.len() < *min {
                    return Some(format!("must be at least {min} characters"));
                }
            }
            if let Some(max) = max_length {
                if s.len() > *max {
                    return Some(format!("must be at most {max} characters"));
                }
            }
            None
        }
        Input::Select { options, .. } => {
            let Some(s) = value.as_str() else {
                return Some("must be a string".to_string());
            };
            if !options.iter().any(|opt| opt.value == s) {
                return Some(format!(
                    "must be one of: {}",
                    options
                        .iter()
                        .map(|o| o.value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            None
        }
        Input::Checkbox => {
            if !value.is_boolean() {
                return Some("must be a boolean".to_string());
            }
            None
        }
        Input::Number { min, max, .. } => {
            let Some(n) = value.as_f64() else {
                return Some("must be a number".to_string());
            };
            if let Some(min_val) = min {
                if n < *min_val {
                    return Some(format!("must be at least {min_val}"));
                }
            }
            if let Some(max_val) = max {
                if n > *max_val {
                    return Some(format!("must be at most {max_val}"));
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::UpsertIssueError;
    use crate::{
        app::{
            ServerEvent,
            test_helpers::{
                issue_with_status, start_test_automation_runner, task_for_issue_with_status,
            },
        },
        domain::actors::ActorRef,
        domain::issues::{IssueDependency, IssueDependencyType},
        job_engine::{JobEngine, JobStatus},
        store::{ReadOnlyStore, Status},
        test_utils::{MockJobEngine, test_state, test_state_with_engine},
    };
    use chrono::Utc;
    use hydra_common::api::v1 as api;
    use hydra_common::principal::Principal;
    use hydra_common::test_utils::status::status;
    use std::sync::Arc;

    /// Wait briefly for automations to process events.
    async fn wait_for_automations() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn open_issue_ready_when_not_blocked() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("open", status("open"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_blocked_on_open_issue() {
        let state = test_state();

        let (blocker_id, blocked_issue_id) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", status("open"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let (blocked_issue_id, _) = store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        status("open"),
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id.clone(),
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap();

            (blocker_id, blocked_issue_id)
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue_with_actor(
                    &blocker_id,
                    issue_with_status("blocker", status("closed"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_issue_ready_after_children_closed() {
        let state = test_state();

        let (parent_id, child_id, child_dependencies) = {
            let store = state.store.as_ref();
            let (parent_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("parent", status("in-progress"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let child_dependencies = vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent_id.clone(),
            )];
            let (child_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("child", status("open"), child_dependencies.clone()),
                    ActorRef::test(),
                )
                .await
                .unwrap();

            (parent_id, child_id, child_dependencies)
        };

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());

        {
            let store = state.store.as_ref();
            store
                .update_issue_with_actor(
                    &child_id,
                    issue_with_status("child", status("closed"), child_dependencies),
                    ActorRef::test(),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", status("dropped"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        status("open"),
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id,
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn upsert_issue_rejects_unknown_user_assignee() {
        let state = test_state();
        let mut issue = issue_with_status("with-assignee", status("open"), vec![]);
        issue.assignee = Some(Principal::User {
            name: hydra_common::api::v1::users::Username::try_new("ghost").unwrap(),
        });

        let err = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap_err();

        match err {
            UpsertIssueError::UnknownAssignee { principal } => {
                assert_eq!(principal.to_string(), "users/ghost");
            }
            other => panic!("expected UnknownAssignee, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upsert_issue_accepts_known_user_assignee() {
        let state = test_state();
        // Seed a user so principal_exists returns true.
        let alice = hydra_common::api::v1::users::Username::try_new("alice").unwrap();
        state
            .store
            .add_user(
                crate::domain::users::User::new(
                    crate::domain::users::Username::from("alice"),
                    None,
                    false,
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut issue = issue_with_status("with-assignee", status("open"), vec![]);
        issue.assignee = Some(Principal::User {
            name: alice.clone(),
        });

        let (issue_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = state.store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(fetched.item.assignee, Some(Principal::User { name: alice }));
    }

    #[tokio::test]
    async fn upsert_issue_accepts_existing_agent_assignee() {
        let state = test_state();
        // Seed an agent so principal_exists returns true.
        let swe_name = hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap();
        state
            .store
            .add_agent(crate::domain::agents::Agent::new(
                swe_name.as_str().to_string(),
                "/agents/swe/prompt.md".to_string(),
                None,
                3,
                4,
                false,
                Vec::new(),
            ))
            .await
            .unwrap();

        let mut issue = issue_with_status("with-assignee", status("open"), vec![]);
        issue.assignee = Some(Principal::Agent {
            name: swe_name.clone(),
        });

        let (issue_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = state.store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item.assignee,
            Some(Principal::Agent { name: swe_name })
        );
    }

    #[tokio::test]
    async fn upsert_issue_accepts_external_assignee_without_db_lookup() {
        let state = test_state();
        let github = hydra_common::principal::ExternalSystem::try_new("github").unwrap();
        let mut issue = issue_with_status("with-assignee", status("open"), vec![]);
        issue.assignee = Some(Principal::External {
            system: github.clone(),
            username: "anyone".to_string(),
        });

        let (issue_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let fetched = state.store.get_issue(&issue_id, false).await.unwrap();
        assert_eq!(
            fetched.item.assignee,
            Some(Principal::External {
                system: github,
                username: "anyone".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn dropping_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", status("open"), vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", status("open"), vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let grandchild_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, child_id.clone());
        let grandchild_issue = issue_with_status(
            "grandchild",
            status("open"),
            vec![grandchild_dependency.clone()],
        );
        let (grandchild_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(grandchild_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Add a closed child -- should NOT be overwritten to Dropped
        let closed_child_issue = issue_with_status(
            "closed_child",
            status("closed"),
            vec![child_dependency.clone()],
        );
        let (closed_child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(closed_child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Add a failed child -- should NOT be overwritten to Dropped
        let failed_child_issue = issue_with_status(
            "failed_child",
            status("failed"),
            vec![child_dependency.clone()],
        );
        let (failed_child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(failed_child_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let (parent_task_id, child_task_id, grandchild_task_id) = {
            let store = state.store.as_ref();
            // Use Running status to avoid triggering start_created_sessions automation
            let (parent_task_id, _) = store
                .add_session_with_actor(
                    task_for_issue_with_status(&parent_id, Status::Running),
                    Utc::now(),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let (child_task_id, _) = store
                .add_session_with_actor(
                    task_for_issue_with_status(&child_id, Status::Running),
                    Utc::now(),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let (grandchild_task_id, _) = store
                .add_session_with_actor(
                    task_for_issue_with_status(&grandchild_id, Status::Running),
                    Utc::now(),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            (parent_task_id, child_task_id, grandchild_task_id)
        };

        job_engine
            .insert_job(&parent_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;
        job_engine
            .insert_job(&grandchild_task_id, JobStatus::Running)
            .await;

        let mut dropped_parent = parent_issue.clone();
        dropped_parent.status = status("dropped");
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(dropped_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            // Open children should be dropped
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                status("dropped")
            );
            assert_eq!(
                store
                    .get_issue(&grandchild_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                status("dropped")
            );
            // Terminal-state children should retain their original status
            assert_eq!(
                store
                    .get_issue(&closed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                status("closed")
            );
            assert_eq!(
                store
                    .get_issue(&failed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                status("failed")
            );
        }

        for task_id in [parent_task_id, child_task_id, grandchild_task_id] {
            let job = job_engine
                .find_job_by_hydra_id(&task_id)
                .await
                .expect("job should exist");
            assert_eq!(job.status, JobStatus::Failed);
        }

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn event_bus_emits_issue_created_and_updated() {
        let state = test_state();
        let mut rx = state.subscribe();

        let issue = issue_with_status("test issue", status("open"), Vec::new());
        let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .expect("create should succeed");

        let event = rx.recv().await.expect("should receive IssueCreated");
        assert!(
            matches!(&event, ServerEvent::IssueCreated { issue_id: id, .. } if *id == issue_id)
        );
        let first_seq = event.seq();
        assert!(first_seq > 0);

        let updated_issue = issue_with_status("updated issue", status("in-progress"), Vec::new());
        let update_request = api::issues::UpsertIssueRequest::new(updated_issue.into(), None);
        state
            .upsert_issue(Some(issue_id.clone()), update_request, ActorRef::test())
            .await
            .expect("update should succeed");

        let event = rx.recv().await.expect("should receive IssueUpdated");
        assert!(
            matches!(&event, ServerEvent::IssueUpdated { issue_id: id, .. } if *id == issue_id)
        );
        assert!(event.seq() > first_seq);
    }

    #[tokio::test]
    async fn event_bus_emits_issue_deleted() {
        let state = test_state();

        let issue = issue_with_status("doomed issue", status("open"), Vec::new());
        let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .expect("create should succeed");

        let mut rx = state.subscribe();

        state
            .archive_issue(&issue_id, ActorRef::test())
            .await
            .expect("delete should succeed");

        let event = rx.recv().await.expect("should receive IssueDeleted");
        assert!(
            matches!(&event, ServerEvent::IssueDeleted { issue_id: id, .. } if *id == issue_id)
        );
    }

    #[tokio::test]
    async fn event_bus_seq_is_monotonically_increasing() {
        let state = test_state();
        let mut rx = state.subscribe();

        let mut seqs = Vec::new();
        for i in 0..5 {
            let issue = issue_with_status(&format!("issue {i}"), status("open"), Vec::new());
            let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
            state
                .upsert_issue(None, request, ActorRef::test())
                .await
                .expect("create should succeed");
            let event = rx.recv().await.expect("should receive event");
            seqs.push(event.seq());
        }

        for window in seqs.windows(2) {
            assert!(
                window[0] < window[1],
                "seq numbers should be strictly increasing: {seqs:?}"
            );
        }
    }

    #[tokio::test]
    async fn terminal_status_issue_with_no_children_is_ready() {
        // The readiness rule is now about ancestors/descendants only: the
        // issue's own status does not appear in it. A failed issue with
        // no blockers and no children satisfies the rule trivially.
        // Dispatch is still gated, but at a different layer — the issue's
        // assignee is cleared by `apply_status_on_enter` on transition
        // into a `clear_assignee` status, and the per-agent queue iterates
        // by assignee, so a None-assignee issue is in no queue.
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("failed", status("failed"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_dropped() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", status("dropped"), vec![child_dep]);
        store
            .add_issue_with_actor(child, ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_failed() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", status("failed"), vec![child_dep]);
        store
            .add_issue_with_actor(child, ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_children_mixed_terminal() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("closed child", status("closed"), vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status("dropped child", status("dropped"), vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status("failed child", status("failed"), vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_not_ready_when_child_open_even_if_blocked() {
        // The readiness rule requires every direct child to have
        // `unblocks_parents = true`. An Open child blocked on a Failed
        // sibling has `unblocks_parents = false`, so the parent is not
        // ready even though the child itself is not ready either.
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Child A: failed
        let (failed_child_id, _) = store
            .add_issue_with_actor(
                issue_with_status("failed child", status("failed"), vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Child B: open, blocked on failed child A
        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, failed_child_id);
        store
            .add_issue_with_actor(
                issue_with_status(
                    "blocked child",
                    status("open"),
                    vec![child_dep, blocked_dep],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_not_ready_when_child_is_open_and_unblocked() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Closed child
        store
            .add_issue_with_actor(
                issue_with_status("closed child", status("closed"), vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Open unblocked child — this child is Ready
        store
            .add_issue_with_actor(
                issue_with_status("open child", status("open"), vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Parent should NOT be ready because the open child is Ready
        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_no_children() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", status("in-progress"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        // No children — trivially, no child is Ready
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_with_nested_stuck_children() {
        let state = test_state();

        let store = state.store.as_ref();
        // Grandparent (InProgress) -> Parent (InProgress) -> Child (Failed)
        let grandparent = issue_with_status("grandparent", status("in-progress"), vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", status("in-progress"), vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("failed child", status("failed"), vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Parent is ready (child is Failed, not Ready).
        // But since parent IS ready, grandparent is NOT ready (has a ready child).
        assert!(state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_grandparent_not_ready_with_ready_grandchild() {
        let state = test_state();

        let store = state.store.as_ref();
        // Grandparent (InProgress) -> Parent (InProgress) -> Child (Open, unblocked)
        let grandparent = issue_with_status("grandparent", status("in-progress"), vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", status("in-progress"), vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("open child", status("open"), vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Child is ready (Open, no blockers).
        // Parent is NOT ready (has a ready child).
        // Grandparent is NOT ready (subtree contains a ready issue).
        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn neither_parent_nor_grandparent_ready_when_only_child_is_blocked_open() {
        // The "every direct child has `unblocks_parents = true`" gate
        // at depth 3: a blocked-open child fails the gate for its
        // parent, and the parent failing it propagates to the
        // grandparent by the same rule applied one level up.
        let state = test_state();

        let store = state.store.as_ref();
        let grandparent = issue_with_status("grandparent", status("in-progress"), vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", status("in-progress"), vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let blocker = issue_with_status("blocker", status("open"), vec![]);
        let (blocker_id, _) = store
            .add_issue_with_actor(blocker, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id);
        store
            .add_issue_with_actor(
                issue_with_status(
                    "blocked child",
                    status("open"),
                    vec![child_dep, blocked_dep],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
        assert!(!state.is_issue_ready(&grandparent_id).await.unwrap());
    }

    #[tokio::test]
    async fn open_issue_not_ready_when_child_non_terminal() {
        // The readiness rule treats every direct child the same way
        // regardless of the parent's own status: an Open parent with a
        // non-terminal child is not ready because the child has
        // `unblocks_parents = false`.
        let state = test_state();
        let store = state.store.as_ref();

        let parent = issue_with_status("parent", status("open"), vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("open child", status("open"), vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(!state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn failed_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", status("failed"), vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        status("open"),
                        vec![IssueDependency::new(
                            IssueDependencyType::BlockedOn,
                            blocker_id,
                        )],
                    ),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&blocked_issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", status("open"), vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", status("open"), vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let (child_task_id,) = {
            let store = state.store.as_ref();
            // Use Running status to avoid triggering start_created_sessions automation
            let (child_task_id, _) = store
                .add_session_with_actor(
                    task_for_issue_with_status(&child_id, Status::Running),
                    Utc::now(),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            (child_task_id,)
        };

        job_engine
            .insert_job(&child_task_id, JobStatus::Running)
            .await;

        let mut dropped_parent = parent_issue;
        dropped_parent.status = status("dropped");
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(dropped_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                status("dropped")
            );
        }

        let job = job_engine
            .find_job_by_hydra_id(&child_task_id)
            .await
            .expect("job should exist");
        assert_eq!(job.status, JobStatus::Failed);

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn failed_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", status("open"), vec![]);
        let (parent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(parent_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let child_dependency =
            IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child_issue =
            issue_with_status("child", status("open"), vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut failed_parent = parent_issue;
        failed_parent.status = status("failed");
        state
            .upsert_issue(
                Some(parent_id.clone()),
                api::issues::UpsertIssueRequest::new(failed_parent.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        {
            let store = state.store.as_ref();
            // A failed parent cascades children to `failed`.
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                status("failed")
            );
        }

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn dropped_blocker_does_not_auto_drop_dependents() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let blocker_issue = issue_with_status("blocker", status("open"), vec![]);
        let (blocker_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(blocker_issue.clone().into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let blocked_dep = IssueDependency::new(IssueDependencyType::BlockedOn, blocker_id.clone());
        let dependent_issue =
            issue_with_status("dependent", status("open"), vec![blocked_dep.clone()]);
        let (dependent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(dependent_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut dropped_blocker = blocker_issue;
        dropped_blocker.status = status("dropped");
        state
            .upsert_issue(
                Some(blocker_id.clone()),
                api::issues::UpsertIssueRequest::new(dropped_blocker.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        wait_for_automations().await;

        // Dependent should remain Open (not dropped) — blocking is retained
        // but status is not changed
        {
            let store = state.store.as_ref();
            assert_eq!(
                store
                    .get_issue(&dependent_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                status("open")
            );
        }

        // Dependent should not be ready (blocker is not Closed)
        assert!(!state.is_issue_ready(&dependent_id).await.unwrap());

        runner.shutdown().await;
    }

    /// Seed a custom project owning a single `parked` status with the
    /// caller-controlled `suppress_sessions` flag, returning the new
    /// project id.
    async fn seed_parked_project(
        state: &crate::app::AppState,
        suppress_sessions: bool,
    ) -> hydra_common::ProjectId {
        use hydra_common::api::v1::projects::{Project, ProjectKey, StatusDefinition, StatusKey};
        use hydra_common::api::v1::users::Username as ApiUsername;

        let mut parked = StatusDefinition::new(
            StatusKey::try_new("parked").unwrap(),
            "Parked".to_string(),
            "#95a5a6".parse().unwrap(),
            false,
            false,
            false,
            None,
        );
        parked.suppress_sessions = suppress_sessions;

        let project = Project::new(
            ProjectKey::try_new(if suppress_sessions {
                "parked-project"
            } else {
                "active-project"
            })
            .unwrap(),
            "Parked project".to_string(),
            vec![],
            ApiUsername::from("alice"),
            false,
            0.0,
        );

        let store = state.store.as_ref();
        let (project_id, _) = store.add_project(project, &ActorRef::test()).await.unwrap();
        store
            .add_status(&project_id, parked, &ActorRef::test())
            .await
            .unwrap();
        project_id
    }

    #[tokio::test]
    async fn issue_not_ready_when_status_suppresses_sessions() {
        // The new readiness clause: a status flagged
        // `suppress_sessions = true` parks the issue out of the spawn
        // dispatcher regardless of blockers/children.
        let state = test_state();
        let project_id = seed_parked_project(&state, true).await;

        let mut issue = issue_with_status("parked", status("parked"), vec![]);
        issue.project_id = project_id;

        let (issue_id, _) = state
            .store
            .as_ref()
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn issue_ready_when_custom_status_does_not_suppress_sessions() {
        // Negative control: the same custom-project setup, but with
        // `suppress_sessions = false`, must leave readiness alone. This
        // proves the field flip is what drives the change above, not
        // any side effect of using a non-default project.
        let state = test_state();
        let project_id = seed_parked_project(&state, false).await;

        let mut issue = issue_with_status("parked", status("parked"), vec![]);
        issue.project_id = project_id;

        let (issue_id, _) = state
            .store
            .as_ref()
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn issue_ready_when_owning_project_is_soft_deleted() {
        // `project_cached` resolves through `get_project(..., true)`, so an
        // archived owning project still surfaces its status list — even
        // when the orphan state is reached through a non-cascading code
        // path (here: a raw `update_project` that flips `archived=true`
        // without using `archive_project`, which would have also
        // cascade-archived the issue). This protects the orphan-500
        // read-path fix (`build_issue_response` no longer 500s) against
        // any future write path that leaves orphan-with-live-issue
        // state on disk.
        use hydra_common::api::v1::projects::Project;
        let state = test_state();
        let project_id = seed_parked_project(&state, false).await;

        let mut issue = issue_with_status("parked", status("parked"), vec![]);
        issue.project_id = project_id.clone();

        let store = state.store.as_ref();
        let (issue_id, _) = store
            .add_issue_with_actor(issue, ActorRef::test())
            .await
            .unwrap();

        // Flip `project.archived = true` without going through
        // `archive_project`, so the issue's `archived` flag stays false
        // and the read-path tolerance fix is the only thing keeping
        // status resolution alive.
        let current = store.get_project(&project_id, true).await.unwrap();
        let mut archived: Project = current.item;
        archived.archived = true;
        store
            .update_project(&project_id, archived, &ActorRef::test())
            .await
            .unwrap();

        assert!(state.is_issue_ready(&issue_id).await.unwrap());
    }
}
