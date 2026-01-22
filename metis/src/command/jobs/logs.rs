use super::create::stream_job_logs_via_server;
use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use metis_common::{jobs::SearchJobsQuery, IssueId, MetisId, TaskId};

pub async fn run(client: &dyn MetisClientInterface, id: MetisId, watch: bool) -> Result<()> {
    if let Some(job_id) = id.as_task_id() {
        return stream_logs_for_job(client, job_id, watch).await;
    }

    if let Some(issue_id) = id.as_issue_id() {
        return stream_logs_for_issue(client, issue_id, watch).await;
    }

    bail!("id '{id}' must be a job or issue id");
}

async fn stream_logs_for_job(
    client: &dyn MetisClientInterface,
    id: TaskId,
    watch: bool,
) -> Result<()> {
    let action = if watch { "Streaming" } else { "Fetching" };
    println!("{action} logs for job '{id}' via metis-server…");

    stream_job_logs_via_server(client, &id, watch).await
}

async fn stream_logs_for_issue(
    client: &dyn MetisClientInterface,
    issue_id: IssueId,
    watch: bool,
) -> Result<()> {
    let mut jobs = client
        .list_jobs(&SearchJobsQuery::new(None, Some(issue_id.clone())))
        .await
        .with_context(|| format!("failed to find jobs for issue '{issue_id}'"))?
        .jobs;

    if jobs.is_empty() {
        bail!("no jobs found spawned from issue '{issue_id}'");
    }

    jobs.sort_by(|a, b| {
        let a_time = a.status_log.creation_time();
        let b_time = b.status_log.creation_time();
        b_time.cmp(&a_time)
    });

    let job_ids: Vec<TaskId> = jobs.into_iter().map(|job| job.id).collect();
    let chosen_job = job_ids.first().cloned().unwrap();
    let found_jobs = job_ids
        .iter()
        .map(|job_id| job_id.as_ref())
        .collect::<Vec<_>>()
        .join(", ");

    println!(
        "Looking for jobs spawned from issue '{issue_id}'… found tasks: {found_jobs}. Using most recent job '{chosen_job}' for logs."
    );

    stream_logs_for_job(client, chosen_job, watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MetisClient;
    use crate::test_utils::ids;
    use httpmock::prelude::*;
    use metis_common::{
        jobs::{JobRecord, ListJobsResponse, Task},
        task_status::{Event, Status, TaskStatusLog},
    };
    use reqwest::Client as HttpClient;
    use std::{collections::HashMap, str::FromStr};

    fn task_id(value: &str) -> TaskId {
        ids::task_id(value)
    }

    fn issue_id(value: &str) -> IssueId {
        ids::issue_id(value)
    }

    fn job_record(id: &str, created_at_secs: i64) -> JobRecord {
        JobRecord::new(
            task_id(id),
            Task::new(
                "demo".to_string(),
                metis_common::jobs::BundleSpec::None,
                None,
                None,
                HashMap::new(),
                None,
            ),
            None,
            TaskStatusLog::from_events(vec![Event::Created {
                at: chrono::Utc::now() + chrono::Duration::seconds(created_at_secs),
                status: Status::Pending,
            }]),
        )
    }

    #[tokio::test]
    async fn logs_streams_job_logs() -> Result<()> {
        let server = MockServer::start();
        let job_id = TaskId::from_str("t-jobxyz")?;
        let log_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/jobs/{job_id}/logs"))
                .query_param("watch", "false");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body("data: job logs\n\n");
        });

        let client = MetisClient::with_http_client(server.base_url(), HttpClient::new())?;
        run(&client, job_id.clone().into(), false).await?;

        log_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn logs_prefers_most_recent_job_for_issue() -> Result<()> {
        let server = MockServer::start();
        let issue_id = issue_id("i-issueabc");
        let list_jobs_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/jobs/")
                .query_param("spawned_from", issue_id.as_ref());
            then.status(200).json_body_obj(&ListJobsResponse::new(vec![
                job_record("t-newest", 5),
                job_record("t-older", 0),
            ]));
        });
        let log_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/jobs/t-newest/logs")
                .query_param("watch", "false");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body("data: issue job logs\n\n");
        });

        let client = MetisClient::with_http_client(server.base_url(), HttpClient::new())?;
        run(&client, issue_id.clone().into(), false).await?;

        list_jobs_mock.assert();
        log_mock.assert();
        Ok(())
    }

    #[tokio::test]
    async fn logs_rejects_unexpected_id_type() -> Result<()> {
        let server = MockServer::start();
        let client = MetisClient::with_http_client(server.base_url(), HttpClient::new())?;
        let unexpected_requests = server.mock(|when, then| {
            when.any_request();
            then.status(500);
        });

        let result = run(&client, MetisId::from_str("p-patchzz")?, false).await;

        assert!(result.is_err());
        unexpected_requests.assert_hits(0);
        Ok(())
    }
}
