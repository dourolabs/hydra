use crate::test_utils::{spawn_test_server, test_client};
use hydra_common::agents::UpsertAgentRequest;
use hydra_common::api::v1::conversations::{
    Conversation, ConversationEvent, ConversationSummary, CreateConversationRequest,
    SendMessageRequest, UpdateConversationRequest,
};
use hydra_common::api::v1::sessions::ListSessionsResponse;
use reqwest::StatusCode;
use std::time::Duration;

/// Poll `f` until it returns `Some` or the timeout elapses. Used in
/// integration tests to wait for the asynchronous
/// `SpawnConversationSessionsAutomation` to settle.
async fn poll_until<T, F, Fut>(timeout: Duration, mut f: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = f().await {
            return Some(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

const POLL_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn create_conversation_returns_conversation_with_session() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Conversation references "test-agent"; register it first so resolution
    // succeeds (unknown agent names now return 400).
    let agent_request = UpsertAgentRequest::new(
        "test-agent",
        "test agent prompt",
        3,
        1,
        None,
        None,
        false,
        false,
        vec![],
    );
    let agent_response = client
        .post(format!("{}/v1/agents", server.base_url()))
        .json(&agent_request)
        .send()
        .await?;
    assert!(agent_response.status().is_success());

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

    let sessions: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        !sessions.sessions.is_empty(),
        "expected create_conversation to create a session"
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
    assert_eq!(
        conversation.status,
        hydra_common::api::v1::conversations::ConversationStatus::Active
    );

    // Poll for the session — it's spawned asynchronously by
    // `SpawnConversationSessionsAutomation` against the default conversation
    // agent seeded by `spawn_test_server`.
    poll_until(POLL_TIMEOUT, || async {
        let sessions: ListSessionsResponse = client
            .get(format!("{}/v1/sessions", server.base_url()))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        (!sessions.sessions.is_empty()).then_some(())
    })
    .await
    .expect("expected create_conversation to spawn a session");

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
async fn send_message_to_closed_conversation_auto_resumes() -> anyhow::Result<()> {
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

    // Sending a message to a Closed conversation should succeed and
    // implicitly transition it back to Active (mirrors a Resume click).
    let msg_request = SendMessageRequest {
        content: "back from the dead".to_string(),
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

    // Conversation should now be Active again.
    let conversation: Conversation = client
        .get(format!(
            "{}/v1/conversations/{}",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        conversation.status,
        hydra_common::api::v1::conversations::ConversationStatus::Active
    );

    // Wait for the resume-on-send to settle: the automation appends a
    // `Resumed` event and spawns a second session asynchronously. Because the
    // spawn is async, the *order* of `Resumed` vs the new `UserMessage` is
    // no longer guaranteed in the event log — the test only verifies both
    // are present.
    let events = poll_until(POLL_TIMEOUT, || async {
        let events: Vec<ConversationEvent> = client
            .get(format!(
                "{}/v1/conversations/{}/events",
                server.base_url(),
                created.conversation_id
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        events
            .iter()
            .any(|e| matches!(e, ConversationEvent::Resumed { .. }))
            .then_some(events)
    })
    .await
    .expect("expected a Resumed event after resume-on-send");
    assert!(
        events.iter().any(|e| matches!(
            e,
            ConversationEvent::UserMessage { content, .. } if content == "back from the dead"
        )),
        "expected the new UserMessage to be appended in the event log, got {events:?}"
    );

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

    // Wait for the resume to settle — the automation appends the Resumed
    // event and spawns the second session asynchronously.
    let resumed_session_id = poll_until(POLL_TIMEOUT, || async {
        let events: Vec<ConversationEvent> = client
            .get(format!(
                "{}/v1/conversations/{}/events",
                server.base_url(),
                created.conversation_id
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        events.iter().rev().find_map(|e| match e {
            ConversationEvent::Resumed { session_id, .. } => Some(session_id.clone()),
            _ => None,
        })
    })
    .await
    .expect("expected a Resumed event after /resume");
    let sessions: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        sessions
            .sessions
            .iter()
            .any(|s| s.session_id == resumed_session_id),
        "resumed session_id should appear in the sessions list"
    );
    assert!(
        sessions.sessions.len() >= 2,
        "resume should produce a second session in addition to the original (got {})",
        sessions.sessions.len()
    );

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

    // Verify all events are recorded. The resume produces a `Resumed` event
    // asynchronously via the automation, so poll until it appears. The
    // relative order of `Resumed` and the post-resume `UserMessage` is no
    // longer guaranteed (it depends on whether the automation processes the
    // status flip before or after the new UserMessage is appended), so this
    // test only verifies the multi-set of event types is correct.
    let events = poll_until(POLL_TIMEOUT, || async {
        let events: Vec<serde_json::Value> = client
            .get(format!(
                "{}/v1/conversations/{}/events",
                server.base_url(),
                created.conversation_id
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        events
            .iter()
            .any(|e| e["type"] == "resumed")
            .then_some(events)
    })
    .await
    .expect("expected a Resumed event in the event log after resume");
    // Expected event multiset: 3× user_message, 1× closed, 1× resumed.
    assert_eq!(events.len(), 5, "got events: {events:?}");
    let user_message_count = events
        .iter()
        .filter(|e| e["type"] == "user_message")
        .count();
    let closed_count = events.iter().filter(|e| e["type"] == "closed").count();
    let resumed_count = events.iter().filter(|e| e["type"] == "resumed").count();
    assert_eq!(user_message_count, 3, "got events: {events:?}");
    assert_eq!(closed_count, 1, "got events: {events:?}");
    assert_eq!(resumed_count, 1, "got events: {events:?}");
    // The first two events were written synchronously by create_conversation
    // + send_message and must appear in chronological order; the close event
    // came next. Everything after the close (Resumed and the post-resume
    // UserMessage) is order-flexible per the async automation.
    assert_eq!(events[0]["type"], "user_message");
    assert_eq!(events[1]["type"], "user_message");
    assert_eq!(events[2]["type"], "closed");

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
