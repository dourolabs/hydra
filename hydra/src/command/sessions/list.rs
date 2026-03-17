use crate::{
    client::HydraClientInterface,
    command::output::{render_session_summary_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::Result;
use hydra_common::{
    sessions::{SearchSessionsQuery, SessionSummaryRecord},
    IssueId,
};
use std::io::{self, Write};
pub const DEFAULT_SESSION_LIMIT: usize = 10;

pub async fn run(
    client: &dyn HydraClientInterface,
    limit: usize,
    spawned_from: Option<IssueId>,
    context: &CommandContext,
) -> Result<()> {
    let response = client
        .list_sessions(&SearchSessionsQuery::new(None, spawned_from, None, vec![]))
        .await?;
    let limit = limit.max(1);
    let total_sessions = response.sessions.len();
    let (sessions, truncated) = truncate_sessions(response.sessions, limit);

    let mut buffer = Vec::new();
    render_session_summary_records(context.output_format, &sessions, &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    if truncated && context.output_format == ResolvedOutputFormat::Pretty {
        println!("Showing {limit} of {total_sessions} sessions. Use --limit to display more.");
    }

    Ok(())
}

pub(crate) fn truncate_sessions(
    sessions: Vec<SessionSummaryRecord>,
    limit: usize,
) -> (Vec<SessionSummaryRecord>, bool) {
    if sessions.len() <= limit {
        return (sessions, false);
    }

    (sessions.into_iter().take(limit).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::HydraClient,
        command::output::{CommandContext, ResolvedOutputFormat},
        test_utils::ids::{issue_id, task_id},
    };
    use chrono::Utc;
    use httpmock::prelude::*;
    use hydra_common::sessions::{BundleSpec, ListSessionsResponse, Session, SessionVersionRecord};
    use hydra_common::task_status::Status;
    use hydra_common::users::Username;
    use std::collections::HashMap;

    const TEST_HYDRA_TOKEN: &str = "test-hydra-token";

    fn only_spawned_from_query(request: &HttpMockRequest) -> bool {
        match &request.query_params {
            Some(params) => params.len() == 1 && params[0].0 == "spawned_from",
            None => false,
        }
    }

    fn sample_session(id: &str) -> SessionSummaryRecord {
        SessionSummaryRecord::from(&SessionVersionRecord::new(
            task_id(id),
            0,
            Utc::now(),
            Session::new(
                "0".to_string(),
                BundleSpec::None,
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
        ))
    }

    #[test]
    fn truncate_sessions_keeps_all_when_below_limit() {
        let sessions = vec![
            sample_session("t-job-1"),
            sample_session("t-job-2"),
            sample_session("t-job-3"),
        ];

        let (kept, truncated) = truncate_sessions(sessions, 5);

        assert!(!truncated);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].session_id, task_id("t-job-1"));
        assert_eq!(kept[2].session_id, task_id("t-job-3"));
    }

    #[test]
    fn truncate_sessions_limits_to_requested_count() {
        let sessions: Vec<SessionSummaryRecord> = (0..12)
            .map(|idx| sample_session(&format!("t-job-{idx}")))
            .collect();

        let (kept, truncated) = truncate_sessions(sessions, 10);

        assert!(truncated);
        assert_eq!(kept.len(), 10);
        assert_eq!(kept.first().unwrap().session_id, task_id("t-job-0"));
        assert_eq!(kept.last().unwrap().session_id, task_id("t-job-9"));
    }

    #[tokio::test]
    async fn run_passes_spawned_from_query() {
        let spawned_from = issue_id("from-filter");
        let server = MockServer::start();
        let client =
            HydraClient::new(server.base_url(), TEST_HYDRA_TOKEN).expect("should construct client");

        let list_response = ListSessionsResponse::new(vec![sample_session("t-job-1")]);

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions")
                .query_param("spawned_from", spawned_from.as_ref())
                .matches(only_spawned_from_query);
            then.status(200).json_body_obj(&list_response);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);

        run(&client, 5, Some(spawned_from.clone()), &context)
            .await
            .expect("list sessions should succeed");

        mock.assert();
    }
}
