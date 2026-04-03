use crate::{
    domain::{
        actors::{Actor, ActorRef, store_github_token_secrets},
        users::{User, UserSummary, Username},
    },
    store::{ReadOnlyStore, StoreError},
};
use hydra_common::{SessionId, api::v1 as api};
use octocrab::Octocrab;
use serde::Deserialize;
use thiserror::Error;

use super::app_state::AppState;

pub(crate) const WORKER_NAME_LOGIN: &str = "login";

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("invalid github token: {0}")]
    InvalidGithubToken(String),
    #[error("github user '{username}' is not in an allowed organization")]
    ForbiddenGithubOrg { username: String },
    #[error("login store operation failed")]
    Store {
        #[source]
        source: StoreError,
    },
}

impl AppState {
    pub async fn login_with_github_token(
        &self,
        github_token: String,
        github_refresh_token: String,
        actor: ActorRef,
    ) -> Result<api::login::LoginResponse, LoginError> {
        let (user, _actor, login_token) = self
            .create_actor_for_github_token(github_token, github_refresh_token, actor)
            .await?;

        let user_summary: api::users::UserSummary = UserSummary::from(user).into();

        Ok(api::login::LoginResponse::new(login_token, user_summary))
    }

    async fn create_actor_for_github_token(
        &self,
        github_token: String,
        github_refresh_token: String,
        login_actor: ActorRef,
    ) -> Result<(User, Actor, String), LoginError> {
        let github_client = Octocrab::builder()
            .base_uri(self.config.github_api_base_url().to_string())
            .map_err(|err| LoginError::Store {
                source: StoreError::Internal(format!("failed to parse github api base url: {err}")),
            })?
            .personal_token(github_token.clone())
            .build()
            .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;

        let github_user = github_client
            .current()
            .user()
            .await
            .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;
        let username = Username::from(github_user.login);

        let allowed_orgs = &self.config.hydra.allowed_orgs;
        if !allowed_orgs.is_empty() {
            #[derive(Deserialize)]
            struct GithubOrg {
                login: String,
            }

            let orgs: Vec<GithubOrg> = github_client
                .get("/user/orgs", None::<&()>)
                .await
                .map_err(|err| LoginError::InvalidGithubToken(format!("{err}")))?;

            let is_allowed = orgs.iter().any(|org| {
                allowed_orgs
                    .iter()
                    .any(|allowed| org.login.eq_ignore_ascii_case(allowed))
            });

            if !is_allowed {
                return Err(LoginError::ForbiddenGithubOrg {
                    username: username.to_string(),
                });
            }
        }

        let user = User {
            username: username.clone(),
            github_user_id: Some(github_user.id.into_inner()),
            deleted: false,
        };

        let (actor, auth_token) = Actor::new_for_user(username);

        if let Err(err) = self.store.add_user(user.clone(), login_actor.clone()).await {
            match err {
                StoreError::UserAlreadyExists(_) => {
                    // User already exists — continue to store tokens in
                    // encrypted user_secrets below.
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        // Store tokens in encrypted user_secrets.
        store_github_token_secrets(self, &user.username, &github_token, &github_refresh_token)
            .await;

        if let Err(err) = self
            .store
            .add_actor(actor.clone(), login_actor.clone())
            .await
        {
            match err {
                StoreError::ActorAlreadyExists(_) => {
                    // Actor already exists — don't overwrite the old token.
                    // Just insert the new token into auth_tokens below.
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        // Store the new token hash in the auth_tokens table so multiple
        // devices can be logged in simultaneously.
        let token_hash = Actor::hash_auth_token(
            auth_token
                .strip_prefix(&format!("{}:", actor.name()))
                .expect("auth token should include actor name prefix"),
        );
        self.store
            .add_auth_token(&actor.name(), &token_hash)
            .await
            .map_err(|source| LoginError::Store { source })?;

        Ok((user, actor, auth_token))
    }

    pub(crate) async fn create_actor_for_job(
        &self,
        task_id: SessionId,
        lifecycle_actor: ActorRef,
    ) -> Result<(Actor, String), StoreError> {
        let task = self.get_session(&task_id).await?;
        let creator = task.creator;
        let (actor, auth_token) = if let Some(issue_id) = task.spawned_from {
            Actor::new_for_issue(issue_id, creator)
        } else {
            Actor::new_for_session(task_id, creator)
        };
        if let Err(err) = self
            .store
            .add_actor(actor.clone(), lifecycle_actor.clone())
            .await
        {
            match err {
                StoreError::ActorAlreadyExists(_) => {
                    // Multiple tasks for the same issue share the same ActorId::Issue
                    // but get separate auth tokens. Insert into auth_tokens below.
                }
                other => return Err(other),
            }
        }

        // Store the new token hash in the auth_tokens table so multiple
        // sessions for the same actor can authenticate independently.
        let token_hash = Actor::hash_auth_token(
            auth_token
                .strip_prefix(&format!("{}:", actor.name()))
                .expect("auth token should include actor name prefix"),
        );
        self.store
            .add_auth_token(&actor.name(), &token_hash)
            .await?;

        Ok((actor, auth_token))
    }

    pub async fn get_actor(&self, name: &str) -> Result<Actor, StoreError> {
        let store = self.store.as_ref();
        store.get_actor(name).await.map(|actor| actor.item)
    }

    pub async fn get_user(&self, username: &Username) -> Result<User, StoreError> {
        let store = self.store.as_ref();
        store.get_user(username, false).await.map(|user| user.item)
    }
}

#[cfg(test)]
mod tests {
    use super::LoginError;
    use crate::{
        domain::{actors::ActorRef, users::Username},
        test_utils::{github_user_response, test_state_with_github_api_base_url},
    };
    use httpmock::prelude::*;

    #[tokio::test]
    async fn login_persists_user_and_actor() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let response = handles
            .state
            .login_with_github_token(
                "gh-token".to_string(),
                "gh-refresh".to_string(),
                ActorRef::test(),
            )
            .await
            .expect("login should succeed");

        assert!(!response.login_token.is_empty());
        assert_eq!(response.user.username.as_str(), "octo");

        let store_read = handles.store.as_ref();
        let user = store_read.get_user(&Username::from("octo"), false).await?;
        let actors = store_read.list_actors().await?;
        assert_eq!(actors.len(), 1);
        assert_eq!(user.item.username.as_str(), "octo");

        Ok(())
    }

    #[tokio::test]
    async fn login_returns_error_for_invalid_token() -> anyhow::Result<()> {
        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(401);
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());
        let err = handles
            .state
            .login_with_github_token(
                "bad-token".to_string(),
                "gh-refresh".to_string(),
                ActorRef::test(),
            )
            .await
            .expect_err("login should fail for invalid token");

        assert!(matches!(err, LoginError::InvalidGithubToken(_)));
        Ok(())
    }

    #[tokio::test]
    async fn login_twice_produces_two_valid_tokens() -> anyhow::Result<()> {
        use crate::domain::actors::{Actor, AuthToken};

        let github_server = MockServer::start_async().await;
        let _mock = github_server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let handles = test_state_with_github_api_base_url(github_server.base_url());

        // First login
        let response1 = handles
            .state
            .login_with_github_token(
                "gh-token".to_string(),
                "gh-refresh".to_string(),
                ActorRef::test(),
            )
            .await
            .expect("first login should succeed");

        // Second login
        let response2 = handles
            .state
            .login_with_github_token(
                "gh-token".to_string(),
                "gh-refresh".to_string(),
                ActorRef::test(),
            )
            .await
            .expect("second login should succeed");

        // Both tokens should be different
        assert_ne!(response1.login_token, response2.login_token);

        // Both tokens should be verifiable via auth_tokens table
        let parsed1 = AuthToken::parse(&response1.login_token)?;
        let parsed2 = AuthToken::parse(&response2.login_token)?;

        let store_read = handles.store.as_ref();
        let hashes = store_read
            .get_auth_token_hashes(parsed1.actor_name())
            .await?;
        assert_eq!(hashes.len(), 2);

        let hash1 = Actor::hash_auth_token(parsed1.raw_token());
        let hash2 = Actor::hash_auth_token(parsed2.raw_token());
        assert!(
            hashes.contains(&hash1),
            "first token hash should be in auth_tokens"
        );
        assert!(
            hashes.contains(&hash2),
            "second token hash should be in auth_tokens"
        );

        Ok(())
    }

    #[tokio::test]
    async fn job_actor_tokens_stored_in_auth_tokens() -> anyhow::Result<()> {
        use crate::app::test_helpers::sample_task;
        use crate::domain::actors::{Actor, AuthToken};

        use crate::test_utils::test_state_handles;

        let handles = test_state_handles();

        // Create a session to use for job actor creation
        let (session_id, _) = handles
            .state
            .store
            .add_session_with_actor(sample_task(), chrono::Utc::now(), ActorRef::test())
            .await?;

        // Create a job actor
        let (actor, auth_token) = handles
            .state
            .create_actor_for_job(session_id, ActorRef::test())
            .await?;

        // Verify token is in auth_tokens
        let parsed = AuthToken::parse(&auth_token)?;
        let store_read = handles.store.as_ref();
        let hashes = store_read.get_auth_token_hashes(&actor.name()).await?;
        let token_hash = Actor::hash_auth_token(parsed.raw_token());
        assert!(
            hashes.contains(&token_hash),
            "job actor token should be in auth_tokens"
        );

        Ok(())
    }
}
