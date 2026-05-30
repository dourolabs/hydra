//! End-to-end integration tests for the chat lifecycle: create → message →
//! idle-suspend → resume → message → close.
//!
//! These tests act as a fake worker that connects to `/v1/sessions/:id/events`
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
            Conversation, ConversationStatus, CreateConversationRequest, SendMessageRequest,
            ServerMessage, SessionStatePayload, WorkerMessage,
        },
        sessions::{ListSessionsResponse, SessionEvent, WorkerContext},
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

/// Poll the sessions list until a session other than `exclude` appears for
/// `conversation_id`, then return its id.
async fn poll_resumed_session_id(
    client: &reqwest::Client,
    base_url: &str,
    conversation_id: &ConversationId,
    exclude: &SessionId,
) -> Option<SessionId> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let response: Option<ListSessionsResponse> = client
            .get(format!(
                "{base_url}/v1/sessions?conversation_id={conversation_id}"
            ))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok();
        if let Some(list) = response {
            if let Some(record) = list
                .sessions
                .into_iter()
                .filter(|s| &s.session_id != exclude)
                .max_by_key(|s| s.session.creation_time)
            {
                return Some(record.session_id);
            }
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
    let uri = format!("{ws_url}/v1/sessions/{session_id}/events");
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

/// Wire-shape captured at the end of Phase 1 for the test helpers'
/// purposes: the events the server delivered (transcript for
/// fresh-with-prior-session, catch-up for reconnecting, empty
/// otherwise) plus the raw text payload.
#[derive(Debug, Default)]
struct HandshakeOutcome {
    events: Vec<SessionEvent>,
    raw: String,
}

/// Complete a Phase-1 handshake under the WS-only protocol, accepting
/// either a `Fresh` or a `Reconnecting` opener. For `Fresh`, the server
/// replies `ResumeContext` (events vec is empty) — and the helper does
/// NOT auto-advance to Ready, since tests that drive the relay want to
/// control that step themselves. For `Reconnecting`, the server replies
/// `CatchUp { events }`.
async fn worker_handshake(
    ws: &mut WsStream,
    opener: WorkerMessage,
) -> anyhow::Result<HandshakeOutcome> {
    let opener_was_fresh = matches!(opener, WorkerMessage::Fresh);
    let connect_json = serde_json::to_string(&opener)?;
    ws.send(tungstenite::Message::Text(connect_json)).await?;
    let msg = ws
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("ws closed before Phase-1 reply"))??;
    let raw = match msg {
        tungstenite::Message::Text(t) => t,
        other => anyhow::bail!("expected text Phase-1 reply, got {other:?}"),
    };
    let parsed: ServerMessage = serde_json::from_str(&raw)?;
    let events = match &parsed {
        ServerMessage::ResumeContext {
            prior_session_id, ..
        } => {
            if opener_was_fresh && prior_session_id.is_some() {
                let prior = prior_session_id.clone().unwrap();
                let req = serde_json::to_string(&WorkerMessage::RequestTranscript {
                    prior_session_id: prior,
                })?;
                ws.send(tungstenite::Message::Text(req)).await?;
                let m = ws
                    .next()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("ws closed before Transcript"))??;
                let t = match m {
                    tungstenite::Message::Text(t) => t,
                    other => anyhow::bail!("expected text Transcript, got {other:?}"),
                };
                match serde_json::from_str::<ServerMessage>(&t)? {
                    ServerMessage::Transcript { events } => events,
                    other => anyhow::bail!("expected Transcript, got {other:?}"),
                }
            } else {
                Vec::new()
            }
        }
        ServerMessage::CatchUp { events } => events.iter().map(|e| e.event.clone()).collect(),
        other => anyhow::bail!("expected ResumeContext or CatchUp, got {other:?}"),
    };
    Ok(HandshakeOutcome { events, raw })
}

/// Raw-text variant of [`worker_handshake`] that still returns the
/// parsed events vec but additionally gives back the serialised wire
/// text so tests can pattern-match on JSON shape.
async fn worker_handshake_raw(
    ws: &mut WsStream,
    opener: WorkerMessage,
) -> anyhow::Result<HandshakeOutcome> {
    worker_handshake(ws, opener).await
}

/// Send `WorkerMessage::Ready` and read the next `ServerMessage`,
/// returning it if it's `FirstMessage { agent_prompt, user_message }`.
///
/// Returns `Ok(None)` if the server stashed the `agent_prompt` as
/// `pending_first_message` (no queued UserMessage yet) and no
/// `FirstMessage` arrives within `timeout`.
async fn send_ready_and_await_first_message(
    ws: &mut WsStream,
    timeout: Duration,
) -> anyhow::Result<Option<(String, String)>> {
    send_worker_message(ws, WorkerMessage::Ready).await?;
    let msgs = drain_server_messages(ws, timeout).await?;
    for m in msgs {
        if let ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        } = m
        {
            return Ok(Some((agent_prompt, user_message)));
        }
    }
    Ok(None)
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
    assert_eq!(
        context.mode_kind,
        hydra_common::sessions::SessionModeKind::Interactive
    );
    assert_eq!(
        context.idle_timeout_secs,
        Some(2),
        "WorkerContext.idle_timeout_secs must surface the configured \
         interactive_idle_timeout_secs so the worker idle-timer fires at \
         the test-tuned interval"
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
    let catch_up = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    // Under the queue-and-deliver design, the user message sent before the
    // worker connected lives on the chat_relay's pending queue, not on the
    // session log — catch-up reflects only the session log, so it's empty
    // here. The queued message is delivered after Ready as the folded
    // `FirstMessage.user_message`.
    assert_eq!(
        catch_up.events.len(),
        0,
        "catch-up reflects session-log state; pre-connect events arrive after Ready"
    );
    send_worker_message(&mut ws, WorkerMessage::Ready).await?;
    let live = drain_server_messages(&mut ws, Duration::from_millis(500)).await?;
    let saw_hello = live.iter().any(|m| match m {
        ServerMessage::FirstMessage { user_message, .. } => user_message == "hello",
        ServerMessage::Event {
            event: SessionEvent::UserMessage { content, .. },
            ..
        } => content == "hello",
        _ => false,
    });
    assert!(
        saw_hello,
        "drained pending UserMessage must be delivered to the worker after Ready, got {live:?}"
    );

    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::Suspending {
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

    // Verify session_state was persisted via the store API.
    let stored_state = store.get_session_state(&session_id).await?;
    assert_eq!(
        stored_state.as_deref(),
        Some(b"claude-session-abc".as_slice()),
        "SessionStateUpload payload should be persisted on the producing session"
    );

    Ok(())
}

#[tokio::test]
async fn resume_after_idle_replays_full_event_log_in_catch_up() -> anyhow::Result<()> {
    // Regression test for the "agent forgets prior context on resume" bug,
    // adapted to the WS-only Phase 1 split: the new worker opens `Fresh`,
    // the server replies `ResumeContext { prior_session_id }`, the worker
    // asks for the prior transcript via `RequestTranscript`, and the
    // server must return the FULL prior-session event log
    // (UserMessage + AssistantMessage + Suspending). The `Resumed` marker
    // now lives on the new session's own log (emitted by the worker after
    // a successful `try_materialize`), so it does NOT show up in this
    // transcript.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    // Step 1: create, exchange messages, then simulate idle-suspend.
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
    let _catch_up = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;

    // Simulate a partial agent reply being recorded before the worker suspends.
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
                content: "first agent reply".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::Suspending {
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

    // Step 2: implicit resume — sending a message to a non-Active
    // conversation flips status back to Active and the automation spawns a
    // new session with conversation_resume_from set to the pre-Resumed
    // event count. The trigger message gets queued and is delivered to
    // the new worker folded into FirstMessage after the Phase 1
    // handshake completes below.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "resume trigger".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let active: Conversation = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(active.status, ConversationStatus::Active);

    // The automation spawns a new session for the resumed conversation
    // asynchronously, so poll until it shows up.
    let resumed_session_id = poll_resumed_session_id(
        &client,
        &server.base_url(),
        &conversation_id,
        &initial_session_id,
    )
    .await
    .expect("expected a new session after implicit resume via /messages");
    assert_ne!(
        resumed_session_id, initial_session_id,
        "resume must create a brand-new session"
    );

    let resumed_session = store.get_session(&resumed_session_id, false).await?.item;
    assert!(
        resumed_session.is_interactive(),
        "resumed session must be interactive"
    );
    assert_eq!(
        resumed_session.resumed_from.as_ref(),
        Some(&initial_session_id),
        "resumed_from must point at the prior session"
    );

    // Step 3: connect as the new worker, passing the real worker's value of
    // resume_from_event_index. The server must still return the FULL event
    // log so the new worker can reconstruct context from it.
    let mut ws2 = connect_relay(&server.base_url(), &resumed_session_id).await?;
    let catch_up = worker_handshake(&mut ws2, WorkerMessage::Fresh).await?;
    // Expected sequence: UserMessage, AssistantMessage, Suspending. The
    // `Resumed` marker lives on the NEW session (the worker emits it
    // post-`try_materialize`); the prior-session transcript does not
    // contain it.
    assert_eq!(
        catch_up.events.len(),
        3,
        "RequestTranscript should return the prior session's event log; \
         got {:?}",
        catch_up.events
    );
    assert!(
        matches!(
            &catch_up.events[0],
            SessionEvent::UserMessage { content, .. } if content == "first user message"
        ),
        "event[0] should be the initial user message, got {:?}",
        catch_up.events[0]
    );
    assert!(
        matches!(
            &catch_up.events[1],
            SessionEvent::AssistantMessage { content, .. } if content == "first agent reply"
        ),
        "event[1] should be the prior assistant reply, got {:?}",
        catch_up.events[1]
    );
    assert!(
        matches!(catch_up.events[2], SessionEvent::Suspending { .. }),
        "event[2] should be Suspending, got {:?}",
        catch_up.events[2]
    );

    // Send Ready so the relay phase advances out of Negotiating; the
    // server folds the prior session's "first user message" into a
    // FirstMessage and any subsequent in-flight POST arrives as Event.
    send_worker_message(&mut ws2, WorkerMessage::Ready).await?;
    let _ = drain_server_messages(&mut ws2, Duration::from_millis(200)).await?;

    // Step 4: send a new user message and verify the worker receives it via
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
                event: SessionEvent::UserMessage { content, .. },
                ..
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
    let initial_session_id = find_session_for_conversation(&store, &conversation_id).await;

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

    // Resume implicitly by sending a message on a non-Active conversation;
    // `send_message` flips the conversation back to Active and the
    // automation spawns a new session.
    let sessions_before: ListSessionsResponse = client
        .get(format!("{}/v1/sessions", server.base_url()))
        .send()
        .await?
        .json()
        .await?;
    let count_before = sessions_before.sessions.len();

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

    // Re-fetch to confirm the status flip landed.
    let active: Conversation = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(active.status, ConversationStatus::Active);

    // Wait for the spawn-conversation-sessions automation to produce the
    // new session.
    let new_session_id = poll_resumed_session_id(
        &client,
        &server.base_url(),
        &conversation_id,
        &initial_session_id,
    )
    .await
    .expect("expected a new session after implicit resume via /messages");

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

    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    assert_eq!(
        new_session.resumed_from.as_ref(),
        Some(&initial_session_id),
        "resumed_from must point at the prior session"
    );

    Ok(())
}

#[tokio::test]
async fn resume_replays_full_history_in_catch_up_and_forwards_only_new_message()
-> anyhow::Result<()> {
    // Regression guard for the close/resume flow, adapted to the WS-only
    // Phase 1 split:
    //   1. After the implicit resume triggered by `POST /messages`, the
    //      freshly-spawned worker opens `Fresh`, sees `ResumeContext {
    //      prior_session_id }`, and requests the prior session's
    //      transcript. That `Transcript { events }` must include the
    //      full prior conversation history (user messages and assistant
    //      replies plus the Suspending + Closed lifecycle markers) so
    //      the agent can reconstruct context.
    //   2. After the new worker re-attaches, the relay must forward ONLY
    //      the next new user message — not replay msg1/msg2/msg3 — so the
    //      assistant does not generate redundant replies for earlier turns.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    let msg1 = "My name is Alice. What's 2+2?";
    let msg2 = "I'm a software engineer. What's 3+3?";
    let msg3 = "I work on Rust projects. What's 4+4?";
    let msg4 = "What's my name and what do I work on?";

    // Step 1: create the conversation with msg1; this also creates the
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

    // Step 2: connect fake-worker #1 and exchange three full turns.
    let mut ws1 = connect_relay(&server.base_url(), &initial_session_id).await?;
    let catch_up = worker_handshake(&mut ws1, WorkerMessage::Fresh).await?;
    // The queue-and-deliver model defers the dual-write of pre-connect
    // events until the worker connects, so catch-up (which reads the
    // session log) is empty here; msg1 arrives folded into FirstMessage
    // after the worker sends Ready.
    assert_eq!(
        catch_up.events.len(),
        0,
        "fresh worker catch-up reflects the session log; pre-connect events arrive after Ready"
    );
    send_worker_message(&mut ws1, WorkerMessage::Ready).await?;
    let drained = drain_server_messages(&mut ws1, Duration::from_millis(500)).await?;
    assert!(
        drained.iter().any(|m| match m {
            ServerMessage::FirstMessage { user_message, .. } => user_message == msg1,
            ServerMessage::Event {
                event: SessionEvent::UserMessage { content, .. },
                ..
            } => content == msg1,
            _ => false,
        }),
        "msg1 should arrive folded into FirstMessage after Ready, got {drained:?}"
    );

    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
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
            ServerMessage::Event { event: SessionEvent::UserMessage { content, .. }, .. }
                if content == msg2
        )),
        "worker #1 should receive msg2 via the relay, got {forwarded:?}"
    );
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
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
            ServerMessage::Event { event: SessionEvent::UserMessage { content, .. }, .. }
                if content == msg3
        )),
        "worker #1 should receive msg3 via the relay, got {forwarded:?}"
    );
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
                content: "reply 3".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;

    // Step 3: suspend worker #1 (Suspending + SessionStateUpload), close the
    // WS, and wait for the conversation to settle into Idle.
    send_worker_message(
        &mut ws1,
        WorkerMessage::Event {
            event: SessionEvent::Suspending {
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

    // Step 4: explicitly /close the conversation (the "End Chat" path).
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

    // Step 5: implicit resume via /messages with msg4 — flipping the
    // conversation back to Active and queueing msg4 for delivery to the
    // new worker. The automation spawns the new session.
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
    let new_session_id = poll_resumed_session_id(
        &client,
        &server.base_url(),
        &conversation_id,
        &initial_session_id,
    )
    .await
    .expect("expected a new session after implicit resume via /messages");
    assert_ne!(
        new_session_id, initial_session_id,
        "resume must create a brand-new session"
    );

    // Step 6: connect fake-worker #2 and handshake as Fresh. The server
    // returns the full prior event log so the new worker can reconstruct
    // context.
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up2 = worker_handshake(&mut ws2, WorkerMessage::Fresh).await?;

    // History-tracked assertion: full prior session log, in insertion
    // order. Expected sequence: msg1, reply1, msg2, reply2, msg3, reply3,
    // Suspending, Closed = 8 events. The `Resumed` marker lives on the
    // new session (worker emits it post-`try_materialize`); it's not in
    // the prior-session transcript.
    assert_eq!(
        catch_up2.events.len(),
        8,
        "transcript should contain 3 user + 3 assistant + Suspending + \
         Closed = 8 events, got {:?}",
        catch_up2.events
    );
    match &catch_up2.events[0] {
        SessionEvent::UserMessage { content, .. } => assert_eq!(content, msg1),
        other => panic!("event[0] should be msg1 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[1] {
        SessionEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 1"),
        other => panic!("event[1] should be reply 1 AssistantMessage, got {other:?}"),
    }
    match &catch_up2.events[2] {
        SessionEvent::UserMessage { content, .. } => assert_eq!(content, msg2),
        other => panic!("event[2] should be msg2 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[3] {
        SessionEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 2"),
        other => panic!("event[3] should be reply 2 AssistantMessage, got {other:?}"),
    }
    match &catch_up2.events[4] {
        SessionEvent::UserMessage { content, .. } => assert_eq!(content, msg3),
        other => panic!("event[4] should be msg3 UserMessage, got {other:?}"),
    }
    match &catch_up2.events[5] {
        SessionEvent::AssistantMessage { content, .. } => assert_eq!(content, "reply 3"),
        other => panic!("event[5] should be reply 3 AssistantMessage, got {other:?}"),
    }
    assert!(
        matches!(catch_up2.events[6], SessionEvent::Suspending { .. }),
        "event[6] should be Suspending, got {:?}",
        catch_up2.events[6]
    );
    assert!(
        matches!(catch_up2.events[7], SessionEvent::Closed { .. }),
        "event[7] should be Closed, got {:?}",
        catch_up2.events[7]
    );

    // Step 7: send Ready and verify the queued msg4 (from Step 5's implicit
    // resume) is the ONLY user-facing message delivered — not a replay of
    // msg1/msg2/msg3. Since msg4 was queued before worker #2 connected,
    // it drains during set_active and arrives as a folded FirstMessage
    // when the phase transitions out of Negotiating.
    send_worker_message(&mut ws2, WorkerMessage::Ready).await?;
    let received_after_resume = drain_server_messages(&mut ws2, Duration::from_millis(500)).await?;
    let delivered_user_messages: Vec<&str> = received_after_resume
        .iter()
        .filter_map(|m| match m {
            ServerMessage::FirstMessage { user_message, .. } => Some(user_message.as_str()),
            ServerMessage::Event {
                event: SessionEvent::UserMessage { content, .. },
                ..
            } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        delivered_user_messages,
        vec![msg4],
        "exactly one user message should be delivered to worker #2 after \
         resume (the queued msg4); a re-broadcast of msg1/2/3 would show \
         up here. Got {received_after_resume:?}"
    );

    // Final cross-check: chat content lives on the per-session `SessionEvent`
    // log post-Phase-E-step-18. Walk every session linked to the conversation
    // (in creation-time order) and concatenate their session-event logs.
    let mut sessions = store
        .list_sessions(&{
            let mut q = hydra_common::api::v1::sessions::SearchSessionsQuery::default();
            q.conversation_id = Some(conversation_id.clone());
            q
        })
        .await?;
    sessions.sort_by_key(|(_, v)| v.creation_time);

    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    let mut all_session_events: Vec<DomainSessionEvent> = Vec::new();
    for (sid, _) in &sessions {
        let evs = store.get_session_events(sid).await?;
        for v in evs {
            all_session_events.push(v.item);
        }
    }

    let user_messages: Vec<&str> = all_session_events
        .iter()
        .filter_map(|e| match e {
            DomainSessionEvent::UserMessage { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        user_messages,
        vec![msg1, msg2, msg3, msg4],
        "exactly 4 user messages in insertion order, no duplicates"
    );
    let assistant_messages: Vec<&str> = all_session_events
        .iter()
        .filter_map(|e| match e {
            DomainSessionEvent::AssistantMessage { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistant_messages,
        vec!["reply 1", "reply 2", "reply 3"],
        "exactly 3 assistant messages in insertion order, no duplicates"
    );

    // The conversation must end up Active after the resume.
    let final_conv: Conversation = client
        .get(format!(
            "{}/v1/conversations/{conversation_id}",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(final_conv.status, ConversationStatus::Active);

    Ok(())
}

#[tokio::test]
async fn close_then_resume_replays_full_history_with_no_session_state() -> anyhow::Result<()> {
    // Regression test for the user-reported "agent forgets context on resume"
    // bug, adapted to the WS-only Phase 1 split. When the user /closes a
    // chat, the worker is killed without uploading session_state. After
    // /resume, the new worker opens `Fresh`, sees `ResumeContext` (with
    // `resume_blob = None` since nothing was uploaded, and
    // `prior_session_id = Some`), and asks for the prior transcript. The
    // server must return the full prior event log
    // (UserMessage + AssistantMessage + Closed) so the new worker can
    // rebuild context — even though no session_state was ever persisted.
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    // Step 1: create the conversation and exchange one full user/assistant turn.
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
    let _catch_up = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
                content: "first agent reply".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    // Drop the WS without sending Suspending/SessionStateUpload — simulating
    // the worker being killed by /close.
    drop(ws);

    // Step 2: /close the conversation. No session_state is uploaded.
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
    let stored_state = store.get_session_state(&initial_session_id).await?;
    assert!(
        stored_state.is_none(),
        "no SessionStateUpload was ever sent, so session_state must be None"
    );

    // Step 3: implicit resume via /messages — flipping the Closed
    // conversation back to Active and queueing the trigger message. The
    // automation spawns a new session.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "resume trigger".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let new_session_id = poll_resumed_session_id(
        &client,
        &server.base_url(),
        &conversation_id,
        &initial_session_id,
    )
    .await
    .expect("expected a new session after implicit resume via /messages");
    assert_ne!(
        new_session_id, initial_session_id,
        "resume must create a brand-new session"
    );
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );
    assert_eq!(
        new_session.resumed_from.as_ref(),
        Some(&initial_session_id),
        "resumed_from must point at the prior session"
    );

    // Step 4: connect fake-worker #2 with the real worker's
    // resume_from_event_index value. The server must return the FULL event
    // log including the prior UserMessage + AssistantMessage so the new
    // worker can rebuild context.
    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up = worker_handshake(&mut ws2, WorkerMessage::Fresh).await?;

    // Expected sequence: UserMessage("first user message"),
    // AssistantMessage("first agent reply"), Closed. The `Resumed` marker
    // is emitted by the worker on the NEW session log after a successful
    // `try_materialize`, so it is NOT in the prior-session transcript.
    assert_eq!(
        catch_up.events.len(),
        3,
        "expected the prior session's event log on resume, got {:?}",
        catch_up.events
    );
    assert!(
        matches!(
            &catch_up.events[0],
            SessionEvent::UserMessage { content, .. } if content == "first user message"
        ),
        "event[0] should be the initial user message, got {:?}",
        catch_up.events[0]
    );
    assert!(
        matches!(
            &catch_up.events[1],
            SessionEvent::AssistantMessage { content, .. } if content == "first agent reply"
        ),
        "event[1] should be the prior assistant reply (the key context the \
         worker would otherwise have lost), got {:?}",
        catch_up.events[1]
    );
    assert!(
        matches!(catch_up.events[2], SessionEvent::Closed { .. }),
        "event[2] should be Closed, got {:?}",
        catch_up.events[2]
    );

    Ok(())
}

#[tokio::test]
async fn resume_after_session_state_upload_persists_but_omits_from_catch_up() -> anyhow::Result<()>
{
    // Regression test for i-xwmoxzhe: the catch-up payload must NEVER carry
    // session_state, even when the predecessor uploaded one. Shipping it
    // pushed long conversations' catch-up frames past the WebSocket 16 MiB
    // cap and silently killed every resume attempt. The upload itself must
    // still persist on the producing session so we can revive catch-up-side
    // delivery later without re-implementing the writer.
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
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    // Drain any events queued by the chat_relay's Pending→Active drain
    // (the initial "hello" arrives here as a live `Event` under the
    // queue-and-deliver design). Reading them stops the kernel from
    // RST-ing the close handshake when we drop the socket below.
    let _ = drain_server_messages(&mut ws, Duration::from_millis(100)).await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
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
            event: SessionEvent::Suspending {
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

    // The store must have persisted the exact payload bytes on the producing
    // session.
    let stored_state = store.get_session_state(&initial_session_id).await?;
    assert_eq!(
        stored_state.as_deref(),
        Some(payload_bytes.as_slice()),
        "SessionStateUpload should be persisted byte-for-byte"
    );

    // Resume the conversation implicitly via /messages and connect worker
    // #2. After the trigger send the previous session is in the store
    // with status Complete (the test drove it terminal above);
    // `find_new_session_for_conversation` polls for the new session that
    // the automation spawns.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "resume trigger".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let new_session_id =
        find_new_session_for_conversation(&store, &conversation_id, &initial_session_id).await;
    assert_ne!(new_session_id, initial_session_id);
    let new_session = store.get_session(&new_session_id, false).await?.item;
    assert!(
        new_session.is_interactive(),
        "resumed session must be interactive"
    );

    let mut ws2 = connect_relay(&server.base_url(), &new_session_id).await?;
    let catch_up_text = worker_handshake_raw(&mut ws2, WorkerMessage::Fresh).await?;

    // The raw wire text must not even mention `session_state` — neither as a
    // null nor as a populated field. Asserting on the JSON string (not just
    // the typed struct) catches a regression where the server reintroduces
    // the field on the wire even after the typed model drops it.
    assert!(
        !catch_up_text.raw.contains("session_state"),
        "catch-up wire payload must not contain session_state, got {}",
        catch_up_text.raw
    );

    // And the persisted upload survives untouched — the upload path itself
    // is intentionally unchanged so we can revisit catch-up-side delivery
    // later.
    let still_stored = store.get_session_state(&initial_session_id).await?;
    assert_eq!(
        still_stored.as_deref(),
        Some(payload_bytes.as_slice()),
        "SessionStateUpload must remain persisted after resume — only the \
         catch-up delivery was removed, not the upload"
    );

    Ok(())
}

#[tokio::test]
async fn reconnecting_handshake_does_not_return_session_state() -> anyhow::Result<()> {
    // `Reconnecting` is a mid-session WS reconnect by the same live worker,
    // not a fresh resume. Even though the catch-up no longer ships
    // session_state on any path (see i-xwmoxzhe), keep this case covered
    // explicitly so a future regression that wires session_state back in for
    // the Reconnecting branch is caught by tests.
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

    // Persist some session_state so a regression that reintroduces the field
    // on either handshake path would have non-empty bytes to ship.
    store
        .store_session_state(&session_id, b"opaque-bytes".to_vec(), &ActorRef::test())
        .await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let catch_up_text = worker_handshake_raw(
        &mut ws,
        WorkerMessage::Reconnecting {
            last_received_session_event_index: Some(0),
        },
    )
    .await?;
    assert!(
        !catch_up_text.raw.contains("session_state"),
        "Reconnecting catch-up must not contain session_state on the wire, got {}",
        catch_up_text.raw
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
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
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
/// EITHER `ServerMessage::CatchUp.events` OR a `ServerMessage::Event` on the
/// relay, which mirrors how the bug actually manifests in production.
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
            agent_name: Some(hydra_common::api::v1::agents::AgentName::try_new("chat").unwrap()),
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
    // message was appended), folded into `FirstMessage.user_message`
    // after Ready (the standard Phase-2 path), or as a `ServerMessage::Event`
    // flushed after FirstMessage. Any of these is sufficient for
    // PromptPrepend to fire on the worker side.
    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let catch_up = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;

    let saw_in_catch_up = catch_up.events.iter().any(|e| {
        matches!(
            e,
            SessionEvent::UserMessage { content, .. } if content == "hello"
        )
    });

    let saw_post_ready = if saw_in_catch_up {
        false
    } else {
        send_worker_message(&mut ws, WorkerMessage::Ready).await?;
        let drained = drain_server_messages(&mut ws, Duration::from_secs(2)).await?;
        drained.iter().any(|m| match m {
            ServerMessage::FirstMessage { user_message, .. } => user_message == "hello",
            ServerMessage::Event {
                event: SessionEvent::UserMessage { content, .. },
                ..
            } => content == "hello",
            _ => false,
        })
    };

    assert!(
        saw_in_catch_up || saw_post_ready,
        "worker must observe the first user message via either catch-up, \
         FirstMessage, or relay Event; catch-up events were: {:?}",
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

    // Step 1: create conversation with a first user message.
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

    // Step 2: connect as worker #1, exchange one assistant turn, then
    // suspend with a session-state upload.
    let mut ws = connect_relay(&server.base_url(), &s1).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
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
            event: SessionEvent::Suspending {
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

    // Step 3: implicit resume via /messages — sending a message on a
    // non-Active conversation flips Active and the automation spawns
    // session #2 and appends Resumed. The trigger message stays in the
    // chat_relay pending queue because no worker connects to s2 in this
    // test; it never lands in any session's event log.
    let _ = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/messages",
            server.base_url()
        ))
        .json(&SendMessageRequest {
            content: "resume trigger".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let s2 = find_new_session_for_conversation(&store, &conversation_id, &s1).await;

    // Step 4: /close — emits Closed lifecycle event.
    let _ = client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url()
        ))
        .send()
        .await?
        .error_for_status()?;

    // ---- Assertions ----

    // The conversation's status-transition log carries the lifecycle: each
    // `update_conversation` call (transparent flips inside send_message,
    // `flip_conversation_to_idle`, and `close_conversation`) creates a new
    // versioned row. We expect at least one Idle and one Closed.
    let versions = store.get_conversation_versions(&conversation_id).await?;
    let statuses: Vec<_> = versions.iter().map(|v| v.item.status).collect::<Vec<_>>();
    assert!(
        statuses.contains(&crate::domain::conversations::ConversationStatus::Idle),
        "expected an Idle version in the lifecycle, got {statuses:?}",
    );
    assert!(
        statuses.contains(&crate::domain::conversations::ConversationStatus::Closed),
        "expected a Closed version in the lifecycle, got {statuses:?}",
    );

    // SessionEvent log: the dual-write puts the lifecycle SessionEvent
    // (Closed) on the active session. UserMessage("hello") and the
    // assistant reply + the Suspending all happen on s1;
    // UserMessage("follow-up") may land on s1 (relay still connected) or
    // s2 depending on relay-map timing — either is acceptable. `Resumed`
    // is no longer dual-written by the server (it moved to the worker per
    // the WS-only lifecycle redesign); the worker
    // now emits `SessionEvent::Resumed { source }` on BOTH the
    // native-materialization path (`source = Native`) and the
    // transcript-replay fallback (`source = Transcript`), so any real
    // worker driving a close→reopen flow will produce one Resumed on
    // s2. This fake-worker-driven test never reaches that emit site, so
    // the session log here still shows zero `SessionEvent::Resumed` —
    // the symmetric-worker behaviour is exercised in the
    // `hydra::worker::model_selector::tests` module
    // (`emit_resumed_native_path_sends_resumed_with_native_source`,
    // `emit_resumed_transcript_path_requests_transcript_then_emits_resumed_transcript`,
    // `emit_resumed_fresh_session_emits_nothing`).
    // Closed is appended after /close and lands on the latest session
    // for the conversation (s2).
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
    // Both UserMessages land on a session: send_message now waits briefly
    // for the spawn-conversation-sessions automation to produce the
    // companion session before writing, so the initial create_conversation
    // message no longer races and is captured on s1, and the follow-up
    // lands on either s1 (if the relay-map still pointed there) or s2
    // depending on timing.
    assert_eq!(total_user, 2, "session-event UserMessage count (s1+s2)");
    assert_eq!(
        total_assistant, 1,
        "dual-write AssistantMessage count (s1+s2)"
    );
    assert_eq!(total_suspending, 1, "dual-write Suspending count (s1+s2)");
    assert_eq!(
        total_resumed, 0,
        "SessionEvent::Resumed is now emitted by the worker after \
         `try_materialize`; this fake worker never emits one, so the count \
         must be zero (count s1+s2)"
    );
    assert_eq!(total_closed, 1, "dual-write Closed count (s1+s2)");

    // session_state: the upload from worker #1 must appear keyed on the
    // producing session id (s1).
    let session_state = store.get_session_state(&s1).await?;
    assert_eq!(
        session_state.as_deref(),
        Some(upload_bytes.as_slice()),
        "session_state must contain the uploaded bytes for s1"
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
    // Post-Phase-E step 18 the seed "seed" UserMessage from
    // create_conversation also lands on the session event log, so filter by
    // content rather than picking the first user_message.
    let user = events
        .iter()
        .find(|e| e["type"] == "user_message" && e["content"] == "hello agent")
        .expect("session events must contain the appended user_message");
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

// ---------------------------------------------------------------------------
// FirstMessage acceptance matrix (design §5 / §1.4 of the WS-only worker
// lifecycle redesign). Each case drives Phase 1 + Phase 2 of a fake worker
// against the real `routes/sessions/relay.rs` handler and asserts on the
// emitted `ServerMessage::FirstMessage`.
// ---------------------------------------------------------------------------

/// Spawn an interactive session attached to a freshly-created conversation,
/// using the provided `system_prompt` and `greet_user` knob. Returns the new
/// session_id.
async fn create_interactive_session_with_settings(
    state: &AppState,
    store: &Arc<dyn Store>,
    system_prompt: &str,
    greet_user: bool,
) -> anyhow::Result<SessionId> {
    use crate::domain::conversations::{Conversation, ConversationStatus};
    use crate::domain::users::Username;
    use hydra_common::api::v1::sessions::{
        AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
    };

    let (conversation_id, _) = store
        .add_conversation(
            Conversation {
                title: None,
                agent_name: None,
                status: ConversationStatus::Active,
                creator: Username::from("creator"),
                session_settings: crate::domain::issues::SessionSettings::default(),
                deleted: false,
            },
            &ActorRef::test(),
        )
        .await?;

    let req = CreateSessionRequest {
        mode: SessionMode::Interactive {
            conversation_id,
            idle_timeout_secs: None,
            conversation_resume_from: None,
            greet_user,
        },
        agent_config: AgentSpec::Adhoc {
            system_prompt: system_prompt.to_string(),
            mcp_config: None,
        },
        model: None,
        mount_spec: MountSpec::default(),
        image: None,
        env_vars: std::collections::HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
    };
    let (id, _) = state
        .create_session(req, ActorRef::test(), Username::from("creator"))
        .await
        .map_err(|err| anyhow::anyhow!("create_session failed: {err}"))?;
    Ok(id)
}

/// Spawn a headless session with the provided `system_prompt`. Returns the new
/// session_id.
async fn create_headless_session_with_prompt(
    state: &AppState,
    system_prompt: &str,
) -> anyhow::Result<SessionId> {
    use crate::domain::users::Username;
    use hydra_common::api::v1::sessions::{
        AgentSpec, CreateSessionRequest, MountSpec, SessionMode,
    };

    let req = CreateSessionRequest {
        mode: SessionMode::Headless,
        agent_config: AgentSpec::Adhoc {
            system_prompt: system_prompt.to_string(),
            mcp_config: None,
        },
        model: None,
        mount_spec: MountSpec::default(),
        image: None,
        env_vars: std::collections::HashMap::new(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
    };
    let (id, _) = state
        .create_session(req, ActorRef::test(), Username::from("creator"))
        .await
        .map_err(|err| anyhow::anyhow!("create_session failed: {err}"))?;
    Ok(id)
}

/// (a) Fresh headless — server emits `FirstMessage` immediately on `Ready`
/// with `agent_prompt = system_prompt` and `user_message = ""`. The Phase-1
/// `ResumeContext` carries `resume_blob = None` and `prior_session_id =
/// None` for a brand-new headless session.
#[tokio::test]
async fn first_message_fresh_headless_emits_system_prompt_immediately() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id =
        create_headless_session_with_prompt(&state, "you are a helpful headless agent").await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    let first = send_ready_and_await_first_message(&mut ws, Duration::from_secs(1)).await?;
    let (agent_prompt, user_message) =
        first.ok_or_else(|| anyhow::anyhow!("expected a FirstMessage; got none"))?;
    assert_eq!(agent_prompt, "you are a helpful headless agent");
    assert_eq!(
        user_message, "",
        "headless FirstMessage.user_message must be empty"
    );

    Ok(())
}

/// (a′) Fresh headless with empty `system_prompt` — `FirstMessage { "", "" }`
/// is the legal, expected payload (G11 of the design: empty strings are valid
/// at every layer; the wrapper accepts an empty prompt).
#[tokio::test]
async fn first_message_fresh_headless_with_empty_system_prompt_accepts_empty_strings()
-> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id = create_headless_session_with_prompt(&state, "").await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    let first = send_ready_and_await_first_message(&mut ws, Duration::from_secs(1)).await?;
    let (agent_prompt, user_message) =
        first.ok_or_else(|| anyhow::anyhow!("expected a FirstMessage; got none"))?;
    assert_eq!(agent_prompt, "");
    assert_eq!(user_message, "");

    Ok(())
}

/// (b) Fresh interactive (`greet_user = false`) with a queued first user
/// message — the relay folds the queued UserMessage into `FirstMessage` on
/// `Ready` and suppresses the redundant `Event` push.
#[tokio::test]
async fn first_message_fresh_interactive_no_greet_with_queued_message_folds_in()
-> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id =
        create_interactive_session_with_settings(&state, &store, "system: be brief", false).await?;

    // Pre-Ready: send a user message. With the chat_relay queue-and-deliver
    // design, before the worker connects the UserMessage lands on the
    // pending queue; on connect it drains. We drive that flow by posting
    // through send_message via the state's app handle.
    let conv_id = store
        .get_session(&session_id, false)
        .await?
        .item
        .conversation_id()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("interactive session must have a conversation_id"))?;
    use crate::domain::users::Username;
    state
        .send_message(
            &conv_id,
            "hello from user".to_string(),
            ActorRef::test(),
            Username::from("creator"),
        )
        .await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    let first = send_ready_and_await_first_message(&mut ws, Duration::from_secs(2)).await?;
    let (agent_prompt, user_message) = first
        .ok_or_else(|| anyhow::anyhow!("expected a FirstMessage with the folded UserMessage"))?;
    assert_eq!(agent_prompt, "system: be brief");
    assert_eq!(
        user_message, "hello from user",
        "queued UserMessage must be folded into FirstMessage.user_message"
    );

    Ok(())
}

/// (c) Fresh interactive (`greet_user = false`) with NO queued user message
/// — `Ready` stashes `pending_first_message`; no `FirstMessage` arrives
/// until a UserMessage shows up. When a UserMessage is then sent, the
/// stashed `agent_prompt` is paired with it and emitted as `FirstMessage`,
/// AND the redundant `Event { UserMessage }` is suppressed.
#[tokio::test]
async fn first_message_fresh_interactive_no_greet_stashes_pending_then_delivers_on_send()
-> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id =
        create_interactive_session_with_settings(&state, &store, "stay quiet", false).await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;

    // Send Ready, then briefly poll: no FirstMessage should arrive yet
    // because no UserMessage is queued — the agent_prompt was stashed as
    // `pending_first_message`.
    send_worker_message(&mut ws, WorkerMessage::Ready).await?;
    let pre = drain_server_messages(&mut ws, Duration::from_millis(200)).await?;
    assert!(
        !pre.iter()
            .any(|m| matches!(m, ServerMessage::FirstMessage { .. })),
        "no FirstMessage expected before a UserMessage arrives; got {pre:?}"
    );

    // Now post a UserMessage; the relay should hand-off as a FirstMessage
    // pairing the stashed `agent_prompt` with the new user content, and
    // suppress the redundant `Event { UserMessage }` push.
    use crate::domain::users::Username;
    let conv_id = store
        .get_session(&session_id, false)
        .await?
        .item
        .conversation_id()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("interactive session must have a conversation_id"))?;
    state
        .send_message(
            &conv_id,
            "delayed hi".to_string(),
            ActorRef::test(),
            Username::from("creator"),
        )
        .await?;

    let post = drain_server_messages(&mut ws, Duration::from_secs(2)).await?;
    let first = post.iter().find_map(|m| match m {
        ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        } => Some((agent_prompt.clone(), user_message.clone())),
        _ => None,
    });
    let (agent_prompt, user_message) = first.ok_or_else(|| {
        anyhow::anyhow!("expected FirstMessage after the UserMessage was posted; got {post:?}")
    })?;
    assert_eq!(agent_prompt, "stay quiet");
    assert_eq!(user_message, "delayed hi");
    let dup_event_user = post.iter().any(|m| {
        matches!(
            m,
            ServerMessage::Event {
                event: SessionEvent::UserMessage { content, .. },
                ..
            } if content == "delayed hi"
        )
    });
    assert!(
        !dup_event_user,
        "the relay must suppress the redundant Event {{ UserMessage }} after folding it into \
         FirstMessage; got {post:?}"
    );

    Ok(())
}

/// (d) Fresh interactive with `greet_user = true` — server emits
/// `FirstMessage { agent_prompt, user_message: "" }` immediately on `Ready`
/// regardless of queued state, so the agent produces a greeting turn first.
#[tokio::test]
async fn first_message_fresh_interactive_greet_user_true_emits_with_empty_user_message()
-> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id =
        create_interactive_session_with_settings(&state, &store, "greet me", true).await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    let first = send_ready_and_await_first_message(&mut ws, Duration::from_secs(2)).await?;
    let (agent_prompt, user_message) =
        first.ok_or_else(|| anyhow::anyhow!("expected a FirstMessage; got none"))?;
    assert_eq!(agent_prompt, "greet me");
    assert_eq!(
        user_message, "",
        "greet_user=true must produce an empty user_message so the agent greets first"
    );

    Ok(())
}

/// First inbound that is not `Fresh` or `Reconnecting` is a protocol error
/// — the relay closes the WS without entering Phase 2.
#[tokio::test]
async fn first_inbound_other_than_fresh_or_reconnecting_is_protocol_error() -> anyhow::Result<()> {
    let (state, store) = state_with_idle_timeout_secs(2);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let session_id = create_headless_session_with_prompt(&state, "any").await?;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    send_worker_message(&mut ws, WorkerMessage::Ready).await?;
    // The server should close the socket rather than reply with anything.
    let msgs = drain_server_messages(&mut ws, Duration::from_secs(1)).await?;
    let first_message_count = msgs
        .iter()
        .filter(|m| matches!(m, ServerMessage::FirstMessage { .. }))
        .count();
    assert_eq!(
        first_message_count, 0,
        "an out-of-order Ready as the first inbound must NOT produce a FirstMessage; \
         got {msgs:?}"
    );

    Ok(())
}

/// Worker-side mid-session reconnect (i-wfpazngu): a Phase-3 worker drops
/// its WS, reopens with `WorkerMessage::Reconnecting { ... }`, and must
/// receive a `CatchUp` slice containing ONLY events past the supplied
/// index — both UserMessages it didn't yet see AND any other-variant
/// dual-writes in between (AssistantMessage etc.). After the reconnect,
/// live forwarding of new user messages resumes on the new socket and
/// nothing already delivered on the old socket gets replayed.
#[tokio::test]
async fn reconnecting_returns_catch_up_strictly_after_supplied_index() -> anyhow::Result<()> {
    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    init_test_tracing();

    let (state, store) = state_with_idle_timeout_secs(60);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let http = test_client();

    // Create a conversation; the worker connects fresh.
    let created: Conversation = http
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
    let session_id = find_session_for_conversation(&store, &conversation_id).await;

    // Phase 1+2 + Phase-3 entry: a Fresh worker handshakes and sends
    // Ready. The server folds the drained pending "hello" into
    // FirstMessage.user_message; no event_index is exposed for that
    // message because it's not delivered as an Event. Then we POST a
    // second user message to capture an event_index that the
    // Reconnecting handshake can use as `last_received_session_event_index`.
    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    send_worker_message(&mut ws, WorkerMessage::Ready).await?;
    let live = drain_server_messages(&mut ws, Duration::from_secs(2)).await?;
    let saw_hello = live.iter().any(|m| match m {
        ServerMessage::FirstMessage { user_message, .. } => user_message == "hello",
        ServerMessage::Event {
            event: SessionEvent::UserMessage { content, .. },
            ..
        } => content == "hello",
        _ => false,
    });
    assert!(
        saw_hello,
        "drained pending UserMessage was not delivered to the worker; got {live:?}"
    );

    // Send a second user message; the worker receives it (and the
    // event_index increments).
    http.post(format!(
        "{}/v1/conversations/{conversation_id}/messages",
        server.base_url()
    ))
    .json(&SendMessageRequest {
        content: "second".to_string(),
    })
    .send()
    .await?
    .error_for_status()?;
    let second_forwarded = drain_server_messages(&mut ws, Duration::from_secs(2)).await?;
    let second_index = second_forwarded
        .iter()
        .find_map(|m| match m {
            ServerMessage::Event {
                event: SessionEvent::UserMessage { content, .. },
                event_index,
            } if content == "second" => Some(*event_index),
            _ => None,
        })
        .expect("second user message must be forwarded with an event_index");

    // Drop the WS mid-session AND simulate other-variant session-event
    // writes that the worker did NOT see — these must be returned in the
    // CatchUp slice (filtered server-side strictly by `event_index >
    // last_received`), but the worker's filter-and-re-inject logic
    // discards them so the model never sees a replay.
    drop(ws);
    let actor = ActorRef::test();
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::AssistantMessage {
                content: "intermediate assistant reply".to_string(),
                timestamp: chrono::Utc::now(),
            },
            &actor,
        )
        .await?;
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::UserMessage {
                content: "third while disconnected".to_string(),
                timestamp: chrono::Utc::now(),
            },
            &actor,
        )
        .await?;

    // Brief poll: the chat_relay map's `disconnect` may run on the
    // background task that owned the dropped socket. Wait until the
    // relay is no longer registering an active session for the
    // conversation before reopening.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while state
        .chat_relay_map
        .active_session_id(&conversation_id)
        .is_some()
    {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Reconnect: ask for events past `second_index`. The server must
    // ONLY include events with `event_index > second_index` (the two
    // appends above), and must NOT replay "hello" or "second".
    let mut ws2 = connect_relay(&server.base_url(), &session_id).await?;
    let handshake = worker_handshake(
        &mut ws2,
        WorkerMessage::Reconnecting {
            last_received_session_event_index: Some(second_index),
        },
    )
    .await?;
    let catch_up_contents: Vec<(String, Option<String>)> = handshake
        .events
        .iter()
        .map(|e| match e {
            SessionEvent::UserMessage { content, .. } => {
                ("user".to_string(), Some(content.clone()))
            }
            SessionEvent::AssistantMessage { content, .. } => {
                ("assistant".to_string(), Some(content.clone()))
            }
            other => (format!("{other:?}"), None),
        })
        .collect();
    assert!(
        !handshake.events.iter().any(
            |e| matches!(e, SessionEvent::UserMessage { content, .. } if content == "hello"
                || content == "second")
        ),
        "events delivered before the drop must NOT reappear in CatchUp; got {catch_up_contents:?}"
    );
    let saw_intermediate_assistant = handshake.events.iter().any(|e| matches!(
        e,
        SessionEvent::AssistantMessage { content, .. } if content == "intermediate assistant reply"
    ));
    let saw_third_user = handshake.events.iter().any(|e| {
        matches!(
            e,
            SessionEvent::UserMessage { content, .. } if content == "third while disconnected"
        )
    });
    assert!(
        saw_intermediate_assistant,
        "AssistantMessage written between drop and reconnect must appear in CatchUp; \
         got {catch_up_contents:?}"
    );
    assert!(
        saw_third_user,
        "UserMessage written between drop and reconnect must appear in CatchUp; \
         got {catch_up_contents:?}"
    );

    // After the reconnect, send one more user message and verify the
    // new socket receives it via the standard `Event` path — and that
    // no event delivered before the drop reappears here.
    http.post(format!(
        "{}/v1/conversations/{conversation_id}/messages",
        server.base_url()
    ))
    .json(&SendMessageRequest {
        content: "after-reconnect".to_string(),
    })
    .send()
    .await?
    .error_for_status()?;
    let post_reconnect = drain_server_messages(&mut ws2, Duration::from_secs(2)).await?;
    let mut after_reconnect_user_contents: Vec<String> = Vec::new();
    for m in &post_reconnect {
        if let ServerMessage::Event {
            event: SessionEvent::UserMessage { content, .. },
            ..
        } = m
        {
            after_reconnect_user_contents.push(content.clone());
        }
    }
    assert_eq!(
        after_reconnect_user_contents,
        vec!["after-reconnect".to_string()],
        "exactly one new user message should arrive after the reconnect; got {post_reconnect:?}"
    );

    Ok(())
}

/// PR-2 graceful End Chat path: when a worker is connected to the relay
/// at the time of `/v1/conversations/:id/close`, the server must
///
/// 1. push `ServerMessage::EndSession` onto the worker's relay channel
///    (instead of immediately calling `job_engine.kill_job`),
/// 2. wait until the worker completes its unified end-of-session sequence
///    (`SessionStateUpload` → `Closed` event → `EndSessionAck` → WS close),
/// 3. observe the WS close as the implicit ack (relay map entry drops),
/// 4. revoke session-scoped auth tokens (parity with `kill.rs`), and
/// 5. return the closed conversation.
///
/// This pins the wire-protocol shape, the "disconnect IS the ack"
/// semantics, and the token-revocation parity all in one test.
#[tokio::test]
async fn close_conversation_sends_end_session_and_awaits_clean_disconnect() -> anyhow::Result<()> {
    use crate::domain::actors::Actor;
    use crate::domain::users::Username;
    use crate::test_utils::register_actor_and_token;
    use hydra_common::ActorId;

    init_test_tracing();
    let (state, store) = state_with_idle_timeout_secs(60);
    let server = spawn_test_server_with_state(state.clone(), store.clone()).await?;
    let client = test_client();

    // Create the conversation and connect a fake worker through Phase 1/2.
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
    let session_id = find_session_for_conversation(&store, &conversation_id).await;

    let mut ws = connect_relay(&server.base_url(), &session_id).await?;
    let _ = worker_handshake(&mut ws, WorkerMessage::Fresh).await?;
    send_worker_message(&mut ws, WorkerMessage::Ready).await?;
    // Drain Phase-2 traffic (FirstMessage etc.) so the next `ws.next()`
    // call only sees Phase-3 frames.
    let _ = drain_server_messages(&mut ws, Duration::from_millis(500)).await?;

    // Register a session-scoped auth token so the parity check has
    // something to flip on the graceful-success branch.
    let (actor, auth_token) = Actor::new_from_actor_id(
        ActorId::Adhoc(session_id.clone()),
        Username::from("graceful-close-test"),
        None,
    );
    register_actor_and_token(store.as_ref(), &actor, &auth_token, Some(&session_id)).await?;
    let prefix = format!("{}:", actor.name());
    let raw_token = auth_token
        .strip_prefix(&prefix)
        .expect("auth token must carry actor-name prefix");
    let token_hash = Actor::hash_auth_token(raw_token);
    let pre_close = store
        .get_auth_token_by_hash(&token_hash)
        .await?
        .expect("session token must exist pre-close");
    assert!(
        !pre_close.is_revoked,
        "session token must be live before /close fires"
    );

    // POST /close in a background task; the foreground task plays the
    // worker side of the unified cleanup-and-close protocol on the WS.
    let base = server.base_url();
    let conv_for_close = conversation_id.clone();
    let close_handle = tokio::spawn(async move {
        let close_client = test_client();
        close_client
            .post(format!("{base}/v1/conversations/{conv_for_close}/close"))
            .send()
            .await
            .and_then(|resp| resp.error_for_status())
    });

    // Expect EndSession on the WS within the graceful deadline (10s).
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    if let Ok(ServerMessage::EndSession) = serde_json::from_str(&t) {
                        return Ok::<(), anyhow::Error>(());
                    }
                    // Tolerate any Phase-3 traffic the server may still
                    // emit (none expected on this short test) and keep
                    // reading.
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => anyhow::bail!("ws error before EndSession: {err}"),
                None => anyhow::bail!("ws closed before EndSession arrived"),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("server did not send EndSession within deadline"))??;

    // Worker side: emit the unified cleanup sequence (PR-1's contract),
    // then close the WS — the server's `pump_phase3 → cleanup` will drop
    // the relay entry, which is what `close_conversation` polls for.
    send_worker_message(
        &mut ws,
        WorkerMessage::SessionStateUpload {
            data: serde_json::to_vec(&SessionStatePayload::V1 {
                session_id: "graceful-close-test-session".to_string(),
                transcript: Some(b"transcript-bytes".to_vec()),
            })?,
        },
    )
    .await?;
    send_worker_message(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::Closed {
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    send_worker_message(&mut ws, WorkerMessage::EndSessionAck).await?;
    ws.send(tungstenite::Message::Close(None)).await?;
    drop(ws);

    // /close must complete with 2xx within the graceful deadline.
    let close_resp = tokio::time::timeout(Duration::from_secs(15), close_handle).await???;
    let closed: Conversation = close_resp.json().await?;
    assert_eq!(closed.status, ConversationStatus::Closed);

    // Worker uploaded session_state before the WS closed — this is the
    // PR-1 contract that PR-2's graceful path exercises end-to-end.
    let stored_state = store.get_session_state(&session_id).await?;
    assert!(
        stored_state.is_some(),
        "graceful close must let the worker upload session_state before disconnect"
    );

    // Auth tokens minted by this session are revoked (parity with kill.rs).
    let post_close = store
        .get_auth_token_by_hash(&token_hash)
        .await?
        .expect("session token row must still exist post-close");
    assert!(
        post_close.is_revoked,
        "session tokens must be revoked after a graceful close (parity with kill.rs)"
    );

    // Sanity: the relay entry was dropped by the worker's WS close.
    assert!(
        state
            .chat_relay_map
            .active_session_id(&conversation_id)
            .is_none(),
        "relay entry must be dropped after the worker closes the WS"
    );

    Ok(())
}
