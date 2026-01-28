use crate::domain::users::Username;
use metis_common::TaskId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::str::FromStr;
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
    pub user_or_worker: UserOrWorker,
}

impl Actor {
    pub fn new_for_user(username: Username) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let user_or_worker = UserOrWorker::Username(username);
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            user_or_worker,
        };
        let auth_token = Self::format_auth_token(&actor, &raw_auth_token);
        (actor, auth_token)
    }

    pub fn name(&self) -> String {
        match &self.user_or_worker {
            UserOrWorker::Username(username) => format!("u-{username}"),
            UserOrWorker::Task(task_id) => format!("w-{task_id}"),
        }
    }

    pub fn verify_auth_token(&self, token: &AuthToken) -> bool {
        if token.actor_name() != self.name() {
            return false;
        }
        self.auth_token_hash == Self::hash_auth_token(token.raw_token())
    }

    pub fn new_for_task(task_id: TaskId) -> (Actor, String) {
        let (raw_auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let user_or_worker = UserOrWorker::Task(task_id);
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            user_or_worker,
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

    pub fn parse_name(name: &str) -> Result<UserOrWorker, ActorError> {
        if let Some(username) = name.strip_prefix("u-") {
            if username.is_empty() {
                return Err(ActorError::InvalidActorName(name.to_string()));
            }
            return Ok(UserOrWorker::Username(Username::from(username)));
        }

        if let Some(task_id) = name.strip_prefix("w-") {
            if task_id.is_empty() {
                return Err(ActorError::InvalidActorName(name.to_string()));
            }
            let task_id = TaskId::from_str(task_id)
                .map_err(|_| ActorError::InvalidActorName(name.to_string()))?;
            return Ok(UserOrWorker::Task(task_id));
        }

        Err(ActorError::InvalidActorName(name.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserOrWorker {
    Username(Username),
    Task(TaskId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_for_user_creates_user_actor() {
        let username = Username::from("octo");
        let (actor, auth_token) = Actor::new_for_user(username.clone());

        assert!(!auth_token.is_empty());
        assert_eq!(
            actor.user_or_worker,
            UserOrWorker::Username(username.clone())
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
        let (actor, auth_token) = Actor::new_for_task(task_id);
        let parsed = AuthToken::parse(&auth_token).expect("auth token should parse");

        assert!(actor.verify_auth_token(&parsed));

        let invalid = format!("u-wrong:{}", auth_token.split_once(':').unwrap().1);
        let parsed_invalid = AuthToken::parse(&invalid).expect("auth token should parse");
        assert!(!actor.verify_auth_token(&parsed_invalid));
    }
}
