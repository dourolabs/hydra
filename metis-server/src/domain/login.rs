use crate::domain::users::{UserSummary, Username};
use metis_common::api::v1 as api;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub github_token: String,
}

impl LoginRequest {
    pub fn new(github_token: String) -> Self {
        Self { github_token }
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
        }
    }
}

impl From<LoginRequest> for api::login::LoginRequest {
    fn from(value: LoginRequest) -> Self {
        api::login::LoginRequest::new(value.github_token)
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

impl From<api::login::LoginUserSummary> for UserSummary {
    fn from(value: api::login::LoginUserSummary) -> Self {
        let mut summary = UserSummary::new(Username::from(value.username.as_str()));
        summary.github_user_id = value.github_user_id;
        summary
    }
}

impl From<UserSummary> for api::login::LoginUserSummary {
    fn from(value: UserSummary) -> Self {
        let mut summary = api::login::LoginUserSummary::new(api::identity::Username::from(
            value.username.as_str(),
        ));
        summary.github_user_id = value.github_user_id;
        summary
    }
}
