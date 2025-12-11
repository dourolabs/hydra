use crate::task_status::{Status, TaskStatusLog};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Definition of a workflow variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableDefinition {
    /// Variable name (must be a valid identifier).
    pub name: String,
    /// Variable value. If None, the variable is defined but unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Represents a workflow consisting of a graph of tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// Workflow-level variables that can be referenced in tasks using $VAR_NAME syntax.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<VariableDefinition>,
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

/// Summary of a workflow's current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    /// Unique identifier for the workflow.
    pub id: String,
    /// Sanitized notes from the most recent completed task or failure reason.
    #[serde(default)]
    pub notes: Option<String>,
    /// Aggregate workflow status derived from task states.
    pub status: Status,
    /// Aggregate timing information for the workflow.
    pub status_log: TaskStatusLog,
    /// Names of tasks that are currently running.
    #[serde(default)]
    pub running_tasks: Vec<String>,
}

/// Response containing all workflows known to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListWorkflowsResponse {
    pub workflows: Vec<WorkflowSummary>,
}

impl Workflow {
    /// Build a map of variable names to their values.
    /// Only includes variables that have a value set.
    fn build_variable_map(&self) -> HashMap<String, String> {
        self.variables
            .iter()
            .filter_map(|var| {
                var.value
                    .as_ref()
                    .map(|val| (var.name.clone(), val.clone()))
            })
            .collect()
    }

    /// Substitute variables in a string using $VAR_NAME or ${VAR_NAME} syntax.
    /// Supports escaping with $$ for a literal dollar sign.
    /// Variables that are not defined are left as-is.
    fn substitute_variables_in_string(s: &str, vars: &HashMap<String, String>) -> String {
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' {
                // Check for escaped dollar sign
                if chars.peek() == Some(&'$') {
                    chars.next(); // consume second $
                    result.push('$');
                    continue;
                }

                // Check for ${VAR_NAME} syntax
                if chars.peek() == Some(&'{') {
                    chars.next(); // consume {
                    let mut var_name = String::new();
                    let mut found_closing = false;

                    while let Some(ch) = chars.next() {
                        if ch == '}' {
                            found_closing = true;
                            break;
                        }
                        var_name.push(ch);
                    }

                    if found_closing && !var_name.is_empty() {
                        if let Some(value) = vars.get(&var_name) {
                            result.push_str(value);
                        } else {
                            // Leave unmatched variables as-is
                            result.push('$');
                            result.push('{');
                            result.push_str(&var_name);
                            result.push('}');
                        }
                    } else {
                        // Invalid ${...} syntax, leave as-is
                        result.push('$');
                        if found_closing {
                            result.push('}');
                        }
                        result.push_str(&var_name);
                    }
                } else {
                    // Check for $VAR_NAME syntax
                    let mut var_name = String::new();
                    let mut chars_clone = chars.clone();

                    // Collect variable name (alphanumeric + underscore)
                    while let Some(&ch) = chars_clone.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            var_name.push(ch);
                            chars_clone.next();
                        } else {
                            break;
                        }
                    }

                    if !var_name.is_empty() {
                        // Consume the chars we've identified
                        for _ in 0..var_name.len() {
                            chars.next();
                        }

                        if let Some(value) = vars.get(&var_name) {
                            result.push_str(value);
                        } else {
                            // Leave unmatched variables as-is
                            result.push('$');
                            result.push_str(&var_name);
                        }
                    } else {
                        // Not a variable, just a dollar sign
                        result.push('$');
                    }
                }
            } else {
                result.push(ch);
            }
        }

        result
    }

    /// Create a new workflow with all variable substitutions applied to task fields.
    /// This resolves all $VAR_NAME references in prompts, setup, and cleanup commands.
    pub fn with_substituted_variables(&self) -> Self {
        let vars = self.build_variable_map();

        let tasks: HashMap<String, TaskDefinition> = self
            .tasks
            .iter()
            .map(|(name, task)| {
                let substituted_task = TaskDefinition {
                    prompt: Self::substitute_variables_in_string(&task.prompt, &vars),
                    setup: task
                        .setup
                        .iter()
                        .map(|cmd| Self::substitute_variables_in_string(cmd, &vars))
                        .collect(),
                    cleanup: task
                        .cleanup
                        .iter()
                        .map(|cmd| Self::substitute_variables_in_string(cmd, &vars))
                        .collect(),
                    task_type: task.task_type.clone(),
                    inputs: task.inputs.clone(),
                };
                (name.clone(), substituted_task)
            })
            .collect();

        Self {
            variables: self.variables.clone(),
            tasks,
        }
    }

    /// Validate that all referenced variables are defined.
    /// Returns a list of undefined variable references.
    pub fn validate_variables(&self) -> Vec<String> {
        let defined_vars: std::collections::HashSet<String> =
            self.variables.iter().map(|v| v.name.clone()).collect();

        let mut undefined = Vec::new();
        let vars_in_use = self.collect_referenced_variables();

        for var in vars_in_use {
            if !defined_vars.contains(&var) {
                undefined.push(var);
            }
        }

        undefined.sort();
        undefined.dedup();
        undefined
    }

    /// Collect all variable names referenced in task prompts, setup, and cleanup commands.
    fn collect_referenced_variables(&self) -> Vec<String> {
        let mut vars = Vec::new();

        for task in self.tasks.values() {
            vars.extend(Self::extract_variable_names(&task.prompt));
            for cmd in &task.setup {
                vars.extend(Self::extract_variable_names(cmd));
            }
            for cmd in &task.cleanup {
                vars.extend(Self::extract_variable_names(cmd));
            }
        }

        vars
    }

    /// Extract variable names from a string that may contain $VAR_NAME or ${VAR_NAME} references.
    fn extract_variable_names(s: &str) -> Vec<String> {
        let mut vars = Vec::new();
        let mut chars = s.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '$' {
                // Skip escaped dollar signs
                if chars.peek() == Some(&'$') {
                    chars.next();
                    continue;
                }

                // Check for ${VAR_NAME} syntax
                if chars.peek() == Some(&'{') {
                    chars.next(); // consume {
                    let mut var_name = String::new();

                    while let Some(ch) = chars.next() {
                        if ch == '}' {
                            break;
                        }
                        var_name.push(ch);
                    }

                    if !var_name.is_empty() {
                        vars.push(var_name);
                    }
                } else {
                    // Check for $VAR_NAME syntax
                    let mut var_name = String::new();
                    let mut chars_clone = chars.clone();

                    while let Some(&ch) = chars_clone.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            var_name.push(ch);
                            chars_clone.next();
                        } else {
                            break;
                        }
                    }

                    if !var_name.is_empty() {
                        // Consume the chars we've identified
                        for _ in 0..var_name.len() {
                            chars.next();
                        }
                        vars.push(var_name);
                    }
                }
            }
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_workflow() -> Workflow {
        let mut tasks = HashMap::new();
        tasks.insert(
            "test-task".to_string(),
            TaskDefinition {
                task_type: "codex".to_string(),
                prompt: "Test prompt with $VAR1 and ${VAR2}".to_string(),
                inputs: None,
                setup: vec!["echo $VAR1".to_string(), "echo ${VAR3}".to_string()],
                cleanup: vec![],
            },
        );

        Workflow {
            variables: vec![
                VariableDefinition {
                    name: "VAR1".to_string(),
                    value: Some("value1".to_string()),
                },
                VariableDefinition {
                    name: "VAR2".to_string(),
                    value: Some("value2".to_string()),
                },
                VariableDefinition {
                    name: "VAR3".to_string(),
                    value: Some("value3".to_string()),
                },
            ],
            tasks,
        }
    }

    #[test]
    fn test_substitute_variables_in_string() {
        let vars: HashMap<String, String> = [
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux".to_string()),
        ]
        .iter()
        .cloned()
        .collect();

        assert_eq!(
            Workflow::substitute_variables_in_string("$FOO", &vars),
            "bar"
        );
        assert_eq!(
            Workflow::substitute_variables_in_string("${BAZ}", &vars),
            "qux"
        );
        assert_eq!(
            Workflow::substitute_variables_in_string("$FOO and ${BAZ}", &vars),
            "bar and qux"
        );
        assert_eq!(
            Workflow::substitute_variables_in_string("$$FOO", &vars),
            "$FOO"
        );
        assert_eq!(Workflow::substitute_variables_in_string("$$", &vars), "$");
        assert_eq!(
            Workflow::substitute_variables_in_string("$UNKNOWN", &vars),
            "$UNKNOWN"
        );
        assert_eq!(
            Workflow::substitute_variables_in_string("No vars here", &vars),
            "No vars here"
        );
    }

    #[test]
    fn test_with_substituted_variables() {
        let workflow = create_test_workflow();
        let substituted = workflow.with_substituted_variables();

        let task = &substituted.tasks["test-task"];
        assert_eq!(task.prompt, "Test prompt with value1 and value2");
        assert_eq!(task.setup[0], "echo value1");
        assert_eq!(task.setup[1], "echo value3");
    }

    #[test]
    fn test_extract_variable_names() {
        assert_eq!(
            Workflow::extract_variable_names("$FOO"),
            vec!["FOO".to_string()]
        );
        assert_eq!(
            Workflow::extract_variable_names("${BAR}"),
            vec!["BAR".to_string()]
        );
        assert_eq!(
            Workflow::extract_variable_names("$FOO and ${BAR}"),
            vec!["FOO".to_string(), "BAR".to_string()]
        );
        assert_eq!(
            Workflow::extract_variable_names("$$FOO"),
            Vec::<String>::new()
        );
        assert_eq!(
            Workflow::extract_variable_names("No vars"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_validate_variables_all_defined() {
        let workflow = create_test_workflow();
        let undefined = workflow.validate_variables();
        assert!(undefined.is_empty());
    }

    #[test]
    fn test_validate_variables_undefined() {
        let mut tasks = HashMap::new();
        tasks.insert(
            "test-task".to_string(),
            TaskDefinition {
                task_type: "codex".to_string(),
                prompt: "Test $UNDEFINED_VAR".to_string(),
                inputs: None,
                setup: vec![],
                cleanup: vec![],
            },
        );

        let workflow = Workflow {
            variables: vec![],
            tasks,
        };

        let undefined = workflow.validate_variables();
        assert_eq!(undefined, vec!["UNDEFINED_VAR".to_string()]);
    }

    #[test]
    fn test_variable_without_value_not_substituted() {
        let mut tasks = HashMap::new();
        tasks.insert(
            "test-task".to_string(),
            TaskDefinition {
                task_type: "codex".to_string(),
                prompt: "Test $VAR_WITHOUT_VALUE".to_string(),
                inputs: None,
                setup: vec![],
                cleanup: vec![],
            },
        );

        let workflow = Workflow {
            variables: vec![VariableDefinition {
                name: "VAR_WITHOUT_VALUE".to_string(),
                value: None,
            }],
            tasks,
        };

        let substituted = workflow.with_substituted_variables();
        let task = &substituted.tasks["test-task"];
        // Variable without value should remain as-is
        assert_eq!(task.prompt, "Test $VAR_WITHOUT_VALUE");
    }

    #[test]
    fn test_empty_variable_value() {
        let mut tasks = HashMap::new();
        tasks.insert(
            "test-task".to_string(),
            TaskDefinition {
                task_type: "codex".to_string(),
                prompt: "Test $EMPTY_VAR".to_string(),
                inputs: None,
                setup: vec![],
                cleanup: vec![],
            },
        );

        let workflow = Workflow {
            variables: vec![VariableDefinition {
                name: "EMPTY_VAR".to_string(),
                value: Some("".to_string()),
            }],
            tasks,
        };

        let substituted = workflow.with_substituted_variables();
        let task = &substituted.tasks["test-task"];
        // Empty value should be substituted (empty string)
        assert_eq!(task.prompt, "Test ");
    }

    #[test]
    fn test_complex_substitution() {
        let vars: HashMap<String, String> = [("PATH".to_string(), "/usr/bin".to_string())]
            .iter()
            .cloned()
            .collect();

        assert_eq!(
            Workflow::substitute_variables_in_string("PATH=$PATH exec command", &vars),
            "PATH=/usr/bin exec command"
        );
        assert_eq!(
            Workflow::substitute_variables_in_string("Escape $$ and use ${PATH}", &vars),
            "Escape $ and use /usr/bin"
        );
    }
}
