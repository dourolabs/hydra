use crate::{
    client::MetisClientInterface,
    command::output::{render_job_summary_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::Result;
use metis_common::{
    jobs::{JobSummaryRecord, SearchJobsQuery},
    IssueId,
};
use std::io::{self, Write};
pub const DEFAULT_JOB_LIMIT: usize = 10;

pub async fn run(
    client: &dyn MetisClientInterface,
    limit: usize,
    spawned_from: Option<IssueId>,
    context: &CommandContext,
) -> Result<()> {
    let response = client
        .list_jobs(&SearchJobsQuery::new(None, spawned_from, None, vec![]))
        .await?;
    let limit = limit.max(1);
    let total_jobs = response.jobs.len();
    let (jobs, truncated) = truncate_jobs(response.jobs, limit);

    let mut buffer = Vec::new();
    render_job_summary_records(context.output_format, &jobs, &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    if truncated && context.output_format == ResolvedOutputFormat::Pretty {
        println!("Showing {limit} of {total_jobs} jobs. Use --limit to display more.");
    }

    Ok(())
}

pub(crate) fn truncate_jobs(
    jobs: Vec<JobSummaryRecord>,
    limit: usize,
) -> (Vec<JobSummaryRecord>, bool) {
    if jobs.len() <= limit {
        return (jobs, false);
    }

    (jobs.into_iter().take(limit).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::MetisClient,
        command::output::{CommandContext, ResolvedOutputFormat},
        test_utils::ids::{issue_id, task_id},
    };
    use chrono::Utc;
    use httpmock::prelude::*;
    use metis_common::jobs::{BundleSpec, JobVersionRecord, ListJobsResponse, Task};
    use metis_common::task_status::Status;
    use metis_common::users::Username;
    use std::collections::HashMap;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn only_spawned_from_query(request: &HttpMockRequest) -> bool {
        match &request.query_params {
            Some(params) => params.len() == 1 && params[0].0 == "spawned_from",
            None => false,
        }
    }

    fn sample_job(id: &str) -> JobSummaryRecord {
        JobSummaryRecord::from(&JobVersionRecord::new(
            task_id(id),
            0,
            Utc::now(),
            Task::new(
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
    fn truncate_jobs_keeps_all_when_below_limit() {
        let jobs = vec![
            sample_job("t-job-1"),
            sample_job("t-job-2"),
            sample_job("t-job-3"),
        ];

        let (kept, truncated) = truncate_jobs(jobs, 5);

        assert!(!truncated);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[0].job_id, task_id("t-job-1"));
        assert_eq!(kept[2].job_id, task_id("t-job-3"));
    }

    #[test]
    fn truncate_jobs_limits_to_requested_count() {
        let jobs: Vec<JobSummaryRecord> = (0..12)
            .map(|idx| sample_job(&format!("t-job-{idx}")))
            .collect();

        let (kept, truncated) = truncate_jobs(jobs, 10);

        assert!(truncated);
        assert_eq!(kept.len(), 10);
        assert_eq!(kept.first().unwrap().job_id, task_id("t-job-0"));
        assert_eq!(kept.last().unwrap().job_id, task_id("t-job-9"));
    }

    #[tokio::test]
    async fn run_passes_spawned_from_query() {
        let spawned_from = issue_id("from-filter");
        let server = MockServer::start();
        let client =
            MetisClient::new(server.base_url(), TEST_METIS_TOKEN).expect("should construct client");

        let list_response = ListJobsResponse::new(vec![sample_job("t-job-1")]);

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/jobs")
                .query_param("spawned_from", spawned_from.as_ref())
                .matches(only_spawned_from_query);
            then.status(200).json_body_obj(&list_response);
        });

        let context = CommandContext::new(ResolvedOutputFormat::Pretty);

        run(&client, 5, Some(spawned_from.clone()), &context)
            .await
            .expect("list jobs should succeed");

        mock.assert();
    }
}
