use crate::domain::users::{User, Username};
use crate::store::{Store, StoreError};
use metis_common::TaskId;
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub auth_token_hash: String,
    pub user_or_worker: UserOrWorker,
}

impl Actor {
    /// Creates a new user-backed actor from a GitHub token.
    ///
    /// Returns `StoreError::UserAlreadyExists` if the resolved username is already present.
    pub async fn new_for_github_token(
        github_token: String,
        store: &mut dyn Store,
        github_client: &Octocrab,
    ) -> Result<(Actor, String), StoreError> {
        let github_user =
            github_client.current().user().await.map_err(|err| {
                StoreError::Internal(format!("failed to resolve GitHub user: {err}"))
            })?;
        let username = Username::from(github_user.login);
        let user = User {
            username: username.clone(),
            github_user_id: Some(github_user.id.into_inner()),
            github_token,
        };
        store.add_user(user).await?;

        let auth_token = Self::generate_auth_token();
        let actor = Actor {
            auth_token_hash: Self::hash_auth_token(&auth_token),
            user_or_worker: UserOrWorker::Username(username),
        };
        store.add_actor(actor.clone()).await?;
        Ok((actor, auth_token))
    }

    pub fn name(&self) -> String {
        match &self.user_or_worker {
            UserOrWorker::Username(username) => format!("u-{username}"),
            UserOrWorker::Task(task_id) => format!("w-{task_id}"),
        }
    }

    fn generate_auth_token() -> String {
        Uuid::new_v4().to_string()
    }

    fn hash_auth_token(auth_token: &str) -> String {
        let digest = Sha256::digest(auth_token.as_bytes());
        hex::encode(digest)
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
    use crate::store::MemoryStore;
    use crate::store::Store;
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
        let mut store = MemoryStore::new();
        let (actor, auth_token) =
            Actor::new_for_github_token("gh-token".to_string(), &mut store, &github_client)
                .await
                .expect("actor should be created");

        assert!(!auth_token.is_empty());
        assert_eq!(
            actor.user_or_worker,
            UserOrWorker::Username(Username::from("octo"))
        );
        assert_eq!(actor.auth_token_hash, Actor::hash_auth_token(&auth_token));

        let stored_user = store
            .get_user(&Username::from("octo"))
            .await
            .expect("user should exist");
        assert_eq!(stored_user.github_user_id, Some(42));
        assert_eq!(stored_user.github_token, "gh-token");
        let actors = store.list_actors().await.expect("actors should list");
        assert_eq!(actors.len(), 1);
    }

    #[tokio::test]
    async fn new_for_github_token_returns_user_exists_error() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/user");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(github_user_response("octo", 42));
        });

        let github_client = build_github_client(server.base_url());
        let mut store = MemoryStore::new();
        let existing_user = User {
            username: Username::from("octo"),
            github_user_id: Some(42),
            github_token: "existing-token".to_string(),
        };
        store
            .add_user(existing_user)
            .await
            .expect("user should be added");

        let err = Actor::new_for_github_token("gh-token".to_string(), &mut store, &github_client)
            .await
            .expect_err("should fail when user exists");
        assert!(matches!(err, StoreError::UserAlreadyExists(_)));
        let actors = store.list_actors().await.expect("actors should list");
        assert!(actors.is_empty());
    }
}
