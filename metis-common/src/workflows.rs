use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a workflow consisting of a graph of tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// Map of task names to their definitions.
    pub tasks: HashMap<String, TaskDefinition>,
}

/// Definition of a single task within a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDefinition {
    /// Type of the task (e.g., "codex").
    #[serde(rename = "type")]
    pub task_type: String,
    /// Prompt for the task.
    pub prompt: String,
    /// Input dependencies - can be a single task name or a list of task names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<TaskInputs>,
    /// Setup commands to run before the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    /// Cleanup commands to run after the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cleanup: Vec<String>,
}

/// Input dependencies for a task - can be a single task name or a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskInputs {
    /// Single task name.
    Single(String),
    /// List of task names.
    Multiple(Vec<String>),
}

impl TaskInputs {
    /// Convert to a vector of task names.
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            TaskInputs::Single(name) => vec![name.clone()],
            TaskInputs::Multiple(names) => names.clone(),
        }
    }
}

/// Request to create a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowRequest {
    /// The workflow definition.
    pub workflow: Workflow,
    /// Context bundle for the workflow (shared across all tasks).
    #[serde(default)]
    pub context: crate::jobs::Bundle,
}

/// Response from creating a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowResponse {
    /// Unique identifier for the workflow.
    pub workflow_id: String,
    /// Map of task names to their generated job IDs.
    pub task_ids: HashMap<String, String>,
}

