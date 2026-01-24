use crate::domain::users::{User, Username};
use metis_common::TaskId;
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ActorError {
    #[error("GitHub user lookup failed: {0}")]
    GithubLookupFailed(String),
    #[error("Invalid actor name: {0}")]
    InvalidActorName(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub auth_token_hash: String,
    #[serde(default = "default_auth_token_salt")]
    pub auth_token_salt: String,
    pub user_or_worker: UserOrWorker,
}

impl Actor {
    /// Creates a new user-backed actor from a GitHub token.
    pub async fn new_for_github_token(
        github_token: String,
    ) -> Result<(User, Actor, String), ActorError> {
        let github_client = Octocrab::builder()
            .personal_token(github_token.clone())
            .build()
            .map_err(|err| ActorError::GithubLookupFailed(format!("{err}")))?;
        Self::new_for_github_token_with_client(github_token, &github_client).await
    }

    pub(crate) async fn new_for_github_token_with_client(
        github_token: String,
        github_client: &Octocrab,
    ) -> Result<(User, Actor, String), ActorError> {
        let github_user = github_client
            .current()
            .user()
            .await
            .map_err(|err| ActorError::GithubLookupFailed(format!("{err}")))?;
        let username = Username::from(github_user.login);
        let user = User {
            username: username.clone(),
            github_user_id: Some(github_user.id.into_inner()),
            github_token,
        };

        let (auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            user_or_worker: UserOrWorker::Username(username),
        };
        Ok((user, actor, auth_token))
    }

    pub fn name(&self) -> String {
        match &self.user_or_worker {
            UserOrWorker::Username(username) => format!("u-{username}"),
            UserOrWorker::Task(task_id) => format!("w-{task_id}"),
        }
    }

    pub fn verify_auth_token(&self, token: &str) -> bool {
        self.auth_token_hash == Self::hash_auth_token(token)
    }

    pub fn new_for_task(task_id: TaskId) -> (Actor, String) {
        let (auth_token, auth_token_hash, auth_token_salt) = Self::generate_auth_token();
        let actor = Actor {
            auth_token_hash,
            auth_token_salt,
            user_or_worker: UserOrWorker::Task(task_id),
        };
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

fn default_auth_token_salt() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserOrWorker {
    Username(Username),
    Task(TaskId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn github_user_response(login: &str, id: u64) -> serde_json::Value {
        json!({
            "login": login,
            "id": id,
            "node_id": "NODEID",
            "avatar_url": "https://example.com/avatar",
            "gravatar_id": "gravatar",
            "url": "https://example.com/user",
            "html_url": "https://example.com/user",
            "followers_url": "https://example.com/followers",
            "following_url": "https://example.com/following",
            "gists_url": "https://example.com/gists",
            "starred_url": "https://example.com/starred",
            "subscriptions_url": "https://example.com/subscriptions",
            "organizations_url": "https://example.com/orgs",
            "repos_url": "https://example.com/repos",
            "events_url": "https://example.com/events",
            "received_events_url": "https://example.com/received_events",
            "type": "User",
            "site_admin": false,
            "name": null,
            "patch_url": null,
            "email": null
        })
    }

    fn build_github_client(base_url: String) -> Octocrab {
        Octocrab::builder()
            .base_uri(base_url)
            .unwrap()
            .personal_token("gh-token".to_string())
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn new_for_github_token_creates_user_and_actor() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let github_client = build_github_client(server.base_url());
        let (user, actor, auth_token) =
            Actor::new_for_github_token_with_client("gh-token".to_string(), &github_client)
                .await
                .expect("actor should be created");

        assert!(!auth_token.is_empty());
        assert_eq!(user.username, Username::from("octo"));
        assert_eq!(user.github_user_id, Some(42));
        assert_eq!(user.github_token, "gh-token");
        assert_eq!(
            actor.user_or_worker,
            UserOrWorker::Username(Username::from("octo"))
        );
        assert!(!actor.auth_token_salt.is_empty());
        assert_eq!(actor.auth_token_hash, Actor::hash_auth_token(&auth_token));
    }

    #[tokio::test]
    async fn new_for_github_token_returns_error_on_github_failure() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(500);
        });

        let github_client = build_github_client(server.base_url());
        let err = Actor::new_for_github_token_with_client("gh-token".to_string(), &github_client)
            .await
            .expect_err("should fail when GitHub lookup fails");
        assert!(matches!(err, ActorError::GithubLookupFailed(_)));
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
}
