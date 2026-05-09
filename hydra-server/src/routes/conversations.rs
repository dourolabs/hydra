use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{
        AppState, CloseConversationError, CreateConversationError, CreateSessionError,
        ResumeConversationError, SendMessageError,
    },
    store::StoreError,
};
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use hydra_common::{
    ConversationId,
    api::v1::{ApiError, conversations as api_conversations},
};
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
    let actor_ref = ActorRef::from(&actor);

    let (conversation_id, versioned) = state
        .create_conversation(payload.message, payload.agent_name, actor_ref, creator)
        .await
        .map_err(map_create_conversation_error)?;

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

    let conversation_ids: Vec<_> = conversations.iter().map(|(id, _)| id.clone()).collect();
    let event_summaries = state
        .store()
        .get_conversation_event_summaries(&conversation_ids)
        .await
        .map_err(|err| {
            error!(error = %err, "failed to fetch conversation event summaries");
            ApiError::internal(format!("conversation store error: {err}"))
        })?;

    let mut summaries = Vec::with_capacity(conversations.len());
    for (id, versioned) in conversations {
        let (event_count, last_event_preview) = event_summaries
            .get(&id)
            .map(|s| (s.event_count, s.last_event_preview.clone()))
            .unwrap_or((0, None));
        summaries.push(api_conversations::ConversationSummary::new(
            id,
            versioned.item.title,
            versioned.item.agent_name,
            versioned.item.status.into(),
            event_count,
            last_event_preview,
            versioned.item.creator.into(),
            versioned.creation_time,
            versioned.timestamp,
        ));
    }

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

pub async fn send_message(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
    Json(payload): Json<api_conversations::SendMessageRequest>,
) -> Result<Json<api_conversations::ConversationEvent>, ApiError> {
    info!(conversation_id = %conversation_id, "send_message invoked");

    let actor_ref = ActorRef::from(&actor);
    let api_event = state
        .send_message(&conversation_id, payload.content, actor_ref)
        .await
        .map_err(map_send_message_error)?;

    info!(conversation_id = %conversation_id, "send_message completed");
    Ok(Json(api_event))
}

pub async fn close_conversation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!(conversation_id = %conversation_id, "close_conversation invoked");

    let actor_ref = ActorRef::from(&actor);
    let versioned = state
        .close_conversation(&conversation_id, actor_ref)
        .await
        .map_err(map_close_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id.clone(),
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %conversation_id, "close_conversation completed");
    Ok(Json(api_conversation))
}

pub async fn resume_conversation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!(conversation_id = %conversation_id, "resume_conversation invoked");

    let creator = actor.creator.clone();
    let actor_ref = ActorRef::from(&actor);
    let versioned = state
        .resume_conversation(&conversation_id, actor_ref, creator)
        .await
        .map_err(map_resume_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id.clone(),
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %conversation_id, "resume_conversation completed");
    Ok(Json(api_conversation))
}

fn map_create_conversation_error(err: CreateConversationError) -> ApiError {
    match err {
        CreateConversationError::Store { source } => map_conversation_error(source),
        CreateConversationError::Session { source } => map_create_session_error(source),
    }
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

fn map_send_message_error(err: SendMessageError) -> ApiError {
    match err {
        SendMessageError::Store { source } => map_conversation_error(source),
        SendMessageError::NotActive { status } => ApiError::conflict(format!(
            "conversation is not active (status: {status:?}). Resume the conversation first."
        )),
    }
}

fn map_close_conversation_error(err: CloseConversationError) -> ApiError {
    match err {
        CloseConversationError::Store { source } => map_conversation_error(source),
    }
}

fn map_resume_conversation_error(err: ResumeConversationError) -> ApiError {
    match err {
        ResumeConversationError::Store { source } => map_conversation_error(source),
        ResumeConversationError::AlreadyActive => {
            ApiError::conflict("conversation is already active".to_string())
        }
        ResumeConversationError::Session { source } => map_create_session_error(source),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversations::ConversationEvent as DomainEvent;
    use hydra_common::{ConversationId, IssueId};

    #[test]
    fn map_conversation_error_not_found_returns_404() {
        let id = ConversationId::new();
        let err = StoreError::ConversationNotFound(id.clone());
        let api_err = map_conversation_error(err);
        assert_eq!(api_err.status().as_u16(), 404);
        assert!(api_err.message().contains(&id.to_string()));
    }

    #[test]
    fn map_conversation_error_internal_returns_500() {
        let err = StoreError::Internal("db broke".to_string());
        let api_err = map_conversation_error(err);
        assert_eq!(api_err.status().as_u16(), 500);
        assert!(api_err.message().contains("db broke"));
    }

    #[test]
    fn map_create_session_error_issue_not_found_returns_404() {
        let issue_id = IssueId::new();
        let err = CreateSessionError::IssueLookup {
            source: StoreError::IssueNotFound(issue_id.clone()),
            issue_id: issue_id.clone(),
        };
        let api_err = map_create_session_error(err);
        assert_eq!(api_err.status().as_u16(), 404);
        assert!(api_err.message().contains(&issue_id.to_string()));
    }

    #[test]
    fn map_create_session_error_store_returns_500() {
        let err = CreateSessionError::Store {
            source: StoreError::Internal("connection lost".to_string()),
        };
        let api_err = map_create_session_error(err);
        assert_eq!(api_err.status().as_u16(), 500);
        assert!(api_err.message().contains("connection lost"));
    }

    #[test]
    fn map_create_session_error_issue_lookup_internal_returns_500() {
        let issue_id = IssueId::new();
        let err = CreateSessionError::IssueLookup {
            source: StoreError::Internal("timeout".to_string()),
            issue_id: issue_id.clone(),
        };
        let api_err = map_create_session_error(err);
        assert_eq!(api_err.status().as_u16(), 500);
    }

    #[test]
    fn event_preview_user_message() {
        let event = DomainEvent::UserMessage {
            content: "Hello".to_string(),
            timestamp: chrono::Utc::now(),
        };
        assert_eq!(event.preview(), "User: Hello");
    }

    #[test]
    fn event_preview_truncates_long_content() {
        let long_content = "x".repeat(200);
        let event = DomainEvent::UserMessage {
            content: long_content,
            timestamp: chrono::Utc::now(),
        };
        let preview = event.preview();
        assert!(preview.len() <= 110); // prefix + 100 chars + ellipsis
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn event_preview_truncates_multibyte_without_panic() {
        // Content with multi-byte chars (emoji = 4 bytes each) that would panic with byte slicing
        let content = "🎉".repeat(50);
        let event = DomainEvent::UserMessage {
            content,
            timestamp: chrono::Utc::now(),
        };
        let preview = event.preview();
        assert!(preview.starts_with("User: "));
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn truncate_preview_at_char_boundary() {
        // 'é' is 2 bytes; 47 * 2 = 94 bytes for 47 chars
        let content = "é".repeat(50);
        let event = DomainEvent::UserMessage {
            content,
            timestamp: chrono::Utc::now(),
        };
        let result = event.preview();
        // Should not panic, and should end with ellipsis
        assert!(result.ends_with('…'));
        assert!(result.starts_with("User: "));
    }

    #[test]
    fn event_preview_closed() {
        let event = DomainEvent::Closed {
            timestamp: chrono::Utc::now(),
        };
        assert_eq!(event.preview(), "Closed");
    }
}
