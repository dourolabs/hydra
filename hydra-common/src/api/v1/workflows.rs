use super::form::Form;
use super::issues::{IssueStatus, IssueType};
use crate::{IssueId, WorkflowId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A declarative workflow definition (FSA template) parsed from a YAML
/// document in the doc store. Templates are storage-managed via the document
/// store; this type is the in-memory shape used by the engine and the API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkflowTemplate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub context: Vec<ContextParam>,
    pub states: Vec<WorkflowState>,
    #[serde(default)]
    pub transitions: Vec<WorkflowTransition>,
    pub initial_state: String,
}

impl WorkflowTemplate {
    pub fn new(
        name: String,
        description: String,
        context: Vec<ContextParam>,
        states: Vec<WorkflowState>,
        transitions: Vec<WorkflowTransition>,
        initial_state: String,
    ) -> Self {
        Self {
            name,
            description,
            context,
            states,
            transitions,
            initial_state,
        }
    }
}

/// A parameter the workflow expects from the caller at instantiation time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ContextParam {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl ContextParam {
    pub fn new(
        name: String,
        description: Option<String>,
        required: bool,
        default: Option<String>,
    ) -> Self {
        Self {
            name,
            description,
            required,
            default,
        }
    }
}

/// A single state in the workflow FSA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkflowState {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub terminal: bool,
    pub on_enter: StateEntryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<IssueStatus>,
}

impl WorkflowState {
    pub fn new(
        id: String,
        name: String,
        terminal: bool,
        on_enter: StateEntryAction,
        terminal_status: Option<IssueStatus>,
    ) -> Self {
        Self {
            id,
            name,
            terminal,
            on_enter,
            terminal_status,
        }
    }
}

/// What happens when the workflow enters a state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum StateEntryAction {
    /// Create a child issue under the workflow's tracking issue.
    #[serde(rename = "create_issue")]
    CreateIssue {
        issue_type: IssueType,
        title_template: String,
        description_template: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assignee: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        form: Option<Form>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_settings: Option<SessionSettingsTemplate>,
    },
    /// No action — the engine immediately evaluates outgoing transitions.
    #[serde(rename = "noop")]
    Noop,
}

/// Template for session settings on a child issue. All fields are template
/// strings (e.g. `"{{context.repo_name}}"`) resolved against the workflow's
/// context at runtime by the domain layer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SessionSettingsTemplate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<String>>,
}

/// A transition between two states in the workflow FSA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkflowTransition {
    pub from: String,
    pub to: String,
    pub trigger: TransitionTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl WorkflowTransition {
    pub fn new(
        from: String,
        to: String,
        trigger: TransitionTrigger,
        label: Option<String>,
    ) -> Self {
        Self {
            from,
            to,
            trigger,
            label,
        }
    }
}

/// What causes a transition to fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "type")]
pub enum TransitionTrigger {
    /// Fires when the active child issue reaches the given terminal status.
    #[serde(rename = "on_child_status")]
    OnChildStatus { status: IssueStatus },
    /// Fires only when an agent or human explicitly invokes it via the
    /// `hydra workflows transition` command (or the equivalent API call).
    #[serde(rename = "explicit")]
    Explicit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition_id: Option<String>,
    },
    /// Fires immediately upon entering the source state.
    #[serde(rename = "auto")]
    Auto,
}

/// Lifecycle status of a workflow instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum WorkflowStatus {
    #[default]
    Active,
    Completed,
    Failed,
    Cancelled,
    #[serde(other)]
    Unknown,
}

impl WorkflowStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowStatus::Active => "active",
            WorkflowStatus::Completed => "completed",
            WorkflowStatus::Failed => "failed",
            WorkflowStatus::Cancelled => "cancelled",
            WorkflowStatus::Unknown => "unknown",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        )
    }
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A running workflow instance. Persisted as a row in the `workflows` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct Workflow {
    pub workflow_id: WorkflowId,
    pub template_path: String,
    pub template_snapshot: WorkflowTemplate,
    pub tracking_issue_id: IssueId,
    pub current_state: String,
    #[serde(default)]
    pub context: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_issue_id: Option<IssueId>,
    #[serde(default)]
    pub history: Vec<WorkflowHistoryEntry>,
    #[serde(default)]
    pub status: WorkflowStatus,
}

impl Workflow {
    pub fn new(
        workflow_id: WorkflowId,
        template_path: String,
        template_snapshot: WorkflowTemplate,
        tracking_issue_id: IssueId,
        current_state: String,
        context: HashMap<String, String>,
        active_issue_id: Option<IssueId>,
        history: Vec<WorkflowHistoryEntry>,
        status: WorkflowStatus,
    ) -> Self {
        Self {
            workflow_id,
            template_path,
            template_snapshot,
            tracking_issue_id,
            current_state,
            context,
            active_issue_id,
            history,
            status,
        }
    }
}

/// One entry in a workflow's transition history. Records the from/to state,
/// the transition's user-facing label, when it happened, and the child issue
/// (if any) created by the destination state's entry action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct WorkflowHistoryEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_state: Option<String>,
    pub to_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_label: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_issue_id: Option<IssueId>,
}

impl WorkflowHistoryEntry {
    pub fn new(
        from_state: Option<String>,
        to_state: String,
        transition_label: Option<String>,
        timestamp: DateTime<Utc>,
        child_issue_id: Option<IssueId>,
    ) -> Self {
        Self {
            from_state,
            to_state,
            transition_label,
            timestamp,
            child_issue_id,
        }
    }
}

/// Request body for `POST /v1/workflows`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct StartWorkflowRequest {
    pub template_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_issue: Option<IssueId>,
    #[serde(default)]
    pub context: HashMap<String, String>,
}

impl StartWorkflowRequest {
    pub fn new(
        template_path: String,
        parent_issue: Option<IssueId>,
        context: HashMap<String, String>,
    ) -> Self {
        Self {
            template_path,
            parent_issue,
            context,
        }
    }
}

/// Request body for `POST /v1/workflows/{workflow_id}/transition`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct TransitionWorkflowRequest {
    pub transition_id: String,
}

impl TransitionWorkflowRequest {
    pub fn new(transition_id: String) -> Self {
        Self { transition_id }
    }
}

/// Query parameters for `GET /v1/workflows`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListWorkflowsQuery {
    #[serde(default)]
    pub status: Option<WorkflowStatus>,
    /// Filter by any issue associated with the workflow (matches the
    /// `workflow_issues` reverse index — i.e., issues created by any state of
    /// the workflow). The CLI's `--issue` flag maps here.
    #[serde(default)]
    pub issue_id: Option<IssueId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_template() -> WorkflowTemplate {
        WorkflowTemplate {
            name: "Patch Review".to_string(),
            description: "Develop -> review -> merge".to_string(),
            context: vec![
                ContextParam {
                    name: "repo_name".to_string(),
                    description: Some("Repository to work in".to_string()),
                    required: true,
                    default: None,
                },
                ContextParam {
                    name: "base_branch".to_string(),
                    description: None,
                    required: false,
                    default: Some("main".to_string()),
                },
            ],
            states: vec![
                WorkflowState {
                    id: "develop".to_string(),
                    name: "Development".to_string(),
                    terminal: false,
                    on_enter: StateEntryAction::CreateIssue {
                        issue_type: IssueType::Task,
                        title_template: "Develop: {{workflow.name}}".to_string(),
                        description_template: "Implement the changes.".to_string(),
                        assignee: Some("swe".to_string()),
                        form: None,
                        session_settings: Some(SessionSettingsTemplate {
                            repo_name: Some("{{context.repo_name}}".to_string()),
                            branch: Some("{{context.branch}}".to_string()),
                            ..Default::default()
                        }),
                    },
                    terminal_status: None,
                },
                WorkflowState {
                    id: "merged".to_string(),
                    name: "Merged".to_string(),
                    terminal: true,
                    on_enter: StateEntryAction::Noop,
                    terminal_status: Some(IssueStatus::Closed),
                },
            ],
            transitions: vec![WorkflowTransition {
                from: "develop".to_string(),
                to: "merged".to_string(),
                trigger: TransitionTrigger::OnChildStatus {
                    status: IssueStatus::Closed,
                },
                label: Some("Done".to_string()),
            }],
            initial_state: "develop".to_string(),
        }
    }

    #[test]
    fn workflow_template_round_trips_through_serde() {
        let template = sample_template();
        let value = serde_json::to_value(&template).expect("serialize");
        let round_trip: WorkflowTemplate = serde_json::from_value(value).expect("deserialize");
        assert_eq!(template, round_trip);
    }

    #[test]
    fn workflow_round_trips_through_serde() {
        let mut context = HashMap::new();
        context.insert("repo_name".to_string(), "dourolabs/hydra".to_string());
        context.insert("branch".to_string(), "feature/widget".to_string());

        let workflow = Workflow {
            workflow_id: WorkflowId::new(),
            template_path: "/workflows/patch-review.yaml".to_string(),
            template_snapshot: sample_template(),
            tracking_issue_id: "i-track".parse().unwrap(),
            current_state: "develop".to_string(),
            context,
            active_issue_id: Some("i-child".parse().unwrap()),
            history: vec![WorkflowHistoryEntry {
                from_state: None,
                to_state: "develop".to_string(),
                transition_label: None,
                timestamp: Utc::now(),
                child_issue_id: Some("i-child".parse().unwrap()),
            }],
            status: WorkflowStatus::Active,
        };

        let value = serde_json::to_value(&workflow).expect("serialize");
        let round_trip: Workflow = serde_json::from_value(value).expect("deserialize");
        assert_eq!(workflow, round_trip);
    }

    #[test]
    fn state_entry_action_serializes_with_type_tag() {
        let action = StateEntryAction::CreateIssue {
            issue_type: IssueType::Task,
            title_template: "t".to_string(),
            description_template: "d".to_string(),
            assignee: None,
            form: None,
            session_settings: None,
        };
        let value = serde_json::to_value(&action).expect("serialize");
        assert_eq!(value["type"], "create_issue");

        let noop = serde_json::to_value(StateEntryAction::Noop).expect("serialize");
        assert_eq!(noop["type"], "noop");
    }

    #[test]
    fn transition_trigger_serializes_with_type_tag() {
        let on_child = TransitionTrigger::OnChildStatus {
            status: IssueStatus::Closed,
        };
        let value = serde_json::to_value(&on_child).expect("serialize");
        assert_eq!(value["type"], "on_child_status");
        assert_eq!(value["status"], "closed");

        let explicit = TransitionTrigger::Explicit {
            transition_id: Some("approve".to_string()),
        };
        let value = serde_json::to_value(&explicit).expect("serialize");
        assert_eq!(value["type"], "explicit");
        assert_eq!(value["transition_id"], "approve");

        let auto = serde_json::to_value(TransitionTrigger::Auto).expect("serialize");
        assert_eq!(auto["type"], "auto");
    }

    #[test]
    fn workflow_status_uses_kebab_case() {
        assert_eq!(
            serde_json::to_value(WorkflowStatus::Active).unwrap(),
            serde_json::json!("active")
        );
        assert_eq!(
            serde_json::to_value(WorkflowStatus::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
        let parsed: WorkflowStatus =
            serde_json::from_value(serde_json::json!("completed")).unwrap();
        assert_eq!(parsed, WorkflowStatus::Completed);
    }

    #[test]
    fn workflow_status_unknown_for_forward_compat() {
        let parsed: WorkflowStatus =
            serde_json::from_value(serde_json::json!("future-state")).unwrap();
        assert_eq!(parsed, WorkflowStatus::Unknown);
    }

    #[test]
    fn start_workflow_request_round_trips() {
        let mut context = HashMap::new();
        context.insert("k".to_string(), "v".to_string());
        let req = StartWorkflowRequest {
            template_path: "/workflows/x.yaml".to_string(),
            parent_issue: Some("i-parent".parse().unwrap()),
            context,
        };
        let value = serde_json::to_value(&req).expect("serialize");
        let round_trip: StartWorkflowRequest = serde_json::from_value(value).expect("deserialize");
        assert_eq!(req, round_trip);
    }

    #[test]
    fn workflow_template_parses_minimal_yaml_shape() {
        let json = serde_json::json!({
            "name": "Tiny",
            "initial_state": "done",
            "states": [
                {
                    "id": "done",
                    "name": "Done",
                    "terminal": true,
                    "terminal_status": "closed",
                    "on_enter": { "type": "noop" }
                }
            ]
        });
        let template: WorkflowTemplate = serde_json::from_value(json).expect("deserialize");
        assert_eq!(template.initial_state, "done");
        assert_eq!(template.states.len(), 1);
        assert!(template.states[0].terminal);
        assert!(template.transitions.is_empty());
        assert!(template.context.is_empty());
        assert!(template.description.is_empty());
    }
}
