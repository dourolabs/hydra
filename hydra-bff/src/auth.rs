use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use hydra_common::api::v1::login::{DevicePollResponse, DevicePollStatus};
use tracing::{error, info};

use crate::state::BffState;
use crate::upstream::Upstream;

pub const COOKIE_NAME: &str = "hydra_token";

/// Build the `/auth` sub-router.
pub fn router<U: Upstream>() -> Router<BffState<U>> {
    Router::new()
        .route("/login/device/start", post(device_start::<U>))
        .route("/login/device/poll", post(device_poll::<U>))
        .route("/logout", post(auth_logout::<U>))
        .route("/me", get(auth_me::<U>))
}

async fn auth_logout<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
) -> impl IntoResponse {
    // When auto_login_token is set, logout is a no-op (no session to clear).
    if bff.auto_login_token.is_some() {
        info!(
            bff_path = "/auth/logout",
            status = 404u16,
            "logout no-op (auto-login mode)"
        );
        return StatusCode::NOT_FOUND.into_response();
    }

    let jar = jar.remove(Cookie::build(COOKIE_NAME).path("/"));
    info!(bff_path = "/auth/logout", status = 200u16, "logout success");
    (jar, axum::Json(serde_json::json!({ "ok": true }))).into_response()
}

async fn auth_me<U: Upstream>(State(bff): State<BffState<U>>, jar: CookieJar) -> impl IntoResponse {
    let token = match bff.resolve_token(&jar) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "not authenticated" })),
            )
                .into_response();
        }
    };

    info!(
        bff_path = "/auth/me",
        upstream_path = "/v1/whoami",
        "proxying auth check to upstream"
    );
    let whoami_req = http::Request::builder()
        .uri("/v1/whoami")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = match bff.upstream.forward(whoami_req).await {
        Ok(r) => {
            info!(bff_path = "/auth/me", upstream_path = "/v1/whoami", status = %r.status(), "upstream response received");
            r
        }
        Err(e) => {
            error!(bff_path = "/auth/me", upstream_path = "/v1/whoami", error = %e, "upstream request failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "internal error" })),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "token expired or invalid" })),
        )
            .into_response();
    }

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .unwrap_or_default();
    let user: serde_json::Value =
        serde_json::from_slice(&body_bytes).unwrap_or(serde_json::json!({}));

    axum::Json(user).into_response()
}

async fn device_start<U: Upstream>(
    State(bff): State<BffState<U>>,
    request: http::Request<Body>,
) -> impl IntoResponse {
    info!(
        bff_path = "/auth/login/device/start",
        upstream_path = "/v1/login/device/start",
        "proxying device flow start to upstream"
    );
    crate::proxy::forward_to_upstream(&bff, "/v1/login/device/start", None, request).await
}

async fn device_poll<U: Upstream>(
    State(bff): State<BffState<U>>,
    jar: CookieJar,
    request: http::Request<Body>,
) -> impl IntoResponse {
    info!(
        bff_path = "/auth/login/device/poll",
        upstream_path = "/v1/login/device/poll",
        "proxying device flow poll to upstream"
    );

    let response =
        crate::proxy::forward_to_upstream(&bff, "/v1/login/device/poll", None, request).await;

    if !response.status().is_success() {
        return response;
    }

    // Read the response body to check if login completed.
    let (parts, body) = response.into_parts();
    let body_bytes = axum::body::to_bytes(body, 1024 * 64)
        .await
        .unwrap_or_default();

    // Try to parse the response to check for a login_token.
    if let Ok(poll_response) = serde_json::from_slice::<DevicePollResponse>(&body_bytes) {
        if poll_response.status == DevicePollStatus::Complete {
            if let Some(ref login_token) = poll_response.login_token {
                info!("device flow login complete, setting auth cookie");
                let cookie = build_auth_cookie(login_token, bff.config.cookie_secure);
                let jar = jar.add(cookie);
                return (
                    jar,
                    axum::response::Response::from_parts(parts, Body::from(body_bytes)),
                )
                    .into_response();
            }
        }
    }

    // Return the response as-is (pending or error status).
    axum::response::Response::from_parts(parts, Body::from(body_bytes))
}

pub fn build_auth_cookie(token: &str, secure: bool) -> Cookie<'static> {
    let mut builder = Cookie::build((COOKIE_NAME, token.to_string()))
        .http_only(true)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .path("/")
        .max_age(time::Duration::days(30));
    if secure {
        builder = builder.secure(true);
    }
    builder.build()
}
