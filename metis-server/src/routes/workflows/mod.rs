use crate::{
    AppState,
    routes::{bundles::resolve_bundle_spec, jobs::ApiError},
    store::{Edge, Status, Store, StoreError, Task, TaskStatusLog},
};
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use metis_common::workflows::{
    CreateWorkflowRequest, CreateWorkflowResponse, ListWorkflowsResponse, WorkflowSummary,
};
use std::collections::{HashMap, HashSet};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct WorkflowRecord {
    pub created_at: DateTime<Utc>,
    pub task_ids: HashMap<String, String>,
    pub prompt: Option<String>,
}

pub async fn list_workflows(
    State(state): State<AppState>,
) -> Result<Json<ListWorkflowsResponse>, ApiError> {
    info!("list_workflows invoked");

    let store_read = state.store.read().await;
    let store = store_read.as_ref();
    let workflows = state.workflows.read().await;

    let mut summaries_with_times = Vec::new();
    for (workflow_id, record) in workflows.iter() {
        match workflow_summary_with_time(workflow_id, record, store).await {
            Ok(summary) => summaries_with_times.push(summary),
            Err(err) => {
                error!(
                    workflow_id = %workflow_id,
                    error = %err,
                    "failed to build workflow summary while listing workflows"
                );
                continue;
            }
        }
    }

    summaries_with_times.sort_by(|a, b| {
        let time_a = a.1;
        let time_b = b.1;
        time_b.cmp(&time_a)
    });

    let workflows: Vec<WorkflowSummary> = summaries_with_times
        .into_iter()
        .map(|(summary, _)| summary)
        .collect();

    info!(
        workflow_count = workflows.len(),
        "list_workflows completed successfully"
    );

    Ok(Json(ListWorkflowsResponse { workflows }))
}

pub async fn get_workflow(
    State(state): State<AppState>,
    Path(workflow_id): Path<String>,
) -> Result<Json<WorkflowSummary>, ApiError> {
    info!(workflow_id = %workflow_id, "get_workflow invoked");
    let workflow_id = workflow_id.trim().to_string();
    if workflow_id.is_empty() {
        return Err(ApiError::bad_request("workflow_id must not be empty"));
    }

    let record = {
        let workflows = state.workflows.read().await;
        workflows.get(&workflow_id).cloned()
    }
    .ok_or_else(|| {
        error!(workflow_id = %workflow_id, "workflow not found");
        ApiError::not_found(format!("workflow '{workflow_id}' not found"))
    })?;

    let store_read = state.store.read().await;
    let store = store_read.as_ref();

    let (summary, _) = workflow_summary_with_time(&workflow_id, &record, store)
        .await
        .map_err(|err| match err {
            StoreError::TaskNotFound(_) => {
                error!(
                    workflow_id = %workflow_id,
                    error = %err,
                    "workflow is missing task data"
                );
                ApiError::internal(anyhow::anyhow!(
                    "workflow '{workflow_id}' is missing task data"
                ))
            }
            err => {
                error!(
                    workflow_id = %workflow_id,
                    error = %err,
                    "failed to load workflow summary"
                );
                ApiError::internal(anyhow::anyhow!(
                    "failed to load workflow '{workflow_id}': {err}"
                ))
            }
        })?;

    Ok(Json(summary))
}

pub async fn create_workflow(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> Result<Json<CreateWorkflowResponse>, ApiError> {
    info!("create_workflow invoked");

    let workflow = &payload.workflow;

    // Validate variables: check that all referenced variables are defined
    let undefined_vars = workflow.validate_variables();
    if !undefined_vars.is_empty() {
        error!(
            undefined_vars = ?undefined_vars,
            "workflow contains undefined variable references"
        );
        return Err(ApiError::bad_request(format!(
            "Workflow references undefined variables: {}",
            undefined_vars.join(", ")
        )));
    }

    // Apply variable substitution to all task fields
    let workflow = workflow.with_substituted_variables();
    let workflow_variables = workflow.variable_map();
    let workflow_prompt = workflow_variables.get("PROMPT").cloned();

    // Check for cycles using DFS
    if has_cycle(&workflow) {
        error!("workflow contains a cycle");
        return Err(ApiError::bad_request("Workflow contains a cycle"));
    }

    // Validate workflow: check that all input references are valid
    for (task_name, task_def) in &workflow.tasks {
        if let Some(inputs) = &task_def.inputs {
            let input_names = inputs.to_vec();
            for input_name in &input_names {
                if !workflow.tasks.contains_key(input_name) {
                    error!(
                        task = %task_name,
                        input = %input_name,
                        "workflow contains invalid input reference"
                    );
                    return Err(ApiError::bad_request(format!(
                        "Task '{task_name}' references unknown input task '{input_name}'"
                    )));
                }
            }
        }
    }

    let (context, github_token) =
        resolve_bundle_spec(payload.context, &state.service_state)?;

    // Generate workflow ID
    let workflow_id = uuid::Uuid::new_v4().hyphenated().to_string();
    let workflow_created_at = Utc::now();

    // Create tasks in topological order
    let mut task_ids: HashMap<String, String> = HashMap::new();
    let mut created_tasks = HashSet::new();

    // Topological sort: process tasks in dependency order
    let mut remaining_tasks: Vec<String> = workflow.tasks.keys().cloned().collect();

    while !remaining_tasks.is_empty() {
        let mut progress = false;
        let mut tasks_to_create: Vec<(String, Vec<String>)> = Vec::new();

        // Find tasks with all dependencies already created
        for task_name in &remaining_tasks {
            let task_def = &workflow.tasks[task_name];
            let parent_names = task_def
                .inputs
                .as_ref()
                .map(|i| i.to_vec())
                .unwrap_or_default();

            // Check if all parents are created
            let all_parents_created = parent_names
                .iter()
                .all(|parent_name| created_tasks.contains(parent_name));

            if all_parents_created {
                tasks_to_create.push((task_name.clone(), parent_names));
                progress = true;
            }
        }

        if !progress {
            // This shouldn't happen if we validated for cycles, but handle it anyway
            error!("workflow has unresolved dependencies (possible cycle)");
            return Err(ApiError::bad_request(
                "Workflow has unresolved dependencies",
            ));
        }

        // Create the tasks that are ready
        let mut store = state.store.write().await;
        for (task_name, parent_names) in tasks_to_create {
            // Map parent names to their job IDs
            let parent_edges: Vec<Edge> = parent_names
                .iter()
                .map(|name| Edge {
                    id: task_ids[name].clone(),
                    name: Some(name.clone()),
                })
                .collect();

            // Generate job ID for this task
            let job_id = uuid::Uuid::new_v4().hyphenated().to_string();

            // Create the task
            let mut env_vars = workflow_variables.clone();
            if let Some(token) = github_token.clone() {
                env_vars.entry("GH_TOKEN".to_string()).or_insert(token);
            }

            let task = Task::Spawn {
                prompt: workflow.tasks[&task_name].prompt.clone(),
                context: context.clone(),
                setup: workflow.tasks[&task_name].setup.clone(),
                cleanup: workflow.tasks[&task_name].cleanup.clone(),
                env_vars,
            };

            store
                .add_task_with_id(job_id.clone(), task, parent_edges, workflow_created_at)
                .await
                .map_err(|err| {
                    error!(
                        error = %err,
                        task = %task_name,
                        "failed to store workflow task"
                    );
                    ApiError::internal(anyhow::anyhow!("Failed to store task '{task_name}': {err}"))
                })?;

            task_ids.insert(task_name.clone(), job_id);
            created_tasks.insert(task_name);
        }
        drop(store);

        // Remove created tasks from remaining
        remaining_tasks.retain(|name| !created_tasks.contains(name));
    }

    {
        let mut workflows = state.workflows.write().await;
        workflows.insert(
            workflow_id.clone(),
            WorkflowRecord {
                created_at: workflow_created_at,
                task_ids: task_ids.clone(),
                prompt: workflow_prompt,
            },
        );
    }

    info!(
        workflow_id = %workflow_id,
        task_count = task_ids.len(),
        "workflow created successfully"
    );

    Ok(Json(CreateWorkflowResponse {
        workflow_id,
        task_ids,
    }))
}

async fn workflow_summary_with_time(
    workflow_id: &str,
    record: &WorkflowRecord,
    store: &dyn Store,
) -> Result<(WorkflowSummary, Option<DateTime<Utc>>), StoreError> {
    let summary = workflow_summary(workflow_id, record, store).await?;
    let reference_time = summary
        .status_log
        .start_time
        .or(Some(summary.status_log.creation_time));

    Ok((summary, reference_time))
}

async fn workflow_summary(
    workflow_id: &str,
    record: &WorkflowRecord,
    store: &dyn Store,
) -> Result<WorkflowSummary, StoreError> {
    let mut running_tasks = Vec::new();
    let mut has_failed = false;
    let mut has_running = false;
    let mut has_pending = false;
    let mut has_blocked = false;
    let mut all_complete = true;
    let mut earliest_start: Option<DateTime<Utc>> = None;
    let mut latest_end: Option<DateTime<Utc>> = None;
    let mut failure_reason: Option<String> = None;
    let mut latest_note: Option<(DateTime<Utc>, String)> = None;
    let mut latest_failure: Option<(DateTime<Utc>, String)> = None;

    for (task_name, task_id) in &record.task_ids {
        let status_log = store.get_status_log(task_id).await?;
        let result = store.get_result(task_id);
        let task_note = result
            .as_ref()
            .and_then(|res| crate::routes::jobs::note_from_result(res));
        match status_log.current_status {
            Status::Failed => {
                has_failed = true;
                all_complete = false;
                let failure_note = match result.as_ref() {
                    Some(Err(_)) => task_note.clone(),
                    _ => None,
                };

                if failure_reason.is_none() {
                    failure_reason = failure_note.clone();
                }
                if let Some(reason) = failure_note {
                    let failure_time = status_log
                        .end_time
                        .or(status_log.start_time)
                        .unwrap_or(status_log.creation_time);
                    latest_failure = select_latest(latest_failure, failure_time, reason);
                }
            }
            Status::Complete => {}
            Status::Running => {
                has_running = true;
                all_complete = false;
                running_tasks.push(task_name.clone());
            }
            Status::Pending => {
                has_pending = true;
                all_complete = false;
            }
            Status::Blocked => {
                has_blocked = true;
                all_complete = false;
            }
        }

        if let Some(start_time) = status_log.start_time {
            earliest_start = Some(match earliest_start {
                Some(current) => current.min(start_time),
                None => start_time,
            });
        }

        if matches!(status_log.current_status, Status::Complete | Status::Failed) {
            if let Some(end_time) = status_log.end_time.or(status_log.start_time) {
                latest_end = Some(match latest_end {
                    Some(current) => current.max(end_time),
                    None => end_time,
                });
            }
        }

        if status_log.current_status == Status::Complete && matches!(result, Some(Ok(_))) {
            if let Some(note) = task_note {
                let note_time = status_log
                    .end_time
                    .or(status_log.start_time)
                    .unwrap_or(status_log.creation_time);
                latest_note = select_latest(latest_note, note_time, note);
            }
        }
    }

    running_tasks.sort();

    let latest_failure_reason = latest_failure.as_ref().map(|(_, reason)| reason.clone());

    let status = if has_failed {
        Status::Failed
    } else if all_complete {
        Status::Complete
    } else if has_running {
        Status::Running
    } else if has_pending {
        Status::Pending
    } else if has_blocked {
        Status::Blocked
    } else {
        Status::Pending
    };

    let failure_reason = latest_failure_reason.clone().or(failure_reason);

    let notes = if status == Status::Failed {
        failure_reason
            .clone()
            .or_else(|| latest_note.map(|(_, note)| note))
    } else {
        latest_note.map(|(_, note)| note)
    };

    let status_log = TaskStatusLog {
        creation_time: record.created_at,
        start_time: earliest_start,
        end_time: match status {
            Status::Complete | Status::Failed => latest_end,
            _ => None,
        },
        current_status: status,
    };

    Ok(WorkflowSummary {
        id: workflow_id.to_string(),
        prompt: record.prompt.clone(),
        notes,
        status,
        status_log,
        running_tasks,
    })
}

fn select_latest(
    current: Option<(DateTime<Utc>, String)>,
    time: DateTime<Utc>,
    value: String,
) -> Option<(DateTime<Utc>, String)> {
    match current {
        Some((existing_time, existing_value)) => {
            if time > existing_time {
                Some((time, value))
            } else {
                Some((existing_time, existing_value))
            }
        }
        None => Some((time, value)),
    }
}

/// Check if the workflow has a cycle using DFS.
fn has_cycle(workflow: &metis_common::workflows::Workflow) -> bool {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    for task_name in workflow.tasks.keys() {
        if !visited.contains(task_name) {
            if has_cycle_dfs(workflow, task_name, &mut visited, &mut rec_stack) {
                return true;
            }
        }
    }

    false
}

fn has_cycle_dfs(
    workflow: &metis_common::workflows::Workflow,
    task_name: &str,
    visited: &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
) -> bool {
    visited.insert(task_name.to_string());
    rec_stack.insert(task_name.to_string());

    if let Some(task_def) = workflow.tasks.get(task_name) {
        if let Some(inputs) = &task_def.inputs {
            for input_name in inputs.to_vec() {
                if !visited.contains(&input_name) {
                    if has_cycle_dfs(workflow, &input_name, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(&input_name) {
                    return true;
                }
            }
        }
    }

    rec_stack.remove(task_name);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{state::{GitRepository, ServiceState}, test::test_state};
    use axum::{
        Json,
        extract::{Path, State},
    };
    use metis_common::{
        jobs::{Bundle, BundleSpec},
        workflows::{CreateWorkflowRequest, TaskDefinition, VariableDefinition, Workflow},
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn workflow_summary_includes_prompt_from_variables() {
        let state = test_state();
        let workflow = Workflow {
            variables: vec![VariableDefinition {
                name: "PROMPT".to_string(),
                value: Some("describe the repo".to_string()),
            }],
            tasks: HashMap::from([(
                "first".to_string(),
                TaskDefinition {
                    task_type: "codex".to_string(),
                    prompt: "Do the work".to_string(),
                    inputs: None,
                    setup: vec![],
                    cleanup: vec![],
                },
            )]),
        };

        let request = CreateWorkflowRequest {
            workflow,
            context: BundleSpec::None,
        };

        let response = create_workflow(State(state.clone()), Json(request))
            .await
            .expect("workflow created");
        let workflow_id = response.0.workflow_id.clone();

        let summary = get_workflow(State(state.clone()), Path(workflow_id.clone()))
            .await
            .expect("workflow summary loaded")
            .0;

        assert_eq!(summary.id, workflow_id);
        assert_eq!(summary.prompt.as_deref(), Some("describe the repo"));
    }

    #[tokio::test]
    async fn workflow_service_repository_context_sets_token_and_bundle() {
        let mut state = test_state();
        let repo = GitRepository {
            name: "svc".to_string(),
            remote_url: "https://example.com/service.git".to_string(),
            default_branch: None,
            github_token: Some("wf-token".to_string()),
        };
        state.service_state = Arc::new(ServiceState {
            repositories: HashMap::from([("svc".to_string(), repo.clone())]),
        });

        let workflow = Workflow {
            variables: vec![],
            tasks: HashMap::from([(
                "first".to_string(),
                TaskDefinition {
                    task_type: "codex".to_string(),
                    prompt: "do it".to_string(),
                    inputs: None,
                    setup: vec![],
                    cleanup: vec![],
                },
            )]),
        };

        let request = CreateWorkflowRequest {
            workflow,
            context: BundleSpec::ServiceRepository {
                name: "svc".to_string(),
                rev: None,
            },
        };

        let response = create_workflow(State(state.clone()), Json(request))
            .await
            .expect("workflow created")
            .0;
        let first_task_id = response.task_ids.get("first").expect("first task id");
        let store_read = state.store.read().await;
        let task = store_read
            .get_task(first_task_id)
            .await
            .expect("task stored");
        match task {
            Task::Spawn {
                context,
                env_vars,
                ..
            } => {
                assert_eq!(
                    context,
                    Bundle::GitRepository {
                        url: repo.remote_url.clone(),
                        rev: "main".to_string()
                    }
                );
                assert_eq!(env_vars.get("GH_TOKEN"), Some(&"wf-token".to_string()));
            }
        }
    }
}
