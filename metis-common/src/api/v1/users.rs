use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct User {
    pub username: Username,
    pub github_token: String,
}

impl User {
    pub fn new(username: Username, github_token: String) -> Self {
        Self {
            username,
            github_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UserSummary {
    pub username: Username,
}

impl From<User> for UserSummary {
    fn from(user: User) -> Self {
        Self {
            username: user.username,
        }
    }
}

impl UserSummary {
    pub fn new(username: Username) -> Self {
        Self { username }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreateUserRequest {
    pub username: Username,
    pub github_token: String,
}

impl CreateUserRequest {
    pub fn new(username: Username, github_token: String) -> Self {
        Self {
            username,
            github_token,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpdateGithubTokenRequest {
    pub github_token: String,
}

impl UpdateGithubTokenRequest {
    pub fn new(github_token: String) -> Self {
        Self { github_token }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UpsertUserResponse {
    pub user: UserSummary,
}

impl UpsertUserResponse {
    pub fn new(user: UserSummary) -> Self {
        Self { user }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeleteUserResponse {
    pub username: Username,
}

impl DeleteUserResponse {
    pub fn new(username: Username) -> Self {
        Self { username }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListUsersResponse {
    pub users: Vec<UserSummary>,
}

impl ListUsersResponse {
    pub fn new(users: Vec<UserSummary>) -> Self {
        Self { users }
    }
}
