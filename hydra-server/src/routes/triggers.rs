//! HTTP routes for `/v1/triggers`.
//!
//! Standard CRUD plus version history, mirroring `routes/issues.rs`.
//! Validation and writes are delegated to `AppState`; this module is
//! responsible only for request extraction, response shaping, and error
//! mapping.

use crate::{
    app::{AppState, UpsertTriggerError},
    domain::actors::{Actor, ActorRef},
    domain::triggers::{ValidationError, ValidationWarning},
    store::{ReadOnlyStore, StoreError},
};
use anyhow::anyhow;
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use hydra_common::{
    TriggerId, Versioned,
    api::v1::{
        ApiError,
        triggers::{
            ListTriggerVersionsResponse, ListTriggersResponse, SearchTriggersQuery, Trigger,
            TriggerVersionRecord, UpsertTriggerRequest, UpsertTriggerResponse,
        },
    },
};
use serde_json::json;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct TriggerIdPath(pub TriggerId);

#[async_trait]
impl<S> FromRequestParts<S> for TriggerIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(trigger_id) = Path::<TriggerId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;
        Ok(Self(trigger_id))
    }
}

#[derive(Debug, Clone)]
pub struct TriggerVersionPath {
    pub trigger_id: TriggerId,
    pub version: super::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for TriggerVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((trigger_id, version)) =
            Path::<(TriggerId, super::RelativeVersionNumber)>::from_request_parts(parts, state)
                .await
                .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;
        Ok(Self {
            trigger_id,
            version,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct GetTriggerQuery {
    #[serde(default)]
    pub include_deleted: Option<bool>,
}

pub async fn create_trigger(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<UpsertTriggerRequest>,
) -> Result<Json<UpsertTriggerResponse>, ApiError> {
    info!("create_trigger invoked");
    let (trigger_id, version, warnings) = state
        .create_trigger(payload, &ActorRef::from(&actor))
        .await
        .map_err(map_upsert_error)?;
    log_warnings(None, &warnings);
    info!(trigger_id = %trigger_id, version, "create_trigger completed");
    Ok(Json(UpsertTriggerResponse::new(trigger_id, version)))
}

pub async fn update_trigger(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    TriggerIdPath(trigger_id): TriggerIdPath,
    Json(payload): Json<UpsertTriggerRequest>,
) -> Result<Json<UpsertTriggerResponse>, ApiError> {
    info!(trigger_id = %trigger_id, "update_trigger invoked");
    let (version, warnings) = state
        .update_trigger(&trigger_id, payload, &ActorRef::from(&actor))
        .await
        .map_err(|err| match err {
            UpsertTriggerError::Store {
                source: StoreError::TriggerNotFound(_),
            } => map_trigger_error(
                StoreError::TriggerNotFound(trigger_id.clone()),
                Some(&trigger_id),
            ),
            other => map_upsert_error(other),
        })?;
    log_warnings(Some(&trigger_id), &warnings);
    info!(trigger_id = %trigger_id, version, "update_trigger completed");
    Ok(Json(UpsertTriggerResponse::new(trigger_id, version)))
}

pub async fn get_trigger(
    State(state): State<AppState>,
    TriggerIdPath(trigger_id): TriggerIdPath,
    Query(query): Query<GetTriggerQuery>,
) -> Result<Json<TriggerVersionRecord>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    info!(trigger_id = %trigger_id, include_deleted, "get_trigger invoked");
    let versioned = state
        .store_with_events()
        .get_trigger(&trigger_id, include_deleted)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    info!(trigger_id = %trigger_id, "get_trigger completed");
    Ok(Json(to_record(&trigger_id, versioned)))
}

pub async fn list_triggers(
    State(state): State<AppState>,
    Query(query): Query<SearchTriggersQuery>,
) -> Result<Json<ListTriggersResponse>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    info!(include_deleted, "list_triggers invoked");
    let rows = state
        .store_with_events()
        .list_triggers(include_deleted)
        .await
        .map_err(|err| map_trigger_error(err, None))?;
    let records: Vec<TriggerVersionRecord> = rows
        .into_iter()
        .map(|(id, versioned)| to_record(&id, versioned))
        .collect();
    info!(returned = records.len(), "list_triggers completed");
    Ok(Json(ListTriggersResponse::new(records)))
}

pub async fn list_trigger_versions(
    State(state): State<AppState>,
    TriggerIdPath(trigger_id): TriggerIdPath,
) -> Result<Json<ListTriggerVersionsResponse>, ApiError> {
    info!(trigger_id = %trigger_id, "list_trigger_versions invoked");
    let versions = state
        .store_with_events()
        .get_trigger_versions(&trigger_id)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let records: Vec<TriggerVersionRecord> = versions
        .into_iter()
        .map(|versioned| to_record(&trigger_id, versioned))
        .collect();
    info!(
        trigger_id = %trigger_id,
        returned = records.len(),
        "list_trigger_versions completed"
    );
    Ok(Json(ListTriggerVersionsResponse::new(records)))
}

pub async fn get_trigger_version(
    State(state): State<AppState>,
    TriggerVersionPath {
        trigger_id,
        version: raw_version,
    }: TriggerVersionPath,
) -> Result<Json<TriggerVersionRecord>, ApiError> {
    info!(trigger_id = %trigger_id, raw_version = raw_version.as_i64(), "get_trigger_version invoked");
    let versions = state
        .store_with_events()
        .get_trigger_versions(&trigger_id)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let latest_version = versions
        .last()
        .map(|v| v.version)
        .ok_or_else(|| ApiError::not_found(format!("trigger '{trigger_id}' not found")))?;
    let resolved =
        super::resolve_version(raw_version, latest_version, "trigger", trigger_id.as_ref())?;
    let versioned = versions
        .into_iter()
        .find(|v| v.version == resolved)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "trigger '{trigger_id}' version {resolved} not found"
            ))
        })?;
    info!(trigger_id = %trigger_id, version = resolved, "get_trigger_version completed");
    Ok(Json(to_record(&trigger_id, versioned)))
}

pub async fn delete_trigger(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    TriggerIdPath(trigger_id): TriggerIdPath,
) -> Result<Json<TriggerVersionRecord>, ApiError> {
    info!(trigger_id = %trigger_id, "delete_trigger invoked");
    let store = state.store_with_events();
    store
        .delete_trigger(&trigger_id, &ActorRef::from(&actor))
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let versioned = store
        .get_trigger(&trigger_id, true)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    info!(trigger_id = %trigger_id, "delete_trigger completed");
    Ok(Json(to_record(&trigger_id, versioned)))
}

// ---- Helpers --------------------------------------------------------

fn to_record(trigger_id: &TriggerId, versioned: Versioned<Trigger>) -> TriggerVersionRecord {
    TriggerVersionRecord::new(
        trigger_id.clone(),
        versioned.version,
        versioned.timestamp,
        versioned.item,
        versioned.actor,
        versioned.creation_time,
    )
}

fn log_warnings(trigger_id: Option<&TriggerId>, warnings: &[ValidationWarning]) {
    for warning in warnings {
        match warning {
            ValidationWarning::PastOnce { at } => {
                warn!(
                    trigger_id = ?trigger_id.map(ToString::to_string),
                    at = %at,
                    "Once trigger scheduled in the past will not fire"
                );
            }
        }
    }
}

fn map_upsert_error(err: UpsertTriggerError) -> ApiError {
    match err {
        UpsertTriggerError::Validation(detail) => validation_error(detail),
        UpsertTriggerError::UnknownRepo(repo_name) => {
            let body = json!({
                "code": "unknown_repo",
                "message": format!("repository '{repo_name}' is not registered"),
            });
            ApiError::bad_request(body.to_string())
        }
        UpsertTriggerError::Store { source } => map_trigger_error(source, None),
    }
}

fn validation_error(err: ValidationError) -> ApiError {
    // Surface a structured body so CLI/UI can render targeted feedback.
    let body = json!({
        "code": "validation_failed",
        "message": err.to_string(),
    });
    ApiError::bad_request(body.to_string())
}

fn map_trigger_error(err: StoreError, trigger_id: Option<&TriggerId>) -> ApiError {
    match err {
        StoreError::TriggerNotFound(id) => {
            error!(trigger_id = %id, "trigger not found");
            ApiError::not_found(format!("trigger '{id}' not found"))
        }
        other => {
            let id = trigger_id.map(ToString::to_string).unwrap_or_default();
            error!(trigger_id = %id, error = %other, "trigger store operation failed");
            ApiError::internal(anyhow!("trigger store error: {other}"))
        }
    }
}
