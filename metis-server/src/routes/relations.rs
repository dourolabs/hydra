use crate::app::AppState;
use crate::domain::actors::{Actor, ActorRef};
use crate::store::{ObjectRelationship, ReadOnlyStore, RelationshipType, StoreError};
use anyhow::anyhow;
use axum::{
    Extension, Json, extract::Query, extract::State, http::StatusCode, response::IntoResponse,
};
use metis_common::{
    MetisId,
    api::v1::{
        ApiError,
        relations::{
            CreateRelationRequest, ListRelationsRequest, ListRelationsResponse, RelationResponse,
            RemoveRelationRequest, RemoveRelationResponse,
        },
    },
};
use tracing::{error, info};

/// Maximum number of IDs allowed in a batch query (source_ids or target_ids).
const MAX_BATCH_IDS: usize = 100;

/// Convert a store `ObjectRelationship` to the wire `RelationResponse`.
fn to_response(rel: &ObjectRelationship) -> RelationResponse {
    RelationResponse {
        source_id: rel.source_id.to_string(),
        target_id: rel.target_id.to_string(),
        rel_type: rel.rel_type.as_str().to_string(),
    }
}

/// Parse a comma-separated string into a Vec of MetisId.
fn parse_id_list(raw: &str) -> Result<Vec<MetisId>, ApiError> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<MetisId>()
                .map_err(|e| ApiError::bad_request(format!("invalid ID '{s}': {e}")))
        })
        .collect()
}

/// Parse an optional rel_type string into a RelationshipType.
fn parse_rel_type(s: &str) -> Result<RelationshipType, ApiError> {
    s.parse::<RelationshipType>().map_err(ApiError::bad_request)
}

/// GET /v1/relations/ — query relations with filters.
pub async fn list_relations(
    State(state): State<AppState>,
    Query(query): Query<ListRelationsRequest>,
) -> Result<Json<ListRelationsResponse>, ApiError> {
    info!("list_relations invoked");

    // --- Validation ---

    // source_id and source_ids are mutually exclusive
    if query.source_id.is_some() && query.source_ids.is_some() {
        return Err(ApiError::bad_request(
            "source_id and source_ids are mutually exclusive",
        ));
    }

    // target_id and target_ids are mutually exclusive
    if query.target_id.is_some() && query.target_ids.is_some() {
        return Err(ApiError::bad_request(
            "target_id and target_ids are mutually exclusive",
        ));
    }

    // object_id cannot combine with source_id/source_ids/target_id/target_ids
    if query.object_id.is_some()
        && (query.source_id.is_some()
            || query.source_ids.is_some()
            || query.target_id.is_some()
            || query.target_ids.is_some())
    {
        return Err(ApiError::bad_request(
            "object_id cannot be combined with source_id, source_ids, target_id, or target_ids",
        ));
    }

    let transitive = query.transitive.unwrap_or(false);

    // transitive=true requires exactly one of source_id/target_id + rel_type
    if transitive {
        let has_source = query.source_id.is_some();
        let has_target = query.target_id.is_some();
        if query.rel_type.is_none() {
            return Err(ApiError::bad_request(
                "transitive=true requires rel_type to be specified",
            ));
        }
        if !(has_source ^ has_target) {
            return Err(ApiError::bad_request(
                "transitive=true requires exactly one of source_id or target_id",
            ));
        }
    }

    let rel_type = query.rel_type.as_deref().map(parse_rel_type).transpose()?;

    let store = state.store.as_ref();

    // --- Dispatch to the appropriate store method ---

    let relations: Vec<ObjectRelationship>;

    if let Some(ref object_id_str) = query.object_id {
        // object_id mode: query as source and as target, merge results
        let object_id: MetisId = object_id_str
            .parse()
            .map_err(|e| ApiError::bad_request(format!("invalid object_id: {e}")))?;

        let (as_source, as_target) = tokio::try_join!(
            store.get_relationships(Some(&object_id), None, rel_type),
            store.get_relationships(None, Some(&object_id), rel_type),
        )
        .map_err(map_store_error)?;

        // Merge and deduplicate
        let mut merged = as_source;
        for rel in as_target {
            if !merged.iter().any(|r| {
                r.source_id == rel.source_id
                    && r.target_id == rel.target_id
                    && r.rel_type == rel.rel_type
            }) {
                merged.push(rel);
            }
        }
        relations = merged;
    } else if query.source_ids.is_some() || query.target_ids.is_some() {
        // Batch mode
        let source_ids = query.source_ids.as_deref().map(parse_id_list).transpose()?;
        let target_ids = query.target_ids.as_deref().map(parse_id_list).transpose()?;

        if let Some(ref ids) = source_ids {
            if ids.len() > MAX_BATCH_IDS {
                return Err(ApiError::bad_request(format!(
                    "source_ids exceeds maximum of {MAX_BATCH_IDS} IDs"
                )));
            }
        }
        if let Some(ref ids) = target_ids {
            if ids.len() > MAX_BATCH_IDS {
                return Err(ApiError::bad_request(format!(
                    "target_ids exceeds maximum of {MAX_BATCH_IDS} IDs"
                )));
            }
        }

        relations = store
            .get_relationships_batch(source_ids.as_deref(), target_ids.as_deref(), rel_type)
            .await
            .map_err(map_store_error)?;
    } else if transitive {
        // Transitive mode
        let source_id = query
            .source_id
            .as_deref()
            .map(|s| s.parse::<MetisId>())
            .transpose()
            .map_err(|e| ApiError::bad_request(format!("invalid source_id: {e}")))?;
        let target_id = query
            .target_id
            .as_deref()
            .map(|s| s.parse::<MetisId>())
            .transpose()
            .map_err(|e| ApiError::bad_request(format!("invalid target_id: {e}")))?;

        relations = store
            .get_relationships_transitive(
                source_id.as_ref(),
                target_id.as_ref(),
                rel_type.expect("validated above"),
            )
            .await
            .map_err(map_store_error)?;
    } else {
        // Simple filter mode
        let source_id = query
            .source_id
            .as_deref()
            .map(|s| s.parse::<MetisId>())
            .transpose()
            .map_err(|e| ApiError::bad_request(format!("invalid source_id: {e}")))?;
        let target_id = query
            .target_id
            .as_deref()
            .map(|s| s.parse::<MetisId>())
            .transpose()
            .map_err(|e| ApiError::bad_request(format!("invalid target_id: {e}")))?;

        relations = store
            .get_relationships(source_id.as_ref(), target_id.as_ref(), rel_type)
            .await
            .map_err(map_store_error)?;
    }

    let response = ListRelationsResponse {
        relations: relations.iter().map(to_response).collect(),
    };

    info!(
        returned = response.relations.len(),
        "list_relations completed"
    );
    Ok(Json(response))
}

/// POST /v1/relations/ — create a relation.
pub async fn create_relation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<CreateRelationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    info!(actor = %actor.name(), "create_relation invoked");

    let source_id: MetisId = payload
        .source_id
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid source_id: {e}")))?;
    let target_id: MetisId = payload
        .target_id
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid target_id: {e}")))?;
    let rel_type = parse_rel_type(&payload.rel_type)?;

    let was_created = state
        .store
        .add_relationship_with_actor(&source_id, &target_id, rel_type, ActorRef::from(&actor))
        .await
        .map_err(map_store_error)?;

    let response_body = RelationResponse {
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        rel_type: rel_type.as_str().to_string(),
    };

    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    info!(actor = %actor.name(), was_created, "create_relation completed");
    Ok((status, Json(response_body)))
}

/// DELETE /v1/relations/ — remove a relation.
pub async fn remove_relation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<RemoveRelationRequest>,
) -> Result<Json<RemoveRelationResponse>, ApiError> {
    info!(actor = %actor.name(), "remove_relation invoked");

    let source_id: MetisId = payload
        .source_id
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid source_id: {e}")))?;
    let target_id: MetisId = payload
        .target_id
        .parse()
        .map_err(|e| ApiError::bad_request(format!("invalid target_id: {e}")))?;
    let rel_type = parse_rel_type(&payload.rel_type)?;

    let removed = state
        .store
        .remove_relationship_with_actor(&source_id, &target_id, rel_type, ActorRef::from(&actor))
        .await
        .map_err(map_store_error)?;

    info!(actor = %actor.name(), removed, "remove_relation completed");
    Ok(Json(RemoveRelationResponse { removed }))
}

fn map_store_error(err: StoreError) -> ApiError {
    match err {
        StoreError::IssueNotFound(id) => ApiError::bad_request(format!("issue '{id}' not found")),
        StoreError::PatchNotFound(id) => ApiError::bad_request(format!("patch '{id}' not found")),
        StoreError::DocumentNotFound(id) => {
            ApiError::bad_request(format!("document '{id}' not found"))
        }
        other => {
            error!(error = %other, "relation store operation failed");
            ApiError::internal(anyhow!("relation store error: {other}"))
        }
    }
}
