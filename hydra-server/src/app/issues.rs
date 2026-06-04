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

#[derive(Debug, Error)]
pub enum SubmitFeedbackError {
    #[error("issue '{issue_id}' not found")]
    IssueNotFound {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
    #[error("issue '{issue_id}' is deleted")]
    IssueDeleted { issue_id: IssueId },
    #[error("issue store operation failed")]
    Store {
        #[source]
        source: StoreError,
        issue_id: IssueId,
    },
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
        // keys; reject unknown keys with a policy violation. When
        // `project_id` is None this resolves against the synthesized
        // default project (open/in-progress/closed/dropped/failed);
        // otherwise it reads the project's declared statuses from the
        // store.
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

                // Run restriction policies (require_creator, issue_lifecycle_validation)
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

                // Run restriction policies (require_creator, issue_lifecycle_validation)
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

    pub async fn delete_issue(
        &self,
        issue_id: &IssueId,
        actor: ActorRef,
    ) -> Result<(), StoreError> {
        self.store.delete_issue_with_actor(issue_id, actor).await?;
        Ok(())
    }

    pub async fn is_issue_ready(&self, issue_id: &IssueId) -> Result<bool, StoreError> {
        let mut visited = HashSet::new();
        self.issue_ready(issue_id, &mut visited).await
    }

    /// Unified readiness rule per `/designs/per-project-issue-statuses.md` §4
    /// "Dependencies, readiness, cascade":
    ///
    /// ```text
    /// ready ⇔
    ///     !resolve_status(issue).unblocks_parents
    ///   ∧ every blocked_on dep    has resolve_status(dep).unblocks_dependents = true
    ///   ∧ every direct child      has resolve_status(child).unblocks_parents  = true
    /// ```
    ///
    /// Status resolution failures (unknown project, malformed key) are
    /// treated as "not ready" and logged — upstream validation should have
    /// prevented these, so the warn flags a real misconfiguration rather
    /// than a routine signal.
    fn issue_ready<'a>(
        &'a self,
        issue_id: &'a IssueId,
        visited: &'a mut HashSet<IssueId>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            if !visited.insert(issue_id.clone()) {
                // Cycle detected: treat as not ready to break the loop.
                return Ok(false);
            }

            let store = self.store.as_ref();
            let issue = store.get_issue(issue_id, false).await?.item;

            let resolved = match self.resolve_status(&issue).await {
                Ok(def) => def,
                Err(err) => {
                    tracing::warn!(
                        issue_id = %issue_id,
                        status = %issue.status,
                        error = %err,
                        "is_issue_ready: failed to resolve status; treating as not ready"
                    );
                    return Ok(false);
                }
            };
            if resolved.unblocks_parents {
                return Ok(false);
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
        })
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

        match effect {
            Effect::UpdateIssue {
                status,
                set_feedback_from,
            } => {
                issue.status = status;
                if let Some(field_key) = set_feedback_from {
                    let coerced = form_response
                        .values
                        .get(&field_key)
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            Value::Null => String::new(),
                            other => other.to_string(),
                        })
                        .unwrap_or_default();
                    issue.feedback = Some(coerced);
                }
            }
            Effect::RecordOnly => {}
            _ => {}
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

    pub async fn submit_feedback(
        &self,
        issue_id: IssueId,
        feedback: String,
        actor: ActorRef,
    ) -> Result<VersionNumber, SubmitFeedbackError> {
        let store = self.store.as_ref();

        // 1. Validate issue exists and is not deleted
        let versioned = store.get_issue(&issue_id, true).await.map_err(|source| {
            SubmitFeedbackError::IssueNotFound {
                source,
                issue_id: issue_id.clone(),
            }
        })?;

        if versioned.item.deleted {
            return Err(SubmitFeedbackError::IssueDeleted {
                issue_id: issue_id.clone(),
            });
        }

        let mut issue = versioned.item;

        // Set feedback. Status side-effects intentionally absent: callers
        // that want to re-route work issue an explicit status transition
        // (typically through a form action), which gives the project's
        // `on_enter` automation a chance to reassign and attach a form
        // deterministically. See `/designs/per-project-issue-statuses.md`
        // §4 "Submit feedback".
        issue.feedback = Some(feedback);

        // Update the issue
        let version = self
            .store
            .update_issue_with_actor(&issue_id, issue, actor)
            .await
            .map_err(|source| SubmitFeedbackError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        // 5. Kill all active sessions for this issue
        let session_ids = store
            .get_sessions_for_issue(&issue_id)
            .await
            .map_err(|source| SubmitFeedbackError::Store {
                source,
                issue_id: issue_id.clone(),
            })?;

        for session_id in session_ids {
            let session = store
                .get_session(&session_id, false)
                .await
                .map_err(|source| SubmitFeedbackError::Store {
                    source,
                    issue_id: issue_id.clone(),
                })?;

            if matches!(
                session.item.status,
                Status::Created | Status::Pending | Status::Running
            ) {
                if let Err(source) = self.job_engine.kill_job(&session_id).await {
                    // Log but don't fail the whole operation if a kill fails
                    tracing::warn!(
                        issue_id = %issue_id,
                        session_id = %session_id,
                        error = %source,
                        "failed to kill session while submitting feedback"
                    );
                }
            }
        }

        info!(issue_id = %issue_id, "feedback submitted");
        Ok(version)
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
        domain::issues::{IssueDependency, IssueDependencyType, IssueStatus},
        job_engine::{JobEngine, JobStatus},
        store::{ReadOnlyStore, Status},
        test_utils::{MockJobEngine, test_state, test_state_with_engine},
    };
    use chrono::Utc;
    use hydra_common::api::v1 as api;
    use hydra_common::principal::Principal;
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
                    issue_with_status("open", IssueStatus::Open, vec![]),
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
                    issue_with_status("blocker", IssueStatus::Open, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            let (blocked_issue_id, _) = store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
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
                    issue_with_status("blocker", IssueStatus::Closed, vec![]),
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
                    issue_with_status("parent", IssueStatus::InProgress, vec![]),
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
                    issue_with_status("child", IssueStatus::Open, child_dependencies.clone()),
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
                    issue_with_status("child", IssueStatus::Closed, child_dependencies),
                    ActorRef::test(),
                )
                .await
                .unwrap();
        }

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("dropped", IssueStatus::Dropped, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropped_blocker_keeps_issue_blocked() {
        let state = test_state();

        let (blocked_issue_id, _) = {
            let store = state.store.as_ref();
            let (blocker_id, _) = store
                .add_issue_with_actor(
                    issue_with_status("blocker", IssueStatus::Dropped, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
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
        let mut issue = issue_with_status("with-assignee", IssueStatus::Open, vec![]);
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

        let mut issue = issue_with_status("with-assignee", IssueStatus::Open, vec![]);
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
                false,
                Vec::new(),
            ))
            .await
            .unwrap();

        let mut issue = issue_with_status("with-assignee", IssueStatus::Open, vec![]);
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
        let mut issue = issue_with_status("with-assignee", IssueStatus::Open, vec![]);
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
    async fn closed_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("closed", IssueStatus::Closed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn dropping_issue_cascades_to_children() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
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
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
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
            IssueStatus::Open,
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
            IssueStatus::Closed,
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
            IssueStatus::Failed,
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
        dropped_parent.status = IssueStatus::Dropped.into();
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
                IssueStatus::Dropped.as_status_key()
            );
            assert_eq!(
                store
                    .get_issue(&grandchild_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Dropped.as_status_key()
            );
            // Terminal-state children should retain their original status
            assert_eq!(
                store
                    .get_issue(&closed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Closed.as_status_key()
            );
            assert_eq!(
                store
                    .get_issue(&failed_child_id, false)
                    .await
                    .unwrap()
                    .item
                    .status,
                IssueStatus::Failed.as_status_key()
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

        let issue = issue_with_status("test issue", IssueStatus::Open, Vec::new());
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

        let updated_issue = issue_with_status("updated issue", IssueStatus::InProgress, Vec::new());
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

        let issue = issue_with_status("doomed issue", IssueStatus::Open, Vec::new());
        let request = api::issues::UpsertIssueRequest::new(issue.into(), None);
        let (issue_id, _) = state
            .upsert_issue(None, request, ActorRef::test())
            .await
            .expect("create should succeed");

        let mut rx = state.subscribe();

        state
            .delete_issue(&issue_id, ActorRef::test())
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
            let issue = issue_with_status(&format!("issue {i}"), IssueStatus::Open, Vec::new());
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
    async fn failed_issue_is_not_ready() {
        let state = test_state();

        let (issue_id, _) = {
            let store = state.store.as_ref();
            store
                .add_issue_with_actor(
                    issue_with_status("failed", IssueStatus::Failed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap()
        };

        assert!(!state.is_issue_ready(&issue_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_ready_when_child_dropped() {
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", IssueStatus::Dropped, vec![child_dep]);
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
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        let child = issue_with_status("child", IssueStatus::Failed, vec![child_dep]);
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
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("closed child", IssueStatus::Closed, vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status(
                    "dropped child",
                    IssueStatus::Dropped,
                    vec![child_dep.clone()],
                ),
                ActorRef::test(),
            )
            .await
            .unwrap();
        store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        assert!(state.is_issue_ready(&parent_id).await.unwrap());
    }

    #[tokio::test]
    async fn in_progress_parent_not_ready_when_child_open_even_if_blocked() {
        // Documented behavior shift #2 (subtree-deeply-stuck): under the
        // unified rule, "every direct child has unblocks_parents = true"
        // — a child being blocked-on-failed does NOT satisfy the parent
        // gate, even though the child itself is not ready.
        //
        // The old "re-plan on stuck subtree" path is intentionally lost
        // to keep one readiness rule for every status.
        let state = test_state();

        let store = state.store.as_ref();
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Child A: failed
        let (failed_child_id, _) = store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep.clone()]),
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
                    IssueStatus::Open,
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
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());

        // Closed child
        store
            .add_issue_with_actor(
                issue_with_status("closed child", IssueStatus::Closed, vec![child_dep.clone()]),
                ActorRef::test(),
            )
            .await
            .unwrap();

        // Open unblocked child — this child is Ready
        store
            .add_issue_with_actor(
                issue_with_status("open child", IssueStatus::Open, vec![child_dep]),
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
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![]);
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
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("failed child", IssueStatus::Failed, vec![child_dep]),
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
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("open child", IssueStatus::Open, vec![child_dep]),
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
        // Documented behavior shift #2 (subtree-deeply-stuck) at depth 3.
        // Old behavior: parent was ready because no ready issue existed
        // anywhere in its subtree. New behavior: parent NOT ready because
        // the only direct child has `unblocks_parents = false`. Grandparent
        // NOT ready by the same rule applied one level up.
        let state = test_state();

        let store = state.store.as_ref();
        let grandparent = issue_with_status("grandparent", IssueStatus::InProgress, vec![]);
        let (grandparent_id, _) = store
            .add_issue_with_actor(grandparent, ActorRef::test())
            .await
            .unwrap();

        let parent_dep = IssueDependency::new(IssueDependencyType::ChildOf, grandparent_id.clone());
        let parent = issue_with_status("parent", IssueStatus::InProgress, vec![parent_dep]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let blocker = issue_with_status("blocker", IssueStatus::Open, vec![]);
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
                    IssueStatus::Open,
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
        // Documented behavior shift #1: an Open issue with non-terminal
        // children is no longer Ready, even when its blockers are
        // satisfied. Open issues with children are rare but the unified
        // rule treats every direct child the same way regardless of the
        // parent's own status.
        let state = test_state();
        let store = state.store.as_ref();

        let parent = issue_with_status("parent", IssueStatus::Open, vec![]);
        let (parent_id, _) = store
            .add_issue_with_actor(parent, ActorRef::test())
            .await
            .unwrap();

        let child_dep = IssueDependency::new(IssueDependencyType::ChildOf, parent_id.clone());
        store
            .add_issue_with_actor(
                issue_with_status("open child", IssueStatus::Open, vec![child_dep]),
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
                    issue_with_status("blocker", IssueStatus::Failed, vec![]),
                    ActorRef::test(),
                )
                .await
                .unwrap();
            store
                .add_issue_with_actor(
                    issue_with_status(
                        "blocked",
                        IssueStatus::Open,
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

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
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
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
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
        dropped_parent.status = IssueStatus::Dropped.into();
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
                IssueStatus::Dropped.as_status_key()
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

        let parent_issue = issue_with_status("parent", IssueStatus::Open, vec![]);
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
            issue_with_status("child", IssueStatus::Open, vec![child_dependency.clone()]);
        let (child_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(child_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut failed_parent = parent_issue;
        failed_parent.status = IssueStatus::Failed.into();
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
            // PR 4 documented behavior shift: failed cascades to failed
            // (was dropped). See /designs/per-project-issue-statuses.md §4.
            assert_eq!(
                store.get_issue(&child_id, false).await.unwrap().item.status,
                IssueStatus::Failed.as_status_key()
            );
        }

        runner.shutdown().await;
    }

    #[tokio::test]
    async fn dropped_blocker_does_not_auto_drop_dependents() {
        let job_engine = Arc::new(MockJobEngine::new());
        let state = test_state_with_engine(job_engine.clone());
        let runner = start_test_automation_runner(&state);

        let blocker_issue = issue_with_status("blocker", IssueStatus::Open, vec![]);
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
            issue_with_status("dependent", IssueStatus::Open, vec![blocked_dep.clone()]);
        let (dependent_id, _) = state
            .upsert_issue(
                None,
                api::issues::UpsertIssueRequest::new(dependent_issue.into(), None),
                ActorRef::test(),
            )
            .await
            .unwrap();

        let mut dropped_blocker = blocker_issue;
        dropped_blocker.status = IssueStatus::Dropped.into();
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
                IssueStatus::Open.as_status_key()
            );
        }

        // Dependent should not be ready (blocker is not Closed)
        assert!(!state.is_issue_ready(&dependent_id).await.unwrap());

        runner.shutdown().await;
    }
}
