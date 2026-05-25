use crate::{
    app::AppState,
    config::GithubAppSection,
    domain::{
        secrets::{SECRET_GITHUB_REFRESH_TOKEN, SECRET_GITHUB_TOKEN},
        users::Username,
    },
};
pub use hydra_common::{ActorId, ActorRef, parse_actor_name};
use hydra_common::{IssueId, SessionId, api::v1::ApiError, github::GithubTokenResponse};
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
    pub actor_id: ActorId,
    pub creator: Username,
    /// Session that minted the request's authenticating token, when the
    /// `Actor` was produced by [`crate::routes::auth::require_auth`].
    ///
    /// Runtime-only: `#[serde(skip)]` keeps the persisted `Actor` row
    /// shape unchanged. `require_auth` sets this from the matched
    /// `auth_tokens.session_id` so that `ActorRef::from(&actor)` carries
    /// the session id into every downstream mutation
    /// (`/designs/actor-system-overhaul.md` §5.2).
    #[serde(skip)]
    pub session_id: Option<SessionId>,
}

impl Actor {
    pub fn new_for_user(username: Username) -> (Actor, String) {
        let creator = username.clone();
        let actor_id = ActorId::Username(username.into());
        Self::new_with_token(actor_id, creator)
    }

    pub fn name(&self) -> String {
        self.actor_id.to_string()
    }

    pub fn new_for_session(session_id: SessionId, creator: Username) -> (Actor, String) {
        Self::new_with_token(ActorId::Session(session_id), creator)
    }

    pub fn new_for_service(service_name: String, creator: Username) -> (Actor, String) {
        Self::new_with_token(ActorId::Service(service_name), creator)
    }

    pub fn new_for_issue(issue_id: IssueId, creator: Username) -> (Actor, String) {
        Self::new_with_token(ActorId::Issue(issue_id), creator)
    }

    /// Build a new `Actor` from an already-constructed [`ActorId`].
    ///
    /// Phase 2 of the actor-system overhaul
    /// (`/designs/actor-system-overhaul.md` §3.4) routes
    /// `create_actor_for_job` through
    /// [`crate::domain::sessions::actor_id_of`]; this constructor accepts
    /// the resulting `ActorId` directly so the agent-vs-adhoc discriminant
    /// stays in one place. It's the typed replacement for
    /// `new_for_session` / `new_for_issue` on the job-spawn path.
    ///
    /// `ActorId::Legacy` is rejected at debug time: new writes must not
    /// produce that variant (see `ActorId` docs).
    pub fn new_from_actor_id(actor_id: ActorId, creator: Username) -> (Actor, String) {
        debug_assert!(
            !matches!(actor_id, ActorId::Legacy(_)),
            "Actor::new_from_actor_id must never be called with ActorId::Legacy"
        );
        Self::new_with_token(actor_id, creator)
    }

    /// Build a fresh `Actor` paired with a `"<actor_name>:<raw_token>"`
    /// authentication string. The caller is responsible for persisting
    /// the actor *and* inserting `Self::hash_auth_token(raw_token)` into
    /// the `auth_tokens` table — Phase 3b (`/designs/actor-system-overhaul.md`
    /// §9) deleted the legacy single-token hash on the actor row, so
    /// `auth_tokens` is now the only source of truth for verifying the
    /// returned string at the auth middleware.
    fn new_with_token(actor_id: ActorId, creator: Username) -> (Actor, String) {
        let raw_token = Uuid::new_v4().to_string();
        let actor = Actor {
            actor_id,
            creator,
            session_id: None,
        };
        let auth_token = format!("{}:{raw_token}", actor.name());
        (actor, auth_token)
    }

    pub fn hash_auth_token(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        let digest = hasher.finalize();
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(&mut encoded, "{byte:02x}").expect("writing to string should not fail");
        }
        encoded
    }

    pub fn parse_name(name: &str) -> Result<ActorId, ActorError> {
        parse_actor_name(name).ok_or_else(|| ActorError::InvalidActorName(name.to_string()))
    }

    pub async fn get_github_token(
        &self,
        state: &AppState,
    ) -> Result<GithubTokenResponse, ApiError> {
        get_github_token_for_user(state, &self.creator).await
    }
}

/// Resolve a GitHub token for the given `username`, refreshing it if expired.
///
/// Tokens are read from the encrypted `user_secrets` store. Refreshed tokens
/// are written back to `user_secrets`.
pub async fn get_github_token_for_user(
    state: &AppState,
    username: &Username,
) -> Result<GithubTokenResponse, ApiError> {
    info!(username = %username, "get_github_token_for_user invoked");

    let mut github_token = read_user_secret(state, username, SECRET_GITHUB_TOKEN).await?;

    // In local mode (no GitHub App configured), PATs don't support OAuth
    // refresh — just return the token as-is.
    let Some(github_app) = state.config.auth.github_app() else {
        info!(username = %username, "get_github_token_for_user completed (local mode, no refresh)");
        return Ok(GithubTokenResponse { github_token });
    };

    let github_refresh_token =
        read_user_secret(state, username, SECRET_GITHUB_REFRESH_TOKEN).await?;
    if !github_token_is_valid(github_app, &github_token).await? {
        let refreshed = refresh_github_token(github_app, &github_refresh_token).await?;

        // Write refreshed tokens to user_secrets (encrypted).
        store_github_token_secrets(
            state,
            username,
            &refreshed.access_token,
            &refreshed.refresh_token,
        )
        .await;

        github_token = refreshed.access_token;
    }

    info!(username = %username, "get_github_token_for_user completed");
    Ok(GithubTokenResponse { github_token })
}

/// Read a secret from encrypted user_secrets storage.
async fn read_user_secret(
    state: &AppState,
    username: &Username,
    secret_name: &str,
) -> Result<String, ApiError> {
    match state.store().get_user_secret(username, secret_name).await {
        Ok(Some(encrypted)) => match state.secret_manager.decrypt(&encrypted) {
            Ok(value) if !value.is_empty() => Ok(value),
            Ok(_) => {
                error!(username = %username, secret = secret_name, "user secret is empty");
                Err(ApiError::not_found(format!(
                    "GitHub token not found for user '{username}'"
                )))
            }
            Err(err) => {
                error!(username = %username, secret = secret_name, error = %err, "failed to decrypt user secret");
                Err(ApiError::internal(format!(
                    "failed to decrypt secret for user '{username}': {err}"
                )))
            }
        },
        Ok(None) => {
            error!(username = %username, secret = secret_name, "user secret not found");
            Err(ApiError::not_found(format!(
                "GitHub token not found for user '{username}'"
            )))
        }
        Err(err) => {
            error!(username = %username, secret = secret_name, error = %err, "failed to look up user secret");
            Err(ApiError::internal(format!(
                "failed to look up secret for user '{username}': {err}"
            )))
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
                    .set_user_secret(username, secret_name, &encrypted, true)
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
        .header(USER_AGENT, "hydra-server")
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
            Err(ApiError::internal(format!(
                "unexpected response while validating github token: {status}"
            )))
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
        .header(USER_AGENT, "hydra-server")
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
    Err(ApiError::unauthorized(format!(
        "GitHub token refresh failed: {message}"
    )))
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
            session_id: actor.session_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydra_common::api::v1::users::Username as CommonUsername;
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
        let prefix = format!("{}:", actor.name());
        let raw_token = auth_token
            .strip_prefix(&prefix)
            .expect("auth token should include actor name prefix");
        assert!(!raw_token.is_empty());
        // Parsing the formatted token must round-trip through `AuthToken`.
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");
        assert_eq!(parsed.actor_name(), actor.name());
        assert_eq!(parsed.raw_token(), raw_token);
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
    fn new_for_session_returns_session_actor() {
        let session_id = SessionId::new();
        let (actor, auth_token) =
            Actor::new_for_session(session_id.clone(), Username::from("creator"));
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");

        assert_eq!(actor.actor_id, ActorId::Session(session_id));
        assert_eq!(parsed.actor_name(), actor.name());
    }

    #[test]
    fn new_for_issue_creates_issue_actor() {
        let issue_id = IssueId::from_str("i-abcdef").unwrap();
        let (actor, auth_token) = Actor::new_for_issue(issue_id.clone(), Username::from("creator"));

        assert!(!auth_token.is_empty());
        assert_eq!(actor.actor_id, ActorId::Issue(issue_id));
        assert_eq!(actor.name(), "a-i-abcdef");

        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");
        assert_eq!(parsed.actor_name(), actor.name());
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
                session_id: None,
            }
        );
    }

    #[test]
    fn from_actor_ref_propagates_session_id() {
        let mut actor = Actor {
            actor_id: ActorId::Agent(
                hydra_common::api::v1::agents::AgentName::try_new("swe").unwrap(),
            ),
            creator: Username::from("creator"),
            session_id: None,
        };
        let sid = SessionId::new();
        actor.session_id = Some(sid.clone());
        let actor_ref = ActorRef::from(&actor);
        match actor_ref {
            ActorRef::Authenticated { session_id, .. } => assert_eq!(session_id, Some(sid)),
            other => panic!("expected Authenticated, got {other:?}"),
        }
    }

    #[test]
    fn new_for_service_creates_service_actor() {
        let (actor, auth_token) =
            Actor::new_for_service("bff".to_string(), Username::from("admin"));

        assert!(!auth_token.is_empty());
        assert_eq!(actor.actor_id, ActorId::Service("bff".to_string()));
        assert_eq!(actor.name(), "svc-bff");
        assert_eq!(actor.creator, Username::from("admin"));

        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");
        assert_eq!(parsed.actor_name(), actor.name());
    }
}
