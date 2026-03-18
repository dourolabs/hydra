use crate::{
    SessionId,
    task_status::{Status, TaskError},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionStatusUpdate {
    Complete {
        last_message: Option<String>,
    },
    Failed {
        reason: String,
    },
    #[serde(other)]
    Unknown,
}

impl SessionStatusUpdate {
    pub fn to_result(&self) -> Result<(), TaskError> {
        match self {
            SessionStatusUpdate::Complete { .. } => Ok(()),
            SessionStatusUpdate::Failed { reason } => Err(TaskError::JobEngineError {
                reason: reason.clone(),
            }),
            SessionStatusUpdate::Unknown => Err(TaskError::Unknown),
        }
    }

    pub fn as_status(&self) -> Status {
        match self {
            SessionStatusUpdate::Complete { .. } => Status::Complete,
            SessionStatusUpdate::Failed { .. } => Status::Failed,
            SessionStatusUpdate::Unknown => Status::Unknown,
        }
    }

    pub fn last_message(&self) -> Option<String> {
        match self {
            SessionStatusUpdate::Complete { last_message } => last_message.clone(),
            SessionStatusUpdate::Failed { .. } => None,
            SessionStatusUpdate::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct SetSessionStatusResponse {
    #[serde(alias = "job_id")]
    pub session_id: SessionId,
    pub status: Status,
}

impl SetSessionStatusResponse {
    pub fn new(session_id: SessionId, status: Status) -> Self {
        Self { session_id, status }
    }
}
