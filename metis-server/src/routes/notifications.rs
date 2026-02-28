use crate::{
    app::AppState,
    domain::actors::Actor,
    store::{ReadOnlyStore, StoreError},
};
use axum::{Extension, Json, extract::Path, extract::Query, extract::State};
use chrono::{DateTime, Utc};
use metis_common::{
    NotificationId,
    api::v1::{
        ApiError,
        notifications::{
            ListNotificationsQuery, ListNotificationsResponse, MarkReadResponse,
            NotificationResponse, UnreadCountResponse,
        },
    },
};
use serde::Deserialize;
use tracing::{error, info};

const DEFAULT_LIMIT: u32 = 50;

/// Optional query parameters for the mark-all-read endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct MarkAllReadQuery {
    pub before: Option<DateTime<Utc>>,
}

/// GET /v1/notifications — list notifications for the authenticated actor.
pub async fn list_notifications(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<ListNotificationsQuery>,
) -> Result<Json<ListNotificationsResponse>, ApiError> {
    info!(actor = %actor.name(), "list_notifications invoked");

    let mut store_query = ListNotificationsQuery::default();
    store_query.recipient = Some(actor.actor_id.to_string());
    store_query.is_read = query.is_read;
    store_query.before = query.before;
    store_query.after = query.after;
    store_query.limit = Some(query.limit.unwrap_or(DEFAULT_LIMIT));

    let results = state
        .store
        .list_notifications(&store_query)
        .await
        .map_err(map_store_error)?;

    let notifications: Vec<NotificationResponse> = results
        .into_iter()
        .map(|(id, notif)| NotificationResponse::new(id, notif.into()))
        .collect();

    info!(
        actor = %actor.name(),
        count = notifications.len(),
        "list_notifications completed"
    );

    Ok(Json(ListNotificationsResponse::new(notifications)))
}

/// GET /v1/notifications/unread-count — count unread notifications for the authenticated actor.
pub async fn unread_count(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
) -> Result<Json<UnreadCountResponse>, ApiError> {
    info!(actor = %actor.name(), "unread_count invoked");

    let count = state
        .store
        .count_unread_notifications(&actor.actor_id)
        .await
        .map_err(map_store_error)?;

    info!(
        actor = %actor.name(),
        count = count,
        "unread_count completed"
    );

    Ok(Json(UnreadCountResponse::new(count)))
}

/// POST /v1/notifications/:notification_id/read — mark a single notification as read.
pub async fn mark_read(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Path(notification_id): Path<NotificationId>,
) -> Result<Json<MarkReadResponse>, ApiError> {
    info!(
        actor = %actor.name(),
        notification_id = %notification_id,
        "mark_read invoked"
    );

    // Verify the notification exists and belongs to the authenticated actor.
    let notification = state
        .store
        .get_notification(&notification_id)
        .await
        .map_err(|err| match err {
            StoreError::NotificationNotFound(_) => {
                ApiError::not_found(format!("notification '{notification_id}' not found"))
            }
            other => map_store_error(other),
        })?;

    if notification.recipient != actor.actor_id {
        return Err(ApiError::not_found(format!(
            "notification '{notification_id}' not found"
        )));
    }

    state
        .store
        .mark_notification_read(&notification_id)
        .await
        .map_err(|err| match err {
            StoreError::NotificationNotFound(id) => {
                error!(notification_id = %id, "notification not found");
                ApiError::not_found(format!("notification '{id}' not found"))
            }
            other => map_store_error(other),
        })?;

    info!(
        actor = %actor.name(),
        notification_id = %notification_id,
        "mark_read completed"
    );

    Ok(Json(MarkReadResponse::new(1)))
}

/// POST /v1/notifications/read-all — mark all notifications as read for the authenticated actor.
pub async fn mark_all_read(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<MarkAllReadQuery>,
) -> Result<Json<MarkReadResponse>, ApiError> {
    info!(actor = %actor.name(), "mark_all_read invoked");

    let marked = state
        .store
        .mark_all_notifications_read(&actor.actor_id, query.before)
        .await
        .map_err(map_store_error)?;

    info!(
        actor = %actor.name(),
        marked = marked,
        "mark_all_read completed"
    );

    Ok(Json(MarkReadResponse::new(marked)))
}

fn map_store_error(err: StoreError) -> ApiError {
    error!(error = %err, "notification store operation failed");
    ApiError::internal(format!("notification store error: {err}"))
}
