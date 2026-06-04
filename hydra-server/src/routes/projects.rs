//! HTTP routes for `/v1/projects`.
//!
//! See `/designs/per-project-issue-statuses.md` §4 "Storage" and §7 PR 3.
//! Auth + error mapping follow the existing `/v1/labels` pattern.

use crate::app::AppState;
use crate::domain::actors::{Actor, ActorRef};
use crate::domain::projects::{DEFAULT_PROJECT_KEY, default_project};
use crate::store::{ReadOnlyStore, StoreError};
use anyhow::anyhow;
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use hydra_common::ProjectId;
use hydra_common::api::v1::{
    ApiError,
    projects::{
        ListProjectsResponse, ProjectRecord, ProjectStatusesResponse, ProjectValidationError,
        UpsertProjectRequest, UpsertProjectResponse,
    },
};
use tracing::{error, info};

/// POST /v1/projects — create a new project.
pub async fn create_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<UpsertProjectRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    info!(actor = %actor.name(), "create_project invoked");
    let project = payload.project;
    project.validate().map_err(map_validation_error)?;

    let actor_ref = ActorRef::from(&actor);
    let (project_id, version) = state
        .store
        .add_project(project, &actor_ref)
        .await
        .map_err(map_store_error)?;

    info!(actor = %actor.name(), project_id = %project_id, "create_project completed");
    Ok(Json(UpsertProjectResponse::new(project_id, version)))
}

/// GET /v1/projects — list non-deleted projects.
pub async fn list_projects(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
) -> Result<Json<ListProjectsResponse>, ApiError> {
    info!(actor = %actor.name(), "list_projects invoked");

    let store: &dyn ReadOnlyStore = state.store.as_ref();
    let entries = store.list_projects(false).await.map_err(map_store_error)?;

    let projects = entries
        .into_iter()
        .map(|(project_id, versioned)| {
            ProjectRecord::new(project_id, versioned.version, versioned.item)
        })
        .collect::<Vec<_>>();

    info!(
        actor = %actor.name(),
        count = projects.len(),
        "list_projects completed"
    );
    Ok(Json(ListProjectsResponse::new(projects)))
}

/// GET /v1/projects/:project_id — fetch a single project. The literal
/// path `default` returns the synthesized [`default_project`] without
/// hitting the store.
pub async fn get_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectRecord>, ApiError> {
    info!(actor = %actor.name(), project_id = %project_id, "get_project invoked");

    if project_id == DEFAULT_PROJECT_KEY {
        // The default project is synthesized in-process and never stored,
        // so we surface a sentinel `project_id` that round-trips through
        // the wire as the same `default` slug. Callers that want a real
        // ProjectId should use the project list endpoint instead.
        return Err(ApiError::bad_request(
            "the default project is read-only; use GET /v1/projects/default/statuses to fetch its status list",
        ));
    }

    let project_id_typed = ProjectId::try_from(project_id.clone())
        .map_err(|e| ApiError::bad_request(format!("invalid project id '{project_id}': {e}")))?;

    let store: &dyn ReadOnlyStore = state.store.as_ref();
    let versioned = store
        .get_project(&project_id_typed, false)
        .await
        .map_err(|e| map_project_not_found(e, &project_id_typed))?;

    info!(
        actor = %actor.name(),
        project_id = %project_id_typed,
        version = versioned.version,
        "get_project completed"
    );

    Ok(Json(ProjectRecord::new(
        project_id_typed,
        versioned.version,
        versioned.item,
    )))
}

/// PUT /v1/projects/:project_id — full-replace update (version-bumping).
pub async fn update_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_id): Path<ProjectId>,
    Json(payload): Json<UpsertProjectRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    info!(actor = %actor.name(), project_id = %project_id, "update_project invoked");
    let project = payload.project;
    project.validate().map_err(map_validation_error)?;

    let actor_ref = ActorRef::from(&actor);
    let version = state
        .store
        .update_project(&project_id, project, &actor_ref)
        .await
        .map_err(|e| match e {
            StoreError::ProjectKeyExists(key) => {
                ApiError::bad_request(format!("a project with key '{key}' already exists"))
            }
            other => map_project_not_found(other, &project_id),
        })?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        version,
        "update_project completed"
    );
    Ok(Json(UpsertProjectResponse::new(project_id, version)))
}

/// DELETE /v1/projects/:project_id — soft-delete a project.
pub async fn delete_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_id): Path<ProjectId>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    info!(actor = %actor.name(), project_id = %project_id, "delete_project invoked");

    let actor_ref = ActorRef::from(&actor);
    let version = state
        .store
        .delete_project(&project_id, &actor_ref)
        .await
        .map_err(|e| map_project_not_found(e, &project_id))?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        version,
        "delete_project completed"
    );
    Ok(Json(UpsertProjectResponse::new(project_id, version)))
}

/// GET /v1/projects/:project_id/statuses — return the project's status list.
pub async fn get_project_statuses(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_id): Path<String>,
) -> Result<Json<ProjectStatusesResponse>, ApiError> {
    info!(actor = %actor.name(), project_id = %project_id, "get_project_statuses invoked");

    if project_id == DEFAULT_PROJECT_KEY {
        let proj = default_project();
        return Ok(Json(ProjectStatusesResponse::new(
            proj.statuses.clone(),
            proj.default_status_key.as_str().to_string(),
        )));
    }

    let project_id_typed = ProjectId::try_from(project_id.clone())
        .map_err(|e| ApiError::bad_request(format!("invalid project id '{project_id}': {e}")))?;

    let store: &dyn ReadOnlyStore = state.store.as_ref();
    let versioned = store
        .get_project(&project_id_typed, false)
        .await
        .map_err(|e| map_project_not_found(e, &project_id_typed))?;

    let response = ProjectStatusesResponse::new(
        versioned.item.statuses.clone(),
        versioned.item.default_status_key.as_str().to_string(),
    );

    info!(
        actor = %actor.name(),
        project_id = %project_id_typed,
        count = response.statuses.len(),
        "get_project_statuses completed"
    );
    Ok(Json(response))
}

fn map_validation_error(err: ProjectValidationError) -> ApiError {
    ApiError::bad_request(err.to_string())
}

fn map_project_not_found(err: StoreError, project_id: &ProjectId) -> ApiError {
    match err {
        StoreError::ProjectNotFound(_) => {
            ApiError::not_found(format!("project '{project_id}' not found"))
        }
        other => {
            error!(
                project_id = %project_id,
                error = %other,
                "project store operation failed"
            );
            ApiError::internal(anyhow!("project store error: {other}"))
        }
    }
}

fn map_store_error(err: StoreError) -> ApiError {
    match err {
        StoreError::ProjectKeyExists(key) => {
            ApiError::bad_request(format!("a project with key '{key}' already exists"))
        }
        other => {
            error!(error = %other, "project store operation failed");
            ApiError::internal(anyhow!("project store error: {other}"))
        }
    }
}
