//! HTTP routes for the workflow engine.
//!
//! Wires the `AppState` workflow lifecycle methods (`create_workflow`,
//! `get_workflow`, `list_workflows`, `transition_workflow`,
//! `cancel_workflow`) into the five endpoints described in design v3:
//!
//! - `POST   /v1/workflows`
//! - `GET    /v1/workflows`
//! - `GET    /v1/workflows/{workflow_id}`
//! - `POST   /v1/workflows/{workflow_id}/transition`
//! - `DELETE /v1/workflows/{workflow_id}`

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use hydra_common::{
    IssueId, Versioned, WorkflowId,
    api::v1::{
        ApiError,
        workflows::{StartWorkflowRequest, TransitionWorkflowRequest, Workflow, WorkflowStatus},
    },
};
use serde::Deserialize;
use tracing::{error, info};

use crate::{
    app::{AppState, CancelWorkflowError, CreateWorkflowError, TransitionWorkflowError},
    domain::{
        actors::{Actor, ActorRef},
        workflows::TransitionTrigger,
    },
    store::{StoreError, WorkflowFilter},
};

/// Query parameters for `GET /v1/workflows`.
#[derive(Debug, Default, Deserialize)]
pub struct ListWorkflowsQuery {
    #[serde(default)]
    pub status: Option<WorkflowStatus>,
    /// Filter by any issue associated with the workflow (matches the
    /// `workflow_issues` reverse index — i.e., issues created by any state of
    /// the workflow). The CLI's `--issue` flag maps here.
    #[serde(default)]
    pub issue_id: Option<IssueId>,
}

/// POST /v1/workflows — start a new workflow.
pub async fn create_workflow(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<StartWorkflowRequest>,
) -> Result<Json<Versioned<Workflow>>, ApiError> {
    info!(
        actor = %actor.name(),
        template_path = %payload.template_path,
        "create_workflow invoked"
    );

    let creator = actor.creator.clone();
    let actor_ref = ActorRef::from(&actor);
    let workflow = state
        .create_workflow(
            payload.template_path.clone(),
            payload.context,
            payload.parent_issue,
            creator,
            actor_ref,
        )
        .await
        .map_err(map_create_workflow_error)?;

    info!(
        actor = %actor.name(),
        workflow_id = %workflow.item.workflow_id,
        "create_workflow completed"
    );
    Ok(Json(workflow))
}

/// GET /v1/workflows — list workflows, with optional filters.
pub async fn list_workflows(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<ListWorkflowsQuery>,
) -> Result<Json<Vec<Versioned<Workflow>>>, ApiError> {
    info!(
        actor = %actor.name(),
        status = ?query.status,
        issue_id = ?query.issue_id,
        "list_workflows invoked"
    );

    let filter = WorkflowFilter {
        status: query.status,
        associated_issue_id: query.issue_id,
        ..WorkflowFilter::default()
    };
    let workflows = state
        .list_workflows(&filter)
        .await
        .map_err(map_store_error)?;

    info!(
        actor = %actor.name(),
        returned = workflows.len(),
        "list_workflows completed"
    );
    Ok(Json(workflows))
}

/// GET /v1/workflows/:workflow_id — fetch a single workflow.
pub async fn get_workflow(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(workflow_id): Path<WorkflowId>,
) -> Result<Json<Versioned<Workflow>>, ApiError> {
    info!(actor = %actor.name(), workflow_id = %workflow_id, "get_workflow invoked");

    let workflow = state
        .get_workflow(&workflow_id)
        .await
        .map_err(map_store_error)?;

    info!(actor = %actor.name(), workflow_id = %workflow_id, "get_workflow completed");
    Ok(Json(workflow))
}

/// POST /v1/workflows/:workflow_id/transition — invoke an explicit transition.
///
/// Per design v3, only [`TransitionTrigger::Explicit`] transitions can be
/// driven through this endpoint. Auto and `on_child_status` triggers fire
/// from the engine automation. We pre-check the workflow's outgoing
/// transitions so that an attempt to drive a non-Explicit one returns `400`
/// (with a "this trigger is not explicit" message) rather than the generic
/// "no such transition" 404 from the AppState layer.
pub async fn transition_workflow(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(workflow_id): Path<WorkflowId>,
    Json(payload): Json<TransitionWorkflowRequest>,
) -> Result<Json<Versioned<Workflow>>, ApiError> {
    info!(
        actor = %actor.name(),
        workflow_id = %workflow_id,
        transition_id = %payload.transition_id,
        "transition_workflow invoked"
    );

    let current = state
        .get_workflow(&workflow_id)
        .await
        .map_err(map_store_error)?
        .item;

    // If the current state has no Explicit outgoing transitions at all, every
    // attempted invocation is by definition asking for a non-Explicit trigger
    // — answer with 400 rather than a misleading 404.
    let has_any_explicit = current
        .template_snapshot
        .transitions
        .iter()
        .any(|t| t.from == current.current_state && is_explicit(&t.trigger));
    if !has_any_explicit {
        return Err(ApiError::bad_request(format!(
            "state '{}' has no explicit transitions; \
             auto and on_child_status triggers fire from the engine",
            current.current_state
        )));
    }

    let actor_ref = ActorRef::from(&actor);
    let workflow = state
        .transition_workflow(&workflow_id, &payload.transition_id, actor_ref)
        .await
        .map_err(map_transition_workflow_error)?;

    info!(
        actor = %actor.name(),
        workflow_id = %workflow_id,
        transition_id = %payload.transition_id,
        new_state = %workflow.item.current_state,
        "transition_workflow completed"
    );
    Ok(Json(workflow))
}

/// DELETE /v1/workflows/:workflow_id — cancel a running workflow.
pub async fn cancel_workflow(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(workflow_id): Path<WorkflowId>,
) -> Result<Json<Versioned<Workflow>>, ApiError> {
    info!(actor = %actor.name(), workflow_id = %workflow_id, "cancel_workflow invoked");

    let actor_ref = ActorRef::from(&actor);
    let workflow = state
        .cancel_workflow(&workflow_id, actor_ref)
        .await
        .map_err(map_cancel_workflow_error)?;

    info!(actor = %actor.name(), workflow_id = %workflow_id, "cancel_workflow completed");
    Ok(Json(workflow))
}

fn is_explicit(trigger: &TransitionTrigger) -> bool {
    matches!(trigger, TransitionTrigger::Explicit { .. })
}

fn map_store_error(err: StoreError) -> ApiError {
    match err {
        StoreError::WorkflowNotFound(id) => {
            ApiError::not_found(format!("workflow '{id}' not found"))
        }
        other => {
            error!(error = %other, "workflow store operation failed");
            ApiError::internal(format!("workflow store error: {other}"))
        }
    }
}

fn map_create_workflow_error(err: CreateWorkflowError) -> ApiError {
    match err {
        CreateWorkflowError::TemplateNotFound { path } => {
            ApiError::not_found(format!("no workflow template at '{path}'"))
        }
        CreateWorkflowError::TemplateInvalid { path, source } => {
            ApiError::bad_request(format!("invalid workflow template at '{path}': {source}"))
        }
        CreateWorkflowError::CreateTrackingIssue { source } => {
            error!(error = %source, "failed to create tracking issue for workflow");
            ApiError::internal(format!("failed to create tracking issue: {source}"))
        }
        CreateWorkflowError::EnterState { state, source } => {
            error!(error = %source, state = %state, "failed to enter initial state");
            ApiError::internal(format!("failed to enter state '{state}': {source}"))
        }
        CreateWorkflowError::Store { source } => map_store_error(source),
    }
}

fn map_transition_workflow_error(err: TransitionWorkflowError) -> ApiError {
    match err {
        TransitionWorkflowError::WorkflowNotFound(id) => {
            ApiError::not_found(format!("workflow '{id}' not found"))
        }
        TransitionWorkflowError::WorkflowTerminal { status } => ApiError::conflict(format!(
            "workflow has reached terminal status '{status}'; cannot transition"
        )),
        TransitionWorkflowError::TransitionNotFound {
            state,
            transition_id,
        } => ApiError::not_found(format!(
            "no explicit transition '{transition_id}' from state '{state}'"
        )),
        TransitionWorkflowError::EnterState { state, source } => {
            error!(error = %source, state = %state, "failed to enter target state");
            ApiError::internal(format!("failed to enter state '{state}': {source}"))
        }
        TransitionWorkflowError::Store { source } => map_store_error(source),
    }
}

fn map_cancel_workflow_error(err: CancelWorkflowError) -> ApiError {
    match err {
        CancelWorkflowError::WorkflowNotFound(id) => {
            ApiError::not_found(format!("workflow '{id}' not found"))
        }
        CancelWorkflowError::AlreadyTerminal { status } => ApiError::conflict(format!(
            "workflow is already terminal (status '{status}'); cannot cancel"
        )),
        CancelWorkflowError::DropTrackingIssue { source } => {
            error!(error = %source, "failed to drop tracking issue while cancelling workflow");
            ApiError::internal(format!("failed to drop tracking issue: {source}"))
        }
        CancelWorkflowError::Store { source } => map_store_error(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_store_error_workflow_not_found_returns_404() {
        let id = WorkflowId::new();
        let err = StoreError::WorkflowNotFound(id.clone());
        let api_err = map_store_error(err);
        assert_eq!(api_err.status().as_u16(), 404);
        assert!(api_err.message().contains(&id.to_string()));
    }

    #[test]
    fn map_store_error_internal_returns_500() {
        let err = StoreError::Internal("db broke".to_string());
        let api_err = map_store_error(err);
        assert_eq!(api_err.status().as_u16(), 500);
        assert!(api_err.message().contains("db broke"));
    }

    #[test]
    fn map_create_workflow_template_not_found_returns_404() {
        let err = CreateWorkflowError::TemplateNotFound {
            path: "/workflows/missing.yaml".to_string(),
        };
        let api_err = map_create_workflow_error(err);
        assert_eq!(api_err.status().as_u16(), 404);
        assert!(api_err.message().contains("missing.yaml"));
    }

    #[test]
    fn map_transition_workflow_terminal_returns_409() {
        let err = TransitionWorkflowError::WorkflowTerminal {
            status: WorkflowStatus::Completed,
        };
        let api_err = map_transition_workflow_error(err);
        assert_eq!(api_err.status().as_u16(), 409);
    }

    #[test]
    fn map_transition_workflow_transition_not_found_returns_404() {
        let err = TransitionWorkflowError::TransitionNotFound {
            state: "develop".to_string(),
            transition_id: "bogus".to_string(),
        };
        let api_err = map_transition_workflow_error(err);
        assert_eq!(api_err.status().as_u16(), 404);
    }

    #[test]
    fn map_cancel_workflow_already_terminal_returns_409() {
        let err = CancelWorkflowError::AlreadyTerminal {
            status: WorkflowStatus::Cancelled,
        };
        let api_err = map_cancel_workflow_error(err);
        assert_eq!(api_err.status().as_u16(), 409);
    }
}
