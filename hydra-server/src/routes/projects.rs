//! HTTP routes for `/v1/projects`.
//!
//! Exposes project CRUD plus the per-project status set (the wire shape
//! every issue's `resolved_status` is computed against). Auth + error
//! mapping follow the existing `/v1/labels` pattern.
//!
//! Every per-project URL accepts either a [`ProjectId`] (`j-…`) or a
//! [`ProjectKey`](hydra_common::api::v1::projects::ProjectKey) (slug),
//! discriminated at the path-extractor by [`ProjectRef`]. Resolution
//! happens at the route boundary via [`resolve_project_ref`] — domain
//! and store calls below this layer continue to address projects
//! exclusively by `ProjectId`.

use crate::app::AppState;
use crate::domain::actors::{Actor, ActorRef};
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
        ListProjectsResponse, ProjectRecord, ProjectRef, ProjectStatusesResponse,
        ProjectValidationError, RenameStatusRequest, UpsertProjectRequest, UpsertProjectResponse,
    },
};
use tracing::{error, info};

/// Resolve a path-segment [`ProjectRef`] to the concrete [`ProjectId`]
/// the rest of the handler operates on. The id branch is a byte-level
/// passthrough; the key branch hits the partial unique index
/// `projects_key_unique_active_idx` via `get_project_by_key`.
///
/// `"key not found"` is mapped to a `404` whose body quotes the missing
/// key — matching the existing id-shape 404 surface so callers see one
/// consistent error contract regardless of which form they passed.
async fn resolve_project_ref(
    store: &dyn ReadOnlyStore,
    project_ref: &ProjectRef,
) -> Result<ProjectId, ApiError> {
    match project_ref {
        ProjectRef::Id(id) => Ok(id.clone()),
        ProjectRef::Key(key) => {
            let resolved = store.get_project_by_key(key, false).await.map_err(|err| {
                error!(
                    project_key = %key,
                    error = %err,
                    "project key lookup failed"
                );
                ApiError::internal(anyhow!("project store error: {err}"))
            })?;
            match resolved {
                Some((project_id, _)) => Ok(project_id),
                None => Err(ApiError::not_found(format!("project '{key}' not found"))),
            }
        }
    }
}

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

/// GET /v1/projects/:project_ref — fetch a single project.
pub async fn get_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
) -> Result<Json<ProjectRecord>, ApiError> {
    let store: &dyn ReadOnlyStore = state.store.as_ref();
    let project_id = resolve_project_ref(store, &project_ref).await?;
    info!(actor = %actor.name(), project_id = %project_id, "get_project invoked");

    let versioned = store
        .get_project(&project_id, false)
        .await
        .map_err(|e| map_project_not_found(e, &project_id))?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        version = versioned.version,
        "get_project completed"
    );

    Ok(Json(ProjectRecord::new(
        project_id,
        versioned.version,
        versioned.item,
    )))
}

/// PUT /v1/projects/:project_ref — full-replace update (version-bumping).
pub async fn update_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
    Json(payload): Json<UpsertProjectRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(actor = %actor.name(), project_id = %project_id, "update_project invoked");
    let project = payload.project;
    project.validate().map_err(map_validation_error)?;

    let actor_ref = ActorRef::from(&actor);
    let version = state
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

/// DELETE /v1/projects/:project_ref — soft-delete a project.
pub async fn delete_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(actor = %actor.name(), project_id = %project_id, "delete_project invoked");

    let actor_ref = ActorRef::from(&actor);
    let version = state
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

/// POST /v1/projects/:project_ref/statuses/rename — rename a status key in place.
///
/// Preserves the status's `(project_id, sequence)` identity, so any
/// issues referencing the old key continue to resolve through the same
/// sequence and read back as the new key. Returns the bumped project
/// version (matches `update_project`'s shape).
pub async fn rename_project_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
    Json(payload): Json<RenameStatusRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(
        actor = %actor.name(),
        project_id = %project_id,
        from = %payload.from,
        to = %payload.to,
        "rename_project_status invoked"
    );

    let actor_ref = ActorRef::from(&actor);
    let version = state
        .rename_status(&project_id, &payload.from, &payload.to, &actor_ref)
        .await
        .map_err(|e| match e {
            StoreError::InvalidIssueStatus(msg) => ApiError::bad_request(msg),
            other => map_project_not_found(other, &project_id),
        })?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        version,
        "rename_project_status completed"
    );
    Ok(Json(UpsertProjectResponse::new(project_id, version)))
}

/// GET /v1/projects/:project_ref/statuses — return the project's status list.
pub async fn get_project_statuses(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
) -> Result<Json<ProjectStatusesResponse>, ApiError> {
    let store: &dyn ReadOnlyStore = state.store.as_ref();
    let project_id = resolve_project_ref(store, &project_ref).await?;
    info!(actor = %actor.name(), project_id = %project_id, "get_project_statuses invoked");

    let versioned = store
        .get_project(&project_id, false)
        .await
        .map_err(|e| map_project_not_found(e, &project_id))?;

    let response = ProjectStatusesResponse::new(versioned.item.statuses.clone());

    info!(
        actor = %actor.name(),
        project_id = %project_id,
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
