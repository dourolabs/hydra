use crate::app::{AppState, CreateLabelError, UpdateLabelError};
use crate::domain::actors::Actor;
use crate::store::StoreError;
use axum::{Extension, Json, extract::Path, extract::Query, extract::State};
use metis_common::{
    LabelId, MetisId,
    api::v1::{
        ApiError,
        labels::{
            LabelRecord, ListLabelsResponse, SearchLabelsQuery, UpsertLabelRequest,
            UpsertLabelResponse,
        },
    },
    issues::IssueId,
};
use serde::Deserialize;
use tracing::{error, info};

#[derive(Debug, Deserialize, Default)]
pub struct CascadeQuery {
    #[serde(default)]
    pub cascade: Option<bool>,
}

/// POST /v1/labels — create a new label.
pub async fn create_label(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<UpsertLabelRequest>,
) -> Result<Json<UpsertLabelResponse>, ApiError> {
    info!(actor = %actor.name(), "create_label invoked");

    let label_id = state
        .create_label(payload.label.name, payload.label.color)
        .await
        .map_err(map_create_label_error)?;

    info!(actor = %actor.name(), label_id = %label_id, "create_label completed");

    Ok(Json(UpsertLabelResponse::new(label_id)))
}

/// GET /v1/labels — list labels.
pub async fn list_labels(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<SearchLabelsQuery>,
) -> Result<Json<ListLabelsResponse>, ApiError> {
    info!(actor = %actor.name(), "list_labels invoked");

    let labels = state.list_labels(&query).await.map_err(map_store_error)?;

    let records: Vec<LabelRecord> = labels
        .into_iter()
        .map(|(label_id, label)| {
            LabelRecord::new(
                label_id,
                label.name,
                label.color,
                label.created_at,
                label.updated_at,
            )
        })
        .collect();

    info!(actor = %actor.name(), count = records.len(), "list_labels completed");

    Ok(Json(ListLabelsResponse::new(records)))
}

/// GET /v1/labels/:label_id — get a single label.
pub async fn get_label(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(label_id): Path<LabelId>,
) -> Result<Json<LabelRecord>, ApiError> {
    info!(actor = %actor.name(), label_id = %label_id, "get_label invoked");

    let label = state
        .get_label(&label_id)
        .await
        .map_err(|e| map_label_not_found(e, &label_id))?;

    info!(actor = %actor.name(), label_id = %label_id, "get_label completed");

    Ok(Json(LabelRecord::new(
        label_id,
        label.name,
        label.color,
        label.created_at,
        label.updated_at,
    )))
}

/// PUT /v1/labels/:label_id — update a label.
pub async fn update_label(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(label_id): Path<LabelId>,
    Json(payload): Json<UpsertLabelRequest>,
) -> Result<Json<LabelRecord>, ApiError> {
    info!(actor = %actor.name(), label_id = %label_id, "update_label invoked");

    state
        .update_label(&label_id, payload.label.name, payload.label.color)
        .await
        .map_err(map_update_label_error)?;

    let label = state
        .get_label(&label_id)
        .await
        .map_err(|e| map_label_not_found(e, &label_id))?;

    info!(actor = %actor.name(), label_id = %label_id, "update_label completed");

    Ok(Json(LabelRecord::new(
        label_id,
        label.name,
        label.color,
        label.created_at,
        label.updated_at,
    )))
}

/// DELETE /v1/labels/:label_id — soft-delete a label.
pub async fn delete_label(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(label_id): Path<LabelId>,
) -> Result<Json<()>, ApiError> {
    info!(actor = %actor.name(), label_id = %label_id, "delete_label invoked");

    state
        .delete_label(&label_id)
        .await
        .map_err(|e| map_label_not_found(e, &label_id))?;

    info!(actor = %actor.name(), label_id = %label_id, "delete_label completed");

    Ok(Json(()))
}

/// PUT /v1/labels/:label_id/objects/:object_id — associate a label with an object.
/// When cascade=true and object_id is an issue, the label is also added to all
/// transitive children of that issue.
pub async fn add_label_association(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((label_id, object_id)): Path<(LabelId, MetisId)>,
    Query(query): Query<CascadeQuery>,
) -> Result<Json<()>, ApiError> {
    let cascade = query.cascade.unwrap_or(false);
    info!(
        actor = %actor.name(),
        label_id = %label_id,
        object_id = %object_id,
        cascade = cascade,
        "add_label_association invoked"
    );

    state
        .add_label_association(&label_id, &object_id)
        .await
        .map_err(map_store_error)?;

    if cascade {
        let issue_id = IssueId::try_from(object_id.clone()).map_err(|_| {
            ApiError::bad_request("cascade=true is only supported for issue objects")
        })?;
        state
            .cascade_label_to_children(&label_id, &issue_id)
            .await
            .map_err(map_store_error)?;
    }

    info!(
        actor = %actor.name(),
        label_id = %label_id,
        object_id = %object_id,
        cascade = cascade,
        "add_label_association completed"
    );

    Ok(Json(()))
}

/// DELETE /v1/labels/:label_id/objects/:object_id — remove a label association.
pub async fn remove_label_association(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path((label_id, object_id)): Path<(LabelId, MetisId)>,
) -> Result<Json<()>, ApiError> {
    info!(
        actor = %actor.name(),
        label_id = %label_id,
        object_id = %object_id,
        "remove_label_association invoked"
    );

    state
        .remove_label_association(&label_id, &object_id)
        .await
        .map_err(map_store_error)?;

    info!(
        actor = %actor.name(),
        label_id = %label_id,
        object_id = %object_id,
        "remove_label_association completed"
    );

    Ok(Json(()))
}

fn map_create_label_error(err: CreateLabelError) -> ApiError {
    match err {
        CreateLabelError::EmptyName => ApiError::bad_request("label name must not be empty"),
        CreateLabelError::AlreadyExists(name) => {
            ApiError::bad_request(format!("a label named '{name}' already exists"))
        }
        CreateLabelError::Store { source } => {
            error!(error = %source, "label store operation failed");
            ApiError::internal(format!("label store error: {source}"))
        }
    }
}

fn map_update_label_error(err: UpdateLabelError) -> ApiError {
    match err {
        UpdateLabelError::NotFound(id) => ApiError::not_found(format!("label '{id}' not found")),
        UpdateLabelError::EmptyName => ApiError::bad_request("label name must not be empty"),
        UpdateLabelError::AlreadyExists(name) => {
            ApiError::bad_request(format!("a label named '{name}' already exists"))
        }
        UpdateLabelError::Store { source } => {
            error!(error = %source, "label store operation failed");
            ApiError::internal(format!("label store error: {source}"))
        }
    }
}

fn map_label_not_found(err: StoreError, label_id: &LabelId) -> ApiError {
    match err {
        StoreError::LabelNotFound(_) => {
            ApiError::not_found(format!("label '{label_id}' not found"))
        }
        other => {
            error!(
                label_id = %label_id,
                error = %other,
                "label store operation failed"
            );
            ApiError::internal(format!("label store error: {other}"))
        }
    }
}

fn map_store_error(err: StoreError) -> ApiError {
    error!(error = %err, "label store operation failed");
    ApiError::internal(format!("label store error: {err}"))
}
