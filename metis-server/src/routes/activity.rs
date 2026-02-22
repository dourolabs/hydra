use crate::app::AppState;
use crate::store::ReadOnlyStore;
use axum::{Json, extract::{Query, State}};
use metis_common::api::v1::{ApiError, activity};
use tracing::info;

pub async fn get_activity(
    State(state): State<AppState>,
    Query(query): Query<activity::SearchActivityQuery>,
) -> Result<Json<activity::ActivityFeedResponse>, ApiError> {
    info!(
        limit = ?query.limit,
        entity_types = ?query.entity_types,
        actor = ?query.actor,
        has_cursor = query.cursor.is_some(),
        "get_activity invoked"
    );

    let store = state.store.as_ref();
    let response = store
        .get_activity_feed(&query)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "activity feed error");
            ApiError::internal(anyhow::anyhow!("activity feed error: {err}"))
        })?;

    info!(
        events = response.events.len(),
        has_next = response.next_cursor.is_some(),
        "get_activity completed"
    );
    Ok(Json(response))
}
