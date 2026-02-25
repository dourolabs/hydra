use crate::{
    app::{AppState, SendMessageError},
    domain::actors::{Actor, ActorRef, parse_actor_name},
    domain::messages::ConversationId,
    store::{ReadOnlyStore, StoreError},
};
use anyhow::anyhow;
use axum::{Extension, Json, extract::Query, extract::State};
use metis_common::api::v1::{
    ApiError,
    messages::{
        self as api_messages, ListMessagesQuery, ListMessagesResponse, SendMessageRequest,
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
        .send_message(&sender_id, &payload.recipient, payload.body, actor_ref)
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

/// GET /v1/messages — list messages for the authenticated actor.
pub async fn list_messages(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<ListMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!(actor = %actor.name(), "list_messages invoked");

    let sender_id = actor.actor_id.clone();
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    let before = query.before.as_deref();

    let messages = if let Some(ref participant) = query.participant {
        // Filter to a specific conversation partner
        let participant_id = parse_actor_name(participant).ok_or_else(|| {
            ApiError::bad_request(format!("invalid participant actor name: {participant}"))
        })?;
        let conversation_id = ConversationId::from_pair(&sender_id, &participant_id);

        let before_id = before
            .map(|s| {
                s.parse()
                    .map_err(|_| ApiError::bad_request(format!("invalid before cursor: {s}")))
            })
            .transpose()?;

        let results = state
            .store
            .list_messages(conversation_id.as_str(), before_id.as_ref(), limit)
            .await
            .map_err(map_store_error)?;

        results
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
            .collect()
    } else {
        // List messages across all conversations for the authenticated actor
        let conversations = state
            .store
            .list_conversations(&sender_id)
            .await
            .map_err(map_store_error)?;

        let before_id = before
            .map(|s| {
                s.parse()
                    .map_err(|_| ApiError::bad_request(format!("invalid before cursor: {s}")))
            })
            .transpose()?;

        let mut all_messages: Vec<VersionedMessage> = Vec::new();
        for convo_id in conversations {
            let results = state
                .store
                .list_messages(&convo_id, before_id.as_ref(), limit)
                .await
                .map_err(map_store_error)?;

            for (id, v) in results {
                all_messages.push(VersionedMessage::new(
                    id,
                    v.version,
                    v.timestamp,
                    v.item.into(),
                    v.actor,
                    v.creation_time,
                ));
            }
        }

        // Sort by timestamp descending (most recent first)
        all_messages.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        all_messages.truncate(limit as usize);
        all_messages
    };

    info!(
        actor = %actor.name(),
        count = messages.len(),
        "list_messages completed"
    );

    Ok(Json(ListMessagesResponse::new(messages)))
}

/// GET /v1/messages/wait — long-poll for the next message.
pub async fn wait_for_message(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Query(query): Query<WaitMessagesQuery>,
) -> Result<Json<ListMessagesResponse>, ApiError> {
    info!(actor = %actor.name(), "wait_for_message invoked");

    let sender_id = actor.actor_id.clone();
    let sender_name = sender_id.to_string();
    let timeout_secs = query
        .timeout
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECS)
        .min(MAX_WAIT_TIMEOUT_SECS);
    let timeout_duration = Duration::from_secs(timeout_secs as u64);

    let participant_id = query
        .participant
        .as_deref()
        .map(|p| {
            parse_actor_name(p).ok_or_else(|| {
                ApiError::bad_request(format!("invalid participant actor name: {p}"))
            })
        })
        .transpose()?;

    // Subscribe FIRST (before checking store) to avoid missing events
    let mut receiver = state.subscribe();

    // Check for existing messages after the cursor
    let existing = check_existing_messages_after_cursor(
        &state,
        &sender_id,
        participant_id.as_ref(),
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
                    conversation_id,
                    ..
                } = &event
                {
                    // Check if this event is for a conversation involving the authenticated actor
                    if !conversation_id.split('+').any(|seg| seg == sender_name) {
                        continue;
                    }

                    // Check participant filter
                    if let Some(ref pid) = participant_id {
                        let expected_convo = ConversationId::from_pair(&sender_id, pid);
                        if *conversation_id != expected_convo.to_string() {
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
    sender_id: &crate::domain::actors::ActorId,
    participant_id: Option<&crate::domain::actors::ActorId>,
    after_cursor: Option<&str>,
) -> Result<Vec<VersionedMessage>, ApiError> {
    // If no cursor, no need to check for existing messages
    let Some(after_str) = after_cursor else {
        return Ok(vec![]);
    };

    let after_id: metis_common::MessageId = after_str
        .parse()
        .map_err(|_| ApiError::bad_request(format!("invalid after cursor: {after_str}")))?;

    let conversation_ids = if let Some(pid) = participant_id {
        vec![ConversationId::from_pair(sender_id, pid).to_string()]
    } else {
        state
            .store
            .list_conversations(sender_id)
            .await
            .map_err(map_store_error)?
    };

    let mut new_messages = Vec::new();

    for convo_id in &conversation_ids {
        // Get recent messages (newest first)
        let messages = state
            .store
            .list_messages(convo_id, None, DEFAULT_LIMIT)
            .await
            .map_err(map_store_error)?;

        // Find the cursor position — everything before it in the list is newer
        let mut found_cursor = false;
        for (id, v) in &messages {
            if *id == after_id {
                found_cursor = true;
                break;
            }
            new_messages.push(VersionedMessage::new(
                id.clone(),
                v.version,
                v.timestamp,
                v.item.clone().into(),
                v.actor.clone(),
                v.creation_time,
            ));
        }

        // If cursor not found in the list, all messages are newer
        if !found_cursor {
            // Re-add all messages (we already added them in the loop above and broke early)
            // Actually, if cursor was never found, we already added all of them.
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
