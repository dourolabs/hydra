use crate::{
    MetisId,
    task_status::{Status, TaskError, TaskStatusLog},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum JobStatusUpdate {
    Complete,
    Failed { reason: String },
}

impl JobStatusUpdate {
    pub fn to_result(&self) -> Result<(), TaskError> {
        match self {
            JobStatusUpdate::Complete => Ok(()),
            JobStatusUpdate::Failed { reason } => Err(TaskError::JobEngineError {
                reason: reason.clone(),
            }),
        }
    }

    pub fn as_status(&self) -> Status {
        match self {
            JobStatusUpdate::Complete => Status::Complete,
            JobStatusUpdate::Failed { .. } => Status::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetJobStatusResponse {
    pub job_id: MetisId,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetJobStatusResponse {
    pub job_id: MetisId,
    pub status_log: TaskStatusLog,
}
