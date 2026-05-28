//! Server-side coverage for the new PR-3 protocol's Phase-2 `FirstMessage`
//! event-driven dispatch and the Phase-3 `Reconnecting` catch-up reply.
//!
//! Companion to [`super::chat_lifecycle`]. The tests here exercise the
//! server side of `routes/sessions/relay.rs` (handler entry points and the
//! `pending_first_message` state machine) without going through the
//! `worker_handshake`-helper hack that the legacy lifecycle tests use to
//! map the old `Fresh → CatchUp` protocol onto the new wire format.

use crate::{
    app::{AppState, ServiceState},
    domain::{actors::ActorRef, agents::Agent, documents::Document},
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
        agents::AgentName,
        conversations::{
            Conversation, CreateConversationRequest, SendMessageRequest, ServerMessage,
            WorkerConnect, WorkerMessage,
        },
        sessions::{
            CreateSessionRequest, CreateSessionResponse, SearchSessionsQuery, SessionEvent,
            SessionMode,
        },
    },
};
use reqwest::StatusCode;
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::tungstenite;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn fresh_state() -> (AppState, Arc<dyn Store>) {
    let mut config = test_app_config();
    config.job.interactive_idle_timeout_secs = 30;
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

async fn register_agent(
    store: &Arc<dyn Store>,
    name: &str,
    prompt: &str,
    greet_user: bool,
) -> anyhow::Result<()> {
    let prompt_path = format!("/agents/{name}/prompt.md");
    let agent = Agent::new(
        name.to_string(),
        prompt_path.clone(),
        None,
        1,
        1,
        greet_user,
        false,
        vec![],
    );
    store.add_agent(agent).await?;
    let doc = Document {
        title: format!("{name} prompt"),
        body_markdown: prompt.to_string(),
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

async fn find_session(store: &Arc<dyn Store>, conversation_id: &ConversationId) -> SessionId {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let mut sessions: Vec<_> = store
            .list_sessions(&SearchSessionsQuery::default())
            .await
            .expect("list sessions")
            .into_iter()
            .filter(|(_, v)| v.item.conversation_id() == Some(conversation_id))
            .collect();
        sessions.sort_by_key(|(_, v)| v.creation_time);
        if let Some((id, _)) = sessions.pop() {
            return id;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("no session for {conversation_id} appeared in time");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn connect_ws(base_url: &str, session_id: &SessionId) -> anyhow::Result<WsStream> {
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
        .context("connect to events ws")?;
    Ok(stream)
}

async fn send(ws: &mut WsStream, msg: WorkerMessage) -> anyhow::Result<()> {
    let json = serde_json::to_string(&msg)?;
    ws.send(tungstenite::Message::Text(json)).await?;
    Ok(())
}

async fn recv_server(ws: &mut WsStream) -> anyhow::Result<ServerMessage> {
    loop {
        let frame = ws
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("ws closed mid-protocol"))??;
        match frame {
            tungstenite::Message::Text(text) => {
                return Ok(serde_json::from_str::<ServerMessage>(&text)?);
            }
            tungstenite::Message::Ping(p) => {
                ws.send(tungstenite::Message::Pong(p)).await?;
            }
            tungstenite::Message::Close(_) => {
                anyhow::bail!("ws closed before message received");
            }
            _ => continue,
        }
    }
}

async fn recv_server_with_timeout(
    ws: &mut WsStream,
    timeout: Duration,
) -> anyhow::Result<Option<ServerMessage>> {
    match tokio::time::timeout(timeout, recv_server(ws)).await {
        Ok(Ok(msg)) => Ok(Some(msg)),
        Ok(Err(err)) => Err(err),
        Err(_) => Ok(None),
    }
}

/// Phase 1: send Fresh, expect ResumeContext. Returns the prior_session_id
/// for tests that want to assert it.
async fn phase1_fresh(ws: &mut WsStream) -> anyhow::Result<(Option<Vec<u8>>, Option<SessionId>)> {
    send(ws, WorkerMessage::Connect(WorkerConnect::Fresh)).await?;
    match recv_server(ws).await? {
        ServerMessage::ResumeContext {
            resume_blob,
            prior_session_id,
        } => Ok((resume_blob, prior_session_id)),
        other => anyhow::bail!("expected ResumeContext, got {other:?}"),
    }
}

/// Phase 2: send Ready, expect either an immediate FirstMessage or `None`
/// (within the supplied timeout) if the server stashed
/// `pending_first_message`.
async fn phase2_ready_with_timeout(
    ws: &mut WsStream,
    timeout: Duration,
) -> anyhow::Result<Option<(Option<String>, Option<String>)>> {
    send(ws, WorkerMessage::Ready).await?;
    match recv_server_with_timeout(ws, timeout).await? {
        Some(ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        }) => Ok(Some((agent_prompt, user_message))),
        Some(other) => anyhow::bail!("expected FirstMessage, got {other:?}"),
        None => Ok(None),
    }
}

// ============================================================================
// FirstMessage matrix (per design §1.5.1 + parent issue acceptance criteria)
// ============================================================================

#[tokio::test]
async fn case_a_fresh_interactive_greet_false_with_user_message_present() -> anyhow::Result<()> {
    // (a) fresh interactive + greet_user=false + UserMessage already present
    // at `Ready` → `FirstMessage` with `Some(prompt) + Some(content)`
    // immediately.
    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    register_agent(&store, "chat", "be helpful", false).await?;
    let created: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("hello there".to_string()),
            agent_name: Some(AgentName::try_new("chat").unwrap()),
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let session_id = find_session(&store, &created.conversation_id).await;
    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    let (_blob, _prior) = phase1_fresh(&mut ws).await?;
    let first = phase2_ready_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("FirstMessage should be emitted immediately (UserMessage present)");
    assert_eq!(first.0.as_deref(), Some("be helpful"));
    assert_eq!(first.1.as_deref(), Some("hello there"));
    Ok(())
}

#[tokio::test]
async fn case_b_fresh_interactive_greet_false_deferred_until_user_message() -> anyhow::Result<()> {
    // (b) fresh interactive + greet_user=false + no UserMessage yet at
    // `Ready` → `pending_first_message` stashed on the connection. When the
    // first UserMessage arrives, FirstMessage is emitted AND the Phase-3
    // push of that same UserMessage is SUPPRESSED. A subsequent UserMessage
    // flows through the normal Phase-3 path.
    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    register_agent(&store, "chat", "deferred prompt", false).await?;
    // Use POST /v1/sessions directly so we can skip the initial UserMessage
    // that POST /v1/conversations seeds.
    let conv: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: None,
            agent_name: Some(AgentName::try_new("chat").unwrap()),
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;

    let session_id = find_session(&store, &conv.conversation_id).await;
    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    let _ = phase1_fresh(&mut ws).await?;
    // No UserMessage; greet_user=false → FirstMessage must NOT arrive.
    let maybe = phase2_ready_with_timeout(&mut ws, Duration::from_millis(400)).await?;
    assert!(
        maybe.is_none(),
        "FirstMessage must be deferred when no UserMessage exists and greet_user=false, got {maybe:?}"
    );

    // Now POST the first user message. Server should drain pending_first_message
    // and emit FirstMessage with user_message=Some(content). The Phase-3 push
    // for the SAME message must be suppressed.
    client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            conv.conversation_id
        ))
        .json(&SendMessageRequest {
            content: "first turn".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    let first = recv_server_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("FirstMessage must arrive after UserMessage lands");
    match first {
        ServerMessage::FirstMessage {
            agent_prompt,
            user_message,
        } => {
            assert_eq!(agent_prompt.as_deref(), Some("deferred prompt"));
            assert_eq!(user_message.as_deref(), Some("first turn"));
        }
        other => panic!("expected FirstMessage, got {other:?}"),
    }
    // No follow-up Event for the same UserMessage (suppression).
    let next = recv_server_with_timeout(&mut ws, Duration::from_millis(300)).await?;
    assert!(
        next.is_none(),
        "Phase-3 push of the first UserMessage must be suppressed, got {next:?}"
    );

    // Second user message — normal Phase-3 push.
    client
        .post(format!(
            "{}/v1/conversations/{}/messages",
            server.base_url(),
            conv.conversation_id
        ))
        .json(&SendMessageRequest {
            content: "second turn".to_string(),
        })
        .send()
        .await?
        .error_for_status()?;
    let second = recv_server_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("subsequent UserMessages flow through Phase-3 push");
    match second {
        ServerMessage::Event {
            event: SessionEvent::UserMessage { content, .. },
        } => assert_eq!(content, "second turn"),
        other => panic!("expected Event(UserMessage second turn), got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn case_c_fresh_interactive_greet_true_emits_immediately_no_user_message()
-> anyhow::Result<()> {
    // (c) fresh interactive + greet_user=true → emit immediately with
    // Some(prompt) + None at Ready time.
    //
    // The conversation create path always sets greet_user=false on the
    // spawned session today (no agent → mode greet_user wiring exists),
    // so this test goes around it by creating an Interactive session
    // directly with greet_user=true. The system_prompt is supplied via the
    // session's `AgentConfig` field.
    use hydra_common::api::v1::sessions::AgentConfig;

    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    // Create a conversation up front so the Interactive session has a real
    // conversation_id to attach to.
    let parent: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: None,
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conv_id = parent.conversation_id.clone();
    let mut agent_config = AgentConfig::default();
    agent_config.system_prompt = Some("say hi first".to_string());
    let req = CreateSessionRequest {
        mode: SessionMode::Interactive {
            conversation_id: conv_id.clone(),
            idle_timeout_secs: None,
            greet_user: true,
        },
        agent_config,
        mount_spec: Default::default(),
        image: None,
        env_vars: Default::default(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
        initial_prompt: None,
    };
    let resp = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&req)
        .send()
        .await?;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "create greet_user=true interactive session: {}",
        resp.text().await.unwrap_or_default()
    );
    let created: CreateSessionResponse = resp.json().await?;
    let session_id = created.session_id;

    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    let _ = phase1_fresh(&mut ws).await?;
    let first = phase2_ready_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("FirstMessage must arrive when greet_user=true even without UserMessage");
    assert_eq!(first.0.as_deref(), Some("say hi first"));
    assert!(
        first.1.is_none(),
        "user_message must be None when greet_user=true with no UserMessage: got {:?}",
        first.1
    );
    let _ = store;
    Ok(())
}

#[tokio::test]
async fn case_e_headless_uses_seeded_first_user_message() -> anyhow::Result<()> {
    // (e) headless → agent_prompt = None (no system prompt for headless),
    // user_message = Some(<seeded prompt>) immediately at Ready. Uses the
    // PR-2-backfilled conversation event.
    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let create_req = CreateSessionRequest {
        mode: SessionMode::Headless {
            // `None` so `create_session` auto-creates the conversation row;
            // the resulting session carries `Some(_)` to satisfy the
            // persisted-shape invariant.
            conversation_id: None,
        },
        agent_config: Default::default(),
        mount_spec: Default::default(),
        image: None,
        env_vars: Default::default(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
        initial_prompt: Some("scan this repo".to_string()),
    };
    let resp = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&create_req)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(status, StatusCode::OK, "create headless session: {body}");
    let created: CreateSessionResponse = serde_json::from_str(&body)?;
    let session_id = created.session_id;

    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    let _ = phase1_fresh(&mut ws).await?;
    let first = phase2_ready_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("FirstMessage must arrive for headless with seeded UserMessage");
    assert!(
        first.0.is_none(),
        "agent_prompt must be None for a headless session with no agent: got {:?}",
        first.0
    );
    assert_eq!(first.1.as_deref(), Some("scan this repo"));
    Ok(())
}

#[tokio::test]
async fn case_d_resumed_session_has_no_agent_prompt() -> anyhow::Result<()> {
    // (d) resumed session → agent_prompt = None regardless of agent
    // configuration (the system prompt was applied at the prior session's
    // first turn; resuming must not re-inject it).
    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    register_agent(&store, "chat", "applied at first turn", false).await?;
    let conv: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("turn 1".to_string()),
            agent_name: Some(AgentName::try_new("chat").unwrap()),
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let conversation_id = conv.conversation_id;

    // Close and resume to create a second session whose `resumed_from` is
    // the first session's id.
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/close",
            server.base_url(),
        ))
        .send()
        .await?
        .error_for_status()?;
    client
        .post(format!(
            "{}/v1/conversations/{conversation_id}/resume",
            server.base_url(),
        ))
        .send()
        .await?
        .error_for_status()?;

    // Find the freshly-spawned resumed session (the one with `resumed_from` set).
    let resumed_session_id = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut matches: Vec<_> = store
                .list_sessions(&SearchSessionsQuery::default())
                .await?
                .into_iter()
                .filter_map(|(id, v)| {
                    if v.item.conversation_id() == Some(&conversation_id)
                        && v.item.resumed_from.is_some()
                    {
                        Some((id, v.creation_time))
                    } else {
                        None
                    }
                })
                .collect();
            matches.sort_by_key(|(_, t)| *t);
            if let Some((id, _)) = matches.pop() {
                break id;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("no resumed session appeared in time");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };

    let mut ws = connect_ws(&server.base_url(), &resumed_session_id).await?;
    let (_blob, prior_session_id) = phase1_fresh(&mut ws).await?;
    assert!(
        prior_session_id.is_some(),
        "resumed session must report a prior_session_id in ResumeContext"
    );
    let first = phase2_ready_with_timeout(&mut ws, Duration::from_secs(2))
        .await?
        .expect("FirstMessage emitted for resumed session");
    assert!(
        first.0.is_none(),
        "agent_prompt must be None on resume; got {:?}",
        first.0
    );
    // user_message is `Some` (the conversation's first UserMessage).
    assert_eq!(first.1.as_deref(), Some("turn 1"));
    Ok(())
}

// ============================================================================
// Reconnecting catch-up (per design §1.6 + parent issue acceptance criteria)
// ============================================================================

#[tokio::test]
async fn reconnecting_catch_up_returns_events_past_index() -> anyhow::Result<()> {
    // Phase-3 mid-session reconnect: the worker reports the highest event
    // index it has already consumed; the server replies with `CatchUp`
    // containing every session event at a larger index. The worker filters
    // UserMessages out of the slice and re-injects them; non-UserMessages
    // are discarded by the worker (per design §1.6).
    //
    // Test path: seed N session events (mixed variants) into the store,
    // open a WS, send `Reconnecting { last_received_session_event_index: 1 }`,
    // and assert the CatchUp reply contains the events at index ≥ 2.
    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    use chrono::Utc;

    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let conv: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("um0".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let session_id = find_session(&store, &conv.conversation_id).await;

    // Index 0 was seeded by /v1/conversations; append 3 more.
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::AssistantMessage {
                content: "am1".to_string(),
                timestamp: Utc::now(),
            },
            &ActorRef::test(),
        )
        .await?;
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::UserMessage {
                content: "um2".to_string(),
                timestamp: Utc::now(),
            },
            &ActorRef::test(),
        )
        .await?;
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::ToolUse {
                tool_name: "Bash".to_string(),
                payload: serde_json::json!({"cmd":"ls"}),
                timestamp: Utc::now(),
            },
            &ActorRef::test(),
        )
        .await?;

    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    send(
        &mut ws,
        WorkerMessage::Connect(WorkerConnect::Reconnecting {
            last_received_session_event_index: 1,
        }),
    )
    .await?;

    let msg = recv_server(&mut ws).await?;
    match msg {
        ServerMessage::CatchUp { events } => {
            // Per `collect_session_events_after`: events with index > 1 →
            // events at index 2 and 3.
            assert_eq!(events.len(), 2, "expected 2 events past index 1");
            assert!(
                matches!(events[0], SessionEvent::UserMessage { ref content, .. } if content == "um2"),
                "first catch-up event must be UserMessage(um2): {:?}",
                events[0]
            );
            assert!(
                matches!(events[1], SessionEvent::ToolUse { ref tool_name, .. } if tool_name == "Bash"),
                "second catch-up event must be ToolUse(Bash): {:?}",
                events[1]
            );
        }
        other => panic!("expected CatchUp, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn reconnecting_catch_up_empty_when_index_at_head() -> anyhow::Result<()> {
    // A live worker reconnecting with the highest current index gets an
    // empty CatchUp.
    use crate::domain::sessions::SessionEvent as DomainSessionEvent;
    use chrono::Utc;

    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let conv: Conversation = client
        .post(format!("{}/v1/conversations", server.base_url()))
        .json(&CreateConversationRequest {
            message: Some("um0".to_string()),
            agent_name: None,
            session_settings: None,
        })
        .send()
        .await?
        .json()
        .await?;
    let session_id = find_session(&store, &conv.conversation_id).await;
    // Add one more event.
    store
        .append_session_event(
            &session_id,
            DomainSessionEvent::AssistantMessage {
                content: "am1".to_string(),
                timestamp: Utc::now(),
            },
            &ActorRef::test(),
        )
        .await?;

    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    send(
        &mut ws,
        WorkerMessage::Connect(WorkerConnect::Reconnecting {
            last_received_session_event_index: 1,
        }),
    )
    .await?;
    match recv_server(&mut ws).await? {
        ServerMessage::CatchUp { events } => {
            assert!(events.is_empty(), "expected empty CatchUp, got {events:?}");
        }
        other => panic!("expected CatchUp, got {other:?}"),
    }
    Ok(())
}

// ============================================================================
// Headless session_events readback — proves the `output_tx` plumbing in
// `drive_headless` is hooked up end-to-end at the server side too.
// ============================================================================

#[tokio::test]
async fn headless_session_can_receive_worker_event_appends() -> anyhow::Result<()> {
    // The fix for "drive_headless drops output_tx" lives in the worker, but
    // its observable behavior is: streamed `WorkerMessage::Event`s land in
    // `session_events` and are visible via `GET /v1/sessions/:id/events`.
    // This test exercises the server side of that contract by simulating a
    // streaming worker: connect to the headless session's events WS, send
    // an AssistantMessage event, and assert it shows up on the read API.
    let (state, store) = fresh_state();
    let server = spawn_test_server_with_state(state, store.clone()).await?;
    let client = test_client();

    let create_req = CreateSessionRequest {
        mode: SessionMode::Headless {
            conversation_id: None,
        },
        agent_config: Default::default(),
        mount_spec: Default::default(),
        image: None,
        env_vars: Default::default(),
        cpu_limit: None,
        memory_limit: None,
        secrets: None,
        spawned_from: None,
        resumed_from: None,
        initial_prompt: Some("scan".to_string()),
    };
    let created: CreateSessionResponse = client
        .post(format!("{}/v1/sessions", server.base_url()))
        .json(&create_req)
        .send()
        .await?
        .json()
        .await?;
    let session_id = created.session_id;

    let mut ws = connect_ws(&server.base_url(), &session_id).await?;
    let _ = phase1_fresh(&mut ws).await?;
    let _ = phase2_ready_with_timeout(&mut ws, Duration::from_secs(2)).await?;
    send(
        &mut ws,
        WorkerMessage::Event {
            event: SessionEvent::AssistantMessage {
                content: "the scan turned up nothing".to_string(),
                timestamp: chrono::Utc::now(),
            },
        },
    )
    .await?;
    // Give the server a moment to append.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let events: Vec<SessionEvent> = client
        .get(format!(
            "{}/v1/sessions/{session_id}/events",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(
        events.iter().any(|e| matches!(
            e,
            SessionEvent::AssistantMessage { content, .. } if content == "the scan turned up nothing"
        )),
        "headless session must surface streamed AssistantMessages on its event log: {events:?}"
    );

    Ok(())
}
