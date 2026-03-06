use crate::{
    app::AppState,
    config::GithubAppSection,
    domain::{
        secrets::{SECRET_GITHUB_REFRESH_TOKEN, SECRET_GITHUB_TOKEN},
        users::Username,
    },
    store::StoreError,
};
pub use metis_common::{ActorId, ActorRef, parse_actor_name};
use metis_common::{IssueId, TaskId, api::v1::ApiError, github::GithubTokenResponse};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Placeholder value used when backfilling NULL creator columns during migration.
/// Every use of this constant represents a location that should eventually be
/// removed once all creators are guaranteed non-NULL in the database.
pub const UNKNOWN_CREATOR: &str = "unknown";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ActorError {
    #[error("Invalid actor name: {0}")]
    InvalidActorName(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthTokenError {
    #[error("Invalid auth token format")]
    InvalidFormat,
    #[error("Invalid actor name: {0}")]
    InvalidActorName(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken {
    actor_name: String,
    raw_token: String,
}

impl AuthToken {
    pub fn parse(token: &str) -> Result<Self, AuthTokenError> {
        let (actor_name, raw_token) = token
            .split_once(':')
            .filter(|(name, raw_token)| !name.is_empty() && !raw_token.is_empty())
            .ok_or(AuthTokenError::InvalidFormat)?;

        Actor::parse_name(actor_name)
            .map_err(|_| AuthTokenError::InvalidActorName(actor_name.to_string()))?;

        Ok(Self {
            actor_name: actor_name.to_string(),
            raw_token: raw_token.to_string(),
        })
    }

    pub fn actor_name(&self) -> &str {
        &self.actor_name
    }

    pub fn raw_token(&self) -> &str {
        &self.raw_token
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub auth_token_hash: String,
    pub auth_token_salt: String,
    pub actor_id: ActorId,
    pub creator: Username,
}

impl Actor {
    pub fn new_for_user(username: Username) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let creator = username.clone();
        let actor_id = ActorId::Username(username.into());
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            actor_id,
            creator,
        };
        let auth_token = Self::format_auth_token(&actor, &raw_auth_token);
        (actor, auth_token)
    }

    pub fn name(&self) -> String {
        self.actor_id.to_string()
    }

    pub fn verify_auth_token(&self, token: &AuthToken) -> bool {
        if token.actor_name() != self.name() {
            return false;
        }
        self.auth_token_hash == Self::hash_auth_token(token.raw_token())
    }

    pub fn new_for_task(task_id: TaskId, creator: Username) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let actor_id = ActorId::Task(task_id);
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            actor_id,
            creator,
        };
        let auth_token = Self::format_auth_token(&actor, &raw_auth_token);
        (actor, auth_token)
    }

    pub fn new_for_issue(issue_id: IssueId, creator: Username) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let actor_id = ActorId::Issue(issue_id);
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            actor_id,
            creator,
        };
        let auth_token = Self::format_auth_token(&actor, &raw_auth_token);
        (actor, auth_token)
    }

    fn generate_auth_token() -> (String, String, String) {
        let token = Uuid::new_v4().to_string();
        let salt = Uuid::new_v4().to_string();
        let hash = Self::hash_auth_token(&token);
        (token, hash, salt)
    }

    fn hash_auth_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let digest = hasher.finalize();
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(&mut encoded, "{byte:02x}").expect("writing to string should not fail");
        }
        encoded
    }

    fn format_auth_token(actor: &Actor, raw_token: &str) -> String {
        format!("{}:{raw_token}", actor.name())
    }

    pub fn parse_name(name: &str) -> Result<ActorId, ActorError> {
        parse_actor_name(name).ok_or_else(|| ActorError::InvalidActorName(name.to_string()))
    }

    pub async fn get_github_token(
        &self,
        state: &AppState,
    ) -> Result<GithubTokenResponse, ApiError> {
        get_github_token_for_user(state, &self.creator, &self.actor_id).await
    }
}

/// Resolve a GitHub token for the given `username`, refreshing it if expired.
///
/// Tokens are read from encrypted `user_secrets` first, falling back to the
/// (deprecated) plaintext columns in `users_v2`. Refreshed tokens are always
/// written back to `user_secrets`.
///
/// The `actor_id` is only used to record which actor triggered a token refresh
/// in the audit trail — it is not required for fetching the token itself.
pub async fn get_github_token_for_user(
    state: &AppState,
    username: &Username,
    actor_id: &ActorId,
) -> Result<GithubTokenResponse, ApiError> {
    info!(username = %username, "get_github_token_for_user invoked");

    let user = state.get_user(username).await.map_err(|err| match err {
        StoreError::UserNotFound(missing) => {
            error!(username = %missing, "user not found");
            ApiError::not_found(format!("user '{missing}' not found"))
        }
        other => {
            error!(username = %username, error = %other, "failed to load user");
            ApiError::internal(format!("failed to load user '{username}': {other}"))
        }
    })?;

    // Read tokens from user_secrets (encrypted), falling back to users_v2 (plaintext).
    let mut github_token =
        resolve_secret_or_fallback(state, username, SECRET_GITHUB_TOKEN, &user.github_token).await;
    let github_refresh_token = resolve_secret_or_fallback(
        state,
        username,
        SECRET_GITHUB_REFRESH_TOKEN,
        &user.github_refresh_token,
    )
    .await;

    if !github_token_is_valid(&state.config.github_app, &github_token).await? {
        let refreshed =
            refresh_github_token(&state.config.github_app, &github_refresh_token).await?;

        // Write refreshed tokens to user_secrets (encrypted).
        store_github_token_secrets(
            state,
            username,
            &refreshed.access_token,
            &refreshed.refresh_token,
        )
        .await;

        // Also update users_v2 for backward compatibility during migration.
        state
            .set_user_github_token(
                username,
                refreshed.access_token.clone(),
                user.github_user_id,
                refreshed.refresh_token.clone(),
                ActorRef::Authenticated {
                    actor_id: actor_id.clone(),
                },
            )
            .await
            .map_err(|err| match err {
                StoreError::UserNotFound(missing) => {
                    error!(username = %missing, "user not found");
                    ApiError::not_found(format!("user '{missing}' not found"))
                }
                other => {
                    error!(username = %username, error = %other, "failed to refresh github token");
                    ApiError::internal(format!(
                        "failed to refresh github token for '{username}': {other}"
                    ))
                }
            })?;

        github_token = refreshed.access_token;
    }

    info!(username = %username, "get_github_token_for_user completed");
    Ok(GithubTokenResponse { github_token })
}

/// Try to read a secret from user_secrets (encrypted). If unavailable, fall back
/// to the plaintext value from users_v2.
async fn resolve_secret_or_fallback(
    state: &AppState,
    username: &Username,
    secret_name: &str,
    fallback: &str,
) -> String {
    let secret_manager = &state.secret_manager;

    match state.store().get_user_secret(username, secret_name).await {
        Ok(Some(encrypted)) => match secret_manager.decrypt(&encrypted) {
            Ok(value) if !value.is_empty() => value,
            Ok(_) => fallback.to_string(),
            Err(err) => {
                warn!(
                    username = %username,
                    secret = secret_name,
                    error = %err,
                    "failed to decrypt user secret, falling back to users_v2"
                );
                fallback.to_string()
            }
        },
        Ok(None) => fallback.to_string(),
        Err(err) => {
            warn!(
                username = %username,
                secret = secret_name,
                error = %err,
                "failed to look up user secret, falling back to users_v2"
            );
            fallback.to_string()
        }
    }
}

/// Encrypt and store GitHub tokens in user_secrets. Logs warnings on failure
/// but does not propagate errors — the caller can still proceed with the
/// plaintext fallback in users_v2.
pub(crate) async fn store_github_token_secrets(
    state: &AppState,
    username: &Username,
    access_token: &str,
    refresh_token: &str,
) {
    let secret_manager = &state.secret_manager;

    for (secret_name, value) in [
        (SECRET_GITHUB_TOKEN, access_token),
        (SECRET_GITHUB_REFRESH_TOKEN, refresh_token),
    ] {
        if value.is_empty() {
            continue;
        }
        match secret_manager.encrypt(value) {
            Ok(encrypted) => {
                if let Err(err) = state
                    .store
                    .set_user_secret(username, secret_name, &encrypted)
                    .await
                {
                    warn!(
                        username = %username,
                        secret = secret_name,
                        error = %err,
                        "failed to store github token in user_secrets"
                    );
                }
            }
            Err(err) => {
                warn!(
                    username = %username,
                    secret = secret_name,
                    error = %err,
                    "failed to encrypt github token for user_secrets"
                );
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct GithubRefreshTokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

struct RefreshedGithubToken {
    access_token: String,
    refresh_token: String,
}

async fn github_token_is_valid(config: &GithubAppSection, token: &str) -> Result<bool, ApiError> {
    let url = join_url(config.api_base_url(), "/user");
    let response = Client::new()
        .get(url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, "metis-server")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to validate github token");
            ApiError::internal("failed to validate github token")
        })?;

    match response.status() {
        StatusCode::OK => Ok(true),
        StatusCode::UNAUTHORIZED => Ok(false),
        status => {
            error!(status = %status, "unexpected github token validation response");
            Err(ApiError::internal(
                "unexpected response while validating github token",
            ))
        }
    }
}

async fn refresh_github_token(
    config: &GithubAppSection,
    current_refresh_token: &str,
) -> Result<RefreshedGithubToken, ApiError> {
    let url = join_url(config.oauth_base_url(), "/login/oauth/access_token");
    let response = Client::new()
        .post(url)
        .header(ACCEPT, "application/json")
        .header(USER_AGENT, "metis-server")
        .form(&[
            ("client_id", config.client_id()),
            ("client_secret", config.client_secret()),
            ("grant_type", "refresh_token"),
            ("refresh_token", current_refresh_token),
        ])
        .send()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to refresh github token");
            ApiError::internal("failed to refresh github token")
        })?;

    let status = response.status();
    let payload = response
        .json::<GithubRefreshTokenResponse>()
        .await
        .map_err(|err| {
            error!(error = %err, "failed to decode github token refresh response");
            ApiError::internal("failed to decode github token refresh response")
        })?;

    if let Some(access_token) = payload.access_token {
        return Ok(RefreshedGithubToken {
            access_token,
            refresh_token: payload
                .refresh_token
                .unwrap_or_else(|| current_refresh_token.to_string()),
        });
    }

    let message = payload
        .error_description
        .or(payload.error)
        .unwrap_or_else(|| "github token refresh failed".to_string());

    error!(status = %status, error = %message, "github token refresh failed");
    Err(ApiError::unauthorized("GitHub token refresh failed"))
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

impl From<&Actor> for ActorRef {
    fn from(actor: &Actor) -> Self {
        ActorRef::Authenticated {
            actor_id: actor.actor_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metis_common::api::v1::users::Username as CommonUsername;
    use std::str::FromStr;

    #[test]
    fn new_for_user_creates_user_actor() {
        let username = Username::from("octo");
        let (actor, auth_token) = Actor::new_for_user(username.clone());

        assert!(!auth_token.is_empty());
        assert_eq!(
            actor.actor_id,
            ActorId::Username(CommonUsername::from("octo"))
        );
        assert!(!actor.auth_token_salt.is_empty());
        let prefix = format!("{}:", actor.name());
        let raw_token = auth_token
            .strip_prefix(&prefix)
            .expect("auth token should include actor name prefix");
        assert_eq!(actor.auth_token_hash, Actor::hash_auth_token(raw_token));
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");
        assert!(actor.verify_auth_token(&parsed));
    }

    #[test]
    fn parse_name_rejects_invalid_prefix() {
        let err = Actor::parse_name("x-123").expect_err("should reject invalid prefix");
        assert!(matches!(
            err,
            ActorError::InvalidActorName(name) if name == "x-123"
        ));
    }

    #[test]
    fn parse_name_rejects_empty_suffix() {
        let err = Actor::parse_name("u-").expect_err("should reject empty username");
        assert!(matches!(
            err,
            ActorError::InvalidActorName(name) if name == "u-"
        ));
    }

    #[test]
    fn verify_auth_token_requires_matching_actor_name() {
        let task_id = TaskId::new();
        let (actor, auth_token) = Actor::new_for_task(task_id, Username::from("creator"));
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");

        assert!(actor.verify_auth_token(&parsed));

        let invalid = format!("u-wrong:{}", auth_token.split_once(':').unwrap().1);
        let parsed_invalid = AuthToken::parse(&invalid).expect("auth token should parse");
        assert!(!actor.verify_auth_token(&parsed_invalid));
    }

    #[test]
    fn new_for_issue_creates_issue_actor() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let (actor, auth_token) = Actor::new_for_issue(issue_id.clone(), Username::from("creator"));

        assert!(!auth_token.is_empty());
        assert_eq!(actor.actor_id, ActorId::Issue(issue_id));
        assert_eq!(actor.name(), "a-i-abcdef");

        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");
        assert!(actor.verify_auth_token(&parsed));
    }

    #[test]
    fn actor_name_returns_a_prefix_for_issue() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let (actor, _) = Actor::new_for_issue(issue_id, Username::from("creator"));
        assert_eq!(actor.name(), "a-i-abcdef");
    }

    #[test]
    fn from_actor_ref() {
        let username = Username::from("alice");
        let (actor, _) = Actor::new_for_user(username);
        let actor_ref = ActorRef::from(&actor);
        assert_eq!(
            actor_ref,
            ActorRef::Authenticated {
                actor_id: ActorId::Username(CommonUsername::from("alice")),
            }
        );
    }
}
