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
