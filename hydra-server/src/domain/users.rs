use hydra_common::api::v1 as api;
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
    #[serde(default)]
    pub deleted: bool,
}

impl User {
    pub fn new(username: Username, github_user_id: Option<u64>, deleted: bool) -> Self {
        Self {
            username,
            github_user_id,
            deleted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub username: Username,
    pub github_user_id: Option<u64>,
}

impl UserSummary {
    pub fn new(username: Username, github_user_id: Option<u64>) -> Self {
        Self {
            username,
            github_user_id,
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
            deleted: value.deleted,
        }
    }
}

impl From<User> for api::users::User {
    fn from(value: User) -> Self {
        api::users::User::new(value.username.into(), value.github_user_id, value.deleted)
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
        api::users::UserSummary::new(value.username.into(), value.github_user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_converts_between_domain_and_api() {
        let domain = Username::from("hydra");
        let api_value: api::users::Username = domain.clone().into();
        let round_trip: Username = api_value.into();

        assert_eq!(round_trip, domain);
    }
}
