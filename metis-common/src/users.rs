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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub username: Username,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateGithubTokenRequest {
    pub github_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertUserResponse {
    pub user: UserSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteUserResponse {
    pub username: Username,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListUsersResponse {
    pub users: Vec<UserSummary>,
}
