use crate::domain::actors::{Actor, ActorRef};
use crate::{
    app::{AppState, CloseConversationError, CreateConversationError, SendMessageError},
    store::StoreError,
};
use axum::{
    Extension, Json, async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::request::Parts,
};
use hydra_common::{
    ConversationId, Versioned,
    api::v1::{
        ApiError, conversations as api_conversations,
        pagination::{compute_next_cursor, effective_limit},
    },
};
use tracing::{error, info};

pub mod proxy_auth;

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

    let session_settings = payload.session_settings.map(Into::into).unwrap_or_default();
    let (conversation_id, versioned) = state
        .create_conversation(
            payload.message,
            payload.agent_name,
            session_settings,
            None,
            actor_ref,
            creator,
        )
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
) -> Result<Json<api_conversations::ListConversationsResponse>, ApiError> {
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

    // Store returns up to `limit + 1` rows ordered by the same keyset as the
    // cursor (version-row created_at DESC, id DESC). That row's timestamp
    // surfaces here as `summary.updated_at`.
    let eff_limit = effective_limit(query.limit);
    let next_cursor = compute_next_cursor(
        &mut summaries,
        eff_limit,
        |s| &s.updated_at,
        |s| s.conversation_id.as_ref(),
    );

    info!(returned = summaries.len(), "list_conversations completed");
    let mut response = api_conversations::ListConversationsResponse::new(summaries);
    response.next_cursor = next_cursor;
    Ok(Json(response))
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

pub async fn list_conversation_versions(
    State(state): State<AppState>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<Vec<Versioned<api_conversations::Conversation>>>, ApiError> {
    info!(conversation_id = %conversation_id, "list_conversation_versions invoked");

    let versions = state
        .store()
        .get_conversation_versions(&conversation_id)
        .await
        .map_err(map_conversation_error)?;

    let api_versions: Vec<Versioned<api_conversations::Conversation>> = versions
        .into_iter()
        .map(|v| {
            let item = v
                .item
                .to_api(conversation_id.clone(), v.creation_time, v.timestamp);
            Versioned::with_optional_actor(item, v.version, v.timestamp, v.actor, v.creation_time)
        })
        .collect();

    info!(
        conversation_id = %conversation_id,
        returned = api_versions.len(),
        "list_conversation_versions completed"
    );
    Ok(Json(api_versions))
}

#[derive(Debug, Clone)]
pub struct ConversationVersionPath {
    pub conversation_id: ConversationId,
    pub version: hydra_common::RelativeVersionNumber,
}

#[async_trait]
impl<S> FromRequestParts<S> for ConversationVersionPath
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((conversation_id, version)) = Path::<(
            ConversationId,
            hydra_common::RelativeVersionNumber,
        )>::from_request_parts(parts, state)
        .await
        .map_err(|rejection| ApiError::bad_request(rejection.to_string()))?;

        Ok(Self {
            conversation_id,
            version,
        })
    }
}

pub async fn get_conversation_version(
    State(state): State<AppState>,
    ConversationVersionPath {
        conversation_id,
        version: raw_version,
    }: ConversationVersionPath,
) -> Result<Json<Versioned<api_conversations::Conversation>>, ApiError> {
    info!(
        conversation_id = %conversation_id,
        raw_version = raw_version.as_i64(),
        "get_conversation_version invoked"
    );
    let versions = state
        .store()
        .get_conversation_versions(&conversation_id)
        .await
        .map_err(map_conversation_error)?;

    let max_version = versions.iter().map(|v| v.version).max().unwrap_or(0);
    let version = super::resolve_version(
        raw_version,
        max_version,
        "conversation",
        conversation_id.as_ref(),
    )?;

    let entry = versions
        .into_iter()
        .find(|entry| entry.version == version)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "conversation '{conversation_id}' version {version} not found"
            ))
        })?;

    let item = entry.item.to_api(
        conversation_id.clone(),
        entry.creation_time,
        entry.timestamp,
    );
    let response = Versioned::with_optional_actor(
        item,
        entry.version,
        entry.timestamp,
        entry.actor,
        entry.creation_time,
    );
    info!(conversation_id = %conversation_id, version, "get_conversation_version completed");
    Ok(Json(response))
}

pub async fn send_message(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
    Json(payload): Json<api_conversations::SendMessageRequest>,
) -> Result<Json<hydra_common::api::v1::sessions::SessionEvent>, ApiError> {
    info!(conversation_id = %conversation_id, "send_message invoked");

    let actor_ref = ActorRef::from(&actor);
    let principal = actor.creator.clone();
    let api_event = state
        .send_message(&conversation_id, payload.content, actor_ref, principal)
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

pub async fn update_conversation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
    Json(payload): Json<api_conversations::UpdateConversationRequest>,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!(conversation_id = %conversation_id, "update_conversation invoked");

    let actor_ref = ActorRef::from(&actor);
    let versioned = state
        .update_conversation_metadata(&conversation_id, payload.title, actor_ref)
        .await
        .map_err(map_close_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id.clone(),
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %conversation_id, "update_conversation completed");
    Ok(Json(api_conversation))
}

pub async fn delete_conversation(
    State(state): State<AppState>,
    Extension(actor): Extension<Actor>,
    ConversationIdPath(conversation_id): ConversationIdPath,
) -> Result<Json<api_conversations::Conversation>, ApiError> {
    info!(conversation_id = %conversation_id, "delete_conversation invoked");

    let actor_ref = ActorRef::from(&actor);
    let versioned = state
        .delete_conversation(&conversation_id, actor_ref)
        .await
        .map_err(map_close_conversation_error)?;

    let api_conversation = versioned.item.to_api(
        conversation_id.clone(),
        versioned.creation_time,
        versioned.timestamp,
    );

    info!(conversation_id = %conversation_id, "delete_conversation completed");
    Ok(Json(api_conversation))
}

fn map_create_conversation_error(err: CreateConversationError) -> ApiError {
    match err {
        CreateConversationError::Store { source } => map_conversation_error(source),
        CreateConversationError::AgentNotFound { name } => {
            ApiError::bad_request(format!("agent '{name}' not found"))
        }
        CreateConversationError::Agent { source } => {
            error!(error = %source, "failed to resolve agent for create_conversation");
            ApiError::internal(format!("agent resolution error: {source}"))
        }
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
        SendMessageError::Forbidden { principal } => ApiError::forbidden(format!(
            "user '{principal}' is not the creator of this conversation",
        )),
    }
}

fn map_close_conversation_error(err: CloseConversationError) -> ApiError {
    match err {
        CloseConversationError::Store { source } => map_conversation_error(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::ConversationId;

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
}
