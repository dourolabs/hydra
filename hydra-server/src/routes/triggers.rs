//! HTTP routes for `/v1/triggers`.
//!
//! Standard CRUD plus version history, mirroring `routes/issues.rs`. The
//! routes call `Trigger::validate()` from the domain layer before write
//! and return `400` with a structured body on validation failure.
//!
//! See `/designs/triggered-actions.md` §4.3 / §4.5 / §7 PR 5.

use crate::{
    app::AppState,
    domain::actors::{Actor, ActorRef},
    domain::triggers::{TriggerValidation, ValidationError, ValidationWarning},
    store::{ReadOnlyStore, StoreError},
};
use anyhow::anyhow;
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use hydra_common::{
    RepoName, TriggerId, Versioned,
    api::v1::{
        ApiError,
        repositories::SearchRepositoriesQuery,
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
    Extension(_actor): Extension<Actor>,
    Json(payload): Json<UpsertTriggerRequest>,
) -> Result<Json<UpsertTriggerResponse>, ApiError> {
    info!("create_trigger invoked");
    let trigger = trigger_from_request(payload, None);
    let known = load_known_repos(&state).await?;
    let warnings = trigger.validate(&known).map_err(validation_error)?;
    log_warnings(None, &warnings);

    let store = state.store.inner().clone();
    let (trigger_id, version) = store
        .add_trigger(trigger, &ActorRef::from(&_actor))
        .await
        .map_err(|err| map_trigger_error(err, None))?;
    info!(trigger_id = %trigger_id, version, "create_trigger completed");
    Ok(Json(UpsertTriggerResponse::new(trigger_id, version)))
}

pub async fn update_trigger(
    State(state): State<AppState>,
    Extension(_actor): Extension<Actor>,
    TriggerIdPath(trigger_id): TriggerIdPath,
    Json(payload): Json<UpsertTriggerRequest>,
) -> Result<Json<UpsertTriggerResponse>, ApiError> {
    info!(trigger_id = %trigger_id, "update_trigger invoked");
    let trigger = trigger_from_request(payload, None);
    let known = load_known_repos(&state).await?;
    let warnings = trigger.validate(&known).map_err(validation_error)?;
    log_warnings(Some(&trigger_id), &warnings);

    let store = state.store.inner().clone();
    let version = store
        .update_trigger(&trigger_id, trigger, &ActorRef::from(&_actor))
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
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
        .store
        .get_trigger(&trigger_id, include_deleted)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    Ok(Json(to_record(&trigger_id, versioned)))
}

pub async fn list_triggers(
    State(state): State<AppState>,
    Query(query): Query<SearchTriggersQuery>,
) -> Result<Json<ListTriggersResponse>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    info!(include_deleted, "list_triggers invoked");
    let rows = state
        .store
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
    // We do not have a dedicated `get_trigger_versions` method (PR 4
    // exposed only the latest-row read). For v1 we return the latest
    // version as a single-entry list; full version history can be added
    // as a follow-up when the store method lands. See §7 PR 5.
    let versioned = state
        .store
        .get_trigger(&trigger_id, true)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let record = to_record(&trigger_id, versioned);
    Ok(Json(ListTriggerVersionsResponse::new(vec![record])))
}

pub async fn get_trigger_version(
    State(state): State<AppState>,
    TriggerVersionPath {
        trigger_id,
        version: raw_version,
    }: TriggerVersionPath,
) -> Result<Json<TriggerVersionRecord>, ApiError> {
    info!(trigger_id = %trigger_id, raw_version = raw_version.as_i64(), "get_trigger_version invoked");
    let versioned = state
        .store
        .get_trigger(&trigger_id, true)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let resolved = super::resolve_version(
        raw_version,
        versioned.version,
        "trigger",
        trigger_id.as_ref(),
    )?;
    if resolved != versioned.version {
        return Err(ApiError::not_found(format!(
            "trigger '{trigger_id}' version {resolved} not found"
        )));
    }
    Ok(Json(to_record(&trigger_id, versioned)))
}

pub async fn delete_trigger(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    TriggerIdPath(trigger_id): TriggerIdPath,
) -> Result<Json<TriggerVersionRecord>, ApiError> {
    info!(trigger_id = %trigger_id, "delete_trigger invoked");
    let store = state.store.inner().clone();
    store
        .delete_trigger(&trigger_id, &ActorRef::from(&actor))
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    let versioned = state
        .store
        .get_trigger(&trigger_id, true)
        .await
        .map_err(|err| map_trigger_error(err, Some(&trigger_id)))?;
    Ok(Json(to_record(&trigger_id, versioned)))
}

// ---- Helpers --------------------------------------------------------

fn trigger_from_request(
    request: UpsertTriggerRequest,
    last_fired_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Trigger {
    Trigger::new(
        request.enabled,
        request.schedule,
        request.actions,
        request.creator,
        last_fired_at,
        false,
    )
}

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

async fn load_known_repos(state: &AppState) -> Result<Vec<RepoName>, ApiError> {
    let query = SearchRepositoriesQuery::default();
    let rows = state.store.list_repositories(&query).await.map_err(|err| {
        error!(error = %err, "failed to list repositories for trigger validation");
        ApiError::internal(anyhow!("failed to list repositories: {err}"))
    })?;
    Ok(rows.into_iter().map(|(name, _)| name).collect())
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
