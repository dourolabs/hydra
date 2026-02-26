use crate::{
    app::{AppState, SendMessageError},
    domain::actors::{Actor, ActorRef, parse_actor_name},
    store::{ReadOnlyStore, StoreError},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::Query, extract::State};
use metis_common::api::v1::{
    ApiError,
    messages::{
        self as api_messages, ListMessagesResponse, SearchMessagesQuery, SendMessageRequest,
        SendMessageResponse, VersionedMessage, WaitMessagesQuery,
    },
};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info};

use crate::app::event_bus::ServerEvent;

const DEFAULT_LIMIT: u32 = 50;
const DEFAULT_WAIT_TIMEOUT_SECS: u32 = 30;
const MAX_WAIT_TIMEOUT_SECS: u32 = 120;

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
        .map_err(map_send_message_error)?;

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
/// include_deleted, limit, mark_as_read. Returns messages in reverse chronological order
/// (newest first). When `mark_as_read=true`, all returned unread messages are marked as read;
/// this requires `recipient` to match the current authenticated actor.
pub async fn list_messages(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<SearchMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!("list_messages invoked");

    let mark_as_read = query.mark_as_read.unwrap_or(false);

    if mark_as_read {
        let actor_name = actor.name();
        match &query.recipient {
            Some(r) if r == &actor_name => {}
            _ => {
                return Err(ApiError::bad_request(
                    "mark_as_read requires recipient to match the current authenticated actor"
                        .to_string(),
                ));
            }
        }
    }

    let mut store_query = SearchMessagesQuery::default();
    store_query.sender = query.sender;
    store_query.recipient = query.recipient;
    store_query.after = query.after;
    store_query.before = query.before;
    store_query.include_deleted = query.include_deleted;
    store_query.limit = Some(query.limit.unwrap_or(DEFAULT_LIMIT));

    let results = state
        .store
        .list_messages(&store_query)
        .await
        .map_err(map_store_error)?;

    if mark_as_read {
        let actor_ref = ActorRef::from(&actor);
        for (id, v) in &results {
            if !v.item.is_read {
                let mut updated = v.item.clone();
                updated.is_read = true;
                state
                    .store
                    .update_message_with_actor(id, updated, actor_ref.clone())
                    .await
                    .map_err(map_store_error)?;
            }
        }
    }

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

/// GET /v1/messages/wait — long-poll for the next message.
pub async fn wait_for_message(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<WaitMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!(actor = %actor.name(), "wait_for_message invoked");

    let actor_id = actor.actor_id.clone();
    let actor_name = actor_id.to_string();
    let timeout_secs = query
        .timeout
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS)
        .min(MAX_WAIT_TIMEOUT_SECS);
    let timeout_duration = Duration::from_secs(timeout_secs as u64);

    let sender_filter = query
        .sender
        .as_deref()
        .map(|s| {
            parse_actor_name(s)
                .ok_or_else(|| ApiError::bad_request(format!("invalid sender actor name: {s}")))
        })
        .transpose()?;

    let recipient_filter = query
        .recipient
        .as_deref()
        .map(|r| {
            parse_actor_name(r)
                .ok_or_else(|| ApiError::bad_request(format!("invalid recipient actor name: {r}")))
        })
        .transpose()?;

    // Subscribe FIRST (before checking store) to avoid missing events
    let mut receiver = state.subscribe();

    // Check for existing messages after the cursor
    let existing = check_existing_messages_after_cursor(
        &state,
        &actor_id,
        sender_filter.as_ref(),
        recipient_filter.as_ref(),
        query.after.as_deref(),
    )
    .await?;

    if !existing.is_empty() {
        info!(
            actor = %actor.name(),
            count = existing.len(),
            "wait_for_message returning existing messages"
        );
        return Ok(Json(ListMessagesResponse::new(existing)));
    }

    // No existing messages — wait for new ones via event bus
    let deadline = tokio::time::Instant::now() + timeout_duration;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            info!(actor = %actor.name(), "wait_for_message timed out");
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
                    // Check if this event involves the authenticated actor
                    let involves_actor = recipient.to_string() == actor_name
                        || sender.as_ref().map(|s| s.to_string()) == Some(actor_name.clone());

                    if !involves_actor {
                        continue;
                    }

                    // Check sender filter
                    if let Some(ref sf) = sender_filter {
                        if sender.as_ref() != Some(sf) {
                            continue;
                        }
                    }

                    // Check recipient filter
                    if let Some(ref rf) = recipient_filter {
                        if *recipient != *rf {
                            continue;
                        }
                    }

                    // Found a matching message — return it
                    let msg = state
                        .store
                        .get_message(message_id)
                        .await
                        .map_err(map_store_error)?;

                    let versioned = VersionedMessage::new(
                        message_id.clone(),
                        msg.version,
                        msg.timestamp,
                        msg.item.into(),
                        msg.actor,
                        msg.creation_time,
                    );

                    info!(
                        actor = %actor.name(),
                        message_id = %message_id,
                        "wait_for_message returning new message"
                    );
                    return Ok(Json(ListMessagesResponse::new(vec![versioned])));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                info!(actor = %actor.name(), "wait_for_message event bus closed");
                return Ok(Json(ListMessagesResponse::new(vec![])));
            }
            Err(_) => {
                // Timeout
                info!(actor = %actor.name(), "wait_for_message timed out");
                return Ok(Json(ListMessagesResponse::new(vec![])));
            }
        }
    }
}

/// Check if there are messages after the given cursor in the relevant conversations.
///
/// Returns messages in ascending order (oldest new message first) for the wait endpoint.
async fn check_existing_messages_after_cursor(
    state: &AppState,
    actor_id: &crate::domain::actors::ActorId,
    sender_filter: Option<&crate::domain::actors::ActorId>,
    recipient_filter: Option<&crate::domain::actors::ActorId>,
    after_cursor: Option<&str>,
) -> Result<Vec<VersionedMessage>, ApiError> {
    // If no cursor, no need to check for existing messages
    let Some(after_str) = after_cursor else {
        return Ok(vec![]);
    };

    let after_id: metis_common::MessageId = after_str
        .parse()
        .map_err(|_| ApiError::bad_request(format!("invalid after cursor: {after_str}")))?;

    // Fetch the cursor message to get its timestamp
    let cursor_msg = state
        .store
        .get_message(&after_id)
        .await
        .map_err(map_store_error)?;
    let after_timestamp = cursor_msg.timestamp;

    let actor_name = actor_id.to_string();

    // Build a query for messages after the cursor timestamp involving this actor
    // We check both as sender and as recipient
    let mut new_messages = Vec::new();

    // Messages where actor is recipient
    let mut query_as_recipient = SearchMessagesQuery::default();
    query_as_recipient.sender = sender_filter.map(|s| s.to_string());
    query_as_recipient.recipient = Some(
        recipient_filter
            .map(|r| r.to_string())
            .unwrap_or_else(|| actor_name.clone()),
    );
    query_as_recipient.after = Some(after_timestamp);
    query_as_recipient.limit = Some(DEFAULT_LIMIT);
    let results = state
        .store
        .list_messages(&query_as_recipient)
        .await
        .map_err(map_store_error)?;
    for (id, v) in results {
        if id != after_id {
            new_messages.push(VersionedMessage::new(
                id,
                v.version,
                v.timestamp,
                v.item.into(),
                v.actor,
                v.creation_time,
            ));
        }
    }

    // Also check messages where actor is sender (unless sender filter is specified)
    if sender_filter.is_none() {
        let mut query_as_sender = SearchMessagesQuery::default();
        query_as_sender.sender = Some(actor_name);
        query_as_sender.recipient = recipient_filter.map(|r| r.to_string());
        query_as_sender.after = Some(after_timestamp);
        query_as_sender.limit = Some(DEFAULT_LIMIT);
        let results = state
            .store
            .list_messages(&query_as_sender)
            .await
            .map_err(map_store_error)?;
        for (id, v) in results {
            if id != after_id && !new_messages.iter().any(|m| m.message_id == id) {
                new_messages.push(VersionedMessage::new(
                    id,
                    v.version,
                    v.timestamp,
                    v.item.into(),
                    v.actor,
                    v.creation_time,
                ));
            }
        }
    }

    // Sort ascending by timestamp (oldest first) for the wait response
    new_messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    Ok(new_messages)
}

fn map_send_message_error(err: SendMessageError) -> ApiError {
    match err {
        SendMessageError::RecipientNotFound { actor_name } => {
            error!(recipient = %actor_name, "recipient not found");
            ApiError::not_found(format!("recipient '{actor_name}' not found"))
        }
        SendMessageError::Store { source } => {
            error!(error = %source, "message store operation failed");
            ApiError::internal(anyhow!("message store error: {source}"))
        }
    }
}

fn map_store_error(err: StoreError) -> ApiError {
    match err {
        StoreError::MessageNotFound(id) => {
            error!(message_id = %id, "message not found");
            ApiError::not_found(format!("message '{id}' not found"))
        }
        other => {
            error!(error = %other, "message store operation failed");
            ApiError::internal(anyhow!("message store error: {other}"))
        }
    }
}
