use crate::{app::AppState, config::GithubAppSection, domain::users::Username, store::StoreError};
use metis_common::{TaskId, api::v1::ApiError, github::GithubTokenResponse};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::str::FromStr;
use tracing::{error, info};
use uuid::Uuid;

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
    #[serde(default)]
    pub creator: Option<Username>,
}

impl Actor {
    pub fn new_for_user(username: Username) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let creator = Some(username.clone());
        let actor_id = ActorId::Username(username);
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
        match &self.actor_id {
            ActorId::Username(username) => format!("u-{username}"),
            ActorId::Task(task_id) => format!("w-{task_id}"),
        }
    }

    pub fn verify_auth_token(&self, token: &AuthToken) -> bool {
        if token.actor_name() != self.name() {
            return false;
        }
        self.auth_token_hash == Self::hash_auth_token(token.raw_token())
    }

    pub fn new_for_task(task_id: TaskId, creator: Option<Username>) -> (Actor, String) {
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
        if let Some(username) = name.strip_prefix("u-") {
            if username.is_empty() {
                return Err(ActorError::InvalidActorName(name.to_string()));
            }
            return Ok(ActorId::Username(Username::from(username)));
        }

        if let Some(task_id) = name.strip_prefix("w-") {
            if task_id.is_empty() {
                return Err(ActorError::InvalidActorName(name.to_string()));
            }
            let task_id = TaskId::from_str(task_id)
                .map_err(|_| ActorError::InvalidActorName(name.to_string()))?;
            return Ok(ActorId::Task(task_id));
        }

        Err(ActorError::InvalidActorName(name.to_string()))
    }

    pub async fn get_github_token(
        &self,
        state: &AppState,
    ) -> Result<GithubTokenResponse, ApiError> {
        info!(actor = %self.name(), "get_github_token invoked");
        let username = self.creator.clone().ok_or_else(|| {
            error!(actor = %self.name(), "actor missing creator");
            ApiError::not_found(format!("actor '{}' has no creator", self.name()))
        })?;

        let user = state.get_user(&username).await.map_err(|err| match err {
            StoreError::UserNotFound(missing) => {
                error!(username = %missing, "user not found");
                ApiError::not_found(format!("user '{missing}' not found"))
            }
            other => {
                error!(username = %username, error = %other, "failed to load user");
                ApiError::internal(format!("failed to load user '{username}': {other}"))
            }
        })?;

        let mut github_token = user.github_token.clone();
        if !github_token_is_valid(&state.config.github_app, &github_token).await? {
            let refreshed =
                refresh_github_token(&state.config.github_app, &user.github_refresh_token).await?;
            let updated = state
                .set_user_github_token(
                    &username,
                    refreshed.access_token.clone(),
                    user.github_user_id,
                    refreshed.refresh_token.clone(),
                    Some(self.name()),
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

            github_token = updated.github_token;
        }

        info!(actor = %self.name(), username = %username, "get_github_token completed");
        Ok(GithubTokenResponse { github_token })
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorId {
    Username(Username),
    Task(TaskId),
}

/// A typed reference to who performed an operation.
///
/// Used in event payloads (`MutationPayload`) to attribute mutations.
/// During the migration period, `From<Option<String>>` provides backward
/// compatibility with callers that still pass `Option<String>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorRef {
    Authenticated {
        actor_id: ActorId,
    },
    System {
        worker_name: String,
        on_behalf_of: Option<ActorId>,
    },
    Automation {
        automation_name: String,
        triggered_by: Option<Box<ActorRef>>,
    },
}

impl ActorRef {
    /// Human-readable display name for this actor reference.
    pub fn display_name(&self) -> String {
        match self {
            ActorRef::Authenticated { actor_id } => match actor_id {
                ActorId::Username(username) => username.to_string(),
                ActorId::Task(task_id) => task_id.to_string(),
            },
            ActorRef::System {
                worker_name,
                on_behalf_of,
            } => {
                if let Some(behalf) = on_behalf_of {
                    let behalf_name = match behalf {
                        ActorId::Username(username) => username.to_string(),
                        ActorId::Task(task_id) => task_id.to_string(),
                    };
                    format!("{worker_name} (on behalf of {behalf_name})")
                } else {
                    worker_name.clone()
                }
            }
            ActorRef::Automation {
                automation_name,
                triggered_by,
            } => {
                if let Some(trigger) = triggered_by {
                    format!(
                        "{automation_name} (triggered by {})",
                        trigger.display_name()
                    )
                } else {
                    automation_name.clone()
                }
            }
        }
    }

    /// Returns a test helper `ActorRef` for use in tests.
    pub fn test() -> ActorRef {
        ActorRef::System {
            worker_name: "test".into(),
            on_behalf_of: None,
        }
    }
}

impl From<Option<String>> for ActorRef {
    fn from(value: Option<String>) -> Self {
        match value {
            None => ActorRef::System {
                worker_name: "unknown".into(),
                on_behalf_of: None,
            },
            Some(name) => match Actor::parse_name(&name) {
                Ok(actor_id) => ActorRef::Authenticated { actor_id },
                Err(_) => ActorRef::System {
                    worker_name: name,
                    on_behalf_of: None,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_for_user_creates_user_actor() {
        let username = Username::from("octo");
        let (actor, auth_token) = Actor::new_for_user(username.clone());

        assert!(!auth_token.is_empty());
        assert_eq!(actor.actor_id, ActorId::Username(username.clone()));
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
        let (actor, auth_token) = Actor::new_for_task(task_id, Some(Username::from("creator")));
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");

        assert!(actor.verify_auth_token(&parsed));

        let invalid = format!("u-wrong:{}", auth_token.split_once(':').unwrap().1);
        let parsed_invalid = AuthToken::parse(&invalid).expect("auth token should parse");
        assert!(!actor.verify_auth_token(&parsed_invalid));
    }

    // ---- ActorRef tests ----

    #[test]
    fn actor_ref_from_none_produces_system_unknown() {
        let actor_ref = ActorRef::from(None);
        assert_eq!(
            actor_ref,
            ActorRef::System {
                worker_name: "unknown".into(),
                on_behalf_of: None,
            }
        );
    }

    #[test]
    fn actor_ref_from_some_user_name() {
        let actor_ref = ActorRef::from(Some("u-alice".to_string()));
        assert_eq!(
            actor_ref,
            ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("alice")),
            }
        );
    }

    #[test]
    fn actor_ref_from_some_task_name() {
        let task_id = TaskId::from_str("t-abcdef").unwrap();
        let actor_ref = ActorRef::from(Some("w-t-abcdef".to_string()));
        assert_eq!(
            actor_ref,
            ActorRef::Authenticated {
                actor_id: ActorId::Task(task_id),
            }
        );
    }

    #[test]
    fn actor_ref_from_some_unparseable_falls_back_to_system() {
        let actor_ref = ActorRef::from(Some("invalid-name".to_string()));
        assert_eq!(
            actor_ref,
            ActorRef::System {
                worker_name: "invalid-name".into(),
                on_behalf_of: None,
            }
        );
    }

    #[test]
    fn actor_ref_serialization_round_trip_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("bob")),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_system() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Username(Username::from("carol"))),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_serialization_round_trip_automation() {
        let actor_ref = ActorRef::Automation {
            automation_name: "cascade_issue_status".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("dave")),
            })),
        };
        let json = serde_json::to_string(&actor_ref).unwrap();
        let deserialized: ActorRef = serde_json::from_str(&json).unwrap();
        assert_eq!(actor_ref, deserialized);
    }

    #[test]
    fn actor_ref_display_name_authenticated() {
        let actor_ref = ActorRef::Authenticated {
            actor_id: ActorId::Username(Username::from("alice")),
        };
        assert_eq!(actor_ref.display_name(), "alice");
    }

    #[test]
    fn actor_ref_display_name_system_with_on_behalf_of() {
        let actor_ref = ActorRef::System {
            worker_name: "task-spawner".into(),
            on_behalf_of: Some(ActorId::Username(Username::from("bob"))),
        };
        assert_eq!(actor_ref.display_name(), "task-spawner (on behalf of bob)");
    }

    #[test]
    fn actor_ref_display_name_system_without_on_behalf_of() {
        let actor_ref = ActorRef::System {
            worker_name: "background".into(),
            on_behalf_of: None,
        };
        assert_eq!(actor_ref.display_name(), "background");
    }

    #[test]
    fn actor_ref_display_name_automation() {
        let actor_ref = ActorRef::Automation {
            automation_name: "cascade".into(),
            triggered_by: Some(Box::new(ActorRef::Authenticated {
                actor_id: ActorId::Username(Username::from("eve")),
            })),
        };
        assert_eq!(actor_ref.display_name(), "cascade (triggered by eve)");
    }

    #[test]
    fn actor_ref_test_helper() {
        let actor_ref = ActorRef::test();
        assert_eq!(
            actor_ref,
            ActorRef::System {
                worker_name: "test".into(),
                on_behalf_of: None,
            }
        );
    }
}
