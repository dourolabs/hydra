use super::common::mark_session_terminal;
use crate::{
    app::{AppState, ServiceState},
    domain::task_status::Status as TaskStatus,
    store::{MemoryStore, Store},
    test_utils::{
        MockJobEngine, spawn_test_server, spawn_test_server_with_state, test_app_config,
        test_client, test_secret_manager,
    },
};
use hydra_common::agents::UpsertAgentRequest;
use hydra_common::api::v1::conversations::{
    Conversation, ConversationEvent, ConversationStatus, ConversationSummary,
    CreateConversationRequest, SendMessageRequest, UpdateConversationRequest,
};
use hydra_common::api::v1::sessions::{ListSessionsResponse, SearchSessionsQuery};
use hydra_common::{ConversationId, SessionId};
use reqwest::StatusCode;
use std::sync::Arc;
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

/// Construct a fresh `AppState` + `Store` pair for an integration test that
/// needs to drive the in-process automation runner (e.g. by marking a session
/// terminal so `SpawnConversationSessionsAutomation` flips the conversation to
/// Idle). The test passes a clone of `state` to `spawn_test_server_with_state`
/// and retains the original to call `state.store.update_session_with_actor`
/// later.
fn integration_state() -> (AppState, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(test_app_config()),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );
    (state, store)
}

/// Count the sessions linked to `conversation_id` via the store. The HTTP
/// `ListSessionsResponse.sessions` does expose `conversation_id` on each
/// `SessionSummary`, but going through the store keeps these helpers
/// independent of the HTTP layer.
async fn session_count_for_conversation(
    store: &Arc<dyn Store>,
    conversation_id: &ConversationId,
) -> usize {
    let sessions = store
        .list_sessions(&SearchSessionsQuery::default())
        .await
        .expect("list sessions");
    sessions
        .into_iter()
        .filter(|(_, s)| s.item.conversation_id() == Some(conversation_id))
        .count()
}

/// Find the single session attached to a conversation via the store, polling
/// briefly for the asynchronous spawn automation to settle.
async fn find_session_for_conversation(
    store: &Arc<dyn Store>,
    conversation_id: &ConversationId,
) -> SessionId {
    poll_until(POLL_TIMEOUT, || async {
        let sessions = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .expect("list sessions");
        sessions
            .into_iter()
            .find(|(_, s)| s.item.conversation_id() == Some(conversation_id))
            .map(|(id, _)| id)
    })
    .await
    .expect("expected a session for the conversation to appear")
}

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
        agent_name: Some(hydra_common::api::v1::agents::AgentName::try_new("test-agent").unwrap()),
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
    assert_eq!(
        conversation.agent_name.as_ref().map(|n| n.as_str()),
        Some("test-agent")
    );

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
    let summary = sessions
        .sessions
        .iter()
        .find(|record| record.session.conversation_id.as_ref() == Some(&conversation.conversation_id))
        .unwrap_or_else(|| {
            panic!(
                "expected one of the listed session summaries to expose conversation_id={}, got {:?}",
                conversation.conversation_id,
                sessions
                    .sessions
                    .iter()
                    .map(|r| (r.session_id.as_ref(), r.session.conversation_id.as_ref()))
                    .collect::<Vec<_>>(),
            )
        });
    assert_eq!(
        summary.session.conversation_id.as_ref(),
        Some(&conversation.conversation_id),
        "summary.conversation_id should match the linked conversation",
    );

    Ok(())
}

#[tokio::test]
async fn create_conversation_with_unknown_agent_name_returns_400() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let request = CreateConversationRequest {
        message: Some("Hello, agent!".to_string()),
        agent_name: Some(
            hydra_common::api::v1::agents::AgentName::try_new("does-not-exist").unwrap(),
        ),
        session_settings: None,
    };

    let response = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&request)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await?;
    let message = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        message.contains("does-not-exist"),
        "error body should mention the unknown agent name, got {body:?}",
    );

    // Conversation should not have been persisted, and no session should
    // have been spawned, since validation runs before the store write.
    let conversations: Vec<ConversationSummary> = client
        .get(format!("{}/v1/conversations", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        conversations.is_empty(),
        "no conversation should be persisted when agent_name validation fails, got {conversations:?}"
    );

    // Give the automation a chance to (not) spawn anything before asserting.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let sessions: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        sessions.sessions.is_empty(),
        "no session should be spawned when agent_name validation fails, got {:?}",
        sessions.sessions
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
    // `event_count` aggregates chat-text SessionEvents across every session
    // linked to the conversation. Under the queue-and-deliver model the
    // first user message stays on the chat-relay's pending queue until a
    // worker connects (no worker connects in this HTTP-only smoke test),
    // so no session log carries it yet — event_count is 0. The summary
    // entry still exists because the conversation row does; it just has
    // zero chat events on any session log.
    assert_eq!(summary.event_count, 0);

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
    // The conversation events log holds only lifecycle events post-Phase-E
    // step 18 (chat content moved to `SessionEvent`); a freshly-created
    // conversation has no lifecycle entries yet.
    let events: Vec<serde_json::Value> = response.json().await?;
    assert!(
        events.is_empty(),
        "expected no lifecycle events, got {events:?}"
    );

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
    let event: hydra_common::api::v1::sessions::SessionEvent = response.json().await?;
    match event {
        hydra_common::api::v1::sessions::SessionEvent::UserMessage { content, .. } => {
            assert_eq!(content, "Follow-up message");
        }
        other => panic!("expected UserMessage, got {other:?}"),
    }

    // Chat content lives on the per-session SessionEvent log post-Phase-E
    // step 18; the conversation events log is for lifecycle only and is
    // empty for a freshly-created, never-suspended conversation.
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
    assert_eq!(events.len(), 0);

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
    // Wait for the Resumed event on the conversation lifecycle log; the new
    // UserMessage (`back from the dead`) lives on the per-session
    // SessionEvent log post-Phase-E step 18 and the test only asserts on
    // the conversation lifecycle here.
    let _ = poll_until(POLL_TIMEOUT, || async {
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

    // Verify all lifecycle events are recorded. Chat-content events
    // (user_message) live on the per-session SessionEvent log post-Phase-E
    // step 18 — only lifecycle events (closed, resumed) appear here. The
    // resume produces a `Resumed` event asynchronously via the automation,
    // so poll until it appears.
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
    let closed_count = events.iter().filter(|e| e["type"] == "closed").count();
    let resumed_count = events.iter().filter(|e| e["type"] == "resumed").count();
    assert_eq!(closed_count, 1, "got events: {events:?}");
    assert_eq!(resumed_count, 1, "got events: {events:?}");

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

// ---- Regression tests for the duplicate-spawn bug ----
//
// These tests are the primary verification that the trigger-on-transition
// design in `SpawnConversationSessionsAutomation` eliminates duplicate spawns
// by construction. They reproduce jayantk's two scenarios verbatim:
//   - `POST /v1/conversations` with a non-empty `message` field
//   - `POST /v1/conversations/:id/messages` to a previously-Idle conversation
// plus a no-op check for `POST /messages` on an already-Active conversation.

#[tokio::test]
async fn create_conversation_with_message_spawns_exactly_one_session() -> anyhow::Result<()> {
    // Repro for: "POST /v1/conversations with a non-empty message field
    // currently spawns 2 sessions". Under the trigger-on-transition design
    // only the ConversationCreated event spawns; `ConversationEventCreated`
    // (the UserMessage append) is not a trigger anymore. This test must FAIL
    // against the parent commit and pass with the redesign.
    let (state, store) = integration_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    // Wait for at least one session to spawn so the automation has had a
    // chance to settle.
    poll_until(POLL_TIMEOUT, || async {
        (session_count_for_conversation(&store, &conversation_id).await > 0).then_some(())
    })
    .await
    .expect("expected at least one session to spawn for the conversation");

    // Give a generous post-settle window so a stray racing spawn would
    // surface here rather than slip past the count check.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let count = session_count_for_conversation(&store, &conversation_id).await;
    assert_eq!(
        count, 1,
        "exactly one session must be spawned per conversation create-with-message; got {count}"
    );

    Ok(())
}

#[tokio::test]
async fn send_message_to_active_conversation_does_not_spawn_new_session() -> anyhow::Result<()> {
    // Sending a follow-up `UserMessage` to an already-Active conversation
    // must NOT trigger a spawn: `ConversationEventCreated` is no longer in
    // the automation's trigger set, and `send_message` does not flip the
    // status when it's already Active.
    let (state, store) = integration_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    // Settle on the initial spawn.
    poll_until(POLL_TIMEOUT, || async {
        (session_count_for_conversation(&store, &conversation_id).await >= 1).then_some(())
    })
    .await
    .expect("expected the initial session to spawn");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let initial_count = session_count_for_conversation(&store, &conversation_id).await;
    assert_eq!(
        initial_count, 1,
        "initial spawn must produce exactly one session"
    );

    // Send two more messages to the already-Active conversation.
    for content in ["follow-up 1", "follow-up 2"] {
        client
            .post(format!(
                "{}/v1/conversations/{conversation_id}/messages",
                server.base_url()
            ))
            .json(&SendMessageRequest {
                content: content.to_string(),
            })
            .send()
            .await?
            .error_for_status()?;
    }

    // Give the automation a chance to mis-fire if it would.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let after_count = session_count_for_conversation(&store, &conversation_id).await;
    assert_eq!(
        after_count, 1,
        "send_message on an Active conversation must not spawn another session; got {after_count}"
    );

    Ok(())
}

#[tokio::test]
async fn send_message_to_closed_conversation_spawns_exactly_one_resume_session()
-> anyhow::Result<()> {
    // Direct exposure of the OLD trigger model's race: when a `Closed`
    // conversation is re-opened via `POST /messages`, the OLD
    // automation fires for BOTH `ConversationUpdated` (Closed→Active) AND
    // `ConversationEventCreated` (UserMessage). Both invocations call
    // `detect_resume_state`, find the prior `Closed` marker, skip the
    // idempotency check, and call `create_session`. Result on OLD main:
    // two spawn attempts plus the initial → 3 sessions (and 2 `Resumed`
    // events). Under the trigger-on-transition design, only
    // `ConversationUpdated` triggers and the transition fires exactly once
    // per Closed→Active flip → 2 sessions (and 1 `Resumed`).
    //
    // This test should FAIL on `7f74f1e1` (today's main) and PASS with the
    // redesign.
    let (state, store) = integration_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    // Settle on the initial spawn.
    let _initial_session_id = find_session_for_conversation(&store, &conversation_id).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        session_count_for_conversation(&store, &conversation_id).await,
        1,
        "initial spawn must produce exactly one session"
    );

    // /close → status Closed, Closed event appended.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url()
        ))
        .send()
        .await?
        .error_for_status()?;

    // Re-open via send_message: flips Closed→Active (ConversationUpdated)
    // AND appends UserMessage (ConversationEventCreated). On OLD main, both
    // events trigger a resume spawn; on this PR only ConversationUpdated
    // does.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "back from the dead".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Wait for the resume spawn to settle, then ensure no second spawn
    // happens.
    poll_until(POLL_TIMEOUT, || async {
        (session_count_for_conversation(&store, &conversation_id).await >= 2).then_some(())
    })
    .await
    .expect("expected a resume session to spawn after send_message to Closed conversation");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let count = session_count_for_conversation(&store, &conversation_id).await;
    assert_eq!(
        count, 2,
        "exactly one NEW session must be spawned by the Closed→Active flip; got {count}"
    );

    // Exactly one Resumed event in the log; OLD main would have appended
    // two.
    let events: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    let resumed_count = events
        .iter()
        .filter(|e| matches!(e, ConversationEvent::Resumed { .. }))
        .count();
    assert_eq!(
        resumed_count, 1,
        "exactly one Resumed event must be appended by the Closed→Active flip; got {resumed_count}"
    );

    Ok(())
}

#[tokio::test]
async fn send_message_to_idle_conversation_spawns_exactly_one_session() -> anyhow::Result<()> {
    // Direct repro for: "if i have no conversations active, and then i send
    // a message, 2 sessions seem to spawn". Drive the initial session to
    // terminal so the automation flips the conversation to Idle; then
    // `POST /messages` flips Idle → Active and a SINGLE resume spawn must
    // result.
    let (state, store) = integration_state();
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    let initial_session_id = find_session_for_conversation(&store, &conversation_id).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        session_count_for_conversation(&store, &conversation_id).await,
        1,
        "initial spawn must produce exactly one session"
    );

    // Drive the initial session terminal so the automation flips the
    // conversation Active → Idle.
    mark_session_terminal(&state, &initial_session_id, TaskStatus::Complete).await;
    poll_until(POLL_TIMEOUT, || async {
        let fetched: Conversation = client
            .get(format!(
                "{}/v1/conversations/{conversation_id}",
                server.base_url()
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        (fetched.status == ConversationStatus::Idle).then_some(())
    })
    .await
    .expect("expected conversation to flip to Idle after session terminal");

    // Send a message to the Idle conversation. The HTTP `send_message`
    // handler flips Idle → Active, which fires exactly one
    // ConversationUpdated event; the automation spawns exactly one new
    // session.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "back from the dead".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Wait for the resume spawn to settle.
    poll_until(POLL_TIMEOUT, || async {
        (session_count_for_conversation(&store, &conversation_id).await >= 2).then_some(())
    })
    .await
    .expect("expected a resume session to spawn after send_message to Idle conversation");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let count = session_count_for_conversation(&store, &conversation_id).await;
    assert_eq!(
        count, 2,
        "send_message to an Idle conversation must spawn exactly one NEW session \
         (2 total — the original and the resume); got {count}"
    );

    Ok(())
}
