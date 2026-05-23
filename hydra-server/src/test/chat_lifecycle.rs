//! End-to-end integration tests for the chat lifecycle: create → message →
//! idle-suspend → resume → message → close.
//!
//! These tests act as a fake worker that connects to `/v1/sessions/:id/relay`
//! over WebSocket and sends the same `WorkerMessage` events a real worker
//! would, allowing the server-side suspension/resume logic to be exercised
//! without spawning a real Claude CLI process.

use super::common::mark_session_terminal;
use crate::{
    app::{AppState, ServiceState},
    domain::{
        actors::ActorRef, agents::Agent,
        conversations::ConversationStatus as DomainConversationStatus, documents::Document,
        sessions::Session, task_status::Status as TaskStatus,
    },
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
            SendMessageRequest, ServerMessage, SessionStatePayload, WorkerCatchUp, WorkerConnect,
            WorkerMessage,
        },
        sessions::{ListSessionsResponse, WorkerContext},
    },
};
use reqwest::StatusCode;
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::tungstenite;

/// Install a process-wide tracing subscriber that routes events through
/// `print!` so `cargo test -- --nocapture` surfaces the
/// upload/store/catch-up instrumentation in `relay.rs`. Idempotent across
/// many tests in the same process.
fn init_test_tracing() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .try_init();
    });
}

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
///
/// The session is spawned asynchronously by
/// `SpawnConversationSessionsAutomation`, so this poll-waits briefly for it
/// to appear before failing.
async fn find_session_for_conversation(
    store: &Arc<dyn Store>,
    conversation_id: &ConversationId,
) -> SessionId {
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
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
        if let Some((id, _)) = matching.pop() {
            return id;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no session for conversation {conversation_id} appeared in time");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Poll until a session whose id is NOT one of `exclude` shows up linked to
/// `conversation_id`. Used after `/resume` to wait for the
/// `SpawnConversationSessionsAutomation` to spawn the new session — the
/// previously-active session is still in the store with status Running, so a
/// naive `find_session_for_conversation` call would return it immediately.
async fn find_new_session_for_conversation(
    store: &Arc<dyn Store>,
    conversation_id: &ConversationId,
    exclude: &SessionId,
) -> SessionId {
    use hydra_common::api::v1::sessions::SearchSessionsQuery;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .expect("list sessions");
        let mut matching: Vec<(SessionId, Session)> = sessions
            .into_iter()
            .filter_map(|(id, v)| {
                if &id != exclude && v.item.conversation_id() == Some(conversation_id) {
                    Some((id, v.item))
                } else {
                    None
                }
            })
            .collect();
        matching.sort_by_key(|(_, s)| s.creation_time);
        if let Some((id, _)) = matching.pop() {
            return id;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no new session for conversation {conversation_id} (excluding {exclude}) appeared in time"
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Poll the events endpoint until a `Resumed` event appears, then return its
/// session_id.
async fn poll_resumed_session_id(
    client: &reqwest::Client,
    base_url: &str,
    conversation_id: &ConversationId,
) -> Option<SessionId> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events: Vec<ConversationEvent> = client
            .get(format!(
                "{base_url}/v1/conversations/{conversation_id}/events"
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        if let Some(id) = events.iter().rev().find_map(|e| match e {
            ConversationEvent::Resumed { session_id, .. } => Some(session_id.clone()),
            _ => None,
        }) {
            return Some(id);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
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

    // Under the trigger-on-transition design, the conversation flip Active →
    // Idle is owned by `SpawnConversationSessionsAutomation` and fires off
    // the session's terminal transition. Simulate the job engine marking the
    // session `Complete` after the worker exited on Suspending.
    mark_session_terminal(&state, &session_id, TaskStatus::Complete).await;

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
async fn resume_after_idle_replays_full_event_log_in_catch_up() -> anyhow::Result<()> {
    // Regression test for the "agent forgets prior context on resume" bug.
    // The fake worker passes the real `resume_from_event_index` value that
    // the resumed session was created with; the server must respond with the
    // full event log (UserMessage + AssistantMessage + Suspending) regardless,
    // so the new worker can rebuild context from it.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
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
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);

    // Drive the session to a terminal status so the automation flips Active
    // → Idle (the WS-close / Suspending sync flip is gone).
    mark_session_terminal(&state, &initial_session_id, TaskStatus::Complete).await;

    wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;

    // Phase 2: resume the conversation. A new session is created with
    // conversation_resume_from set to the pre-Resumed event count.
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

    // Find the Resumed event's session_id — that's the new session. The
    // automation spawns the new session and appends `Resumed`
    // asynchronously, so poll until it shows up.
    let resumed_session_id = poll_resumed_session_id(&client, &server.base_url(), &conversation_id)
        .await
        .expect("expected a Resumed event after /resume");
    assert_ne!(
        resumed_session_id, initial_session_id,
        "resume must create a brand-new session"
    );

    // Inspect the resumed session's resume hint: conversation_resume_from
    // should equal the event count captured before /resume appended Resumed.
    // At suspend we had: UserMessage, AssistantMessage, Suspending = 3 events.
    let resumed_session = store.get_session(&resumed_session_id, false).await?.item;
    assert!(
        resumed_session.is_interactive(),
        "resumed session must be interactive"
    );
    assert_eq!(
        resumed_session.mode.conversation_resume_from(),
        Some(3),
        "conversation_resume_from should equal pre-Resumed event count"
    );

    // Phase 3: connect as the new worker, passing the real worker's value of
    // resume_from_event_index. The server must still return the FULL event
    // log so the new worker can reconstruct context from it.
    let mut ws2 = connect_relay(&server.base_url(), &resumed_session_id).await?;
    let catch_up = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: resumed_session.mode.conversation_resume_from(),
        },
    )
    .await?;
    assert!(
        catch_up.session_state.is_none(),
        "the server no longer uses session_state; catch-up should report None, got {:?}",
        catch_up.session_state
    );

    // Expected sequence: UserMessage, AssistantMessage, Suspending, Resumed.
    assert_eq!(
        catch_up.events.len(),
        4,
        "Fresh catch-up must return the full event log; got {:?}",
        catch_up.events
    );
    assert!(
        matches!(
            &catch_up.events[0],
            ConversationEvent::UserMessage { content, .. } if content == "first user message"
        ),
        "event[0] should be the initial user message, got {:?}",
        catch_up.events[0]
    );
    assert!(
        matches!(
            &catch_up.events[1],
            ConversationEvent::AssistantMessage { content, .. } if content == "first agent reply"
        ),
        "event[1] should be the prior assistant reply, got {:?}",
        catch_up.events[1]
    );
    assert!(
        matches!(catch_up.events[2], ConversationEvent::Suspending { .. }),
        "event[2] should be Suspending, got {:?}",
        catch_up.events[2]
    );
    assert!(
        matches!(catch_up.events[3], ConversationEvent::Resumed { .. }),
        "event[3] should be Resumed, got {:?}",
        catch_up.events[3]
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
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    assert!(
        new_session.mode.conversation_resume_from().is_some(),
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
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
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

    // Drive the session terminal so the automation flips the conversation
    // Active → Idle.
    mark_session_terminal(&state, &initial_session_id, TaskStatus::Complete).await;

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

    // Phase 6: connect fake-worker #2 and handshake as Fresh with the
    // resume_from_event_index value the real worker would send (the
    // conversation_resume_from on the resumed session). The server must
    // ignore that value and return the full prior event log so the new
    // worker can reconstruct context.
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up2 = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: new_session.mode.conversation_resume_from(),
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
async fn close_then_resume_replays_full_history_with_no_session_state() -> anyhow::Result<()> {
    // Regression test for the user-reported "agent forgets context on resume"
    // bug. When the user /closes a chat, the worker is killed without
    // uploading session_state. After /resume, the new worker must still
    // receive the full prior event log in its catch-up so it can rebuild
    // context — even though no session_state was ever persisted.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    // Phase 1: create the conversation and exchange one full user/assistant turn.
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
    // Drop the WS without sending Suspending/SessionStateUpload — simulating
    // the worker being killed by /close.
    drop(ws);

    // Phase 2: /close the conversation. No session_state is uploaded.
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

    // Sanity: confirm no session_state was persisted by the prior worker.
    let stored_state = store
        .get_conversation_session_state(&conversation_id)
        .await?;
    assert!(
        stored_state.is_none(),
        "no SessionStateUpload was ever sent, so session_state must be None"
    );

    // Phase 3: /resume creates a new session with conversation_resume_from
    // set to the event count snapshotted at /resume time.
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
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    let events_len_at_resume = new_session
        .mode
        .conversation_resume_from()
        .expect("conversation_resume_from must be set on a session created by /resume");

    // Phase 4: connect fake-worker #2 with the real worker's
    // resume_from_event_index value. The server must return the FULL event
    // log including the prior UserMessage + AssistantMessage so the new
    // worker can rebuild context.
    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: Some(events_len_at_resume),
        },
    )
    .await?;

    assert!(
        catch_up.session_state.is_none(),
        "no session_state was uploaded, so catch-up must report None"
    );

    // Expected sequence: UserMessage("first user message"), AssistantMessage("first agent reply"), Closed, Resumed.
    assert_eq!(
        catch_up.events.len(),
        4,
        "expected the full event log on resume, got {:?}",
        catch_up.events
    );
    assert!(
        matches!(
            &catch_up.events[0],
            ConversationEvent::UserMessage { content, .. } if content == "first user message"
        ),
        "event[0] should be the initial user message, got {:?}",
        catch_up.events[0]
    );
    assert!(
        matches!(
            &catch_up.events[1],
            ConversationEvent::AssistantMessage { content, .. } if content == "first agent reply"
        ),
        "event[1] should be the prior assistant reply (the key context the \
         worker would otherwise have lost), got {:?}",
        catch_up.events[1]
    );
    assert!(
        matches!(catch_up.events[2], ConversationEvent::Closed { .. }),
        "event[2] should be Closed, got {:?}",
        catch_up.events[2]
    );
    assert!(
        matches!(catch_up.events[3], ConversationEvent::Resumed { .. }),
        "event[3] should be Resumed, got {:?}",
        catch_up.events[3]
    );

    Ok(())
}

#[tokio::test]
async fn resume_after_session_state_upload_delivers_payload_in_catch_up() -> anyhow::Result<()> {
    // End-to-end smoke test for the transcript-based ("primary") resume
    // path. Worker #1 uploads a structured `SessionStatePayload::V1` with a
    // transcript blob; after /resume, worker #2's catch-up must carry the
    // same bytes back so it can write the transcript to disk and invoke
    // `claude --resume`. Verifies the wire envelope is preserved and the
    // payload survives the store round-trip.
    init_test_tracing();
    let (state, store) = state_with_idle_timeout_secs(2);
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

    // Worker #1: connect, exchange one turn, then suspend with an upload.
    let mut ws = connect_relay(&server.base_url(), &initial_session_id).await?;
    let _ = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "reply".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;

    let transcript_bytes = b"{\"type\":\"summary\",\"text\":\"prior turn\"}\n".to_vec();
    let payload = SessionStatePayload::V1 {
        session_id: "claude-session-uuid-1".to_string(),
        transcript: Some(transcript_bytes.clone()),
    };
    let payload_bytes = serde_json::to_vec(&payload)?;

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
            data: payload_bytes.clone(),
        },
    )
    .await?;
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);

    // Drive the session terminal so the automation flips Active → Idle.
    mark_session_terminal(&state, &initial_session_id, TaskStatus::Complete).await;

    wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;

    // The store must have persisted the exact payload bytes.
    let stored_state = store
        .get_conversation_session_state(&conversation_id)
        .await?;
    assert_eq!(
        stored_state.as_deref(),
        Some(payload_bytes.as_slice()),
        "SessionStateUpload should be persisted byte-for-byte"
    );

    // Resume the conversation and connect worker #2.
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

    // After /resume the previous session is in the store with status
    // Complete (the test drove it terminal above). `find_new_session_for_conversation`
    // polls for the new session that the automation spawns.
    let new_session_id =
        find_new_session_for_conversation(&store, &conversation_id, &initial_session_id).await;
    assert_ne!(new_session_id, initial_session_id);
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );

    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up = worker_handshake(
        &mut ws2,
        WorkerConnect::Fresh {
            resume_from_event_index: new_session.mode.conversation_resume_from(),
        },
    )
    .await?;

    let returned_bytes = catch_up
        .session_state
        .expect("server must return the persisted session_state for Fresh resume");
    assert_eq!(
        returned_bytes, payload_bytes,
        "catch-up must echo the exact payload bytes the prior worker uploaded"
    );

    // Sanity-check the bytes still parse as the original payload — i.e.
    // nothing en-route mutated them.
    let round_trip: SessionStatePayload = serde_json::from_slice(&returned_bytes)?;
    match round_trip {
        SessionStatePayload::V1 {
            session_id,
            transcript,
        } => {
            assert_eq!(session_id, "claude-session-uuid-1");
            assert_eq!(transcript, Some(transcript_bytes));
        }
    }

    Ok(())
}

#[tokio::test]
async fn reconnecting_handshake_does_not_return_session_state() -> anyhow::Result<()> {
    // `Reconnecting` is a mid-session WS reconnect by the same live worker,
    // not a fresh resume. session_state is irrelevant on this path and must
    // be omitted so we don't waste bandwidth shipping the transcript to a
    // worker that already has it in Claude's process memory.
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

    // Persist some session_state so the Fresh path would otherwise return it.
    store
        .store_conversation_session_state(&conversation_id, b"opaque-bytes".to_vec())
        .await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let catch_up = worker_handshake(
        &mut ws,
        WorkerConnect::Reconnecting {
            last_received_event_index: 0,
        },
    )
    .await?;
    assert!(
        catch_up.session_state.is_none(),
        "Reconnecting handshakes should not carry session_state, got {:?}",
        catch_up.session_state
    );

    Ok(())
}

#[tokio::test]
async fn conversation_marked_idle_when_companion_session_reaches_terminal_status()
-> anyhow::Result<()> {
    // Covers the unhappy path of the idle/resume flow: a worker drops the
    // WebSocket without sending Suspending (e.g. a crash). Under the
    // trigger-on-transition design, the WS-close itself no longer flips the
    // conversation Idle — that's owned by
    // `SpawnConversationSessionsAutomation`, which fires when the companion
    // session reaches a terminal status (Complete / Failed). In production the
    // session is driven terminal by the job engine (worker exit) and the
    // background `monitor_running_sessions` worker; here we simulate that step
    // directly.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
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
    // Drop the WS without sending Suspending — bare crash.
    drop(ws);

    // Simulate the job engine marking the session Failed once the worker is
    // gone. The automation listens on SessionUpdated and flips the
    // conversation to Idle.
    mark_session_terminal(&state, &session_id, TaskStatus::Failed).await;

    let idle = wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;
    assert_eq!(idle.status, ConversationStatus::Idle);

    // Sanity-check the store mirror too.
    let domain = store.get_conversation(&conversation_id, false).await?.item;
    assert_eq!(domain.status, DomainConversationStatus::Idle);

    Ok(())
}

/// Register an agent under `name` with a prompt document whose body is
/// `prompt_body`. The companion document is stored at
/// `/agents/<name>/prompt.md`, matching the convention used by the
/// per-conversation tests in `app::conversations`.
async fn register_agent_with_prompt_body(
    store: &Arc<dyn Store>,
    name: &str,
    prompt_body: &str,
) -> anyhow::Result<()> {
    let prompt_path = format!("/agents/{name}/prompt.md");
    let agent = Agent::new(
        name.to_string(),
        prompt_path.clone(),
        None,
        1,
        1,
        false,
        false,
        vec![],
    );
    store.add_agent(agent).await?;

    let doc = Document {
        title: format!("{name} prompt"),
        body_markdown: prompt_body.to_string(),
        path: Some(
            prompt_path
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid prompt path: {e:?}"))?,
        ),
        created_by: None,
        deleted: false,
    };
    store.add_document(doc, &ActorRef::test()).await?;
    Ok(())
}

/// Regression test for the chat-agent-prompt-prepend race fixed by routing
/// the first user message through `AppState::send_message` (which appends
/// the event AND forwards it through the relay) instead of inlining the
/// `append_conversation_event_with_actor` call inside `create_conversation`.
///
/// Pre-fix, `POST /v1/conversations { message, agent_name }` only appended
/// the `UserMessage` to the event log; the worker observed it via
/// `feed_catch_up` once it connected. But `SpawnConversationSessionsAutomation`
/// fires on `ConversationCreated`, so the worker could race ahead of the
/// append and catch up with zero events — `PromptPrepend` then never saw a
/// first `UserMessage` and the agent prompt was silently dropped on the
/// first turn.
///
/// With the fix, `create_conversation` calls `self.send_message(...)` for
/// the first message. `send_message` both appends to the log AND attempts
/// the relay path, so the first message reaches the worker via *whichever*
/// branch (catch-up or relay) wins. Either path is sufficient for the
/// `PromptPrepend` middleware to fire.
///
/// The test passes if the worker observes the first user message via
/// EITHER `WorkerCatchUp.events` OR a `ServerMessage::Event` on the relay,
/// which mirrors how the bug actually manifests in production.
#[tokio::test]
async fn create_conversation_with_first_message_reaches_worker_via_relay() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    register_agent_with_prompt_body(&store, "chat", "you are a chat agent").await?;

    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello".to_string()),
            agent_name: Some("chat".to_string()),
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();

    let session_id = find_session_for_conversation(&store, &conversation_id).await;

    // Verify that the chat agent's prompt actually flowed through to the
    // spawned session — the SpawnConversationSessionsAutomation should have
    // resolved the prompt document body into the session's `prompt` field.
    let session = store.get_session(&session_id, false).await?.item;
    assert_eq!(
        session.resolved_prompt(),
        "you are a chat agent",
        "session prompt should be the resolved agent prompt body"
    );

    // Connect as a fake worker, handshake, and observe the first user
    // message. It may arrive via catch-up (worker connected after the
    // message was appended) or via the relay (worker connected before the
    // message landed). Either is sufficient for PromptPrepend to fire.
    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let catch_up = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;

    let saw_in_catch_up = catch_up.events.iter().any(|e| {
        matches!(
            e,
            ConversationEvent::UserMessage { content, .. } if content == "hello"
        )
    });

    let saw_via_relay = if saw_in_catch_up {
        false
    } else {
        let drained = drain_server_messages(&mut ws, Duration::from_secs(2)).await?;
        drained.iter().any(|m| {
            matches!(
                m,
                ServerMessage::Event {
                    event: ConversationEvent::UserMessage { content, .. },
                } if content == "hello"
            )
        })
    };

    assert!(
        saw_in_catch_up || saw_via_relay,
        "worker must observe the first user message via either catch-up or relay; \
         catch-up events were: {:?}",
        catch_up.events
    );

    Ok(())
}

/// Phase C step 7 dual-write regression test: a complete chat lifecycle
/// (user → assistant → suspend → resume → close → reopen) must produce
/// matching rows in `session_events_v2` and `session_state_v2` alongside the
/// existing `conversation_events_v2` writes.
#[tokio::test]
async fn dual_write_replicates_chat_lifecycle_to_session_logs() -> anyhow::Result<()> {
    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    init_test_tracing();
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    // Phase 1: create conversation with a first user message.
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
    let s1 = find_session_for_conversation(&store, &conversation_id).await;

    // Phase 2: connect as worker #1, exchange one assistant turn, then
    // suspend with a session-state upload.
    let mut ws = connect_relay(&server.base_url(), &s1).await?;
    let _ = worker_handshake(
        &mut ws,
        WorkerConnect::Fresh {
            resume_from_event_index: None,
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: ConversationEvent::AssistantMessage {
                content: "reply 1".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    // A follow-up user message while the worker is connected — this is the
    // `send_message` dual-write path inside `app/conversations.rs`.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "follow-up".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let _ = drain_server_messages(&mut ws, Duration::from_secs(1)).await?;
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
    let upload_bytes = b"opaque-state-v1".to_vec();
    send_worker_message(
        &mut ws,
        WorkerMessage::SessionStateUpload {
            data: upload_bytes.clone(),
        },
    )
    .await?;
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);
    mark_session_terminal(&state, &s1, TaskStatus::Complete).await;

    wait_for_status(
        &client,
        &server.base_url(),
        &conversation_id,
        ConversationStatus::Idle,
    )
    .await?;

    // Phase 3: /resume — automation spawns session #2 and appends Resumed.
    let _ = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/resume",
            server.base_url()
        ))
        .send()
        .await?
        .error_for_status()?;
    let s2 = find_new_session_for_conversation(&store, &conversation_id, &s1).await;

    // Phase 4: /close — emits Closed lifecycle event.
    let _ = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url()
        ))
        .send()
        .await?
        .error_for_status()?;

    // ---- Assertions ----

    // ConversationEvent log (the existing source of truth).
    let convo_events = store.get_conversation_events(&conversation_id).await?;
    let convo_user: usize = convo_events
        .iter()
        .filter(|e| {
            matches!(
                e.item,
                crate::domain::conversations::ConversationEvent::UserMessage { .. }
            )
        })
        .count();
    let convo_assistant: usize = convo_events
        .iter()
        .filter(|e| {
            matches!(
                e.item,
                crate::domain::conversations::ConversationEvent::AssistantMessage { .. }
            )
        })
        .count();
    let convo_suspending: usize = convo_events
        .iter()
        .filter(|e| {
            matches!(
                e.item,
                crate::domain::conversations::ConversationEvent::Suspending { .. }
            )
        })
        .count();
    let convo_resumed: usize = convo_events
        .iter()
        .filter(|e| {
            matches!(
                e.item,
                crate::domain::conversations::ConversationEvent::Resumed { .. }
            )
        })
        .count();
    let convo_closed: usize = convo_events
        .iter()
        .filter(|e| {
            matches!(
                e.item,
                crate::domain::conversations::ConversationEvent::Closed { .. }
            )
        })
        .count();
    assert_eq!(convo_user, 2, "convo UserMessage count");
    assert_eq!(convo_assistant, 1, "convo AssistantMessage count");
    assert_eq!(convo_suspending, 1, "convo Suspending count");
    assert_eq!(convo_resumed, 1, "convo Resumed count");
    assert_eq!(convo_closed, 1, "convo Closed count");

    // SessionEvent log: the dual-write puts each ConversationEvent on the
    // active session. UserMessage("hello") and the assistant reply + the
    // Suspending all happen on s1; UserMessage("follow-up") may land on s1
    // (relay still connected) or s2 depending on relay-map timing — either
    // is acceptable. Resumed lands on s2. Closed is appended after /close
    // and lands on the latest session for the conversation (s2).
    let s1_events: Vec<DomainSessionEvent> = store
        .get_session_events(&s1)
        .await?
        .into_iter()
        .map(|v| v.item)
        .collect();
    let s2_events: Vec<DomainSessionEvent> = store
        .get_session_events(&s2)
        .await?
        .into_iter()
        .map(|v| v.item)
        .collect();
    let total_user: usize = s1_events
        .iter()
        .chain(s2_events.iter())
        .filter(|e| matches!(e, DomainSessionEvent::UserMessage { .. }))
        .count();
    let total_assistant: usize = s1_events
        .iter()
        .chain(s2_events.iter())
        .filter(|e| matches!(e, DomainSessionEvent::AssistantMessage { .. }))
        .count();
    let total_suspending: usize = s1_events
        .iter()
        .chain(s2_events.iter())
        .filter(|e| matches!(e, DomainSessionEvent::Suspending { .. }))
        .count();
    let total_resumed: usize = s1_events
        .iter()
        .chain(s2_events.iter())
        .filter(|e| matches!(e, DomainSessionEvent::Resumed { .. }))
        .count();
    let total_closed: usize = s1_events
        .iter()
        .chain(s2_events.iter())
        .filter(|e| matches!(e, DomainSessionEvent::Closed { .. }))
        .count();
    // The initial UserMessage from POST /v1/conversations is appended
    // *before* `SpawnConversationSessionsAutomation` creates s1, so the
    // dual-write at that instant has no session to attach to and is
    // skipped (logged at warn-level by the helper). That gap is filled by
    // step 8's historical backfill, which is out of scope for this PR.
    // The follow-up UserMessage is the one we expect to see here.
    assert_eq!(total_user, 1, "dual-write UserMessage count (s1+s2)");
    assert_eq!(
        total_assistant, 1,
        "dual-write AssistantMessage count (s1+s2)"
    );
    assert_eq!(total_suspending, 1, "dual-write Suspending count (s1+s2)");
    assert_eq!(total_resumed, 1, "dual-write Resumed count (s1+s2)");
    assert_eq!(total_closed, 1, "dual-write Closed count (s1+s2)");

    // The Resumed event lands on the new session, carrying the prior id.
    let resumed_from = s2_events.iter().find_map(|e| match e {
        DomainSessionEvent::Resumed {
            from_session_id, ..
        } => Some(from_session_id.clone()),
        _ => None,
    });
    assert_eq!(
        resumed_from.as_ref(),
        Some(&s1),
        "SessionEvent::Resumed on s2 must carry s1 as from_session_id"
    );

    // session_state dual-write: the upload from worker #1 must appear under
    // both `conversation_session_state` (old key) and `session_state` (new
    // key) for s1.
    let conv_state = store
        .get_conversation_session_state(&conversation_id)
        .await?;
    let session_state = store.get_session_state(&s1).await?;
    assert_eq!(
        conv_state.as_deref(),
        Some(upload_bytes.as_slice()),
        "conversation_session_state must contain the uploaded bytes"
    );
    assert_eq!(
        session_state.as_deref(),
        Some(upload_bytes.as_slice()),
        "session_state must contain the same bytes for s1 (dual-write)"
    );

    Ok(())
}

/// `GET /v1/sessions/:session_id/events` returns the appended `SessionEvent`
/// log as JSON and 404s for unknown session ids. Mirrors
/// `get_conversation_events_returns_events` in `test/conversations.rs`.
#[tokio::test]
async fn get_session_events_route_returns_events() -> anyhow::Result<()> {
    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    init_test_tracing();
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    // Drive a session into existence via the conversation create path, then
    // append events directly to the session-event log so the test is
    // independent of dual-write timing.
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("seed".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = created.conversation_id.clone();
    let session_id = find_session_for_conversation(&store, &conversation_id).await;

    let actor = ActorRef::test();
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::UserMessage {
                content: "hello agent".to_string(),
                timestamp: chrono::Utc::now(),
            },
            &actor,
        )
        .await?;
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::AssistantMessage {
                content: "hi there".to_string(),
                timestamp: chrono::Utc::now(),
            },
            &actor,
        )
        .await?;

    let response = client
        .get(format!(
            "{}/v1/sessions/{session_id}/events",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let events: Vec<serde_json::Value> = response.json().await?;
    let user = events
        .iter()
        .find(|e| e["type"] == "user_message")
        .expect("session events must contain a user_message");
    assert_eq!(user["content"], "hello agent");
    let assistant = events
        .iter()
        .find(|e| e["type"] == "assistant_message")
        .expect("session events must contain an assistant_message");
    assert_eq!(assistant["content"], "hi there");

    // Unknown session id must 404, not 500.
    let unknown = SessionId::new();
    let response = client
        .get(format!(
            "{}/v1/sessions/{unknown}/events",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}
