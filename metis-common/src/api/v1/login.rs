use crate::api::v1::users::UserSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoginRequest {
    pub github_token: String,
    pub github_refresh_token: String,
}

impl LoginRequest {
    pub fn new(github_token: String, github_refresh_token: String) -> Self {
        Self {
            github_token,
            github_refresh_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LoginResponse {
    pub login_token: String,
    pub user: UserSummary,
}

impl LoginResponse {
    pub fn new(login_token: String, user: UserSummary) -> Self {
        Self { login_token, user }
    }
}
