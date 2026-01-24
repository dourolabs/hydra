use crate::api::v1::identity::Username;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoginRequest {
    pub github_token: String,
}

impl LoginRequest {
    pub fn new(github_token: String) -> Self {
        Self { github_token }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoginResponse {
    pub login_token: String,
    pub user: LoginUserSummary,
}

impl LoginResponse {
    pub fn new(login_token: String, user: LoginUserSummary) -> Self {
        Self { login_token, user }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoginUserSummary {
    pub username: Username,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
}

impl LoginUserSummary {
    pub fn new(username: Username) -> Self {
        Self {
            username,
            github_user_id: None,
        }
    }
}
