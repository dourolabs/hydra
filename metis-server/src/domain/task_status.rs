use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
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
            .unwrap_or(Status::Pending)
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

impl From<metis_common::task_status::Status> for Status {
    fn from(value: metis_common::task_status::Status) -> Self {
        match value {
            metis_common::task_status::Status::Pending => Status::Pending,
            metis_common::task_status::Status::Running => Status::Running,
            metis_common::task_status::Status::Complete => Status::Complete,
            metis_common::task_status::Status::Failed => Status::Failed,
        }
    }
}

impl From<Status> for metis_common::task_status::Status {
    fn from(value: Status) -> Self {
        match value {
            Status::Pending => metis_common::task_status::Status::Pending,
            Status::Running => metis_common::task_status::Status::Running,
            Status::Complete => metis_common::task_status::Status::Complete,
            Status::Failed => metis_common::task_status::Status::Failed,
        }
    }
}

impl From<metis_common::task_status::TaskError> for TaskError {
    fn from(value: metis_common::task_status::TaskError) -> Self {
        match value {
            metis_common::task_status::TaskError::JobEngineError { reason } => {
                TaskError::JobEngineError { reason }
            }
        }
    }
}

impl From<TaskError> for metis_common::task_status::TaskError {
    fn from(value: TaskError) -> Self {
        match value {
            TaskError::JobEngineError { reason } => {
                metis_common::task_status::TaskError::JobEngineError { reason }
            }
        }
    }
}

impl From<metis_common::task_status::Event> for Event {
    fn from(value: metis_common::task_status::Event) -> Self {
        match value {
            metis_common::task_status::Event::Created { at, status } => Event::Created {
                at,
                status: status.into(),
            },
            metis_common::task_status::Event::Started { at } => Event::Started { at },
            metis_common::task_status::Event::Completed { at, last_message } => {
                Event::Completed { at, last_message }
            }
            metis_common::task_status::Event::Failed { at, error } => Event::Failed {
                at,
                error: error.into(),
            },
        }
    }
}

impl From<Event> for metis_common::task_status::Event {
    fn from(value: Event) -> Self {
        match value {
            Event::Created { at, status } => metis_common::task_status::Event::Created {
                at,
                status: status.into(),
            },
            Event::Started { at } => metis_common::task_status::Event::Started { at },
            Event::Completed { at, last_message } => {
                metis_common::task_status::Event::Completed { at, last_message }
            }
            Event::Failed { at, error } => metis_common::task_status::Event::Failed {
                at,
                error: error.into(),
            },
        }
    }
}

impl From<metis_common::task_status::TaskStatusLog> for TaskStatusLog {
    fn from(value: metis_common::task_status::TaskStatusLog) -> Self {
        Self {
            events: value.events.into_iter().map(Event::from).collect(),
        }
    }
}

impl From<TaskStatusLog> for metis_common::task_status::TaskStatusLog {
    fn from(value: TaskStatusLog) -> Self {
        metis_common::task_status::TaskStatusLog {
            events: value.events.into_iter().map(Into::into).collect(),
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
        let mut log = TaskStatusLog::new(Status::Pending, now);
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
