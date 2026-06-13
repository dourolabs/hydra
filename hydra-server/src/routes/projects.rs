//! HTTP routes for `/v1/projects`.
//!
//! Exposes project CRUD plus per-status CRUD (the wire shape every
//! issue's `resolved_status` is computed against). Auth + error
//! mapping follow the existing `/v1/labels` pattern.
//!
//! Every per-project URL accepts either a [`ProjectId`] (`j-…`) or a
//! [`ProjectKey`](hydra_common::api::v1::projects::ProjectKey) (slug),
//! discriminated at the path-extractor by [`ProjectRef`]. Resolution
//! happens at the route boundary via [`resolve_project_ref`] — domain
//! and store calls below this layer continue to address projects
//! exclusively by `ProjectId`.
//!
//! `POST /v1/projects` and `PUT /v1/projects/:project_ref` carry only
//! project-level fields. Per-status add / update / delete lives at
//! `POST/PUT/DELETE /v1/projects/:project_ref/statuses[/:status_key]`.

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
        ListProjectsResponse, Project, ProjectRecord, ProjectRef, ProjectStatusesResponse,
        StatusDefinition, StatusKey, UpsertProjectRequest, UpsertProjectResponse,
        UpsertProjectStatusResponse,
    },
    users::Username,
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

/// Build the domain [`Project`] from a project-level request body and
/// the request actor. Post-cutover the wire `UpsertProjectRequest`
/// no longer carries statuses, so the handler synthesizes an empty
/// `statuses: Vec<_>` on the domain object — the store layer ignores
/// it on writes anyway, and the read-side rebuilds it from the
/// `statuses` table.
fn project_from_upsert(payload: UpsertProjectRequest, creator: Username) -> Project {
    let mut project = Project::new(
        payload.key,
        payload.name,
        Vec::new(),
        creator,
        false,
        payload.priority,
    );
    project.prompt_path = payload.prompt_path;
    project
}

/// Resolve the actor's username so `add_project` can stamp it as the
/// project's `creator`. Falls back to the actor's display name for
/// non-user actors — matches the existing CLI pattern for system /
/// agent actors that operate on projects (e.g. seeded data).
fn creator_for_actor(actor: &Actor) -> Username {
    Username::from(actor.name().to_string())
}

/// POST /v1/projects — create a new project.
pub async fn create_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<UpsertProjectRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    info!(actor = %actor.name(), "create_project invoked");
    let creator = creator_for_actor(&actor);
    let project = project_from_upsert(payload, creator);

    let actor_ref = ActorRef::from(&actor);
    let (project_id, version) = state
        .add_project(project, &actor_ref)
        .await
        .map_err(map_store_error)?;

    info!(actor = %actor.name(), project_id = %project_id, "create_project completed");
    Ok(Json(UpsertProjectResponse::new(project_id, version)))
}

/// GET /v1/projects — list non-archived projects.
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

/// PUT /v1/projects/:project_ref — update project-level fields.
pub async fn update_project(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
    Json(payload): Json<UpsertProjectRequest>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(actor = %actor.name(), project_id = %project_id, "update_project invoked");

    // Preserve the existing creator: `update_project` is a full
    // project-level rewrite from the store's POV, so the route must
    // carry the existing creator through unchanged. The
    // `UpsertProjectRequest` wire shape doesn't carry it, so we read
    // the current project's creator and stamp it on the domain object.
    let current = state
        .store
        .get_project(&project_id, false)
        .await
        .map_err(|e| map_project_not_found(e, &project_id))?;
    let project = project_from_upsert(payload, current.item.creator.clone());

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

/// POST /v1/projects/:project_ref/statuses — add a new status.
pub async fn create_project_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(project_ref): Path<ProjectRef>,
    Json(status): Json<StatusDefinition>,
) -> Result<Json<UpsertProjectStatusResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %status.key,
        "create_project_status invoked"
    );

    if let Some(on_enter) = status.on_enter.as_ref() {
        on_enter.validate().map_err(ApiError::bad_request)?;
    }

    let actor_ref = ActorRef::from(&actor);
    let (persisted, version) = state
        .add_status(&project_id, status, &actor_ref)
        .await
        .map_err(|e| match e {
            StoreError::InvalidIssueStatus(msg) => ApiError::bad_request(msg),
            other => map_project_not_found(other, &project_id),
        })?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %persisted.key,
        version,
        "create_project_status completed"
    );
    Ok(Json(UpsertProjectStatusResponse::new(
        project_id, version, persisted,
    )))
}

/// PUT /v1/projects/:project_ref/statuses/:status_key — update an
/// existing status. A body whose `key` differs from `status_key` is a
/// rename: the row's `(project_id, sequence)` storage identity is
/// preserved so existing issues continue to resolve through the same
/// sequence and read back as the new key.
pub async fn update_project_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((project_ref, status_key)): Path<(ProjectRef, StatusKey)>,
    Json(status): Json<StatusDefinition>,
) -> Result<Json<UpsertProjectStatusResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %status_key,
        new_key = %status.key,
        "update_project_status invoked"
    );

    if let Some(on_enter) = status.on_enter.as_ref() {
        on_enter.validate().map_err(ApiError::bad_request)?;
    }

    let actor_ref = ActorRef::from(&actor);
    let (persisted, version) = state
        .update_status(&project_id, &status_key, status, &actor_ref)
        .await
        .map_err(|e| match e {
            StoreError::InvalidIssueStatus(msg) => ApiError::bad_request(msg),
            other => map_project_not_found(other, &project_id),
        })?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %persisted.key,
        version,
        "update_project_status completed"
    );
    Ok(Json(UpsertProjectStatusResponse::new(
        project_id, version, persisted,
    )))
}

/// DELETE /v1/projects/:project_ref/statuses/:status_key — remove a
/// status. The DB FK on `issues_v2.status_sequence` is the
/// authoritative guard; a still-referenced row surfaces as
/// `400 InvalidIssueStatus`.
pub async fn delete_project_status(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((project_ref, status_key)): Path<(ProjectRef, StatusKey)>,
) -> Result<Json<UpsertProjectResponse>, ApiError> {
    let project_id = resolve_project_ref(state.store.as_ref(), &project_ref).await?;
    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %status_key,
        "delete_project_status invoked"
    );

    let actor_ref = ActorRef::from(&actor);
    let version = state
        .delete_status(&project_id, &status_key, &actor_ref)
        .await
        .map_err(|e| match e {
            StoreError::InvalidIssueStatus(msg) => ApiError::bad_request(msg),
            other => map_project_not_found(other, &project_id),
        })?;

    info!(
        actor = %actor.name(),
        project_id = %project_id,
        status_key = %status_key,
        version,
        "delete_project_status completed"
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
