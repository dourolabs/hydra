use crate::test_utils::{spawn_test_server, test_client};
use hydra_common::api::v1::conversations::{
    Conversation, ConversationEvent, ConversationSummary, CreateConversationRequest,
    SendMessageRequest, UpdateConversationRequest,
};
use reqwest::StatusCode;

#[tokio::test]
async fn create_conversation_returns_conversation_with_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: Some("Hello, agent!".to_string()),
        agent_name: Some("test-agent".to_string()),
        session_settings: None,
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
async fn create_conversation_without_message_starts_with_zero_events() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&serde_json::json!({}))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let conversation: Conversation = response.json().await?;
    assert!(!conversation.conversation_id.as_ref().is_empty());
    assert!(
        conversation.active_session_id.is_some(),
        "expected active_session_id to be set"
    );
    assert_eq!(
        conversation.status,
        hydra_common::api::v1::conversations::ConversationStatus::Active
    );

    let events: Vec<serde_json::Value> = client
        .get(format!(
            "{}/v1/conversations/{}/events",
            server.base_url(),
            conversation.conversation_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(events.is_empty(), "expected zero events, got {events:?}");

    Ok(())
}

#[tokio::test]
async fn get_conversation_returns_created_conversation() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: Some("test message".to_string()),
        agent_name: None,
        session_settings: None,
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
        message: Some("Hello!".to_string()),
        agent_name: None,
        session_settings: None,
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
        message: Some("What is Rust?".to_string()),
        agent_name: None,
        session_settings: None,
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

#[tokio::test]
async fn send_message_returns_event() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create a conversation
    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    // Send a message
    let msg_request = SendMessageRequest {
        content: "Follow-up message".to_string(),
    };
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            created.conversation_id
        ))
        .json(&msg_request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let event: ConversationEvent = response.json().await?;
    match event {
        ConversationEvent::UserMessage { content, .. } => {
            assert_eq!(content, "Follow-up message");
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }

    // Verify events list now has 2 events
    let events: Vec<serde_json::Value> = client
        .get(format!(
            "{}/v1/conversations/{}/events",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(events.len(), 2);

    Ok(())
}

#[tokio::test]
async fn send_message_to_closed_conversation_returns_409() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create and close a conversation
    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    // Try to send a message — should fail with 409
    let msg_request = SendMessageRequest {
        content: "Should fail".to_string(),
    };
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            created.conversation_id
        ))
        .json(&msg_request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[tokio::test]
async fn close_conversation_sets_status_closed() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let closed: Conversation = response.json().await?;
    assert_eq!(
        closed.status,
        hydra_common::api::v1::conversations::ConversationStatus::Closed
    );
    assert!(closed.active_session_id.is_none());

    Ok(())
}

#[tokio::test]
async fn close_already_closed_conversation_is_idempotent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    // Close twice
    client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    let response = client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn resume_conversation_creates_new_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    // Close the conversation
    client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    // Resume it
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/resume",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let resumed: Conversation = response.json().await?;
    assert_eq!(
        resumed.status,
        hydra_common::api::v1::conversations::ConversationStatus::Active
    );
    assert!(resumed.active_session_id.is_some());
    // New session should be different from the original
    assert_ne!(resumed.active_session_id, created.active_session_id);

    Ok(())
}

#[tokio::test]
async fn resume_active_conversation_returns_409() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    // Try to resume an already-active conversation — should fail with 409
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/resume",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[tokio::test]
async fn full_lifecycle_create_message_close_resume_message() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // 1. Create
    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        created.status,
        hydra_common::api::v1::conversations::ConversationStatus::Active
    );

    // 2. Send message
    let msg_request = SendMessageRequest {
        content: "First message".to_string(),
    };
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            created.conversation_id
        ))
        .json(&msg_request)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Close
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/close",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // 4. Resume
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/resume",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // 5. Send another message after resume
    let msg_request = SendMessageRequest {
        content: "After resume".to_string(),
    };
    let response = client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            created.conversation_id
        ))
        .json(&msg_request)
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    // Verify all events are recorded
    let events: Vec<serde_json::Value> = client
        .get(format!(
            "{}/v1/conversations/{}/events",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?
        .json()
        .await?;
    // Events: UserMessage("Hello"), UserMessage("First message"), Closed, Resumed, UserMessage("After resume")
    assert_eq!(events.len(), 5);
    assert_eq!(events[0]["type"], "user_message");
    assert_eq!(events[1]["type"], "user_message");
    assert_eq!(events[2]["type"], "closed");
    assert_eq!(events[3]["type"], "resumed");
    assert_eq!(events[4]["type"], "user_message");

    Ok(())
}

#[tokio::test]
async fn update_conversation_sets_title() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    let update_request = UpdateConversationRequest {
        title: Some("New Title".to_string()),
    };
    let response = client
        .patch(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            created.conversation_id
        ))
        .json(&update_request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let updated: Conversation = response.json().await?;
    assert_eq!(updated.title.as_deref(), Some("New Title"));
    assert_eq!(updated.conversation_id, created.conversation_id);

    Ok(())
}

#[tokio::test]
async fn update_conversation_not_found_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let fake_id = hydra_common::ConversationId::new();
    let update_request = UpdateConversationRequest {
        title: Some("Title".to_string()),
    };
    let response = client
        .patch(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            fake_id
        ))
        .json(&update_request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn delete_conversation_soft_deletes() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let create_request = CreateConversationRequest {
        message: Some("Hello".to_string()),
        agent_name: None,
        session_settings: None,
    };
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&create_request)
        .send()
        .await?
        .json()
        .await?;

    let response = client
        .delete(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let deleted: Conversation = response.json().await?;
    assert_eq!(deleted.conversation_id, created.conversation_id);

    // Verify the conversation is no longer returned by GET (which excludes deleted)
    let response = client
        .get(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn delete_conversation_not_found_returns_404() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let fake_id = hydra_common::ConversationId::new();
    let response = client
        .delete(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            fake_id
        ))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
