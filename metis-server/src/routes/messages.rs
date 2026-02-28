use crate::{
    app::{AppState, MessageError},
    domain::actors::{Actor, ActorRef, parse_actor_name},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::Query, extract::State};
use metis_common::api::v1::{
    ApiError,
    messages::{
        self as api_messages, ListMessagesResponse, ReceiveMessagesQuery, SearchMessagesQuery,
        SendMessageRequest, SendMessageResponse, VersionedMessage,
    },
};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::app::event_bus::ServerEvent;

const DEFAULT_LIMIT: u32 = 50;
const DEFAULT_RECEIVE_TIMEOUT_SECS: u32 = 30;
const MAX_RECEIVE_TIMEOUT_SECS: u32 = 120;

/// POST /v1/messages — send a message to a recipient.
pub async fn send_message(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, ApiError> {
    info!(actor = %actor.name(), "send_message invoked");

    let sender_id = actor.actor_id.clone();
    let actor_ref = ActorRef::from(&actor);

    let (message_id, version, versioned) = state
        .send_message(
            &sender_id,
            &payload.recipient,
            payload.body,
            payload.is_read,
            actor_ref,
        )
        .await
        .map_err(map_message_error)?;

    info!(message_id = %message_id, "send_message completed");

    let api_message: api_messages::Message = versioned.item.into();

    Ok(Json(SendMessageResponse::new(
        message_id,
        version,
        api_message,
        versioned.timestamp,
    )))
}

/// GET /v1/messages — list messages (authenticated, any actor may query any messages).
///
/// Accepts query params: sender, recipient, after (timestamp), before (timestamp),
/// include_deleted, limit. Returns messages in reverse chronological order (newest first).
pub async fn list_messages(
    State(state): State<AppState>,
    Extension(_actor): Extension<Actor>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!("list_messages invoked");

    let mut store_query = SearchMessagesQuery::default();
    store_query.sender = query.sender;
    store_query.recipient = query.recipient;
    store_query.after = query.after;
    store_query.before = query.before;
    store_query.include_deleted = query.include_deleted;
    store_query.is_read = query.is_read;
    store_query.limit = Some(query.limit.unwrap_or(DEFAULT_LIMIT));

    let results = state
        .list_messages(&store_query)
        .await
        .map_err(map_message_error)?;

    let messages: Vec<VersionedMessage> = results
        .into_iter()
        .map(|(id, v)| {
            VersionedMessage::new(
                id,
                v.version,
                v.timestamp,
                v.item.into(),
                v.actor,
                v.creation_time,
            )
        })
        .collect();

    info!(count = messages.len(), "list_messages completed");

    Ok(Json(ListMessagesResponse::new(messages)))
}

/// GET /v1/messages/receive — receive unread messages for the current actor.
///
/// Fetches all unread messages where the current authenticated actor is the recipient,
/// returns the original unread versions, then marks them as read. If no unread messages
/// exist, long-polls until a new message arrives (up to the specified timeout).
/// Optionally filters by sender.
pub async fn receive_messages(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<ReceiveMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!(actor = %actor.name(), "receive_messages invoked");

    let actor_name = actor.name();
    let timeout_secs = query
        .timeout
        .unwrap_or(DEFAULT_RECEIVE_TIMEOUT_SECS)
        .min(MAX_RECEIVE_TIMEOUT_SECS);
    let timeout_duration = Duration::from_secs(timeout_secs as u64);

    let sender_filter = query
        .sender
        .as_deref()
        .map(|s| {
            parse_actor_name(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid sender actor name: {s}")))
        })
        .transpose()?;

    // Subscribe FIRST (before checking store) to avoid missing events
    let mut receiver = state.subscribe();

    // Check for existing unread messages addressed to the current actor
    let mut store_query = SearchMessagesQuery::default();
    store_query.recipient = Some(actor_name.clone());
    store_query.sender = sender_filter.as_ref().map(|s| s.to_string());
    store_query.limit = Some(DEFAULT_LIMIT);

    let results = state
        .list_messages(&store_query)
        .await
        .map_err(map_message_error)?;

    // Filter for unread messages
    let unread: Vec<_> = results
        .into_iter()
        .filter(|(_, v)| !v.item.is_read)
        .collect();

    if !unread.is_empty() {
        // Mark all unread messages as read
        let actor_ref = ActorRef::from(&actor);
        for (id, _) in &unread {
            state
                .mark_message_read(id, actor_ref.clone())
                .await
                .map_err(map_message_error)?;
        }

        // Return the original unread versions of the messages
        let mut messages: Vec<VersionedMessage> = unread
            .into_iter()
            .map(|(id, v)| {
                let msg: api_messages::Message = v.item.into();
                VersionedMessage::new(id, v.version, v.timestamp, msg, v.actor, v.creation_time)
            })
            .collect();

        // Sort ascending by timestamp (oldest first)
        messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

        info!(
            actor = %actor.name(),
            count = messages.len(),
            "receive_messages returning existing unread messages"
        );
        return Ok(Json(ListMessagesResponse::new(messages)));
    }

    // No unread messages — wait for new ones via event bus
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            info!(actor = %actor.name(), "receive_messages timed out");
            return Ok(Json(ListMessagesResponse::new(vec![])));
        }

        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(event)) => {
                if let ServerEvent::MessageCreated {
                    message_id,
                    recipient,
                    sender,
                    ..
                } = &event
                {
                    // Must be addressed to the current actor
                    if recipient.to_string() != actor_name {
                        continue;
                    }

                    // Check sender filter
                    if let Some(ref sf) = sender_filter {
                        if sender.as_ref() != Some(sf) {
                            continue;
                        }
                    }

                    // Found a matching message — fetch it from the app layer
                    let msg = state
                        .get_message(message_id)
                        .await
                        .map_err(map_message_error)?;

                    // Mark as read
                    let actor_ref = ActorRef::from(&actor);
                    if !msg.item.is_read {
                        state
                            .mark_message_read(message_id, actor_ref)
                            .await
                            .map_err(map_message_error)?;
                    }

                    let api_msg: api_messages::Message = msg.item.into();
                    let versioned = VersionedMessage::new(
                        message_id.clone(),
                        msg.version,
                        msg.timestamp,
                        api_msg,
                        msg.actor,
                        msg.creation_time,
                    );

                    info!(
                        actor = %actor.name(),
                        message_id = %message_id,
                        "receive_messages returning new message"
                    );
                    return Ok(Json(ListMessagesResponse::new(vec![versioned])));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                info!(actor = %actor.name(), "receive_messages event bus closed");
                return Ok(Json(ListMessagesResponse::new(vec![])));
            }
            Err(_) => {
                // Timeout
                info!(actor = %actor.name(), "receive_messages timed out");
                return Ok(Json(ListMessagesResponse::new(vec![])));
            }
        }
    }
}

fn map_message_error(err: MessageError) -> ApiError {
    match err {
        MessageError::RecipientNotFound { actor_name } => {
            error!(recipient = %actor_name, "recipient not found");
            ApiError::not_found(format!("recipient '{actor_name}' not found"))
        }
        MessageError::NotFound { message_id } => {
            error!(message_id = %message_id, "message not found");
            ApiError::not_found(format!("message '{message_id}' not found"))
        }
        MessageError::Store { source } => {
            error!(error = %source, "message store operation failed");
            ApiError::internal(anyhow!("message store error: {source}"))
        }
    }
}
