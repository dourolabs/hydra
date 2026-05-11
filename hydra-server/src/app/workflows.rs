//! `AppState` methods for the workflow engine lifecycle.
//!
//! Covers instantiation, retrieval, listing, explicit transition, and
//! cancellation. State-entry effects (creating the child issue for the new
//! state, recording history, and terminal-state handling) live in the
//! private `enter_state` helper that both `create_workflow` and
//! `transition_workflow` route through.
//!
//! Out of scope (handled by sibling tasks): HTTP routes, FSA automation
//! reacting to child-issue events, and the `hydra workflows` CLI.

use std::collections::HashMap;
use std::str::FromStr;

use chrono::Utc;
use hydra_common::{
    IssueId, RepoName, Versioned, WorkflowId, api::v1::issues::IssueStatus as ApiIssueStatus,
};
use thiserror::Error;
use tracing::info;

use crate::{
    domain::{
        actors::ActorRef,
        issues::{
            Issue, IssueDependency, IssueDependencyType, IssueStatus, IssueType, SessionSettings,
        },
        users::Username,
        workflows::{
            SessionSettingsTemplate, StateEntryAction, TemplateError, TemplateScope,
            TransitionTrigger, Workflow, WorkflowHistoryEntry, WorkflowState, WorkflowStatus,
            WorkflowTemplate, parse_template, render_template, resolve_context,
        },
    },
    store::{ReadOnlyStore, StoreError, WorkflowFilter},
};

use super::{UpsertIssueError, app_state::AppState};

#[derive(Debug, Error)]
pub enum CreateWorkflowError {
    #[error("no workflow template document at path '{path}'")]
    TemplateNotFound { path: String },
    #[error("invalid workflow template at '{path}'")]
    TemplateInvalid {
        path: String,
        #[source]
        source: TemplateError,
    },
    #[error("failed to create tracking issue for workflow")]
    CreateTrackingIssue {
        #[source]
        source: Box<UpsertIssueError>,
    },
    #[error("failed to enter initial state '{state}' of workflow")]
    EnterState {
        state: String,
        #[source]
        source: EnterStateError,
    },
    #[error("workflow store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum EnterStateError {
    #[error("template interpolation failed")]
    Template(#[from] TemplateError),
    #[error("interpolated repo_name '{value}' is not a valid 'org/repo' name")]
    InvalidRepoName { value: String },
    #[error("failed to create child issue")]
    CreateChildIssue {
        #[source]
        source: Box<UpsertIssueError>,
    },
    #[error("failed to update tracking issue")]
    UpdateTrackingIssue {
        #[source]
        source: Box<UpsertIssueError>,
    },
    #[error("workflow store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum TransitionWorkflowError {
    #[error("workflow '{0}' not found")]
    WorkflowNotFound(WorkflowId),
    #[error("workflow has reached terminal status '{status}'; cannot transition")]
    WorkflowTerminal { status: WorkflowStatus },
    #[error("no explicit transition '{transition_id}' from state '{state}'")]
    TransitionNotFound {
        state: String,
        transition_id: String,
    },
    #[error("failed to enter target state '{state}'")]
    EnterState {
        state: String,
        #[source]
        source: EnterStateError,
    },
    #[error("workflow store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

#[derive(Debug, Error)]
pub enum CancelWorkflowError {
    #[error("workflow '{0}' not found")]
    WorkflowNotFound(WorkflowId),
    #[error("workflow is already terminal (status '{status}'); cannot cancel")]
    AlreadyTerminal { status: WorkflowStatus },
    #[error("failed to drop tracking issue")]
    DropTrackingIssue {
        #[source]
        source: StoreError,
    },
    #[error("workflow store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    /// Instantiate a workflow from a template document.
    ///
    /// Loads the YAML template from the document store at `template_path`,
    /// validates the supplied `context` against the template's parameter
    /// schema, creates a tracking issue (optionally as a child of
    /// `parent_issue_id`), persists the Workflow record, and finally enters
    /// the template's initial state.
    pub async fn create_workflow(
        &self,
        template_path: String,
        context: HashMap<String, String>,
        parent_issue_id: Option<IssueId>,
        creator: Username,
        actor: ActorRef,
    ) -> Result<Versioned<Workflow>, CreateWorkflowError> {
        // 1. Load the template document.
        let store = self.store.as_ref();
        let doc_id = store
            .find_non_deleted_document_by_exact_path(&template_path)
            .await
            .map_err(|source| CreateWorkflowError::Store { source })?
            .ok_or_else(|| CreateWorkflowError::TemplateNotFound {
                path: template_path.clone(),
            })?;
        let document = store
            .get_document(&doc_id, false)
            .await
            .map_err(|source| CreateWorkflowError::Store { source })?
            .item;

        // 2. Parse and validate the template.
        let template = parse_template(&document.body_markdown).map_err(|source| {
            CreateWorkflowError::TemplateInvalid {
                path: template_path.clone(),
                source,
            }
        })?;

        // 3. Validate / resolve the caller's context against the schema.
        let resolved_context = resolve_context(&template, &context).map_err(|source| {
            CreateWorkflowError::TemplateInvalid {
                path: template_path.clone(),
                source,
            }
        })?;

        // 4. Create the tracking issue. The tracking issue starts in-progress
        //    and gets a child-of dependency on `parent_issue_id` when one is
        //    supplied (so the workflow appears under its caller in the tree).
        let dependencies = match &parent_issue_id {
            Some(parent) => vec![IssueDependency::new(
                IssueDependencyType::ChildOf,
                parent.clone(),
            )],
            None => Vec::new(),
        };
        let tracking_issue = Issue::new(
            IssueType::Task,
            template.name.clone(),
            template.description.clone(),
            creator,
            String::new(),
            IssueStatus::InProgress,
            None,
            None,
            Vec::new(),
            dependencies,
            Vec::new(),
            None,
            None,
            None,
        );
        let (tracking_issue_id, _tracking_version) = self
            .store
            .add_issue_with_actor(tracking_issue, actor.clone())
            .await
            .map_err(|source| CreateWorkflowError::CreateTrackingIssue {
                source: Box::new(UpsertIssueError::Store {
                    source,
                    issue_id: None,
                }),
            })?;

        // 5. Build the initial Workflow record and persist it (so the
        //    `workflow_issues` reverse index has a workflow row to reference
        //    when the state-entry helper inserts).
        let workflow_id = WorkflowId::new();
        let initial_state = template.initial_state.clone();
        let mut workflow = Workflow::new(
            workflow_id.clone(),
            template_path.clone(),
            template.clone(),
            tracking_issue_id.clone(),
            initial_state.clone(),
            resolved_context,
            None,
            Vec::new(),
            WorkflowStatus::Active,
        );
        self.store
            .upsert_workflow_with_actor(workflow.clone(), actor.clone())
            .await
            .map_err(|source| CreateWorkflowError::Store { source })?;

        // 6. Enter the initial state. The history entry for the initial
        //    state has no `from_state` (it's the workflow's first state).
        let entry_state =
            find_state(&template, &initial_state).expect("validated template has initial state");
        self.enter_state(&mut workflow, entry_state, None, None, None, actor.clone())
            .await
            .map_err(|source| CreateWorkflowError::EnterState {
                state: initial_state,
                source,
            })?;

        // 7. Persist the workflow's post-entry state and return the latest
        //    versioned record.
        self.store
            .upsert_workflow_with_actor(workflow.clone(), actor)
            .await
            .map_err(|source| CreateWorkflowError::Store { source })?;

        info!(
            workflow_id = %workflow_id,
            tracking_issue_id = %tracking_issue_id,
            template_path = %template_path,
            "workflow created"
        );

        self.store
            .get_workflow(&workflow_id)
            .await
            .map_err(|source| CreateWorkflowError::Store { source })
    }

    /// Look up a workflow by id.
    pub async fn get_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<Versioned<Workflow>, StoreError> {
        self.store.as_ref().get_workflow(workflow_id).await
    }

    /// List workflows matching the supplied filter.
    pub async fn list_workflows(
        &self,
        filter: &WorkflowFilter,
    ) -> Result<Vec<Versioned<Workflow>>, StoreError> {
        self.store.as_ref().list_workflows(filter).await
    }

    /// Invoke an explicit transition on a workflow.
    ///
    /// Errors if the workflow has reached a terminal status, if no matching
    /// transition exists from the current state, or if the matching
    /// transition's trigger is not [`TransitionTrigger::Explicit`]. Automatic
    /// (`on_child_status` / `auto`) transitions cannot be invoked here —
    /// those fire from the engine automation.
    pub async fn transition_workflow(
        &self,
        workflow_id: &WorkflowId,
        transition_id: &str,
        actor: ActorRef,
    ) -> Result<Versioned<Workflow>, TransitionWorkflowError> {
        let mut workflow = self
            .store
            .as_ref()
            .get_workflow(workflow_id)
            .await
            .map_err(|source| match source {
                StoreError::WorkflowNotFound(id) => TransitionWorkflowError::WorkflowNotFound(id),
                other => TransitionWorkflowError::Store { source: other },
            })?
            .item;

        if workflow.status.is_terminal() {
            return Err(TransitionWorkflowError::WorkflowTerminal {
                status: workflow.status,
            });
        }

        // Find the matching explicit transition: same `from`, and the
        // transition's id (carried on TransitionTrigger::Explicit) matches.
        // Per the design we accept only explicit triggers here — auto and
        // on_child_status transitions are driven by the automation engine.
        let template = workflow.template_snapshot.clone();
        let transition = template
            .transitions
            .iter()
            .find(|t| {
                t.from == workflow.current_state
                    && matches!(
                        &t.trigger,
                        TransitionTrigger::Explicit { transition_id: Some(id) } if id == transition_id
                    )
            })
            .cloned()
            .ok_or_else(|| TransitionWorkflowError::TransitionNotFound {
                state: workflow.current_state.clone(),
                transition_id: transition_id.to_string(),
            })?;

        let target_state = find_state(&template, &transition.to)
            .expect("validated template only references known states");
        let from_state = workflow.current_state.clone();
        let previous_progress = self
            .load_previous_step_progress(workflow.active_issue_id.as_ref())
            .await
            .map_err(|source| TransitionWorkflowError::Store { source })?;
        workflow.current_state = transition.to.clone();

        self.enter_state(
            &mut workflow,
            target_state,
            Some(from_state),
            transition.label.clone(),
            previous_progress,
            actor.clone(),
        )
        .await
        .map_err(|source| TransitionWorkflowError::EnterState {
            state: transition.to.clone(),
            source,
        })?;

        self.store
            .upsert_workflow_with_actor(workflow.clone(), actor)
            .await
            .map_err(|source| TransitionWorkflowError::Store { source })?;

        info!(
            workflow_id = %workflow_id,
            transition_id = transition_id,
            from_state = %workflow.history.iter().next_back().and_then(|h| h.from_state.as_deref()).unwrap_or(""),
            to_state = %transition.to,
            "workflow transitioned"
        );

        self.store
            .as_ref()
            .get_workflow(workflow_id)
            .await
            .map_err(|source| TransitionWorkflowError::Store { source })
    }

    /// Cancel a running workflow.
    ///
    /// Sets the workflow status to [`WorkflowStatus::Cancelled`] and drops
    /// the tracking issue (soft-delete via `delete_issue`).
    pub async fn cancel_workflow(
        &self,
        workflow_id: &WorkflowId,
        actor: ActorRef,
    ) -> Result<Versioned<Workflow>, CancelWorkflowError> {
        let mut workflow = self
            .store
            .as_ref()
            .get_workflow(workflow_id)
            .await
            .map_err(|source| match source {
                StoreError::WorkflowNotFound(id) => CancelWorkflowError::WorkflowNotFound(id),
                other => CancelWorkflowError::Store { source: other },
            })?
            .item;

        if workflow.status.is_terminal() {
            return Err(CancelWorkflowError::AlreadyTerminal {
                status: workflow.status,
            });
        }

        workflow.status = WorkflowStatus::Cancelled;
        let tracking_issue_id = workflow.tracking_issue_id.clone();

        self.store
            .upsert_workflow_with_actor(workflow, actor.clone())
            .await
            .map_err(|source| CancelWorkflowError::Store { source })?;

        // Drop (soft-delete) the tracking issue so it disappears from
        // default issue listings. If the issue has already been deleted we
        // treat that as a no-op.
        if let Err(source) = self
            .store
            .delete_issue_with_actor(&tracking_issue_id, actor)
            .await
        {
            if !matches!(source, StoreError::IssueNotFound(_)) {
                return Err(CancelWorkflowError::DropTrackingIssue { source });
            }
        }

        info!(
            workflow_id = %workflow_id,
            tracking_issue_id = %tracking_issue_id,
            "workflow cancelled"
        );

        self.store
            .as_ref()
            .get_workflow(workflow_id)
            .await
            .map_err(|source| CancelWorkflowError::Store { source })
    }

    /// Execute the entry effect of a workflow's current state.
    ///
    /// Mutates `workflow` in-place: appends a history entry, updates
    /// `active_issue_id`, and (for terminal states) flips `workflow.status`.
    /// Side effects: creates the child issue (when the state's `on_enter` is
    /// `CreateIssue`), inserts into `workflow_issues`, and — for terminal
    /// states — updates the tracking issue's status.
    ///
    /// The caller is responsible for the final `upsert_workflow` that
    /// persists the new workflow state.
    async fn enter_state(
        &self,
        workflow: &mut Workflow,
        state: &WorkflowState,
        from_state: Option<String>,
        transition_label: Option<String>,
        previous_step_progress: Option<String>,
        actor: ActorRef,
    ) -> Result<(), EnterStateError> {
        let scope = TemplateScope::new(
            workflow.workflow_id.to_string(),
            workflow.template_snapshot.name.clone(),
            workflow.context.clone(),
            previous_step_progress,
        );

        // Execute on_enter. For Noop nothing happens — the engine will pick
        // up auto transitions on the next event tick.
        let mut child_issue_id: Option<IssueId> = None;
        if let StateEntryAction::CreateIssue {
            issue_type,
            title_template,
            description_template,
            assignee,
            form,
            session_settings,
        } = &state.on_enter
        {
            let title = render_template(title_template, &scope)?;
            let description = render_template(description_template, &scope)?;
            let session_settings = resolve_session_settings(session_settings.as_ref(), &scope)?;
            let tracking_issue = self
                .store
                .as_ref()
                .get_issue(&workflow.tracking_issue_id, true)
                .await
                .map_err(|source| EnterStateError::Store { source })?
                .item;
            let creator = tracking_issue.creator.clone();
            let issue = Issue::new(
                IssueType::from(*issue_type),
                title,
                description,
                creator,
                String::new(),
                IssueStatus::Open,
                assignee.clone(),
                Some(session_settings),
                Vec::new(),
                vec![IssueDependency::new(
                    IssueDependencyType::ChildOf,
                    workflow.tracking_issue_id.clone(),
                )],
                Vec::new(),
                form.clone(),
                None,
                None,
            );
            let (new_issue_id, _version) = self
                .store
                .add_issue_with_actor(issue, actor.clone())
                .await
                .map_err(|source| EnterStateError::CreateChildIssue {
                    source: Box::new(UpsertIssueError::Store {
                        source,
                        issue_id: None,
                    }),
                })?;
            self.store
                .insert_workflow_issue(&workflow.workflow_id, &new_issue_id, &state.id)
                .await
                .map_err(|source| EnterStateError::Store { source })?;
            child_issue_id = Some(new_issue_id);
        }

        workflow.active_issue_id = child_issue_id.clone();
        workflow.history.push(WorkflowHistoryEntry::new(
            from_state,
            state.id.clone(),
            transition_label,
            Utc::now(),
            child_issue_id,
        ));

        // Terminal handling: flip the workflow status and the tracking
        // issue's status to the template's `terminal_status`. The validator
        // guarantees `terminal_status` is set when `terminal` is true.
        if state.terminal {
            let terminal_status: ApiIssueStatus = state
                .terminal_status
                .expect("validated template: terminal state has terminal_status");
            workflow.status = match terminal_status {
                ApiIssueStatus::Closed => WorkflowStatus::Completed,
                ApiIssueStatus::Failed | ApiIssueStatus::Dropped => WorkflowStatus::Failed,
                _ => WorkflowStatus::Active,
            };

            let mut tracking_issue = self
                .store
                .as_ref()
                .get_issue(&workflow.tracking_issue_id, true)
                .await
                .map_err(|source| EnterStateError::Store { source })?
                .item;
            tracking_issue.status = IssueStatus::from(terminal_status);
            self.store
                .update_issue_with_actor(&workflow.tracking_issue_id, tracking_issue, actor.clone())
                .await
                .map_err(|source| EnterStateError::UpdateTrackingIssue {
                    source: Box::new(UpsertIssueError::Store {
                        source,
                        issue_id: Some(workflow.tracking_issue_id.clone()),
                    }),
                })?;
        }

        Ok(())
    }

    /// Look up the `progress` field of an issue so the next state can
    /// reference it via `{{previous_step.progress}}`. Returns `None` if no
    /// previous issue is in scope or the issue has no progress text.
    async fn load_previous_step_progress(
        &self,
        previous_issue_id: Option<&IssueId>,
    ) -> Result<Option<String>, StoreError> {
        let Some(issue_id) = previous_issue_id else {
            return Ok(None);
        };
        match self.store.as_ref().get_issue(issue_id, true).await {
            // A previous child issue is in scope: its progress is the value
            // bound to `{{previous_step.progress}}`, even when empty. We
            // only fall through to `None` when there is no previous issue
            // at all (e.g., on the very first state entry).
            Ok(versioned) => Ok(Some(versioned.item.progress)),
            Err(StoreError::IssueNotFound(_)) => Ok(None),
            Err(other) => Err(other),
        }
    }
}

fn find_state<'a>(template: &'a WorkflowTemplate, state_id: &str) -> Option<&'a WorkflowState> {
    template.states.iter().find(|s| s.id == state_id)
}

/// Render a `SessionSettingsTemplate` into a concrete `SessionSettings`.
fn resolve_session_settings(
    template: Option<&SessionSettingsTemplate>,
    scope: &TemplateScope,
) -> Result<SessionSettings, EnterStateError> {
    let Some(template) = template else {
        return Ok(SessionSettings::default());
    };

    let repo_name = match template.repo_name.as_deref() {
        Some(raw) => {
            let rendered = render_template(raw, scope)?;
            Some(
                RepoName::from_str(&rendered)
                    .map_err(|_| EnterStateError::InvalidRepoName { value: rendered })?,
            )
        }
        None => None,
    };

    let branch = match template.branch.as_deref() {
        Some(raw) => Some(render_template(raw, scope)?),
        None => None,
    };

    let image = match template.image.as_deref() {
        Some(raw) => Some(render_template(raw, scope)?),
        None => None,
    };

    let model = match template.model.as_deref() {
        Some(raw) => Some(render_template(raw, scope)?),
        None => None,
    };

    // Secrets are static identifiers, not templated — pass through as-is.
    let secrets = template.secrets.clone();

    Ok(SessionSettings {
        repo_name,
        remote_url: None,
        image,
        model,
        branch,
        max_retries: None,
        cpu_limit: None,
        memory_limit: None,
        secrets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{actors::ActorRef, documents::Document, users::Username},
        test_utils::test_state,
    };

    const TEMPLATE_PATH: &str = "/workflows/test.yaml";

    fn upload_template(state: &AppState, yaml: &str) -> impl std::future::Future<Output = ()> {
        let yaml = yaml.to_string();
        let state = state.clone();
        async move {
            let document = Document {
                title: "Test Template".to_string(),
                body_markdown: yaml,
                path: Some(TEMPLATE_PATH.parse().expect("valid path")),
                created_by: None,
                deleted: false,
            };
            state
                .upsert_document(None, document, ActorRef::test())
                .await
                .expect("upload template");
        }
    }

    fn full_lifecycle_yaml() -> &'static str {
        r#"
name: "Patch Review"
description: "End to end."
initial_state: develop
context:
  - name: repo_name
    description: "Repository to work in"
    required: true
  - name: branch
    description: "Branch for the work"
    required: true
  - name: base_branch
    description: "Base branch"
    default: "main"

states:
  - id: develop
    name: "Development"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Develop: {{workflow.name}}"
      description_template: "Implement on {{context.branch}}."
      assignee: "swe"
      session_settings:
        repo_name: "{{context.repo_name}}"
        branch: "{{context.branch}}"

  - id: review
    name: "Review"
    on_enter:
      type: create_issue
      issue_type: review-request
      title_template: "Review: {{workflow.name}}"
      description_template: "Review the diff. Prior progress: {{previous_step.progress}}"
      assignee: "reviewer"

  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop

transitions:
  - from: develop
    to: review
    label: "Ready for Review"
    trigger:
      type: explicit
      transition_id: ready-for-review
  - from: review
    to: merged
    label: "Merged"
    trigger:
      type: on_child_status
      status: closed
"#
    }

    fn sample_context() -> HashMap<String, String> {
        let mut ctx = HashMap::new();
        ctx.insert("repo_name".to_string(), "dourolabs/hydra".to_string());
        ctx.insert("branch".to_string(), "feature/widget".to_string());
        ctx
    }

    #[tokio::test]
    async fn create_workflow_persists_workflow_and_initial_child_issue() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let versioned = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow");

        let workflow = versioned.item;
        assert_eq!(workflow.template_path, TEMPLATE_PATH);
        assert_eq!(workflow.current_state, "develop");
        assert_eq!(workflow.status, WorkflowStatus::Active);
        assert_eq!(workflow.history.len(), 1);
        let entry = workflow.history.first().unwrap();
        assert_eq!(entry.from_state, None);
        assert_eq!(entry.to_state, "develop");
        let child_id = entry.child_issue_id.clone().expect("child issue created");
        assert_eq!(workflow.active_issue_id.as_ref(), Some(&child_id));

        // Tracking issue created with in-progress status and the workflow's
        // name as the title.
        let tracking = state
            .get_issue(&workflow.tracking_issue_id, false)
            .await
            .expect("tracking issue stored");
        assert_eq!(tracking.item.title, "Patch Review");
        assert_eq!(tracking.item.status, IssueStatus::InProgress);

        // The child issue is a `child-of` the tracking issue, with the
        // interpolated title and session settings.
        let child = state
            .get_issue(&child_id, false)
            .await
            .expect("child issue stored");
        let child = child.item;
        assert_eq!(child.title, "Develop: Patch Review");
        assert_eq!(child.issue_type, IssueType::Task);
        assert_eq!(child.assignee.as_deref(), Some("swe"));
        assert_eq!(child.dependencies.len(), 1);
        assert_eq!(child.dependencies[0].issue_id, workflow.tracking_issue_id);
        assert_eq!(
            child.dependencies[0].dependency_type,
            IssueDependencyType::ChildOf
        );
        let resolved_repo = child
            .session_settings
            .repo_name
            .clone()
            .expect("repo_name interpolated");
        assert_eq!(resolved_repo.to_string(), "dourolabs/hydra");
        assert_eq!(
            child.session_settings.branch.as_deref(),
            Some("feature/widget")
        );

        // The workflow_issues reverse index points the child back at this
        // workflow.
        let by_issue = state
            .store
            .find_workflow_by_issue_id(&child_id)
            .await
            .expect("find by issue");
        assert!(by_issue.is_some());
        assert_eq!(by_issue.unwrap().item.workflow_id, workflow.workflow_id);
    }

    #[tokio::test]
    async fn create_workflow_attaches_parent_dependency() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        // Create a parent issue to hang the tracking issue off of.
        let parent_issue = Issue::new(
            IssueType::Task,
            "Parent".to_string(),
            "parent".to_string(),
            Username::from("jayantk"),
            String::new(),
            IssueStatus::Open,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        );
        let (parent_id, _) = state
            .store
            .add_issue_with_actor(parent_issue, ActorRef::test())
            .await
            .expect("create parent");

        let versioned = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                Some(parent_id.clone()),
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow");

        let tracking = state
            .get_issue(&versioned.item.tracking_issue_id, false)
            .await
            .expect("tracking exists");
        let parents: Vec<&IssueDependency> = tracking
            .item
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == IssueDependencyType::ChildOf)
            .collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].issue_id, parent_id);
    }

    #[tokio::test]
    async fn create_workflow_missing_required_context_returns_error() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        // `repo_name` is required; supply only `branch`.
        let mut ctx = HashMap::new();
        ctx.insert("branch".to_string(), "feature/widget".to_string());

        let err = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                ctx,
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect_err("missing required param");
        match err {
            CreateWorkflowError::TemplateInvalid { source, .. } => {
                assert!(matches!(
                    source,
                    TemplateError::MissingRequiredContextParam(name) if name == "repo_name"
                ));
            }
            other => panic!("expected TemplateInvalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_workflow_applies_context_default() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let versioned = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow");

        // `base_branch` has a default of "main" and was not supplied.
        assert_eq!(
            versioned.item.context.get("base_branch"),
            Some(&"main".to_string())
        );
    }

    #[tokio::test]
    async fn create_workflow_returns_template_not_found_for_missing_path() {
        let state = test_state();

        let err = state
            .create_workflow(
                "/workflows/does-not-exist.yaml".to_string(),
                HashMap::new(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect_err("template missing");
        assert!(matches!(err, CreateWorkflowError::TemplateNotFound { .. }));
    }

    #[tokio::test]
    async fn transition_workflow_executes_explicit_transition() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let workflow = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        // Set some progress on the develop-state child issue so we can
        // verify {{previous_step.progress}} is rendered into the review
        // state's description.
        let develop_child_id = workflow
            .active_issue_id
            .clone()
            .expect("develop has child issue");
        let mut develop_child = state
            .get_issue(&develop_child_id, false)
            .await
            .unwrap()
            .item;
        develop_child.progress = "did the thing".to_string();
        state
            .store
            .update_issue_with_actor(&develop_child_id, develop_child, ActorRef::test())
            .await
            .expect("update progress");

        let after = state
            .transition_workflow(&workflow.workflow_id, "ready-for-review", ActorRef::test())
            .await
            .expect("transition")
            .item;

        assert_eq!(after.current_state, "review");
        assert_eq!(after.history.len(), 2);
        let last = after.history.last().unwrap();
        assert_eq!(last.from_state.as_deref(), Some("develop"));
        assert_eq!(last.to_state, "review");
        assert_eq!(last.transition_label.as_deref(), Some("Ready for Review"));

        let review_child_id = last.child_issue_id.clone().expect("review child");
        assert_eq!(after.active_issue_id.as_ref(), Some(&review_child_id));

        let review_child = state.get_issue(&review_child_id, false).await.unwrap().item;
        assert_eq!(review_child.title, "Review: Patch Review");
        assert_eq!(review_child.issue_type, IssueType::ReviewRequest);
        assert!(
            review_child
                .description
                .contains("Prior progress: did the thing")
        );
    }

    #[tokio::test]
    async fn transition_workflow_rejects_non_explicit_trigger() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let workflow = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        // First move to `review` legitimately.
        state
            .transition_workflow(&workflow.workflow_id, "ready-for-review", ActorRef::test())
            .await
            .expect("transition to review");

        // From `review`, the only outgoing transition is on_child_status, so
        // an explicit invocation must be rejected.
        let err = state
            .transition_workflow(&workflow.workflow_id, "anything", ActorRef::test())
            .await
            .expect_err("non-explicit trigger");
        match err {
            TransitionWorkflowError::TransitionNotFound { state, .. } => {
                assert_eq!(state, "review");
            }
            other => panic!("expected TransitionNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_workflow_marks_cancelled_and_drops_tracking_issue() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let workflow = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow")
            .item;

        let after = state
            .cancel_workflow(&workflow.workflow_id, ActorRef::test())
            .await
            .expect("cancel")
            .item;
        assert_eq!(after.status, WorkflowStatus::Cancelled);

        // Tracking issue should be soft-deleted; the latest non-deleted view
        // returns NotFound, but include_deleted=true still finds it.
        let err = state
            .get_issue(&workflow.tracking_issue_id, false)
            .await
            .expect_err("tracking issue dropped");
        assert!(matches!(err, StoreError::IssueNotFound(_)));
        let deleted = state
            .get_issue(&workflow.tracking_issue_id, true)
            .await
            .expect("still retrievable with include_deleted");
        assert!(deleted.item.deleted);
    }

    #[tokio::test]
    async fn list_workflows_returns_active_workflows() {
        let state = test_state();
        upload_template(&state, full_lifecycle_yaml()).await;

        let _ = state
            .create_workflow(
                TEMPLATE_PATH.to_string(),
                sample_context(),
                None,
                Username::from("jayantk"),
                ActorRef::test(),
            )
            .await
            .expect("create workflow");

        let all = state
            .list_workflows(&WorkflowFilter::default())
            .await
            .expect("list");
        assert_eq!(all.len(), 1);
        let active = state
            .list_workflows(&WorkflowFilter {
                status: Some(WorkflowStatus::Active),
                ..Default::default()
            })
            .await
            .expect("list active");
        assert_eq!(active.len(), 1);
    }
}
