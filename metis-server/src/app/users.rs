use crate::{
    domain::{
        actors::{Actor, ActorRef},
        users::{User, UserSummary, Username},
    },
    store::{ReadOnlyStore, StoreError},
};
use metis_common::{TaskId, api::v1 as api};
use octocrab::Octocrab;
use serde::Deserialize;
use thiserror::Error;

use super::app_state::AppState;

const WORKER_NAME_LOGIN: &str = "login";
const WORKER_NAME_TASK_LIFECYCLE: &str = "task_lifecycle";

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
    ) -> Result<api::login::LoginResponse, LoginError> {
        let (user, _actor, login_token) = self
            .create_actor_for_github_token(github_token, github_refresh_token)
            .await?;

        let user_summary: api::users::UserSummary = UserSummary::from(user).into();

        Ok(api::login::LoginResponse::new(login_token, user_summary))
    }

    async fn create_actor_for_github_token(
        &self,
        github_token: String,
        github_refresh_token: String,
    ) -> Result<(User, Actor, String), LoginError> {
        let github_client = Octocrab::builder()
            .base_uri(self.config.github_app.api_base_url().to_string())
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

        let allowed_orgs = &self.config.metis.allowed_orgs;
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
            github_user_id: github_user.id.into_inner(),
            github_token,
            github_refresh_token,
            deleted: false,
        };

        let (actor, auth_token) = Actor::new_for_user(username);

        let login_actor = ActorRef::System {
            worker_name: WORKER_NAME_LOGIN.into(),
            on_behalf_of: None,
        };

        if let Err(err) = self.store.add_user(user.clone(), login_actor.clone()).await {
            match err {
                StoreError::UserAlreadyExists(_) => {
                    self.set_user_github_token(
                        &user.username,
                        user.github_token.clone(),
                        user.github_user_id,
                        user.github_refresh_token.clone(),
                        login_actor.clone(),
                    )
                    .await
                    .map_err(|source| LoginError::Store { source })?;
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        if let Err(err) = self
            .store
            .add_actor(actor.clone(), login_actor.clone())
            .await
        {
            match err {
                StoreError::ActorAlreadyExists(_) => {
                    self.store
                        .update_actor(actor.clone(), login_actor)
                        .await
                        .map_err(|source| LoginError::Store { source })?;
                }
                other => return Err(LoginError::Store { source: other }),
            }
        }

        Ok((user, actor, auth_token))
    }

    pub(crate) async fn create_actor_for_task(
        &self,
        task_id: TaskId,
    ) -> Result<(Actor, String), StoreError> {
        let task = self.get_task(&task_id).await?;
        let (actor, auth_token) = Actor::new_for_task(task_id, task.creator);
        self.store
            .add_actor(
                actor.clone(),
                ActorRef::System {
                    worker_name: WORKER_NAME_TASK_LIFECYCLE.into(),
                    on_behalf_of: None,
                },
            )
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

    pub async fn set_user_github_token(
        &self,
        username: &Username,
        github_token: String,
        github_user_id: u64,
        github_refresh_token: String,
        actor: ActorRef,
    ) -> Result<User, StoreError> {
        let mut user = self.store.get_user(username, false).await?.item;
        user.github_token = github_token;
        user.github_user_id = github_user_id;
        user.github_refresh_token = github_refresh_token;
        self.store
            .update_user(user, actor)
            .await
            .map(|user| user.item)
    }
}

#[cfg(test)]
mod tests {
    use super::LoginError;
    use crate::{
        domain::users::Username,
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
            .login_with_github_token("gh-token".to_string(), "gh-refresh".to_string())
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
            .login_with_github_token("bad-token".to_string(), "gh-refresh".to_string())
            .await
            .expect_err("login should fail for invalid token");

        assert!(matches!(err, LoginError::InvalidGithubToken(_)));
        Ok(())
    }
}
