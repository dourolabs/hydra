use crate::domain::actors::{Actor, ActorRef};
use crate::domain::conversations::{Conversation, ConversationEvent as DomainEvent};
use crate::{
    app::{AppState, CreateSessionError},
    store::StoreError,
};
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use hydra_common::{
    ConversationId,
    api::v1::{
        ApiError, conversations as api_conversations,
        sessions::{BundleSpec, CreateSessionRequest},
    },
};
use std::collections::HashMap;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub struct ConversationIdPath(pub ConversationId);

#[async_trait]
impl<S> FromRequestParts<S> for ConversationIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(conversation_id) = Path::<ConversationId>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self(conversation_id))
    }
}

pub async fn create_conversation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    Json(payload): Json<api_conversations::CreateConversationRequest>,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!("create_conversation invoked");

    let creator = actor.creator.clone();

    // 1. Create a domain Conversation with status Active
    let conversation = Conversation {
        title: None,
        agent_name: payload.agent_name.clone(),
        active_session_id: None,
        status: crate::domain::conversations::ConversationStatus::Active,
        creator: creator.clone(),
        deleted: false,
    };

    // 2. Persist the conversation
    let actor_ref = ActorRef::from(&actor);
    let (conversation_id, _version) = state
        .store
        .add_conversation_with_actor(conversation.clone(), actor_ref.clone())
        .await
        .map_err(map_conversation_error)?;

    // 3. Append the first UserMessage event
    let event = DomainEvent::UserMessage {
        content: payload.message.clone(),
        timestamp: chrono::Utc::now(),
    };
    state
        .store
        .append_conversation_event_with_actor(&conversation_id, event, actor_ref.clone())
        .await
        .map_err(map_conversation_error)?;

    // 4. Create an interactive session
    let session_request = CreateSessionRequest::new(
        payload.message,
        None,
        BundleSpec::None,
        HashMap::new(),
        None,
        true,
    );
    let session_id = state
        .create_session(session_request, actor_ref.clone(), creator)
        .await
        .map_err(map_create_session_error)?;

    // 5. Update conversation with active_session_id
    let mut updated_conversation = conversation;
    updated_conversation.active_session_id = Some(session_id);
    state
        .store
        .update_conversation_with_actor(&conversation_id, updated_conversation, actor_ref)
        .await
        .map_err(map_conversation_error)?;

    // 6. Return the conversation
    let versioned = state
        .store()
        .get_conversation(&conversation_id, false)
        .await
        .map_err(map_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id,
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %api_conversation.conversation_id, "create_conversation completed");
    Ok(Json(api_conversation))
}

pub async fn list_conversations(
    State(state): State<AppState>,
    Query(query): Query<api_conversations::SearchConversationsQuery>,
) -> Result<Json<Vec<api_conversations::ConversationSummary>>, ApiError> {
    info!(
        status = ?query.status,
        creator = ?query.creator,
        query = ?query.q,
        "list_conversations invoked"
    );

    let conversations = state
        .store()
        .list_conversations(&query)
        .await
        .map_err(map_conversation_error)?;

    let summaries: Vec<api_conversations::ConversationSummary> = conversations
        .into_iter()
        .map(|(id, versioned)| {
            api_conversations::ConversationSummary::new(
                id,
                versioned.item.title,
                versioned.item.agent_name,
                versioned.item.status.into(),
                0,
                None,
                versioned.item.creator.into(),
                versioned.creation_time,
                versioned.timestamp,
            )
        })
        .collect();

    info!(returned = summaries.len(), "list_conversations completed");
    Ok(Json(summaries))
}

pub async fn get_conversation(
    State(state): State<AppState>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!(conversation_id = %conversation_id, "get_conversation invoked");

    let versioned = state
        .store()
        .get_conversation(&conversation_id, false)
        .await
        .map_err(map_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id,
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %api_conversation.conversation_id, "get_conversation completed");
    Ok(Json(api_conversation))
}

pub async fn get_conversation_events(
    State(state): State<AppState>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<Vec<api_conversations::ConversationEvent>>, ApiError> {
    info!(conversation_id = %conversation_id, "get_conversation_events invoked");

    let events = state
        .store()
        .get_conversation_events(&conversation_id)
        .await
        .map_err(map_conversation_error)?;

    let api_events: Vec<api_conversations::ConversationEvent> =
        events.into_iter().map(|v| v.item.into()).collect();

    info!(
        conversation_id = %conversation_id,
        returned = api_events.len(),
        "get_conversation_events completed"
    );
    Ok(Json(api_events))
}

fn map_conversation_error(err: StoreError) -> ApiError {
    match err {
        StoreError::ConversationNotFound(id) => {
            error!(conversation_id = %id, "conversation not found");
            ApiError::not_found(format!("conversation '{id}' not found"))
        }
        other => {
            error!(error = %other, "conversation store operation failed");
            ApiError::internal(format!("conversation store error: {other}"))
        }
    }
}

fn map_create_session_error(err: CreateSessionError) -> ApiError {
    match err {
        CreateSessionError::TaskResolution(err) => ApiError::from(err),
        CreateSessionError::IssueLookup { source, issue_id } => match source {
            StoreError::IssueNotFound(_) => {
                ApiError::not_found(format!("issue '{issue_id}' not found"))
            }
            other => {
                error!(
                    error = %other,
                    issue_id = %issue_id,
                    "failed to load issue for session creation"
                );
                ApiError::internal(format!("Failed to load issue '{issue_id}': {other}"))
            }
        },
        CreateSessionError::Store { source } => {
            error!(error = %source, "failed to store session");
            ApiError::internal(format!("Failed to store session: {source}"))
        }
    }
}
