use crate::MetisId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Blocked,
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
    Emitted {
        at: DateTime<Utc>,
        /// MetisIds for any artifacts produced by the task at this moment.
        artifact_ids: Vec<MetisId>,
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
            .find_map(|event| match event {
                Event::Created { status, .. } => Some(*status),
                Event::Started { .. } => Some(Status::Running),
                Event::Completed { .. } => Some(Status::Complete),
                Event::Failed { .. } => Some(Status::Failed),
                Event::Emitted { .. } => None,
            })
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

    pub fn emitted_artifacts(&self) -> Option<Vec<MetisId>> {
        let mut artifact_ids = Vec::new();

        for event in &self.events {
            if let Event::Emitted {
                artifact_ids: ids, ..
            } = event
            {
                artifact_ids.extend(ids.clone());
            }
        }

        if artifact_ids.is_empty() {
            None
        } else {
            Some(artifact_ids)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn current_status_ignores_emitted_events() {
        let now = Utc::now();
        let mut log = TaskStatusLog::new(Status::Pending, now);
        log.events.push(Event::Started { at: now });
        log.events.push(Event::Emitted {
            at: now,
            artifact_ids: vec!["artifact-1".into(), "artifact-2".into()],
        });

        assert_eq!(log.current_status(), Status::Running);
    }

    #[test]
    fn emitted_artifacts_returns_none_when_missing() {
        let now = Utc::now();
        let log = TaskStatusLog::new(Status::Pending, now);

        assert_eq!(log.emitted_artifacts(), None);
    }

    #[test]
    fn emitted_artifacts_collects_all_in_order() {
        let now = Utc::now();
        let mut log = TaskStatusLog::new(Status::Pending, now);
        log.events.push(Event::Started { at: now });
        log.events.push(Event::Emitted {
            at: now,
            artifact_ids: vec!["artifact-1".into()],
        });
        log.events.push(Event::Emitted {
            at: now,
            artifact_ids: vec!["artifact-2".into(), "artifact-3".into()],
        });

        assert_eq!(
            log.emitted_artifacts(),
            Some(vec![
                "artifact-1".into(),
                "artifact-2".into(),
                "artifact-3".into()
            ])
        );
    }
}
