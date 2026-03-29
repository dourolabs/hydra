use crate::{
    app::DeviceSession,
    config::AuthConfig,
    test::{spawn_test_server_with_state, test_client_without_auth, test_state_with_github_urls},
    test_utils::{TestStateHandles, test_app_config, test_secret_manager},
};
use httpmock::prelude::*;
use hydra_common::api::v1::login::{DevicePollResponse, DeviceStartResponse};
use reqwest::StatusCode;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    app::{AppState, ServiceState},
    store::MemoryStore,
    test_utils::MockJobEngine,
};

fn test_state_with_local_auth() -> TestStateHandles {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test".to_string(),
        username: None,
        auth_token_file: None,
    };
    let store = Arc::new(MemoryStore::new());
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

// --- Auth mode gating ---

#[tokio::test]
async fn device_start_returns_404_when_auth_is_local() -> anyhow::Result<()> {
    let handles = test_state_with_local_auth();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let response = client
        .post(format!("{}/v1/login/device/start", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn device_poll_returns_404_when_auth_is_local() -> anyhow::Result<()> {
    let handles = test_state_with_local_auth();
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": "ds-nonexistent" });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

// --- device_start success ---

#[tokio::test]
async fn device_start_returns_session_on_success() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/device/code");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "device_code": "dc-test-123",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 5
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let response = client
        .post(format!("{}/v1/login/device/start", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    let body: DeviceStartResponse = response.json().await?;
    assert!(body.device_session_id.starts_with("ds-"));
    assert_eq!(body.user_code, "ABCD-1234");
    assert_eq!(body.verification_uri, "https://github.com/login/device");
    assert_eq!(body.expires_in, 900);
    assert_eq!(body.interval, 5);

    Ok(())
}

// --- device_poll session not found ---

#[tokio::test]
async fn device_poll_returns_not_found_for_unknown_session() -> anyhow::Result<()> {
    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        "https://github.com".to_string(),
    );
    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": "ds-nonexistent" });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

// --- Session expiry ---

#[tokio::test]
async fn device_poll_returns_error_for_expired_session() -> anyhow::Result<()> {
    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        "https://github.com".to_string(),
    );

    // Insert an already-expired session directly into the DashMap.
    let session_id = "ds-expired-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-expired".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: "https://github.com".to_string(),
            expires_at: Instant::now() - Duration::from_secs(1),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: DevicePollResponse = response.json().await?;
    assert_eq!(
        body.status,
        hydra_common::api::v1::login::DevicePollStatus::Error
    );
    assert_eq!(body.error.as_deref(), Some("expired"));

    // Session should be cleaned up.
    assert!(!handles.state.device_sessions.contains_key(&session_id));

    Ok(())
}

// --- Rate limiting ---

#[tokio::test]
async fn device_poll_rejects_when_polling_too_fast() -> anyhow::Result<()> {
    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        "https://github.com".to_string(),
    );

    // Insert a session that was just polled.
    let session_id = "ds-ratelimit-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-rate".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: "https://github.com".to_string(),
            expires_at: Instant::now() + Duration::from_secs(900),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now(), // just polled
        },
    );

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

// --- GitHub error code mapping ---

#[tokio::test]
async fn device_poll_returns_pending_for_authorization_pending() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/oauth/access_token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "error": "authorization_pending"
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );

    let session_id = "ds-pending-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-pending".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: github_server.base_url(),
            expires_at: Instant::now() + Duration::from_secs(900),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    let server = spawn_test_server_with_state(handles.state, handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: DevicePollResponse = response.json().await?;
    assert_eq!(
        body.status,
        hydra_common::api::v1::login::DevicePollStatus::Pending
    );
    assert!(body.login_token.is_none());

    Ok(())
}

#[tokio::test]
async fn device_poll_increases_interval_on_slow_down() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/oauth/access_token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "error": "slow_down"
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );

    let session_id = "ds-slowdown-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-slow".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: github_server.base_url(),
            expires_at: Instant::now() + Duration::from_secs(900),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: DevicePollResponse = response.json().await?;
    assert_eq!(
        body.status,
        hydra_common::api::v1::login::DevicePollStatus::Pending
    );

    // Verify the poll interval was increased by 5 seconds.
    let session = handles
        .state
        .device_sessions
        .get(&session_id)
        .expect("session should still exist");
    assert_eq!(session.poll_interval, Duration::from_secs(10));

    Ok(())
}

#[tokio::test]
async fn device_poll_returns_error_for_access_denied() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/oauth/access_token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "error": "access_denied"
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );

    let session_id = "ds-denied-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-denied".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: github_server.base_url(),
            expires_at: Instant::now() + Duration::from_secs(900),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: DevicePollResponse = response.json().await?;
    assert_eq!(
        body.status,
        hydra_common::api::v1::login::DevicePollStatus::Error
    );
    assert_eq!(body.error.as_deref(), Some("access_denied"));

    // Session should be removed.
    assert!(!handles.state.device_sessions.contains_key(&session_id));

    Ok(())
}

#[tokio::test]
async fn device_poll_returns_error_for_expired_token() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/oauth/access_token");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "error": "expired_token"
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );

    let session_id = "ds-expired-token-test".to_string();
    handles.state.device_sessions.insert(
        session_id.clone(),
        DeviceSession {
            device_code: "dc-exp-token".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: github_server.base_url(),
            expires_at: Instant::now() + Duration::from_secs(900),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store).await?;
    let client = test_client_without_auth();

    let payload = serde_json::json!({ "device_session_id": session_id });
    let response = client
        .post(format!("{}/v1/login/device/poll", server.base_url()))
        .json(&payload)
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: DevicePollResponse = response.json().await?;
    assert_eq!(
        body.status,
        hydra_common::api::v1::login::DevicePollStatus::Error
    );
    assert_eq!(body.error.as_deref(), Some("expired"));

    // Session should be removed.
    assert!(!handles.state.device_sessions.contains_key(&session_id));

    Ok(())
}

// --- Lazy cleanup ---

#[tokio::test]
async fn device_start_cleans_up_expired_sessions() -> anyhow::Result<()> {
    let github_server = MockServer::start_async().await;
    let _mock = github_server.mock(|when, then| {
        when.method(POST).path("/login/device/code");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(serde_json::json!({
                "device_code": "dc-new",
                "user_code": "NEW-CODE",
                "verification_uri": "https://github.com/login/device",
                "expires_in": 900,
                "interval": 5
            }));
    });

    let handles = test_state_with_github_urls(
        "https://api.github.com".to_string(),
        github_server.base_url(),
    );

    // Insert an expired session.
    handles.state.device_sessions.insert(
        "ds-stale".to_string(),
        DeviceSession {
            device_code: "dc-stale".to_string(),
            github_client_id: "client-id".to_string(),
            oauth_base_url: github_server.base_url(),
            expires_at: Instant::now() - Duration::from_secs(1),
            poll_interval: Duration::from_secs(5),
            last_poll: Instant::now() - Duration::from_secs(10),
        },
    );

    assert!(handles.state.device_sessions.contains_key("ds-stale"));

    let server = spawn_test_server_with_state(handles.state.clone(), handles.store).await?;
    let client = test_client_without_auth();

    let response = client
        .post(format!("{}/v1/login/device/start", server.base_url()))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);

    // The stale session should have been cleaned up.
    assert!(!handles.state.device_sessions.contains_key("ds-stale"));

    Ok(())
}
