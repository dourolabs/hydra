use crate::test_utils::{spawn_test_server, test_client};
use hydra_common::api::v1::conversations::{
    Conversation, ConversationSummary, CreateConversationRequest,
};
use reqwest::StatusCode;

#[tokio::test]
async fn create_conversation_returns_conversation_with_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: "Hello, agent!".to_string(),
        agent_name: Some("test-agent".to_string()),
    };

    let response = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let conversation: Conversation = response.json().await?;
    assert!(!conversation.conversation_id.as_ref().is_empty());
    assert_eq!(conversation.agent_name.as_deref(), Some("test-agent"));
    assert!(
        conversation.active_session_id.is_some(),
        "expected active_session_id to be set"
    );

    Ok(())
}

#[tokio::test]
async fn get_conversation_returns_created_conversation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: "test message".to_string(),
        agent_name: None,
    };

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&request)
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let fetched: Conversation = response.json().await?;
    assert_eq!(fetched.conversation_id, created.conversation_id);

    Ok(())
}

#[tokio::test]
async fn get_conversation_not_found_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let fake_id = hydra_common::ConversationId::new();
    let response = client
        .get(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            fake_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn list_conversations_returns_summaries_with_event_count() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: "Hello!".to_string(),
        agent_name: None,
    };

    client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&request)
        .send()
        .await?;

    let response = client
        .get(format!("{}/v1/conversations", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let summaries: Vec<ConversationSummary> = response.json().await?;
    assert!(!summaries.is_empty());

    let summary = &summaries[0];
    assert!(
        summary.event_count > 0,
        "expected event_count > 0, got {}",
        summary.event_count
    );

    Ok(())
}

#[tokio::test]
async fn get_conversation_events_returns_events() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: "What is Rust?".to_string(),
        agent_name: None,
    };

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&request)
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .get(format!(
            "{}/v1/conversations/{}/events",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let events: Vec<serde_json::Value> = response.json().await?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "user_message");
    assert_eq!(events[0]["content"], "What is Rust?");

    Ok(())
}
