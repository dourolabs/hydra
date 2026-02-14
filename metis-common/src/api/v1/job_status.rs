use crate::{
    TaskId,
    task_status::{Status, TaskError},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobStatusUpdate {
    Complete {
        last_message: Option<String>,
    },
    Failed {
        reason: String,
    },
    #[serde(other)]
    Unknown,
}

impl JobStatusUpdate {
    pub fn to_result(&self) -> Result<(), TaskError> {
        match self {
            JobStatusUpdate::Complete { .. } => Ok(()),
            JobStatusUpdate::Failed { reason } => Err(TaskError::JobEngineError {
                reason: reason.clone(),
            }),
            JobStatusUpdate::Unknown => Err(TaskError::Unknown),
        }
    }

    pub fn as_status(&self) -> Status {
        match self {
            JobStatusUpdate::Complete { .. } => Status::Complete,
            JobStatusUpdate::Failed { .. } => Status::Failed,
            JobStatusUpdate::Unknown => Status::Unknown,
        }
    }

    pub fn last_message(&self) -> Option<String> {
        match self {
            JobStatusUpdate::Complete { last_message } => last_message.clone(),
            JobStatusUpdate::Failed { .. } => None,
            JobStatusUpdate::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SetJobStatusResponse {
    pub job_id: TaskId,
    pub status: Status,
}

impl SetJobStatusResponse {
    pub fn new(job_id: TaskId, status: Status) -> Self {
        Self { job_id, status }
    }
}
