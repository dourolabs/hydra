use crate::{
    client::HydraClientInterface,
    command::output::{render, CommandContext, ResolvedOutputFormat, SessionSummaryRecords},
    output_writer::write_stdout,
};
use anyhow::Result;
use hydra_common::{
    sessions::{SearchSessionsQuery, SessionSummaryRecord},
    ConversationId, IssueId,
};
pub const DEFAULT_SESSION_LIMIT: usize = 10;

pub async fn run(
    client: &dyn HydraClientInterface,
    limit: usize,
    spawned_from: Option<IssueId>,
    creator: Option<String>,
    conversation: Option<ConversationId>,
    context: &CommandContext,
) -> Result<()> {
    let mut query = SearchSessionsQuery::new(None, spawned_from, None, vec![]);
    query.creator = creator;
    query.conversation_id = conversation;
    let response = client.list_sessions(&query).await?;
    let limit = limit.max(1);
    let total_sessions = response.sessions.len();
    let (sessions, truncated) = truncate_sessions(response.sessions, limit);

    let mut buffer = Vec::new();
    render(
        SessionSummaryRecords(&sessions),
        context.output_format,
        &mut buffer,
    )?;
    write_stdout(&buffer)?;

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
        client::{HydraClient, HydraClientTimeouts},
        command::output::{CommandContext, ResolvedOutputFormat},
        test_utils::ids::{issue_id, task_id},
    };
    use chrono::Utc;
    use httpmock::prelude::*;
    use hydra_common::sessions::{ListSessionsResponse, Session, SessionVersionRecord};
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

    fn test_session_for_list() -> Session {
        use hydra_common::api::v1::sessions::{MountItem, MountSpec, RelativePath, SessionMode};
        Session::new(
            Username::from("test-creator"),
            None,
            None,
            None,
            None,
            None,
            None,
            MountSpec::new(
                RelativePath::new("repo").unwrap(),
                vec![MountItem::Documents {
                    target: RelativePath::new("documents").unwrap(),
                }],
            ),
            None,
            HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless,
            Status::Created,
            None,
            None,
            false,
            None,
            None,
            None,
        )
    }

    fn sample_session(id: &str) -> SessionSummaryRecord {
        SessionSummaryRecord::from(&SessionVersionRecord::new(
            task_id(id),
            0,
            Utc::now(),
            test_session_for_list(),
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
        let client = HydraClient::new(
            server.base_url(),
            TEST_HYDRA_TOKEN,
            &HydraClientTimeouts::default(),
        )
        .expect("should construct client");

        let list_response = ListSessionsResponse::new(vec![sample_session("t-job-1")]);

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions")
                .query_param("spawned_from", spawned_from.as_ref())
                .matches(only_spawned_from_query);
            then.status(200).json_body_obj(&list_response);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);

        run(&client, 5, Some(spawned_from.clone()), None, None, &context)
            .await
            .expect("list sessions should succeed");

        mock.assert();
    }

    fn only_creator_query(request: &HttpMockRequest) -> bool {
        match &request.query_params {
            Some(params) => params.len() == 1 && params[0].0 == "creator",
            None => false,
        }
    }

    #[tokio::test]
    async fn run_passes_creator_query() {
        let server = MockServer::start();
        let client = HydraClient::new(
            server.base_url(),
            TEST_HYDRA_TOKEN,
            &HydraClientTimeouts::default(),
        )
        .expect("should construct client");

        let list_response = ListSessionsResponse::new(vec![sample_session("t-job-1")]);

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions")
                .query_param("creator", "alice")
                .matches(only_creator_query);
            then.status(200).json_body_obj(&list_response);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);

        run(&client, 5, None, Some("alice".to_string()), None, &context)
            .await
            .expect("list sessions should succeed");

        mock.assert();
    }

    fn only_conversation_query(request: &HttpMockRequest) -> bool {
        match &request.query_params {
            Some(params) => params.len() == 1 && params[0].0 == "conversation_id",
            None => false,
        }
    }

    #[tokio::test]
    async fn run_passes_conversation_query() {
        let conversation_id = ConversationId::new();
        let server = MockServer::start();
        let client = HydraClient::new(
            server.base_url(),
            TEST_HYDRA_TOKEN,
            &HydraClientTimeouts::default(),
        )
        .expect("should construct client");

        let list_response = ListSessionsResponse::new(vec![sample_session("t-job-1")]);

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/sessions")
                .query_param("conversation_id", conversation_id.as_ref())
                .matches(only_conversation_query);
            then.status(200).json_body_obj(&list_response);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);

        run(
            &client,
            5,
            None,
            None,
            Some(conversation_id.clone()),
            &context,
        )
        .await
        .expect("list sessions should succeed");

        mock.assert();
    }
}
