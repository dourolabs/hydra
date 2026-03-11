use crate::app::AppState;
use crate::domain::actors::{ActorId, AuthToken};
use crate::domain::whoami::{ActorIdentity, WhoAmIResponse};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use metis_common::api::v1::ApiError;
use serde::Deserialize;
use tracing::info;

const COOKIE_NAME: &str = "metis_token";

#[derive(Deserialize)]
pub struct LoginRequest {
    token: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
}

async fn validate_token(state: &AppState, raw_token: &str) -> Result<ActorIdentity, ApiError> {
    let auth_token =
        AuthToken::parse(raw_token).map_err(|_| ApiError::unauthorized("invalid token"))?;

    let actor = state
        .get_actor(auth_token.actor_name())
        .await
        .map_err(|_| ApiError::unauthorized("invalid token"))?;

    if !actor.verify_auth_token(&auth_token) {
        return Err(ApiError::unauthorized("invalid token"));
    }

    let identity = match actor.actor_id {
        ActorId::Username(username) => ActorIdentity::User {
            username: username.into(),
        },
        ActorId::Task(task_id) => ActorIdentity::Task {
            task_id,
            creator: actor.creator.clone(),
        },
        ActorId::Issue(issue_id) => ActorIdentity::Issue {
            issue_id,
            creator: actor.creator.clone(),
        },
    };

    Ok(identity)
}

fn build_cookie(value: String) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, value))
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Strict)
        .path("/")
        .build()
}

fn build_removal_cookie() -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, ""))
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(time::Duration::ZERO)
        .build()
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(payload): Json<LoginRequest>,
) -> Result<
    (
        CookieJar,
        Json<metis_common::api::v1::whoami::WhoAmIResponse>,
    ),
    ApiError,
> {
    let token = payload
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::bad_request("token is required"))?;

    let identity = validate_token(&state, token).await?;
    let response: metis_common::api::v1::whoami::WhoAmIResponse =
        WhoAmIResponse::new(identity).into();

    info!(actor = ?response.actor, "bff login success");

    let jar = jar.add(build_cookie(token.to_string()));
    Ok((jar, Json(response)))
}

async fn me(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<Json<metis_common::api::v1::whoami::WhoAmIResponse>, ApiError> {
    let token = jar
        .get(COOKIE_NAME)
        .map(|c| c.value())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::unauthorized("not authenticated"))?;

    let identity = validate_token(&state, token).await?;
    let response: metis_common::api::v1::whoami::WhoAmIResponse =
        WhoAmIResponse::new(identity).into();

    Ok(Json(response))
}

async fn logout(jar: CookieJar) -> (CookieJar, Json<serde_json::Value>) {
    let jar = jar.add(build_removal_cookie());
    (jar, Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::actors::ActorRef;
    use crate::test_utils::{test_actor, test_auth_token, test_state_handles};
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;

    fn test_router(state: AppState) -> Router {
        router().with_state(state)
    }

    async fn setup() -> (AppState, String) {
        let handles = test_state_handles();
        let token = test_auth_token();
        let actor = test_actor();
        let _ = handles.store.add_actor(actor, &ActorRef::test()).await;
        (handles.state, token)
    }

    #[tokio::test]
    async fn login_with_valid_token_sets_cookie() {
        let (state, token) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "token": token }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("should set cookie")
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("metis_token="));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Strict"));
        assert!(set_cookie.contains("Path=/"));
    }

    #[tokio::test]
    async fn login_with_invalid_token_returns_401() {
        let (state, _) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "token": "u-fake:badtoken" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn login_with_missing_token_returns_400() {
        let (state, _) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::json!({}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn me_with_valid_cookie_returns_user_info() {
        let (state, token) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/auth/me")
                    .header(header::COOKIE, format!("metis_token={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn me_without_cookie_returns_401() {
        let (state, _) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/auth/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_with_invalid_cookie_returns_401() {
        let (state, _) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/auth/me")
                    .header(header::COOKIE, "metis_token=u-fake:badtoken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_clears_cookie() {
        let (state, token) = setup().await;

        let response = test_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/logout")
                    .header(header::COOKIE, format!("metis_token={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("should clear cookie")
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("metis_token="));
        assert!(set_cookie.contains("Max-Age=0"));
    }
}
