use chrono::{DateTime, Utc};
use metis_common::api::v1::task_status as api_task_status;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Created,
    Pending,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskError {
    JobEngineError { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Created {
        at: DateTime<Utc>,
        status: Status,
    },
    Started {
        at: DateTime<Utc>,
    },
    Completed {
        at: DateTime<Utc>,
        #[serde(default)]
        last_message: Option<String>,
    },
    Failed {
        at: DateTime<Utc>,
        error: TaskError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStatusLog {
    #[serde(default)]
    pub events: Vec<Event>,
}

impl TaskStatusLog {
    pub fn new(initial_status: Status, at: DateTime<Utc>) -> Self {
        Self {
            events: vec![Event::Created {
                at,
                status: initial_status,
            }],
        }
    }

    pub fn current_status(&self) -> Status {
        self.events
            .iter()
            .rev()
            .map(|event| match event {
                Event::Created { status, .. } => *status,
                Event::Started { .. } => Status::Running,
                Event::Completed { .. } => Status::Complete,
                Event::Failed { .. } => Status::Failed,
            })
            .next()
            .unwrap_or(Status::Created)
    }

    pub fn creation_time(&self) -> Option<DateTime<Utc>> {
        self.events.iter().find_map(|event| match event {
            Event::Created { at, .. } => Some(*at),
            _ => None,
        })
    }

    pub fn start_time(&self) -> Option<DateTime<Utc>> {
        self.events.iter().find_map(|event| match event {
            Event::Started { at } => Some(*at),
            _ => None,
        })
    }

    pub fn end_time(&self) -> Option<DateTime<Utc>> {
        self.events.iter().rev().find_map(|event| match event {
            Event::Completed { at, .. } | Event::Failed { at, .. } => Some(*at),
            _ => None,
        })
    }

    pub fn result(&self) -> Option<Result<(), TaskError>> {
        self.events.iter().rev().find_map(|event| match event {
            Event::Completed { .. } => Some(Ok(())),
            Event::Failed { error, .. } => Some(Err(error.clone())),
            _ => None,
        })
    }
}

impl From<api_task_status::Status> for Status {
    fn from(value: api_task_status::Status) -> Self {
        match value {
            api_task_status::Status::Created => Status::Created,
            api_task_status::Status::Pending => Status::Pending,
            api_task_status::Status::Running => Status::Running,
            api_task_status::Status::Complete => Status::Complete,
            api_task_status::Status::Failed => Status::Failed,
            other => panic!("unsupported task status variant: {other:?}"),
        }
    }
}

impl From<Status> for api_task_status::Status {
    fn from(value: Status) -> Self {
        match value {
            Status::Created => api_task_status::Status::Created,
            Status::Pending => api_task_status::Status::Pending,
            Status::Running => api_task_status::Status::Running,
            Status::Complete => api_task_status::Status::Complete,
            Status::Failed => api_task_status::Status::Failed,
        }
    }
}

impl From<api_task_status::TaskError> for TaskError {
    fn from(value: api_task_status::TaskError) -> Self {
        match value {
            api_task_status::TaskError::JobEngineError { reason } => {
                TaskError::JobEngineError { reason }
            }
            other => panic!("unsupported task error variant: {other:?}"),
        }
    }
}

impl From<TaskError> for api_task_status::TaskError {
    fn from(value: TaskError) -> Self {
        match value {
            TaskError::JobEngineError { reason } => {
                api_task_status::TaskError::JobEngineError { reason }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn result_returns_last_completion_state() {
        let now = Utc::now();
        let mut log = TaskStatusLog::new(Status::Created, now);
        log.events.push(Event::Started { at: now });
        log.events.push(Event::Failed {
            at: now,
            error: TaskError::JobEngineError {
                reason: "boom".to_string(),
            },
        });

        assert!(matches!(
            log.result(),
            Some(Err(TaskError::JobEngineError { reason })) if reason == "boom"
        ));
    }
}
