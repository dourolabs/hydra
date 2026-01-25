use crate::domain::users::UserSummary;
use metis_common::api::v1 as api;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub github_token: String,
    pub github_refresh_token: Option<String>,
}

impl LoginRequest {
    pub fn new(github_token: String, github_refresh_token: Option<String>) -> Self {
        Self {
            github_token,
            github_refresh_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    pub login_token: String,
    pub user: UserSummary,
}

impl LoginResponse {
    pub fn new(login_token: String, user: UserSummary) -> Self {
        Self { login_token, user }
    }
}

impl From<api::login::LoginRequest> for LoginRequest {
    fn from(value: api::login::LoginRequest) -> Self {
        Self {
            github_token: value.github_token,
            github_refresh_token: value.github_refresh_token,
        }
    }
}

impl From<LoginRequest> for api::login::LoginRequest {
    fn from(value: LoginRequest) -> Self {
        api::login::LoginRequest::new(value.github_token, value.github_refresh_token)
    }
}

impl From<api::login::LoginResponse> for LoginResponse {
    fn from(value: api::login::LoginResponse) -> Self {
        Self {
            login_token: value.login_token,
            user: value.user.into(),
        }
    }
}

impl From<LoginResponse> for api::login::LoginResponse {
    fn from(value: LoginResponse) -> Self {
        api::login::LoginResponse::new(value.login_token, value.user.into())
    }
}
