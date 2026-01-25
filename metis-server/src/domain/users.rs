use metis_common::api::v1 as api;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Username {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Username {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<Username> for String {
    fn from(value: Username) -> Self {
        value.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for Username {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Username {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub username: Username,
    pub github_user_id: Option<u64>,
    pub github_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_refresh_token: Option<String>,
}

impl User {
    pub fn new(username: Username, github_token: String) -> Self {
        Self {
            username,
            github_user_id: None,
            github_token,
            github_refresh_token: None,
        }
    }

    pub fn with_github_refresh_token(mut self, github_refresh_token: Option<String>) -> Self {
        self.github_refresh_token = github_refresh_token;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub username: Username,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
}

impl UserSummary {
    pub fn new(username: Username) -> Self {
        Self {
            username,
            github_user_id: None,
        }
    }
}

impl From<User> for UserSummary {
    fn from(user: User) -> Self {
        Self {
            username: user.username,
            github_user_id: user.github_user_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: Username,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
    pub github_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_refresh_token: Option<String>,
}

impl CreateUserRequest {
    pub fn new(username: Username, github_token: String) -> Self {
        Self {
            username,
            github_user_id: None,
            github_token,
            github_refresh_token: None,
        }
    }

    pub fn with_github_refresh_token(mut self, github_refresh_token: Option<String>) -> Self {
        self.github_refresh_token = github_refresh_token;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateGithubTokenRequest {
    pub github_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_refresh_token: Option<String>,
}

impl UpdateGithubTokenRequest {
    pub fn new(github_token: String) -> Self {
        Self {
            github_token,
            github_user_id: None,
            github_refresh_token: None,
        }
    }

    pub fn with_github_refresh_token(mut self, github_refresh_token: Option<String>) -> Self {
        self.github_refresh_token = github_refresh_token;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveUserRequest {
    pub github_token: String,
}

impl ResolveUserRequest {
    pub fn new(github_token: String) -> Self {
        Self { github_token }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveUserResponse {
    pub user: UserSummary,
}

impl ResolveUserResponse {
    pub fn new(user: UserSummary) -> Self {
        Self { user }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertUserResponse {
    pub user: UserSummary,
}

impl UpsertUserResponse {
    pub fn new(user: UserSummary) -> Self {
        Self { user }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteUserResponse {
    pub username: Username,
}

impl DeleteUserResponse {
    pub fn new(username: Username) -> Self {
        Self { username }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListUsersResponse {
    pub users: Vec<UserSummary>,
}

impl ListUsersResponse {
    pub fn new(users: Vec<UserSummary>) -> Self {
        Self { users }
    }
}

impl From<api::users::Username> for Username {
    fn from(value: api::users::Username) -> Self {
        Username(value.into())
    }
}

impl From<Username> for api::users::Username {
    fn from(value: Username) -> Self {
        api::users::Username::from(value.0)
    }
}

impl From<api::users::User> for User {
    fn from(value: api::users::User) -> Self {
        User {
            username: value.username.into(),
            github_user_id: value.github_user_id,
            github_token: value.github_token,
            github_refresh_token: value.github_refresh_token,
        }
    }
}

impl From<User> for api::users::User {
    fn from(value: User) -> Self {
        let mut user = api::users::User::new(value.username.into(), value.github_token)
            .with_github_refresh_token(value.github_refresh_token);
        user.github_user_id = value.github_user_id;
        user
    }
}

impl From<api::users::UserSummary> for UserSummary {
    fn from(value: api::users::UserSummary) -> Self {
        UserSummary {
            username: value.username.into(),
            github_user_id: value.github_user_id,
        }
    }
}

impl From<UserSummary> for api::users::UserSummary {
    fn from(value: UserSummary) -> Self {
        let mut summary = api::users::UserSummary::new(value.username.into());
        summary.github_user_id = value.github_user_id;
        summary
    }
}

impl From<api::users::CreateUserRequest> for CreateUserRequest {
    fn from(value: api::users::CreateUserRequest) -> Self {
        CreateUserRequest {
            username: value.username.into(),
            github_user_id: value.github_user_id,
            github_token: value.github_token,
            github_refresh_token: value.github_refresh_token,
        }
    }
}

impl From<CreateUserRequest> for api::users::CreateUserRequest {
    fn from(value: CreateUserRequest) -> Self {
        api::users::CreateUserRequest::new(value.username.into(), value.github_token)
            .with_github_user_id(value.github_user_id)
            .with_github_refresh_token(value.github_refresh_token)
    }
}

impl From<api::users::UpdateGithubTokenRequest> for UpdateGithubTokenRequest {
    fn from(value: api::users::UpdateGithubTokenRequest) -> Self {
        UpdateGithubTokenRequest {
            github_token: value.github_token,
            github_user_id: value.github_user_id,
            github_refresh_token: value.github_refresh_token,
        }
    }
}

impl From<UpdateGithubTokenRequest> for api::users::UpdateGithubTokenRequest {
    fn from(value: UpdateGithubTokenRequest) -> Self {
        api::users::UpdateGithubTokenRequest::new(value.github_token)
            .with_github_user_id(value.github_user_id)
            .with_github_refresh_token(value.github_refresh_token)
    }
}

impl From<api::users::ResolveUserRequest> for ResolveUserRequest {
    fn from(value: api::users::ResolveUserRequest) -> Self {
        ResolveUserRequest {
            github_token: value.github_token,
        }
    }
}

impl From<ResolveUserRequest> for api::users::ResolveUserRequest {
    fn from(value: ResolveUserRequest) -> Self {
        api::users::ResolveUserRequest::new(value.github_token)
    }
}

impl From<api::users::ResolveUserResponse> for ResolveUserResponse {
    fn from(value: api::users::ResolveUserResponse) -> Self {
        ResolveUserResponse {
            user: value.user.into(),
        }
    }
}

impl From<ResolveUserResponse> for api::users::ResolveUserResponse {
    fn from(value: ResolveUserResponse) -> Self {
        api::users::ResolveUserResponse::new(value.user.into())
    }
}

impl From<api::users::UpsertUserResponse> for UpsertUserResponse {
    fn from(value: api::users::UpsertUserResponse) -> Self {
        UpsertUserResponse {
            user: value.user.into(),
        }
    }
}

impl From<UpsertUserResponse> for api::users::UpsertUserResponse {
    fn from(value: UpsertUserResponse) -> Self {
        api::users::UpsertUserResponse::new(value.user.into())
    }
}

impl From<api::users::DeleteUserResponse> for DeleteUserResponse {
    fn from(value: api::users::DeleteUserResponse) -> Self {
        DeleteUserResponse {
            username: value.username.into(),
        }
    }
}

impl From<DeleteUserResponse> for api::users::DeleteUserResponse {
    fn from(value: DeleteUserResponse) -> Self {
        api::users::DeleteUserResponse::new(value.username.into())
    }
}

impl From<api::users::ListUsersResponse> for ListUsersResponse {
    fn from(value: api::users::ListUsersResponse) -> Self {
        ListUsersResponse {
            users: value.users.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ListUsersResponse> for api::users::ListUsersResponse {
    fn from(value: ListUsersResponse) -> Self {
        api::users::ListUsersResponse::new(value.users.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_converts_between_domain_and_api() {
        let domain = Username::from("metis");
        let api_value: api::users::Username = domain.clone().into();
        let round_trip: Username = api_value.into();

        assert_eq!(round_trip, domain);
    }
}
