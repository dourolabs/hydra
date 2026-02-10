use crate::test_utils::{spawn_test_server, test_client};
use metis_common::api::v1::issues::UpsertIssueResponse;
use serde_json::json;

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
async fn events_endpoint_sends_snapshot_on_first_connect() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create an issue first so the snapshot has something to include.
    let create_resp = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&json!({
            "issue": {
                "type": "task",
                "description": "test issue for snapshot",
                "creator": "tester",
                "progress": "",
                "status": "open"
            }
        }))
        .send()
        .await?;
    assert!(create_resp.status().is_success());
    let created: UpsertIssueResponse = create_resp.json().await?;

    // Connect to SSE stream and read the first events.
    let mut response = client
        .get(format!("{}/v1/events", server.base_url()))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;

    assert!(response.status().is_success());

    // Read chunks until we get the snapshot event or timeout.
    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                accumulated.push_str(&String::from_utf8_lossy(&chunk));
                if accumulated.contains("event: snapshot") {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        accumulated.contains("event: snapshot"),
        "expected snapshot event in SSE stream, got: {accumulated}"
    );
    // The snapshot should include the issue we created.
    assert!(
        accumulated.contains(&created.issue_id.to_string()),
        "snapshot should contain the created issue ID {}, got: {accumulated}",
        created.issue_id
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
                "progress": "",
                "status": "open"
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
async fn events_endpoint_sends_resync_on_reconnect() -> anyhow::Result<()> {
    let server = spawn_test_server().await?;
    let client = test_client();

    // Create an issue to advance the event bus sequence counter.
    let create_resp = client
        .post(format!("{}/v1/issues", server.base_url()))
        .json(&json!({
            "issue": {
                "type": "task",
                "description": "advance seq",
                "creator": "tester",
                "progress": "",
                "status": "open"
            }
        }))
        .send()
        .await?;
    assert!(create_resp.status().is_success());

    // Connect with Last-Event-ID of 1, which is older than current seq.
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
                "progress": "",
                "status": "open"
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
