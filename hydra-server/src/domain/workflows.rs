//! Server-side domain types for workflow templates.
//!
//! Workflow templates are authored as YAML documents in the doc store. This
//! module re-exports the API types as domain types (no separate shape is
//! needed yet), and owns the YAML parser, validator, and template-string
//! variable interpolation used by the engine.

use std::collections::{HashMap, HashSet};

use regex::Regex;
use std::sync::LazyLock;
use tracing::warn;

pub use hydra_common::WorkflowId;
pub use hydra_common::api::v1::workflows::{
    ContextParam, SessionSettingsTemplate, StartWorkflowRequest, StateEntryAction,
    TransitionTrigger, TransitionWorkflowRequest, Workflow, WorkflowHistoryEntry, WorkflowState,
    WorkflowStatus, WorkflowTemplate, WorkflowTransition,
};

/// Errors produced while parsing, validating, or interpolating workflow
/// templates.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml_ng::Error),

    #[error("initial_state '{0}' is not declared in states")]
    InitialStateUnknown(String),

    #[error("duplicate state id '{0}'")]
    DuplicateState(String),

    #[error("transition references unknown state '{state}' (from='{from}', to='{to}')")]
    TransitionUnknownState {
        state: String,
        from: String,
        to: String,
    },

    #[error("terminal state '{0}' must set terminal_status")]
    TerminalStateMissingStatus(String),

    #[error(
        "non-terminal state '{0}' has on_enter=noop and no outgoing auto transition; it would be a dead-end"
    )]
    DeadEndState(String),

    #[error("duplicate context parameter '{0}'")]
    DuplicateContextParam(String),

    #[error(
        "context parameter '{0}' is required but also has a default; required and default are mutually exclusive"
    )]
    ContextParamRequiredWithDefault(String),

    #[error("required context parameter '{0}' was not provided")]
    MissingRequiredContextParam(String),

    #[error("placeholder '{placeholder}' is malformed")]
    MalformedPlaceholder { placeholder: String },

    #[error("unknown template placeholder '{placeholder}'")]
    UnknownPlaceholder { placeholder: String },

    #[error("unknown context key '{key}' in placeholder '{{{{context.{key}}}}}'")]
    UnknownContextKey { key: String },

    #[error("placeholder '{{{{previous_step.progress}}}}' used without previous step in scope")]
    PreviousStepMissing,
}

/// Variables available when rendering a template string.
///
/// The engine constructs this from the active workflow's current state and
/// passes it to [`render_template`]. Fields are owned strings so the renderer
/// can produce results without lifetime gymnastics.
#[derive(Debug, Clone)]
pub struct TemplateScope {
    pub workflow_id: String,
    pub workflow_name: String,
    pub context: HashMap<String, String>,
    /// Progress text from the most recently completed child issue, if any.
    pub previous_step_progress: Option<String>,
}

impl TemplateScope {
    pub fn new(
        workflow_id: String,
        workflow_name: String,
        context: HashMap<String, String>,
        previous_step_progress: Option<String>,
    ) -> Self {
        Self {
            workflow_id,
            workflow_name,
            context,
            previous_step_progress,
        }
    }
}

/// Parse a YAML document into a [`WorkflowTemplate`] and validate it.
///
/// Validation rules:
/// - `initial_state` exists in `states`.
/// - All `transitions.from` and `transitions.to` reference declared states.
/// - Every terminal state has `terminal_status` set.
/// - Every non-terminal state either has a non-`Noop` `on_enter` or has an
///   outgoing `Auto` transition (otherwise the state is a dead-end).
/// - No duplicate state ids.
/// - No duplicate context parameter names; a parameter cannot be both
///   `required` and have a `default`.
///
/// Ambiguous transitions (multiple `OnChildStatus` triggers with the same
/// status leaving the same state) are logged at warn level but do not fail
/// validation.
pub fn parse_template(yaml: &str) -> Result<WorkflowTemplate, TemplateError> {
    let template: WorkflowTemplate = serde_yaml_ng::from_str(yaml)?;
    validate_template(&template)?;
    Ok(template)
}

/// Validate an already-deserialized [`WorkflowTemplate`].
///
/// Exposed separately from [`parse_template`] so callers that construct
/// templates programmatically (tests, future builders) can validate without
/// round-tripping through YAML.
pub fn validate_template(template: &WorkflowTemplate) -> Result<(), TemplateError> {
    let mut state_ids: HashSet<&str> = HashSet::new();
    for state in &template.states {
        if !state_ids.insert(state.id.as_str()) {
            return Err(TemplateError::DuplicateState(state.id.clone()));
        }
    }

    if !state_ids.contains(template.initial_state.as_str()) {
        return Err(TemplateError::InitialStateUnknown(
            template.initial_state.clone(),
        ));
    }

    for transition in &template.transitions {
        if !state_ids.contains(transition.from.as_str()) {
            return Err(TemplateError::TransitionUnknownState {
                state: transition.from.clone(),
                from: transition.from.clone(),
                to: transition.to.clone(),
            });
        }
        if !state_ids.contains(transition.to.as_str()) {
            return Err(TemplateError::TransitionUnknownState {
                state: transition.to.clone(),
                from: transition.from.clone(),
                to: transition.to.clone(),
            });
        }
    }

    for state in &template.states {
        if state.terminal {
            if state.terminal_status.is_none() {
                return Err(TemplateError::TerminalStateMissingStatus(state.id.clone()));
            }
        } else {
            let is_noop = matches!(state.on_enter, StateEntryAction::Noop);
            let has_auto_out = template
                .transitions
                .iter()
                .any(|t| t.from == state.id && matches!(t.trigger, TransitionTrigger::Auto));
            if is_noop && !has_auto_out {
                return Err(TemplateError::DeadEndState(state.id.clone()));
            }
        }
    }

    let mut seen_params: HashSet<&str> = HashSet::new();
    for param in &template.context {
        if !seen_params.insert(param.name.as_str()) {
            return Err(TemplateError::DuplicateContextParam(param.name.clone()));
        }
        if param.required && param.default.is_some() {
            return Err(TemplateError::ContextParamRequiredWithDefault(
                param.name.clone(),
            ));
        }
    }

    // Ambiguous OnChildStatus transitions: warn but don't fail.
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    for transition in &template.transitions {
        if let TransitionTrigger::OnChildStatus { status } = &transition.trigger {
            let key = (transition.from.clone(), status.to_string());
            if !seen_pairs.insert(key) {
                warn!(
                    state = %transition.from,
                    status = %status,
                    "ambiguous workflow transitions: multiple on_child_status triggers with the same status leave this state"
                );
            }
        }
    }

    Ok(())
}

/// Check that a caller-supplied context map satisfies the template's context
/// schema. Returns the resolved context map (with defaults applied) on
/// success.
///
/// Required parameters with no value supplied produce
/// [`TemplateError::MissingRequiredContextParam`]. Optional parameters with a
/// default get the default; optional parameters with no default and no
/// supplied value are simply absent from the resulting map.
pub fn resolve_context(
    template: &WorkflowTemplate,
    supplied: &HashMap<String, String>,
) -> Result<HashMap<String, String>, TemplateError> {
    let mut resolved: HashMap<String, String> = HashMap::new();
    for param in &template.context {
        match supplied.get(&param.name) {
            Some(v) => {
                resolved.insert(param.name.clone(), v.clone());
            }
            None => {
                if let Some(default) = &param.default {
                    resolved.insert(param.name.clone(), default.clone());
                } else if param.required {
                    return Err(TemplateError::MissingRequiredContextParam(
                        param.name.clone(),
                    ));
                }
            }
        }
    }
    // Pass through any extra supplied keys not declared in the schema; the
    // engine may use them for ad-hoc parameters. Template authors who want
    // strict context can keep their schema closed by requiring all keys.
    for (k, v) in supplied {
        resolved.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Ok(resolved)
}

static PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").expect("valid regex"));

/// Render a template string by substituting the supported placeholders.
///
/// Supported placeholders (whitespace inside `{{ }}` is tolerated):
/// - `{{workflow.id}}`
/// - `{{workflow.name}}`
/// - `{{context.<key>}}`
/// - `{{previous_step.progress}}`
///
/// Unknown placeholders return [`TemplateError::UnknownPlaceholder`]. An
/// unknown `context.<key>` returns [`TemplateError::UnknownContextKey`]. A
/// reference to `previous_step.progress` when none is in scope returns
/// [`TemplateError::PreviousStepMissing`]. By design the renderer does not
/// support arbitrary expressions.
pub fn render_template(input: &str, scope: &TemplateScope) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(input.len());
    let mut last_end = 0;
    for caps in PLACEHOLDER_RE.captures_iter(input) {
        let m = caps.get(0).expect("whole match");
        out.push_str(&input[last_end..m.start()]);
        let key = caps.get(1).expect("captured key").as_str();
        let value = resolve_placeholder(key, scope)?;
        out.push_str(&value);
        last_end = m.end();
    }
    out.push_str(&input[last_end..]);
    Ok(out)
}

fn resolve_placeholder(key: &str, scope: &TemplateScope) -> Result<String, TemplateError> {
    match key {
        "workflow.id" => Ok(scope.workflow_id.clone()),
        "workflow.name" => Ok(scope.workflow_name.clone()),
        "previous_step.progress" => scope
            .previous_step_progress
            .clone()
            .ok_or(TemplateError::PreviousStepMissing),
        other => {
            if let Some(ctx_key) = other.strip_prefix("context.") {
                // Reject nested access (we only support flat string context).
                if ctx_key.is_empty() || ctx_key.contains('.') {
                    return Err(TemplateError::MalformedPlaceholder {
                        placeholder: format!("{{{{{other}}}}}"),
                    });
                }
                scope.context.get(ctx_key).cloned().ok_or_else(|| {
                    TemplateError::UnknownContextKey {
                        key: ctx_key.to_string(),
                    }
                })
            } else {
                Err(TemplateError::UnknownPlaceholder {
                    placeholder: format!("{{{{{other}}}}}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::issues::{IssueStatus, IssueType};

    fn minimal_two_state_template() -> WorkflowTemplate {
        WorkflowTemplate::new(
            "Tiny".to_string(),
            "test".to_string(),
            vec![],
            vec![
                WorkflowState::new(
                    "develop".to_string(),
                    "Development".to_string(),
                    false,
                    StateEntryAction::CreateIssue {
                        issue_type: IssueType::Task,
                        title_template: "t".to_string(),
                        description_template: "d".to_string(),
                        assignee: None,
                        form: None,
                        session_settings: None,
                    },
                    None,
                ),
                WorkflowState::new(
                    "merged".to_string(),
                    "Merged".to_string(),
                    true,
                    StateEntryAction::Noop,
                    Some(IssueStatus::Closed),
                ),
            ],
            vec![WorkflowTransition::new(
                "develop".to_string(),
                "merged".to_string(),
                TransitionTrigger::OnChildStatus {
                    status: IssueStatus::Closed,
                },
                Some("Done".to_string()),
            )],
            "develop".to_string(),
        )
    }

    const PATCH_REVIEW_YAML: &str = r#"
name: "Patch Review"
description: "Full PR lifecycle"
initial_state: develop
context:
  - name: repo_name
    description: "Repository to work in"
    required: true
  - name: branch
    description: "Branch for the work"
    required: true
  - name: base_branch
    description: "Base branch to merge into"
    default: "main"

states:
  - id: develop
    name: "Development"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Develop: {{workflow.name}}"
      description_template: "Implement the changes."
      assignee: "swe"
      session_settings:
        repo_name: "{{context.repo_name}}"
        branch: "{{context.branch}}"

  - id: review
    name: "Code Review"
    on_enter:
      type: create_issue
      issue_type: review-request
      title_template: "Review: {{workflow.name}}"
      description_template: "Review the diff."
      assignee: "reviewer"

  - id: fix
    name: "Address Review Feedback"
    on_enter:
      type: create_issue
      issue_type: task
      title_template: "Fix"
      description_template: "Feedback: {{previous_step.progress}}"
      assignee: "swe"

  - id: merge
    name: "Merge"
    on_enter:
      type: create_issue
      issue_type: merge-request
      title_template: "Merge"
      description_template: "Merge it."
      assignee: "swe"

  - id: merged
    name: "Merged"
    terminal: true
    terminal_status: closed
    on_enter:
      type: noop

  - id: abandoned
    name: "Abandoned"
    terminal: true
    terminal_status: dropped
    on_enter:
      type: noop

transitions:
  - from: develop
    to: review
    label: "Ready for Review"
    trigger:
      type: on_child_status
      status: closed
  - from: develop
    to: abandoned
    label: "Abandoned"
    trigger:
      type: on_child_status
      status: failed
  - from: review
    to: merge
    label: "Approved"
    trigger:
      type: on_child_status
      status: closed
  - from: review
    to: fix
    label: "Changes Requested"
    trigger:
      type: on_child_status
      status: failed
  - from: fix
    to: review
    label: "Ready for Re-review"
    trigger:
      type: on_child_status
      status: closed
  - from: merge
    to: merged
    label: "Merge Complete"
    trigger:
      type: on_child_status
      status: closed
  - from: merge
    to: review
    label: "Merge Failed"
    trigger:
      type: on_child_status
      status: failed
"#;

    #[test]
    fn parses_patch_review_template() {
        let template = parse_template(PATCH_REVIEW_YAML).expect("parse + validate");
        assert_eq!(template.name, "Patch Review");
        assert_eq!(template.initial_state, "develop");
        assert_eq!(template.states.len(), 6);
        assert_eq!(template.transitions.len(), 7);
        assert_eq!(template.context.len(), 3);
        assert!(
            template
                .context
                .iter()
                .any(|p| p.name == "base_branch" && p.default.as_deref() == Some("main"))
        );
        let merged = template.states.iter().find(|s| s.id == "merged").unwrap();
        assert!(merged.terminal);
        assert_eq!(merged.terminal_status, Some(IssueStatus::Closed));
    }

    #[test]
    fn template_round_trips_through_yaml() {
        let template = minimal_two_state_template();
        let yaml = serde_yaml_ng::to_string(&template).expect("serialize");
        let round_trip = parse_template(&yaml).expect("parse + validate");
        assert_eq!(template, round_trip);
    }

    #[test]
    fn unknown_initial_state_fails_validation() {
        let mut template = minimal_two_state_template();
        template.initial_state = "ghost".to_string();
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(err, TemplateError::InitialStateUnknown(s) if s == "ghost"));
    }

    #[test]
    fn duplicate_state_id_fails_validation() {
        let mut template = minimal_two_state_template();
        template.states.push(WorkflowState::new(
            "develop".to_string(),
            "Dup".to_string(),
            false,
            StateEntryAction::Noop,
            None,
        ));
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(err, TemplateError::DuplicateState(s) if s == "develop"));
    }

    #[test]
    fn transition_to_unknown_state_fails_validation() {
        let mut template = minimal_two_state_template();
        template.transitions.push(WorkflowTransition::new(
            "develop".to_string(),
            "nowhere".to_string(),
            TransitionTrigger::Auto,
            None,
        ));
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(
            err,
            TemplateError::TransitionUnknownState { state, .. } if state == "nowhere"
        ));
    }

    #[test]
    fn terminal_state_without_terminal_status_fails_validation() {
        let mut template = minimal_two_state_template();
        template.states.iter_mut().for_each(|s| {
            if s.id == "merged" {
                s.terminal_status = None;
            }
        });
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(err, TemplateError::TerminalStateMissingStatus(s) if s == "merged"));
    }

    #[test]
    fn noop_non_terminal_without_auto_out_is_dead_end() {
        let template = WorkflowTemplate::new(
            "Dead".to_string(),
            String::new(),
            vec![],
            vec![
                WorkflowState::new(
                    "wait".to_string(),
                    "Wait".to_string(),
                    false,
                    StateEntryAction::Noop,
                    None,
                ),
                WorkflowState::new(
                    "end".to_string(),
                    "End".to_string(),
                    true,
                    StateEntryAction::Noop,
                    Some(IssueStatus::Closed),
                ),
            ],
            vec![],
            "wait".to_string(),
        );
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(err, TemplateError::DeadEndState(s) if s == "wait"));
    }

    #[test]
    fn noop_non_terminal_with_auto_out_is_ok() {
        let template = WorkflowTemplate::new(
            "Auto".to_string(),
            String::new(),
            vec![],
            vec![
                WorkflowState::new(
                    "start".to_string(),
                    "Start".to_string(),
                    false,
                    StateEntryAction::Noop,
                    None,
                ),
                WorkflowState::new(
                    "end".to_string(),
                    "End".to_string(),
                    true,
                    StateEntryAction::Noop,
                    Some(IssueStatus::Closed),
                ),
            ],
            vec![WorkflowTransition::new(
                "start".to_string(),
                "end".to_string(),
                TransitionTrigger::Auto,
                None,
            )],
            "start".to_string(),
        );
        validate_template(&template).expect("valid");
    }

    #[test]
    fn duplicate_context_param_fails_validation() {
        let mut template = minimal_two_state_template();
        template
            .context
            .push(ContextParam::new("repo_name".to_string(), None, true, None));
        template.context.push(ContextParam::new(
            "repo_name".to_string(),
            None,
            false,
            None,
        ));
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(err, TemplateError::DuplicateContextParam(n) if n == "repo_name"));
    }

    #[test]
    fn required_with_default_fails_validation() {
        let mut template = minimal_two_state_template();
        template.context.push(ContextParam::new(
            "branch".to_string(),
            None,
            true,
            Some("main".to_string()),
        ));
        let err = validate_template(&template).unwrap_err();
        assert!(matches!(
            err,
            TemplateError::ContextParamRequiredWithDefault(n) if n == "branch"
        ));
    }

    #[test]
    fn ambiguous_transitions_pass_with_warning() {
        let mut template = minimal_two_state_template();
        template.transitions.push(WorkflowTransition::new(
            "develop".to_string(),
            "merged".to_string(),
            TransitionTrigger::OnChildStatus {
                status: IssueStatus::Closed,
            },
            Some("Second path".to_string()),
        ));
        validate_template(&template).expect("ambiguous transitions are not a hard error");
    }

    fn scope_with(workflow_name: &str, ctx: &[(&str, &str)]) -> TemplateScope {
        TemplateScope::new(
            "w-abc123".to_string(),
            workflow_name.to_string(),
            ctx.iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            Some("did the thing".to_string()),
        )
    }

    #[test]
    fn renders_supported_placeholders() {
        let scope = scope_with("Patch Review", &[("repo_name", "dourolabs/hydra")]);
        let out = render_template(
            "PR for {{workflow.name}} ({{workflow.id}}) in {{context.repo_name}} — {{previous_step.progress}}",
            &scope,
        )
        .unwrap();
        assert_eq!(
            out,
            "PR for Patch Review (w-abc123) in dourolabs/hydra — did the thing"
        );
    }

    #[test]
    fn render_tolerates_whitespace_inside_braces() {
        let scope = scope_with("X", &[("k", "v")]);
        let out = render_template("{{  context.k  }} {{ workflow.name }}", &scope).unwrap();
        assert_eq!(out, "v X");
    }

    #[test]
    fn render_unknown_placeholder_errors() {
        let scope = scope_with("X", &[]);
        let err = render_template("hello {{bogus.thing}}", &scope).unwrap_err();
        assert!(matches!(err, TemplateError::UnknownPlaceholder { .. }));
    }

    #[test]
    fn render_unknown_context_key_errors() {
        let scope = scope_with("X", &[]);
        let err = render_template("{{context.missing}}", &scope).unwrap_err();
        assert!(matches!(err, TemplateError::UnknownContextKey { key } if key == "missing"));
    }

    #[test]
    fn render_previous_step_without_scope_errors() {
        let scope = TemplateScope::new("w-1".to_string(), "x".to_string(), HashMap::new(), None);
        let err = render_template("{{previous_step.progress}}", &scope).unwrap_err();
        assert!(matches!(err, TemplateError::PreviousStepMissing));
    }

    #[test]
    fn render_passes_through_text_with_no_placeholders() {
        let scope = scope_with("X", &[]);
        let out = render_template("plain text {with braces}", &scope).unwrap();
        assert_eq!(out, "plain text {with braces}");
    }

    #[test]
    fn render_rejects_nested_context_access() {
        let scope = scope_with("X", &[("k", "v")]);
        let err = render_template("{{context.k.deep}}", &scope).unwrap_err();
        assert!(matches!(err, TemplateError::MalformedPlaceholder { .. }));
    }

    #[test]
    fn resolve_context_applies_defaults_and_passes_required() {
        let template = WorkflowTemplate::new(
            "X".to_string(),
            String::new(),
            vec![
                ContextParam::new("repo_name".to_string(), None, true, None),
                ContextParam::new(
                    "base_branch".to_string(),
                    None,
                    false,
                    Some("main".to_string()),
                ),
            ],
            vec![WorkflowState::new(
                "end".to_string(),
                "End".to_string(),
                true,
                StateEntryAction::Noop,
                Some(IssueStatus::Closed),
            )],
            vec![],
            "end".to_string(),
        );
        let mut supplied = HashMap::new();
        supplied.insert("repo_name".to_string(), "owner/repo".to_string());
        let resolved = resolve_context(&template, &supplied).unwrap();
        assert_eq!(resolved.get("repo_name").unwrap(), "owner/repo");
        assert_eq!(resolved.get("base_branch").unwrap(), "main");
    }

    #[test]
    fn resolve_context_missing_required_errors() {
        let template = WorkflowTemplate::new(
            "X".to_string(),
            String::new(),
            vec![ContextParam::new("repo_name".to_string(), None, true, None)],
            vec![WorkflowState::new(
                "end".to_string(),
                "End".to_string(),
                true,
                StateEntryAction::Noop,
                Some(IssueStatus::Closed),
            )],
            vec![],
            "end".to_string(),
        );
        let err = resolve_context(&template, &HashMap::new()).unwrap_err();
        assert!(matches!(
            err,
            TemplateError::MissingRequiredContextParam(p) if p == "repo_name"
        ));
    }
}
