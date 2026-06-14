use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// A Hydra user identity.
///
/// Newtype around `String` that backs [`crate::principal::Principal::User`]
/// and every other typed user reference. [`Username::try_new`] rejects
/// empty, whitespace-containing, and slash-containing values. The bare
/// [`From<String>`] / [`From<&str>`] conversions remain unchecked for
/// backwards compatibility with pre-migration callers; new code should
/// prefer [`Username::try_new`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export, type = "string"))]
#[serde(transparent)]
#[non_exhaustive]
pub struct Username(String);

/// Validation failure for [`Username::try_new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsernameError {
    Empty,
    ContainsWhitespace,
    ContainsSlash,
}

impl fmt::Display for UsernameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsernameError::Empty => f.write_str("username must not be empty"),
            UsernameError::ContainsWhitespace => {
                f.write_str("username must not contain whitespace")
            }
            UsernameError::ContainsSlash => f.write_str("username must not contain '/'"),
        }
    }
}

impl std::error::Error for UsernameError {}

impl Username {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Validating constructor: rejects empty strings, whitespace, and
    /// `/`. The unchecked [`From<String>`] / [`From<&str>`] conversions
    /// remain available for legacy call sites; this is the validation
    /// entry point preferred by new code.
    pub fn try_new(value: impl Into<String>) -> Result<Self, UsernameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(UsernameError::Empty);
        }
        if value.chars().any(char::is_whitespace) {
            return Err(UsernameError::ContainsWhitespace);
        }
        if value.contains('/') {
            return Err(UsernameError::ContainsSlash);
        }
        Ok(Self(value))
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct User {
    pub username: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_user_id: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
}

impl User {
    pub fn new(username: Username, github_user_id: Option<u64>, archived: bool) -> Self {
        Self {
            username,
            github_user_id,
            archived,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct UserSummary {
    pub username: Username,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

impl UserSummary {
    pub fn new(username: Username, github_user_id: Option<u64>) -> Self {
        Self {
            username,
            github_user_id,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct ListUsersResponse {
    pub users: Vec<UserSummary>,
}

impl ListUsersResponse {
    pub fn new(users: Vec<UserSummary>) -> Self {
        Self { users }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SearchUsersQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub include_archived: Option<bool>,
}

impl SearchUsersQuery {
    pub fn new(q: Option<String>, include_archived: Option<bool>) -> Self {
        Self {
            q,
            include_archived,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_accepts_well_formed_username() {
        let u = Username::try_new("alice").unwrap();
        assert_eq!(u.as_str(), "alice");
    }

    #[test]
    fn try_new_rejects_empty() {
        assert_eq!(Username::try_new(""), Err(UsernameError::Empty));
    }

    #[test]
    fn try_new_rejects_whitespace() {
        assert_eq!(
            Username::try_new("al ice"),
            Err(UsernameError::ContainsWhitespace)
        );
        assert_eq!(
            Username::try_new("\talice"),
            Err(UsernameError::ContainsWhitespace)
        );
    }

    #[test]
    fn try_new_rejects_slash() {
        assert_eq!(
            Username::try_new("users/alice"),
            Err(UsernameError::ContainsSlash)
        );
    }
}
