use crate::app::AppState;
use crate::app::ServiceState;
use crate::config::EventsSection;
use crate::job_engine::{JobEngine, JobStatus};
use crate::store::{MemoryStore, Store};
use crate::test_utils::{
    MockJobEngine, TestStateHandles, spawn_test_server, spawn_test_server_with_state,
    test_app_config, test_client, test_secret_manager, test_state_with_engine_handles,
};
use hydra_common::SessionId;
use hydra_common::api::v1::issues::UpsertIssueResponse;
use serde_json::json;
use std::sync::Arc;

fn state_with_replay_capacity(capacity: usize) -> TestStateHandles {
    let mut config = test_app_config();
    config.events = EventsSection {
        replay_buffer_capacity: capacity,
    };
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        test_secret_manager(),
    );
    TestStateHandles { state, store }
}

#[tokio::test]
async fn events_endpoint_returns_sse_content_type() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    let response = client
        .get(format!("{}/v1/events", server.base_url()))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?;

    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get("content-type")
        .expect("response should have content-type");
    assert!(
        content_type.to_str().unwrap().contains("text/event-stream"),
        "expected text/event-stream, got {content_type:?}",
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_requires_auth() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/v1/events", server.base_url()))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?;

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn events_endpoint_sends_connected_on_first_connect() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Connect to SSE stream and read the first events.
    let mut response = client
        .get(format!("{}/v1/events", server.base_url()))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;

    assert!(response.status().is_success());

    // Read chunks until we get the connected event or timeout.
    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains("event: connected") {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        accumulated.contains("event: connected"),
        "expected connected event in SSE stream, got: {accumulated}"
    );
    // The connected event should include a current_seq field.
    assert!(
        accumulated.contains("current_seq"),
        "connected event should contain current_seq, got: {accumulated}",
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_streams_issue_mutations() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Start SSE stream in background.
    let base_url = server.base_url();
    let stream_client = test_client();
    let stream_handle = tokio::spawn(async move {
        let mut response = stream_client
            .get(format!("{base_url}/v1/events"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .expect("SSE request should succeed");

        let mut accumulated = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk())
                .await
            {
                Ok(Ok(Some(chunk))) => {
                    accumulated.push_str(&String::from_utf8_lossy(&chunk));
                    // Wait until we see an issue_created event (not just the snapshot).
                    if accumulated.contains("event: issue_created") {
                        break;
                    }
                }
                _ => break,
            }
        }
        accumulated
    });

    // Give the SSE connection time to establish.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Create an issue via the API.
    let create_resp = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&json!({
            "issue": {
                "type": "task",
                "description": "test issue for SSE",
                "creator": "tester",
                "status": "open",
                "project_id": "j-defaul"
            }
        }))
        .send()
        .await?;
    assert!(create_resp.status().is_success());
    let created: UpsertIssueResponse = create_resp.json().await?;

    // Wait for the SSE stream to receive the event.
    let sse_body = stream_handle.await?;

    assert!(
        sse_body.contains("event: issue_created"),
        "SSE stream should contain issue_created event, got: {sse_body}"
    );
    assert!(
        sse_body.contains(&created.issue_id.to_string()),
        "SSE stream should contain created issue ID {}, got: {sse_body}",
        created.issue_id
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_sends_resync_when_last_event_id_below_buffer() -> anyhow::Result<()> {
    // Replay buffer of 1 — every additional event evicts the oldest, so a
    // client reconnecting with last_event_id=1 ends up below the buffer
    // and must receive a resync.
    let handles = state_with_replay_capacity(1);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    // Create three issues so the buffer (capacity=1) holds only seq=3 and
    // the requested cursor (last_event_id=1, needing 2..) is below the oldest.
    for description in ["advance seq 1", "advance seq 2", "advance seq 3"] {
        let create_resp = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&json!({
                "issue": {
                    "type": "task",
                    "description": description,
                    "creator": "tester",
                    "status": "open",
                    "project_id": "j-defaul"
                }
            }))
            .send()
            .await?;
        assert!(create_resp.status().is_success());
    }

    let mut response = client
        .get(format!("{}/v1/events", server.base_url()))
        .header("last-event-id", "1")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;

    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains("event: resync") {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        accumulated.contains("event: resync"),
        "expected resync event on reconnect, got: {accumulated}"
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_replays_buffered_events_on_reconnect() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create three issues so seqs 1..=3 sit in the (default-sized) replay buffer.
    let mut issue_ids = Vec::new();
    for description in ["first", "second", "third"] {
        let create_resp = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&json!({
                "issue": {
                    "type": "task",
                    "description": description,
                    "creator": "tester",
                    "status": "open",
                    "project_id": "j-defaul"
                }
            }))
            .send()
            .await?;
        assert!(create_resp.status().is_success());
        let body: UpsertIssueResponse = create_resp.json().await?;
        issue_ids.push(body.issue_id);
    }

    // Reconnect with Last-Event-ID=1. Buffer holds seqs 1..=3, so the server
    // should stream issue_created events for seqs 2 and 3 (and NOT resync).
    let mut response = client
        .get(format!("{}/v1/events", server.base_url()))
        .header("last-event-id", "1")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.matches("event: issue_created").count() >= 2 {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        !accumulated.contains("event: resync"),
        "buffer covered the client cursor; no resync should be sent, got: {accumulated}"
    );
    let replayed = accumulated.matches("event: issue_created").count();
    assert_eq!(
        replayed, 2,
        "expected 2 replayed issue_created events (seqs 2 and 3), got {replayed}: {accumulated}"
    );
    assert!(
        accumulated.contains(&issue_ids[1].to_string())
            && accumulated.contains(&issue_ids[2].to_string()),
        "replay should include the second and third issue ids, got: {accumulated}"
    );
    assert!(
        !accumulated.contains(&issue_ids[0].to_string()),
        "seq=1 should NOT be replayed (client already has it), got: {accumulated}"
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_replays_using_query_param_when_header_absent() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Emit two issues.
    let mut issue_ids = Vec::new();
    for description in ["one", "two"] {
        let create_resp = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&json!({
                "issue": {
                    "type": "task",
                    "description": description,
                    "creator": "tester",
                    "status": "open",
                    "project_id": "j-defaul"
                }
            }))
            .send()
            .await?;
        assert!(create_resp.status().is_success());
        let body: UpsertIssueResponse = create_resp.json().await?;
        issue_ids.push(body.issue_id);
    }

    // Reconnect using only the query param (no header).
    let mut response = client
        .get(format!("{}/v1/events?last_event_id=1", server.base_url()))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains(&issue_ids[1].to_string()) {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        accumulated.contains("event: issue_created"),
        "expected replayed issue_created via query-param last_event_id, got: {accumulated}"
    );
    assert!(
        accumulated.contains(&issue_ids[1].to_string()),
        "expected the second issue (seq=2) in the replay, got: {accumulated}"
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_replay_respects_entity_type_filter() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Emit two issues so they sit in the replay buffer.
    let mut issue_ids = Vec::new();
    for description in ["alpha", "beta"] {
        let create_resp = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&json!({
                "issue": {
                    "type": "task",
                    "description": description,
                    "creator": "tester",
                    "status": "open",
                    "project_id": "j-defaul"
                }
            }))
            .send()
            .await?;
        assert!(create_resp.status().is_success());
        let body: UpsertIssueResponse = create_resp.json().await?;
        issue_ids.push(body.issue_id);
    }

    // Reconnect with types=patches, asking to replay from seq 0. Even though
    // the buffer holds issue_created events, the filter excludes them.
    let mut response = client
        .get(format!(
            "{}/v1/events?types=patches&last_event_id=0",
            server.base_url()
        ))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?;
    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(300), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
            }
            _ => break,
        }
    }

    assert!(
        !accumulated.contains("event: issue_created"),
        "filter=patches must drop buffered issue_created events from replay, got: {accumulated}"
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_header_last_event_id_overrides_query_param() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Emit three issues.
    let mut issue_ids = Vec::new();
    for description in ["one", "two", "three"] {
        let create_resp = client
            .post(format!("{}/v1/issues", server.base_url()))
            .json(&json!({
                "issue": {
                    "type": "task",
                    "description": description,
                    "creator": "tester",
                    "status": "open",
                    "project_id": "j-defaul"
                }
            }))
            .send()
            .await?;
        assert!(create_resp.status().is_success());
        let body: UpsertIssueResponse = create_resp.json().await?;
        issue_ids.push(body.issue_id);
    }

    // Header says last_event_id=2, query says 0. Header wins, so we expect a
    // single replayed event for the third issue (seq=3) — not the second.
    let mut response = client
        .get(format!("{}/v1/events?last_event_id=0", server.base_url()))
        .header("last-event-id", "2")
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains(&issue_ids[2].to_string()) {
                    break;
                }
            }
            _ => break,
        }
    }

    let replayed = accumulated.matches("event: issue_created").count();
    assert_eq!(
        replayed, 1,
        "expected exactly 1 replayed event (seq=3) when header overrides query, got {replayed}: {accumulated}"
    );
    assert!(
        accumulated.contains(&issue_ids[2].to_string()),
        "expected the third issue id in the replay, got: {accumulated}"
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_multiplexes_session_logs_for_subscribed_sessions() -> anyhow::Result<()> {
    let mock_engine = Arc::new(MockJobEngine::new());

    let session_a = SessionId::new();
    let session_b = SessionId::new();
    mock_engine.insert_job(&session_a, JobStatus::Running).await;
    mock_engine.insert_job(&session_b, JobStatus::Running).await;
    mock_engine
        .set_logs(&session_a, vec!["alpha-line\n".to_string()])
        .await;
    mock_engine
        .set_logs(&session_b, vec!["beta-line\n".to_string()])
        .await;

    let job_engine: Arc<dyn JobEngine> = mock_engine.clone();
    let handles = test_state_with_engine_handles(job_engine);
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client();

    let url = format!("{}/v1/events?session_ids={}", server.base_url(), session_a,);
    let mut response = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    assert!(response.status().is_success());

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains("alpha-line") {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        accumulated.contains("event: session_log"),
        "expected session_log event in stream, got: {accumulated}",
    );
    assert!(
        accumulated.contains("alpha-line"),
        "expected log chunk for subscribed session A, got: {accumulated}",
    );
    assert!(
        accumulated.contains(&session_a.to_string()),
        "session_log payload should include subscribed session_id {session_a}, got: {accumulated}",
    );
    assert!(
        !accumulated.contains("beta-line"),
        "stream must not include log chunks for unsubscribed session B, got: {accumulated}",
    );
    assert!(
        !accumulated.contains(&session_b.to_string()),
        "stream must not reference unsubscribed session B id {session_b}, got: {accumulated}",
    );

    Ok(())
}

#[tokio::test]
async fn events_endpoint_filters_by_entity_type() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Start SSE stream filtered to only patches.
    let base_url = server.base_url();
    let stream_client = test_client();
    let stream_handle = tokio::spawn(async move {
        let mut response = stream_client
            .get(format!("{base_url}/v1/events?types=patches"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .expect("SSE request should succeed");

        let mut accumulated = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk())
                .await
            {
                Ok(Ok(Some(chunk))) => {
                    accumulated.push_str(&String::from_utf8_lossy(&chunk));
                }
                _ => break,
            }
        }
        accumulated
    });

    // Give the SSE connection time to establish.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Create an issue (should not appear in the filtered stream).
    let create_resp = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&json!({
            "issue": {
                "type": "task",
                "description": "should be filtered out",
                "creator": "tester",
                "status": "open",
                "project_id": "j-defaul"
            }
        }))
        .send()
        .await?;
    assert!(create_resp.status().is_success());

    let sse_body = stream_handle.await?;

    // The stream should NOT contain issue events since we filtered to patches only.
    assert!(
        !sse_body.contains("event: issue_created"),
        "filtered stream should not contain issue events, got: {sse_body}"
    );

    Ok(())
}
