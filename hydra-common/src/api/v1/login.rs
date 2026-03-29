use crate::api::v1::users::UserSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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

// --- Device Flow types ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DeviceStartResponse {
    pub device_session_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u32,
    pub interval: u32,
}

impl DeviceStartResponse {
    pub fn new(
        device_session_id: String,
        user_code: String,
        verification_uri: String,
        expires_in: u32,
        interval: u32,
    ) -> Self {
        Self {
            device_session_id,
            user_code,
            verification_uri,
            expires_in,
            interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DevicePollRequest {
    pub device_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct DevicePollResponse {
    pub status: DevicePollStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DevicePollResponse {
    pub fn pending() -> Self {
        Self {
            status: DevicePollStatus::Pending,
            login_token: None,
            user: None,
            error: None,
        }
    }

    pub fn complete(login_token: String, user: UserSummary) -> Self {
        Self {
            status: DevicePollStatus::Complete,
            login_token: Some(login_token),
            user: Some(user),
            error: None,
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            status: DevicePollStatus::Error,
            login_token: None,
            user: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "lowercase")]
pub enum DevicePollStatus {
    Pending,
    Complete,
    Error,
}
