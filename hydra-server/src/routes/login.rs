use crate::app::{AppState, DeviceSession, LoginError, WORKER_NAME_LOGIN};
use crate::config::AuthConfig;
use crate::domain::actors::ActorRef;
use crate::routes::sessions::ApiError;
use axum::{Json, extract::State};
use hydra_common::api::v1;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use uuid::Uuid;

const GITHUB_SCOPE: &str = "read:user";
const USER_AGENT_VALUE: &str = "hydra-server";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<v1::login::LoginRequest>,
) -> Result<Json<v1::login::LoginResponse>, ApiError> {
    let github_token = normalize_non_empty("github_token", payload.github_token)?;
    let github_refresh_token =
        normalize_non_empty("github_refresh_token", payload.github_refresh_token)?;
    info!("login invoked");

    let login_actor = ActorRef::System {
        worker_name: WORKER_NAME_LOGIN.into(),
        on_behalf_of: None,
    };
    let response = state
        .login_with_github_token(github_token, github_refresh_token, login_actor)
        .await
        .map_err(map_login_error)?;

    info!(username = %response.user.username, "login completed");
    Ok(Json(response))
}

// --- Device Flow handlers ---

#[derive(Debug, Deserialize)]
struct GithubDeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct GithubTokenPollResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
}

pub async fn device_start(
    State(state): State<AppState>,
) -> Result<Json<v1::login::DeviceStartResponse>, ApiError> {
    let github_app = require_github_auth(&state.config.auth)?;

    // Lazy cleanup of expired sessions before creating a new one.
    cleanup_expired_sessions(&state);

    let client_id = github_app.client_id().to_string();
    let oauth_base_url = github_app.oauth_base_url().to_string();
    let device_code_url = format!("{oauth_base_url}/login/device/code");

    info!("device_start: requesting device code from GitHub");

    let http = reqwest::Client::new();
    let response = http
        .post(&device_code_url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, USER_AGENT_VALUE)
        .form(&[("client_id", client_id.as_str()), ("scope", GITHUB_SCOPE)])
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to contact GitHub device code endpoint");
            ApiError::internal("failed to contact GitHub for device flow")
        })?
        .error_for_status()
        .map_err(|err| {
            error!(error = %err, "GitHub device code endpoint returned error");
            ApiError::internal("GitHub device code request failed")
        })?;

    let github_response: GithubDeviceCodeResponse = response.json().await.map_err(|err| {
        error!(error = %err, "failed to decode GitHub device code response");
        ApiError::internal("failed to decode GitHub device flow response")
    })?;

    let device_session_id = format!("ds-{}", Uuid::new_v4());
    let interval = github_response.interval.max(1);

    let session = DeviceSession {
        device_code: github_response.device_code,
        github_client_id: client_id,
        oauth_base_url,
        expires_at: Instant::now() + Duration::from_secs(github_response.expires_in),
        poll_interval: Duration::from_secs(interval),
        last_poll: Instant::now() - Duration::from_secs(interval), // allow immediate first poll
    };

    state
        .device_sessions
        .insert(device_session_id.clone(), session);

    info!(device_session_id = %device_session_id, "device flow session created");

    Ok(Json(v1::login::DeviceStartResponse::new(
        device_session_id,
        github_response.user_code,
        github_response.verification_uri,
        github_response.expires_in,
        interval,
    )))
}

pub async fn device_poll(
    State(state): State<AppState>,
    Json(payload): Json<v1::login::DevicePollRequest>,
) -> Result<Json<v1::login::DevicePollResponse>, ApiError> {
    require_github_auth(&state.config.auth)?;

    let device_session_id = &payload.device_session_id;

    // Check session exists and extract fields needed for the GitHub request.
    let (device_code, client_id, oauth_base_url) = {
        let mut session = state
            .device_sessions
            .get_mut(device_session_id)
            .ok_or_else(|| ApiError::not_found("device session not found or expired"))?;

        // Check expiry.
        if Instant::now() >= session.expires_at {
            drop(session);
            state.device_sessions.remove(device_session_id);
            return Ok(Json(v1::login::DevicePollResponse::error(
                "expired".to_string(),
            )));
        }

        // Rate-limit: reject polls that arrive faster than the GitHub-mandated interval.
        let elapsed = session.last_poll.elapsed();
        if elapsed < session.poll_interval {
            return Err(ApiError::too_many_requests(format!(
                "polling too fast; retry after {} seconds",
                (session.poll_interval - elapsed).as_secs() + 1
            )));
        }

        session.last_poll = Instant::now();

        (
            session.device_code.clone(),
            session.github_client_id.clone(),
            session.oauth_base_url.clone(),
        )
    };

    let token_url = format!("{oauth_base_url}/login/oauth/access_token");

    let http = reqwest::Client::new();
    let response = http
        .post(&token_url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, USER_AGENT_VALUE)
        .form(&[
            ("client_id", client_id.as_str()),
            ("device_code", device_code.as_str()),
            ("grant_type", DEVICE_GRANT_TYPE),
        ])
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to poll GitHub token endpoint");
            ApiError::internal("failed to poll GitHub for device flow token")
        })?
        .error_for_status()
        .map_err(|err| {
            error!(error = %err, "GitHub token endpoint returned error");
            ApiError::internal("GitHub token poll request failed")
        })?;

    let github_response: GithubTokenPollResponse = response.json().await.map_err(|err| {
        error!(error = %err, "failed to decode GitHub token poll response");
        ApiError::internal("failed to decode GitHub token response")
    })?;

    // If we got an access token, complete the login.
    if let Some(access_token) = github_response.access_token {
        let refresh_token = github_response.refresh_token.unwrap_or_default();

        let login_actor = ActorRef::System {
            worker_name: WORKER_NAME_LOGIN.into(),
            on_behalf_of: None,
        };
        let login_response = state
            .login_with_github_token(access_token, refresh_token, login_actor)
            .await
            .map_err(map_login_error)?;

        // Clean up the session on success.
        state.device_sessions.remove(device_session_id);

        info!(username = %login_response.user.username, "device flow login completed");

        return Ok(Json(v1::login::DevicePollResponse::complete(
            login_response.login_token,
            login_response.user,
        )));
    }

    // Handle GitHub error codes.
    match github_response.error.as_deref() {
        Some("authorization_pending") => Ok(Json(v1::login::DevicePollResponse::pending())),
        Some("slow_down") => {
            // Increase the poll interval as GitHub requests.
            if let Some(mut session) = state.device_sessions.get_mut(device_session_id) {
                session.poll_interval += Duration::from_secs(5);
            }
            Ok(Json(v1::login::DevicePollResponse::pending()))
        }
        Some("expired_token") => {
            state.device_sessions.remove(device_session_id);
            Ok(Json(v1::login::DevicePollResponse::error(
                "expired".to_string(),
            )))
        }
        Some("access_denied") => {
            state.device_sessions.remove(device_session_id);
            Ok(Json(v1::login::DevicePollResponse::error(
                "access_denied".to_string(),
            )))
        }
        Some(other) => {
            warn!(error = %other, "unexpected GitHub device flow error");
            state.device_sessions.remove(device_session_id);
            Ok(Json(v1::login::DevicePollResponse::error(
                other.to_string(),
            )))
        }
        None => {
            error!("GitHub token response had no access_token and no error field");
            Ok(Json(v1::login::DevicePollResponse::error(
                "unknown".to_string(),
            )))
        }
    }
}

/// Returns the GitHub app config or 404 if auth mode is Local.
fn require_github_auth(auth: &AuthConfig) -> Result<&crate::config::GithubAppSection, ApiError> {
    auth.github_app()
        .ok_or_else(|| ApiError::not_found("device flow login is not available"))
}

/// Remove expired device sessions from the in-memory map.
fn cleanup_expired_sessions(state: &AppState) {
    let now = Instant::now();
    state
        .device_sessions
        .retain(|_, session| session.expires_at > now);
}

fn normalize_non_empty(field: &str, value: String) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(trimmed.to_string())
}

fn map_login_error(error: LoginError) -> ApiError {
    match error {
        LoginError::InvalidGithubToken(message) => {
            error!(error = %message, "login failed with invalid token");
            ApiError::bad_request("invalid GitHub token")
        }
        LoginError::ForbiddenGithubOrg { username } => {
            error!(username = %username, "login rejected by allowed orgs");
            ApiError::unauthorized("GitHub user is not in an allowed organization")
        }
        LoginError::Store { source } => {
            error!(error = %source, "login failed to store actor");
            ApiError::internal(format!("failed to login: {source}"))
        }
    }
}
