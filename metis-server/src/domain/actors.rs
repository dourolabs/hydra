use crate::domain::users::Username;
use crate::store::{Store, StoreError};
use metis_common::TaskId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub auth_token_hash: String,
    pub user_or_worker: UserOrWorker,
}

impl Actor {
    pub fn name(&self) -> String {
        match &self.user_or_worker {
            UserOrWorker::Username(username) => format!("u-{username}"),
            UserOrWorker::Task(task_id) => format!("w-{task_id}"),
        }
    }

    pub fn verify_auth_token(&self, token: &str) -> bool {
        self.auth_token_hash == Self::hash_auth_token(token)
    }

    pub async fn new_for_task(
        task_id: TaskId,
        store: &mut dyn Store,
    ) -> Result<(Actor, String), StoreError> {
        let (auth_token, auth_token_hash) = Self::generate_auth_token();
        let actor = Actor {
            auth_token_hash,
            user_or_worker: UserOrWorker::Task(task_id),
        };
        store.add_actor(actor.clone()).await?;
        Ok((actor, auth_token))
    }

    pub async fn lookup_by_name(store: &dyn Store, name: &str) -> Result<Actor, StoreError> {
        Self::parse_name(name)?;
        store.get_actor(name).await
    }

    fn generate_auth_token() -> (String, String) {
        let token = Uuid::new_v4().to_string();
        let hash = Self::hash_auth_token(&token);
        (token, hash)
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

    fn parse_name(name: &str) -> Result<UserOrWorker, StoreError> {
        if let Some(username) = name.strip_prefix("u-") {
            if username.is_empty() {
                return Err(StoreError::InvalidActorName(name.to_string()));
            }
            return Ok(UserOrWorker::Username(Username::from(username)));
        }

        if let Some(task_id) = name.strip_prefix("w-") {
            if task_id.is_empty() {
                return Err(StoreError::InvalidActorName(name.to_string()));
            }
            let task_id = TaskId::from_str(task_id)
                .map_err(|_| StoreError::InvalidActorName(name.to_string()))?;
            return Ok(UserOrWorker::Task(task_id));
        }

        Err(StoreError::InvalidActorName(name.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserOrWorker {
    Username(Username),
    Task(TaskId),
}
