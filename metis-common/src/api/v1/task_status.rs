use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Status {
    Created,
    Pending,
    Running,
    Complete,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaskError {
    JobEngineError {
        reason: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TaskErrorHelper {
    JobEngineError { reason: String },
}

impl<'de> Deserialize<'de> for TaskError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<TaskErrorHelper>(value) {
            Ok(TaskErrorHelper::JobEngineError { reason }) => {
                Ok(TaskError::JobEngineError { reason })
            }
            Err(_) => Ok(TaskError::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EventHelper {
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

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match serde_json::from_value::<EventHelper>(value) {
            Ok(EventHelper::Created { at, status }) => Ok(Event::Created { at, status }),
            Ok(EventHelper::Started { at }) => Ok(Event::Started { at }),
            Ok(EventHelper::Completed { at, last_message }) => {
                Ok(Event::Completed { at, last_message })
            }
            Ok(EventHelper::Failed { at, error }) => Ok(Event::Failed { at, error }),
            Err(_) => Ok(Event::Unknown),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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

    pub fn from_events(events: Vec<Event>) -> Self {
        Self { events }
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
                Event::Unknown => Status::Unknown,
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
            Event::Unknown | Event::Created { .. } | Event::Started { .. } => None,
        })
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
