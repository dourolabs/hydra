use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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

impl FromStr for Status {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "created" => Status::Created,
            "pending" => Status::Pending,
            "running" => Status::Running,
            "complete" => Status::Complete,
            "failed" => Status::Failed,
            _ => Status::Unknown,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_str_known_variants() {
        assert_eq!("created".parse::<Status>().unwrap(), Status::Created);
        assert_eq!("pending".parse::<Status>().unwrap(), Status::Pending);
        assert_eq!("running".parse::<Status>().unwrap(), Status::Running);
        assert_eq!("complete".parse::<Status>().unwrap(), Status::Complete);
        assert_eq!("failed".parse::<Status>().unwrap(), Status::Failed);
    }

    #[test]
    fn status_from_str_unknown_fallback() {
        assert_eq!("cancelled".parse::<Status>().unwrap(), Status::Unknown);
        assert_eq!("".parse::<Status>().unwrap(), Status::Unknown);
        assert_eq!("CREATED".parse::<Status>().unwrap(), Status::Unknown);
    }
}
