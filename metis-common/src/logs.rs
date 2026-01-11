use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub watch: Option<bool>,
    #[serde(default)]
    pub tail_lines: Option<i64>,
}
