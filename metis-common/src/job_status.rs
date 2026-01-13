use crate::{
    TaskId,
    task_status::{Status, TaskError, TaskStatusLog},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum JobStatusUpdate {
    Complete { last_message: Option<String> },
    Failed { reason: String },
}

impl JobStatusUpdate {
    pub fn to_result(&self) -> Result<(), TaskError> {
        match self {
            JobStatusUpdate::Complete { .. } => Ok(()),
            JobStatusUpdate::Failed { reason } => Err(TaskError::JobEngineError {
                reason: reason.clone(),
            }),
        }
    }

    pub fn as_status(&self) -> Status {
        match self {
            JobStatusUpdate::Complete { .. } => Status::Complete,
            JobStatusUpdate::Failed { .. } => Status::Failed,
        }
    }

    pub fn last_message(&self) -> Option<String> {
        match self {
            JobStatusUpdate::Complete { last_message } => last_message.clone(),
            JobStatusUpdate::Failed { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetJobStatusResponse {
    pub job_id: TaskId,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetJobStatusResponse {
    pub job_id: TaskId,
    pub status_log: TaskStatusLog,
}
