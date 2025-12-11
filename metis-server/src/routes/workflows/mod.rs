use crate::{
    AppState,
    routes::jobs::ApiError,
    store::Task,
};
use axum::Json;
use axum::extract::State;
use chrono::Utc;
use metis_common::workflows::{CreateWorkflowRequest, CreateWorkflowResponse};
use std::collections::{HashMap, HashSet};
use tracing::{error, info};

pub async fn create_workflow(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkflowRequest>,
) -> Result<Json<CreateWorkflowResponse>, ApiError> {
    info!("create_workflow invoked");
    
    let workflow = &payload.workflow;
    
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
    
    // Check for cycles using DFS
    if has_cycle(workflow) {
        error!("workflow contains a cycle");
        return Err(ApiError::bad_request("Workflow contains a cycle"));
    }
    
    // Generate workflow ID
    let workflow_id = uuid::Uuid::new_v4().hyphenated().to_string();
    
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
                "Workflow has unresolved dependencies"
            ));
        }
        
        // Create the tasks that are ready
        let mut store = state.store.write().await;
        for (task_name, parent_names) in tasks_to_create {
            // Map parent names to their job IDs
            let parent_ids: Vec<String> = parent_names
                .iter()
                .map(|name| task_ids[name].clone())
                .collect();
            
            // Generate job ID for this task
            let job_id = uuid::Uuid::new_v4().hyphenated().to_string();
            
            // Create the task
            let task = Task::Spawn {
                prompt: workflow.tasks[&task_name].prompt.clone(),
                context: payload.context.clone(),
                setup: workflow.tasks[&task_name].setup.clone(),
                cleanup: workflow.tasks[&task_name].cleanup.clone(),
            };
            
            store
                .add_task_with_id(job_id.clone(), task, parent_ids, Utc::now())
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

