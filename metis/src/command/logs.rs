use crate::{client::MetisClientInterface, command::spawn::stream_session_logs_via_server};
use anyhow::{bail, Result};

pub async fn run(client: &dyn MetisClientInterface, id: String, watch: bool) -> Result<()> {
    let id = id.trim();
    if id.is_empty() {
        bail!("ID must not be empty.");
    }
    let id = id.to_string();

    let action = if watch { "Streaming" } else { "Fetching" };
    println!("{action} logs for session '{id}' via metis-server…");

    stream_session_logs_via_server(client, &id, watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;

    #[tokio::test]
    async fn logs_streams_session_logs() {
        let client = MockMetisClient::default();
        client.push_log_lines(["session logs\n"]);

        run(&client, "session-xyz".into(), false).await.unwrap();

        assert_eq!(
            client.recorded_log_requests(),
            vec!["session-xyz".to_string()]
        );
    }

    #[tokio::test]
    async fn logs_rejects_empty_id() {
        let client = MockMetisClient::default();
        assert!(run(&client, "   ".into(), false).await.is_err());
    }
}
