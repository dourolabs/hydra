use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub watch: Option<bool>,
    #[serde(default)]
    pub tail_lines: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_query_serializes_with_reqwest() {
        let query = LogsQuery {
            watch: Some(true),
            tail_lines: Some(100),
        };

        // Test that reqwest can serialize the query when building the request
        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/jobs/job-id/logs")
            .query(&query)
            .build();
        result.expect("Failed to serialize LogsQuery with reqwest");
    }

    #[test]
    fn logs_query_serializes_with_false_watch() {
        let query = LogsQuery {
            watch: Some(false),
            tail_lines: None,
        };

        // Test that reqwest can serialize the query when building the request
        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/jobs/job-id/logs")
            .query(&query)
            .build();
        result.expect("Failed to serialize LogsQuery with false watch");
    }

    #[test]
    fn logs_query_serializes_empty_query() {
        let query = LogsQuery::default();

        // Test that reqwest can serialize an empty query when building the request
        let client = reqwest::Client::new();
        let result = client
            .get("http://example.com/v1/jobs/job-id/logs")
            .query(&query)
            .build();
        result.expect("Failed to serialize empty LogsQuery");
    }
}
