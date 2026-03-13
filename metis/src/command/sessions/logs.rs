use super::create::{stream_session_logs_via_server, LogOutputTarget};
use crate::{
    client::MetisClientInterface,
    command::output::{CommandContext, ResolvedOutputFormat},
};
use anyhow::{bail, Context, Result};
use metis_common::{sessions::SearchSessionsQuery, IssueId, MetisId, SessionId};

pub async fn run(
    client: &dyn MetisClientInterface,
    id: MetisId,
    watch: bool,
    context: &CommandContext,
) -> Result<()> {
    if let Some(session_id) = id.as_session_id() {
        return stream_logs_for_session(client, session_id, watch, context.output_format).await;
    }

    if let Some(issue_id) = id.as_issue_id() {
        return stream_logs_for_issue(client, issue_id, watch, context.output_format).await;
    }

    bail!("id '{id}' must be a session or issue id");
}

async fn stream_logs_for_session(
    client: &dyn MetisClientInterface,
    id: SessionId,
    watch: bool,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let action = if watch { "Streaming" } else { "Fetching" };
    if output_format == ResolvedOutputFormat::Pretty {
        eprintln!("{action} logs for session '{id}' via metis-server…");
    }

    stream_session_logs_via_server(client, &id, watch, LogOutputTarget::Stdout).await
}

async fn stream_logs_for_issue(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
    watch: bool,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    let sessions = client
        .list_sessions(&SearchSessionsQuery::new(
            None,
            Some(issue_id.clone()),
            None,
            vec![],
        ))
        .await
        .with_context(|| format!("failed to find sessions for issue '{issue_id}'"))?
        .sessions;

    if sessions.is_empty() {
        bail!("no sessions found spawned from issue '{issue_id}'");
    }

    // Sessions are returned from the server sorted by most recent activity,
    // so the first session is the most recently updated one.
    let session_ids: Vec<SessionId> = sessions.into_iter().map(|s| s.session_id).collect();
    let chosen_session = session_ids.first().cloned().unwrap();
    let found_sessions = session_ids
        .iter()
        .map(|session_id| session_id.as_ref())
        .collect::<Vec<_>>()
        .join(", ");

    if output_format == ResolvedOutputFormat::Pretty {
        eprintln!(
            "Looking for sessions spawned from issue '{issue_id}'… found tasks: {found_sessions}. Using most recent session '{chosen_session}' for logs."
        );
    }

    stream_logs_for_session(client, chosen_session, watch, output_format).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::ids;
    use crate::{
        client::MetisClient,
        command::output::{CommandContext, ResolvedOutputFormat},
    };
    use chrono::Utc;
    use httpmock::prelude::*;
    use metis_common::sessions::{
        ListSessionsResponse, Session, SessionSummaryRecord, SessionVersionRecord,
    };
    use metis_common::task_status::Status;
    use metis_common::users::Username;
    use reqwest::Client as HttpClient;
    use std::{collections::HashMap, str::FromStr};

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn task_id(value: &str) -> SessionId {
        ids::task_id(value)
    }

    fn issue_id(value: &str) -> IssueId {
        ids::issue_id(value)
    }

    fn session_record(id: &str) -> SessionVersionRecord {
        SessionVersionRecord::new(
            task_id(id),
            0,
            Utc::now(),
            Session::new(
                "demo".to_string(),
                metis_common::sessions::BundleSpec::None,
                None,
                Username::from("test-creator"),
                None,
                None,
                HashMap::new(),
                None,
                None,
                None,
                Status::Created,
                None,
                None,
                false,
                None,
                None,
                None,
            ),
            None,
        )
    }

    #[tokio::test]
    async fn logs_streams_session_logs() -> Result<()> {
        let server = MockServer::start();
        let session_id = SessionId::from_str("s-jobxyz")?;
        let log_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/sessions/{session_id}/logs"))
                .query_param("watch", "false");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body("data: session logs\n\n");
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        run(&client, session_id.clone().into(), false, &context).await?;

        log_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn logs_prefers_most_recent_session_for_issue() -> Result<()> {
        let server = MockServer::start();
        let issue_id = issue_id("i-issueabc");
        let list_sessions_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions")
                .query_param("spawned_from", issue_id.as_ref());
            then.status(200)
                .json_body_obj(&ListSessionsResponse::new(vec![
                    SessionSummaryRecord::from(&session_record("s-newest")),
                    SessionSummaryRecord::from(&session_record("s-older")),
                ]));
        });
        let log_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions/s-newest/logs")
                .query_param("watch", "false");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body("data: issue session logs\n\n");
        });

        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        run(&client, issue_id.clone().into(), false, &context).await?;

        list_sessions_mock.assert();
        log_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn logs_rejects_unexpected_id_type() -> Result<()> {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())?;
        let unexpected_requests = server.mock(|when, then| {
            when.any_request();
            then.status(500);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);
        let result = run(&client, MetisId::from_str("p-patchzz")?, false, &context).await;

        assert!(result.is_err());
        unexpected_requests.assert_hits(0);
        Ok(())
    }
}
