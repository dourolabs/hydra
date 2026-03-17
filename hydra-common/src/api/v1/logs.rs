use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[non_exhaustive]
pub struct LogsQuery {
    #[serde(default)]
    pub watch: Option<bool>,
    #[serde(default)]
    pub tail_lines: Option<i64>,
}

impl LogsQuery {
    pub fn new(watch: Option<bool>, tail_lines: Option<i64>) -> Self {
        Self { watch, tail_lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::serialize_query_params;
    use std::collections::HashMap;

    #[test]
    fn logs_query_serializes_with_reqwest() {
        let query = LogsQuery {
            watch: Some(true),
            tail_lines: Some(100),
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("watch").map(String::as_str), Some("true"));
        assert_eq!(params.get("tail_lines").map(String::as_str), Some("100"));
    }

    #[test]
    fn logs_query_serializes_with_false_watch() {
        let query = LogsQuery {
            watch: Some(false),
            tail_lines: None,
        };

        let params = serialize_query_params(&query)
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert_eq!(params.get("watch").map(String::as_str), Some("false"));
        assert!(
            !params.contains_key("tail_lines"),
            "tail_lines should not be serialized when absent"
        );
    }

    #[test]
    fn logs_query_serializes_empty_query() {
        let query = LogsQuery::default();

        let params = serialize_query_params(&query);
        assert!(
            params.is_empty(),
            "expected default LogsQuery to produce no parameters"
        );
    }
}
