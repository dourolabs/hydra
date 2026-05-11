//! End-to-end integration tests for the chat lifecycle: create → message →
//! idle-suspend → resume → message → close.
//!
//! These tests act as a fake worker that connects to `/v1/sessions/:id/relay`
//! over WebSocket and sends the same `WorkerMessage` events a real worker
//! would, allowing the server-side suspension/resume logic to be exercised
//! without spawning a real Claude CLI process.

use crate::{
    app::{AppState, ServiceState},
    domain::{conversations::ConversationStatus as DomainConversationStatus, sessions::Session},
    store::{MemoryStore, Store},
    test_utils::{
        MockJobEngine, spawn_test_server_with_state, test_app_config, test_auth_token, test_client,
        test_secret_manager,
    },
};
use anyhow::Context;
use futures::{SinkExt, StreamExt};
use hydra_common::{
    ConversationId, SessionId,
    api::v1::{
        conversations::{
            Conversation, ConversationEvent, ConversationStatus, CreateConversationRequest,
            SendMessageRequest, ServerMessage, WorkerCatchUp, WorkerConnect, WorkerMessage,
        },
        sessions::{ListSessionsResponse, WorkerContext},
    },
};
use reqwest::StatusCode;
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::tungstenite;

/// Test helper for short idle timeout: build an `AppState` whose JobSection
/// reports `interactive_idle_timeout_secs = secs` in the WorkerContext it
/// serves to interactive workers. In production this defaults to 600s; tests
/// use a small value (e.g. 2s) so the worker idle-timer fires quickly.
fn state_with_idle_timeout_secs(secs: u64) -> (AppState, Arc<dyn Store>) {
    let mut config = test_app_config();
    config.job.interactive_idle_timeout_secs = secs;
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );
    (state, store)
}

/// Find the (currently-only) interactive session linked to `conversation_id`
/// by scanning the store. Tests use this to discover the session id that the
/// fake worker should connect to via the relay WebSocket.
async fn find_session_for_conversation(
    store: &Arc<dyn Store>,
    conversation_id: &ConversationId,
) -> SessionId {
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    let sessions = store
        .list_sessions(&SearchSessionsQuery::default())
        .await
        .expect("list sessions");
    let mut matching: Vec<(SessionId, Session)> = sessions
        .into_iter()
        .filter_map(|(id, v)| {
            if v.item.conversation_id() == Some(conversation_id) {
                Some((id, v.item))
            } else {
                None
            }
        })
        .collect();
    // Pick the most recently-created session if multiple exist (e.g. after resume).
    matching.sort_by_key(|(_, s)| s.creation_time);
    matching
        .pop()
        .map(|(id, _)| id)
        .expect("expected a session for the conversation")
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open the relay WebSocket as a fake worker, authenticating with the
/// shared test bearer token.
async fn connect_relay(base_url: &str, session_id: &SessionId) -> anyhow::Result<WsStream> {
    let ws_url = base_url
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let uri = format!("{ws_url}/v1/sessions/{session_id}/relay");
    let host = base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();
    let auth_value = format!("Bearer {}", test_auth_token());
    let request = tungstenite::http::Request::builder()
        .uri(uri)
        .header("Host", host)
        .header("Authorization", auth_value)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .context("build ws request")?;
    let (stream, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .context("connect to relay ws")?;
    Ok(stream)
}

/// Complete the worker handshake: send `WorkerConnect` and wait for the
/// `CatchUp` response. Returns the catch-up payload the server sent back.
async fn worker_handshake(
    ws: &mut WsStream,
    connect: WorkerConnect,
) -> anyhow::Result<WorkerCatchUp> {
    let connect_json = serde_json::to_string(&connect)?;
    ws.send(tungstenite::Message::Text(connect_json)).await?;

    let msg = ws
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("ws closed before catch-up"))??;
    let text = match msg {
        tungstenite::Message::Text(t) => t,
        other => anyhow::bail!("expected text catch-up, got {other:?}"),
    };
    match serde_json::from_str::<ServerMessage>(&text)? {
        ServerMessage::CatchUp(cu) => Ok(cu),
        other => anyhow::bail!("expected CatchUp, got {other:?}"),
    }
}

/// Send a single `WorkerMessage` to the server over the relay WebSocket.
async fn send_worker_message(ws: &mut WsStream, msg: WorkerMessage) -> anyhow::Result<()> {
    let json = serde_json::to_string(&msg)?;
    ws.send(tungstenite::Message::Text(json)).await?;
    Ok(())
}

/// Drain any pending messages from the WebSocket up to `timeout`, returning
/// the parsed `ServerMessage`s. Used to verify that the server forwards user
/// messages to the worker.
async fn drain_server_messages(
    ws: &mut WsStream,
    timeout: Duration,
) -> anyhow::Result<Vec<ServerMessage>> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(timeout, ws.next()).await {
            Ok(Some(Ok(tungstenite::Message::Text(t)))) => {
                if let Ok(msg) = serde_json::from_str::<ServerMessage>(&t) {
                    out.push(msg);
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    Ok(out)
}

/// Poll the conversation endpoint until its status matches `expected` or the
/// timeout elapses. Workers transition the conversation to Idle asynchronously
/// (via the relay's event handler), so tests cannot assume status updates are
/// visible immediately after the worker sends `Suspending`.
async fn wait_for_status(
    client: &reqwest::Client,
    base_url: &str,
    conversation_id: &ConversationId,
    expected: ConversationStatus,
) -> anyhow::Result<Conversation> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut last: Option<Conversation> = None;
    while tokio::time::Instant::now() < deadline {
        let resp = client
            .get(format!("{base_url}/v1/conversations/{conversation_id}"))
            .send()
            .await?;
        if resp.status() == StatusCode::OK {
            let c: Conversation = resp.json().await?;
            if c.status == expected {
                return Ok(c);
            }
            last = Some(c);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    anyhow::bail!(
        "conversation status did not become {expected:?} within 5s (last: {:?})",
        last.map(|c| c.status)
    );
}

#[tokio::test]
async fn worker_context_includes_configured_idle_timeout() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hi".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let session_id = find_session_for_conversation(&store, &created.conversation_id).await;

    let context: WorkerContext = client
        .get(format!(
            "{}/v1/sessions/{session_id}/context",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    let interactive = context
        .interactive
        .expect("interactive session must include InteractiveOptions");
    assert_eq!(
        interactive.idle_timeout_secs,
        Some(2),
        "WorkerContext must surface the configured interactive_idle_timeout_secs so the \
         worker idle-timer fires at the test-tuned interval"
    );

    Ok(())
}

#[tokio::test]
async fn worker_suspending_transitions_conversation_to_idle_and_stores_session_state()
-> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    // Create a conversation; this also creates the initial interactive session.
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
    assert_eq!(created.status, ConversationStatus::Active);

    let session_id = find_session_for_conversation(&store, &created.conversation_id).await;

    // Connect as a fake worker, handshake, then send Suspending +
    // SessionStateUpload to simulate the worker's idle-timeout flow.
    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let catch_up = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;
    assert_eq!(
        catch_up.events.len(),
        1,
        "fresh worker should receive the initial user message in catch-up"
    );
    assert!(catch_up.session_state.is_none());

    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: ConversationEvent::Suspending {
                reason: "idle_timeout".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::SessionStateUpload {
            data: b"claude-session-abc".to_vec(),
        },
    )
    .await?;

    // Close the WS gracefully so the server cleans up the relay entry.
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);

    let idle = wait_for_status(
        &client,
        &server.base_url(),
        &created.conversation_id,
        ConversationStatus::Idle,
    )
    .await?;
    assert_eq!(idle.status, ConversationStatus::Idle);

    // Verify the Suspending event was appended.
    let events: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{}/events",
            server.base_url(),
            created.conversation_id
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ConversationEvent::Suspending { reason, .. } if reason == "idle_timeout")),
        "expected a Suspending event in the conversation history, got {events:?}"
    );

    // Verify session_state was persisted via the store API.
    let stored_state = store
        .get_conversation_session_state(&created.conversation_id)
        .await?;
    assert_eq!(
        stored_state.as_deref(),
        Some(b"claude-session-abc".as_slice()),
        "SessionStateUpload payload should be persisted on the conversation"
    );

    Ok(())
}

#[tokio::test]
async fn resume_after_idle_replays_session_state_in_catch_up() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    // Phase 1: create, exchange messages, then simulate idle-suspend.
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("first user message".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    let initial_session_id = find_session_for_conversation(&store, &conversation_id).await;

    let mut ws = connect_relay(&server.base_url(), &initial_session_id).await?;
    let _catch_up = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;

    // Simulate a partial agent reply being recorded before the worker suspends.
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "first agent reply".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: ConversationEvent::Suspending {
                reason: "idle_timeout".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::SessionStateUpload {
            data: b"claude-session-xyz".to_vec(),
        },
    )
    .await?;
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);

    wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;

    // Phase 2: resume the conversation. A new session must be created with
    // conversation_resume_from set so its catch-up skips replayed events and
    // includes the prior session_state for `claude --resume`.
    let resumed: Conversation = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/resume",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(resumed.status, ConversationStatus::Active);

    // Find the Resumed event's session_id — that's the new session.
    let events: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    let resumed_session_id = events
        .iter()
        .rev()
        .find_map(|e| match e {
            ConversationEvent::Resumed { session_id, .. } => Some(session_id.clone()),
            _ => None,
        })
        .expect("expected a Resumed event after /resume");
    assert_ne!(
        resumed_session_id, initial_session_id,
        "resume must create a brand-new session"
    );

    // Inspect the resumed session's InteractiveOptions: conversation_resume_from
    // should equal the event count captured before /resume appended Resumed.
    // At suspend we had: UserMessage, AssistantMessage, Suspending = 3 events.
    let resumed_session = store.get_session(&resumed_session_id, false).await?.item;
    let opts = resumed_session
        .interactive
        .expect("resumed session must be interactive");
    assert_eq!(
        opts.conversation_resume_from,
        Some(3),
        "conversation_resume_from should equal pre-Resumed event count"
    );

    // Phase 3: connect as the new worker. The Fresh catch-up should skip the
    // already-replayed events and include the stored session_state.
    let mut ws2 = connect_relay(&server.base_url(), &resumed_session_id).await?;
    let catch_up = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: opts.conversation_resume_from,
        },
    )
    .await?;
    assert_eq!(
        catch_up.session_state.as_deref(),
        Some(b"claude-session-xyz".as_slice()),
        "catch-up on resume must include the stored Claude session_state"
    );
    assert!(
        catch_up
            .events
            .iter()
            .all(|e| !matches!(e, ConversationEvent::Suspending { .. })),
        "Suspending event should not be replayed; got {:?}",
        catch_up.events
    );

    // Phase 4: send a new user message and verify the worker receives it via
    // the relay (i.e. the new session is actively relaying).
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "second user message".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let received = drain_server_messages(&mut ws2, Duration::from_secs(2)).await?;
    let saw_user_message = received.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Event {
                event: ConversationEvent::UserMessage { content, .. }
            } if content == "second user message"
        )
    });
    assert!(
        saw_user_message,
        "expected the resumed worker to receive the new user message over the relay, got {received:?}"
    );

    Ok(())
}

#[tokio::test]
async fn close_then_resume_full_lifecycle() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hi".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    // Send a follow-up message (no worker connected — message is queued).
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "follow up".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    // Close: status must become Closed.
    let closed: Conversation = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(closed.status, ConversationStatus::Closed);

    // Resume: status must return to Active and a new session created.
    let sessions_before: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    let count_before = sessions_before.sessions.len();

    let resumed: Conversation = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/resume",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(resumed.status, ConversationStatus::Active);

    let sessions_after: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        sessions_after.sessions.len() > count_before,
        "resume must create a new session (before={count_before}, after={})",
        sessions_after.sessions.len()
    );

    // Confirm the new session is interactive and conversation_resume_from is
    // set to the event count snapshotted by resume_conversation.
    let new_session_id = find_session_for_conversation(&store, &conversation_id).await;
    let new_session = store.get_session(&new_session_id, false).await?.item;
    let opts = new_session
        .interactive
        .expect("resumed session must be interactive");
    assert!(
        opts.conversation_resume_from.is_some(),
        "conversation_resume_from must be set on a session created by /resume"
    );

    // Send a message after resume to verify the conversation continues to accept input.
    let final_resp = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "after resume".to_string(),
        })
        .send()
        .await?;
    assert_eq!(final_resp.status(), StatusCode::OK);

    // Sanity-check the event sequence end-to-end.
    let events: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    // Expected: hi, follow up, Closed, Resumed, after resume
    assert_eq!(events.len(), 5, "unexpected event sequence: {events:?}");
    assert!(matches!(events[0], ConversationEvent::UserMessage { .. }));
    assert!(matches!(events[1], ConversationEvent::UserMessage { .. }));
    assert!(matches!(events[2], ConversationEvent::Closed { .. }));
    assert!(matches!(events[3], ConversationEvent::Resumed { .. }));
    assert!(matches!(events[4], ConversationEvent::UserMessage { .. }));

    Ok(())
}

#[tokio::test]
async fn resume_replays_full_history_in_catch_up_and_forwards_only_new_message()
-> anyhow::Result<()> {
    // Regression guard for the close/resume flow:
    //   1. The catch-up sent to a freshly-spawned worker after /resume must
    //      include the full prior conversation history (user messages and
    //      assistant replies) so the agent can reconstruct context.
    //   2. After the new worker re-attaches, the server must forward ONLY
    //      the next new user message — not replay msg1/msg2/msg3 — so the
    //      assistant does not generate redundant replies for earlier turns.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let msg1 = "My name is Alice. What's 2+2?";
    let msg2 = "I'm a software engineer. What's 3+3?";
    let msg3 = "I work on Rust projects. What's 4+4?";
    let msg4 = "What's my name and what do I work on?";

    // Phase 1: create the conversation with msg1; this also creates the
    // initial interactive session.
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some(msg1.to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();
    let initial_session_id = find_session_for_conversation(&store, &conversation_id).await;

    // Phase 2: connect fake-worker #1 and exchange three full turns.
    let mut ws1 = connect_relay(&server.base_url(), &initial_session_id).await?;
    let catch_up = worker_handshake(
        &mut ws1,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;
    assert_eq!(
        catch_up.events.len(),
        1,
        "fresh worker should see msg1 in the initial catch-up"
    );
    assert!(
        matches!(
            &catch_up.events[0],
            ConversationEvent::UserMessage { content, .. } if content == msg1
        ),
        "first catch-up event should be msg1, got {:?}",
        catch_up.events[0]
    );

    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "reply 1".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;

    // Turn 2: client sends msg2; verify the relay forwards it to worker #1.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: msg2.to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let forwarded = drain_server_messages(&mut ws1, Duration::from_secs(2)).await?;
    assert!(
        forwarded.iter().any(|m| matches!(
            m,
            ServerMessage::Event { event: ConversationEvent::UserMessage { content, .. } }
                if content == msg2
        )),
        "worker #1 should receive msg2 via the relay, got {forwarded:?}"
    );
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "reply 2".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;

    // Turn 3: client sends msg3; verify the relay forwards it to worker #1.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: msg3.to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let forwarded = drain_server_messages(&mut ws1, Duration::from_secs(2)).await?;
    assert!(
        forwarded.iter().any(|m| matches!(
            m,
            ServerMessage::Event { event: ConversationEvent::UserMessage { content, .. } }
                if content == msg3
        )),
        "worker #1 should receive msg3 via the relay, got {forwarded:?}"
    );
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "reply 3".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;

    // Phase 3: suspend worker #1 (Suspending + SessionStateUpload), close the
    // WS, and wait for the conversation to settle into Idle.
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: ConversationEvent::Suspending {
                reason: "idle_timeout".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(
        &mut ws1,
        WorkerMessage::SessionStateUpload {
            data: b"claude-session-history".to_vec(),
        },
    )
    .await?;
    ws1.send(tungstenite::Message::Close(None)).await?;
    drop(ws1);

    wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;

    // Phase 4: explicitly /close the conversation (the "End Chat" path).
    let closed: Conversation = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(closed.status, ConversationStatus::Closed);
    let events_after_close: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        matches!(
            events_after_close.last(),
            Some(ConversationEvent::Closed { .. })
        ),
        "Closed event should be appended by /close, got {events_after_close:?}"
    );

    // Phase 5: /resume; a new session is created and a Resumed event is
    // appended.
    let resumed: Conversation = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/resume",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(resumed.status, ConversationStatus::Active);
    let new_session_id = find_session_for_conversation(&store, &conversation_id).await;
    assert_ne!(
        new_session_id, initial_session_id,
        "resume must create a brand-new session"
    );

    // Phase 6: connect fake-worker #2 and handshake as Fresh with no
    // resume_from_event_index — the server must return the full prior event
    // log so the new worker can reconstruct context.
    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up2 = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;

    // History-tracked assertion: full prior log, in insertion order.
    // Expected sequence: msg1, reply1, msg2, reply2, msg3, reply3, Suspending,
    // Closed, Resumed = 9 events.
    assert_eq!(
        catch_up2.events.len(),
        9,
        "catch-up should contain 3 user + 3 assistant + Suspending + Closed + \
         Resumed = 9 events, got {:?}",
        catch_up2.events
    );
    match &catch_up2.events[0] {
        ConversationEvent::UserMessage { content, .. } => assert_eq!(content, msg1),
        other => panic!("event[0] should be msg1 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[1] {
        ConversationEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 1"),
        other => panic!("event[1] should be reply 1 AssistantMessage, got {other:?}"),
    }
    match &catch_up2.events[2] {
        ConversationEvent::UserMessage { content, .. } => assert_eq!(content, msg2),
        other => panic!("event[2] should be msg2 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[3] {
        ConversationEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 2"),
        other => panic!("event[3] should be reply 2 AssistantMessage, got {other:?}"),
    }
    match &catch_up2.events[4] {
        ConversationEvent::UserMessage { content, .. } => assert_eq!(content, msg3),
        other => panic!("event[4] should be msg3 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[5] {
        ConversationEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 3"),
        other => panic!("event[5] should be reply 3 AssistantMessage, got {other:?}"),
    }
    assert!(
        matches!(catch_up2.events[6], ConversationEvent::Suspending { .. }),
        "event[6] should be Suspending, got {:?}",
        catch_up2.events[6]
    );
    assert!(
        matches!(catch_up2.events[7], ConversationEvent::Closed { .. }),
        "event[7] should be Closed, got {:?}",
        catch_up2.events[7]
    );
    assert!(
        matches!(catch_up2.events[8], ConversationEvent::Resumed { .. }),
        "event[8] should be Resumed, got {:?}",
        catch_up2.events[8]
    );

    // Phase 7: send msg4 and verify the resumed relay forwards ONLY this new
    // message — not a replay of msg1/msg2/msg3.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: msg4.to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let received_after_resume = drain_server_messages(&mut ws2, Duration::from_millis(500)).await?;
    let event_forwards: Vec<&ConversationEvent> = received_after_resume
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Event { event } => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(
        event_forwards.len(),
        1,
        "exactly one event should be forwarded to worker #2 after resume; \
         a re-broadcast of msg1/2/3 would show up here. Got {event_forwards:?}"
    );
    match event_forwards[0] {
        ConversationEvent::UserMessage { content, .. } => assert_eq!(
            content, msg4,
            "the only forwarded event must be msg4, not a replay"
        ),
        other => panic!("forwarded event must be msg4 UserMessage, got {other:?}"),
    }

    // Final cross-check via GET /events: exactly 4 user messages, 3 assistant
    // messages, and one each of Suspending/Closed/Resumed — no duplicates.
    let final_events: Vec<ConversationEvent> = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;

    let user_messages: Vec<&str> = final_events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::UserMessage { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        user_messages,
        vec![msg1, msg2, msg3, msg4],
        "exactly 4 user messages in insertion order, no duplicates"
    );
    let assistant_messages: Vec<&str> = final_events
        .iter()
        .filter_map(|e| match e {
            ConversationEvent::AssistantMessage { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistant_messages,
        vec!["reply 1", "reply 2", "reply 3"],
        "exactly 3 assistant messages in insertion order, no duplicates"
    );
    let suspending_count = final_events
        .iter()
        .filter(|e| matches!(e, ConversationEvent::Suspending { .. }))
        .count();
    let closed_count = final_events
        .iter()
        .filter(|e| matches!(e, ConversationEvent::Closed { .. }))
        .count();
    let resumed_count = final_events
        .iter()
        .filter(|e| matches!(e, ConversationEvent::Resumed { .. }))
        .count();
    assert_eq!(
        (suspending_count, closed_count, resumed_count),
        (1, 1, 1),
        "exactly one Suspending, one Closed, one Resumed event in the final \
         history; got Suspending={suspending_count}, Closed={closed_count}, \
         Resumed={resumed_count}"
    );

    Ok(())
}

#[tokio::test]
async fn worker_disconnect_without_suspending_still_marks_conversation_idle() -> anyhow::Result<()>
{
    // The relay handler defensively marks an Active conversation Idle if the
    // worker disconnects without sending Suspending (e.g. a crash). This
    // covers the unhappy path of the idle/resume flow.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hi".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    let session_id = find_session_for_conversation(&store, &conversation_id).await;
    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;
    // Drop the WS without sending Suspending. The cleanup branch in the relay
    // handler should still flip status to Idle.
    drop(ws);

    let idle = wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;
    assert_eq!(idle.status, ConversationStatus::Idle);

    // The domain status mirror should match too (sanity-check the store path
    // used by the cleanup branch).
    let domain = store.get_conversation(&conversation_id, false).await?.item;
    assert_eq!(domain.status, DomainConversationStatus::Idle);

    Ok(())
}
