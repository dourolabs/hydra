use super::create::stream_job_logs_via_server;
use crate::client::MetisClientInterface;
use anyhow::Result;
use metis_common::TaskId;

pub async fn run(client: &dyn MetisClientInterface, id: TaskId, watch: bool) -> Result<()> {
    let action = if watch { "Streaming" } else { "Fetching" };
    println!("{action} logs for job '{id}' via metis-server…");

    stream_job_logs_via_server(client, &id, watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use std::str::FromStr;

    #[tokio::test]
    async fn logs_streams_job_logs() {
        let client = MockMetisClient::default();
        client.push_log_lines(["job logs\n"]);

        let job_id = TaskId::from_str("t-jobxyz").unwrap();
        run(&client, job_id.clone(), false).await.unwrap();

        assert_eq!(client.recorded_log_requests(), vec![job_id]);
    }

    #[tokio::test]
    async fn logs_rejects_empty_id() {
        let client = MockMetisClient::default();
        assert!(TaskId::from_str("invalid").is_err());
        drop(client);
    }
}
