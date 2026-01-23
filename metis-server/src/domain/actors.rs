use crate::domain::users::Username;
use metis_common::TaskId;
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserOrWorker {
    Username(Username),
    Task(TaskId),
}
