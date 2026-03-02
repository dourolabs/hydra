use crate::{
    app::{AppState, NotificationError},
    domain::actors::Actor,
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

/// Optional query parameters for the mark-all-read endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct MarkAllReadQuery {
    pub before: Option<DateTime<Utc>>,
}

const DEFAULT_LIMIT: u32 = 50;

/// GET /v1/notifications — list notifications for the authenticated actor.
pub async fn list_notifications(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<ListNotificationsQuery>,
) -> Result<Json<ListNotificationsResponse>, ApiError> {
    info!(actor = %actor.name(), "list_notifications invoked");

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);

    // Request one extra row to determine if more results exist beyond this page.
    let mut fetch_query = query.clone();
    fetch_query.limit = Some(limit.saturating_add(1));

    let results = state
        .list_notifications(&actor.actor_id, &fetch_query)
        .await
        .map_err(map_notification_error)?;

    let has_more = results.len() > limit as usize;

    let notifications: Vec<NotificationResponse> = results
        .into_iter()
        .take(limit as usize)
        .map(|(id, notif)| NotificationResponse::new(id, notif.into()))
        .collect();

    info!(
        actor = %actor.name(),
        count = notifications.len(),
        has_more = has_more,
        "list_notifications completed"
    );

    Ok(Json(ListNotificationsResponse::new(
        notifications,
        has_more,
    )))
}

/// GET /v1/notifications/unread-count — count unread notifications for the authenticated actor.
pub async fn unread_count(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
) -> Result<Json<UnreadCountResponse>, ApiError> {
    info!(actor = %actor.name(), "unread_count invoked");

    let count = state
        .count_unread_notifications(&actor.actor_id)
        .await
        .map_err(map_notification_error)?;

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

    state
        .mark_notification_read(&actor.actor_id, &notification_id)
        .await
        .map_err(map_notification_error)?;

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
        .mark_all_notifications_read(&actor.actor_id, query.before)
        .await
        .map_err(map_notification_error)?;

    info!(
        actor = %actor.name(),
        marked = marked,
        "mark_all_read completed"
    );

    Ok(Json(MarkReadResponse::new(marked)))
}

fn map_notification_error(err: NotificationError) -> ApiError {
    match err {
        NotificationError::NotFound { notification_id } => {
            ApiError::not_found(format!("notification '{notification_id}' not found"))
        }
        NotificationError::Store { source } => {
            error!(error = %source, "notification store operation failed");
            ApiError::internal(format!("notification store error: {source}"))
        }
    }
}
