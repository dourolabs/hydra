use crate::{
    client::MetisClientInterface,
    command::output::{render_job_records, CommandContext, ResolvedOutputFormat},
};
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use metis_common::{
    api::v1::events::{EventsQuery, SseEventType},
    jobs::{BundleSpec, CreateJobRequest, SearchJobsQuery},
    logs::LogsQuery,
    task_status::{Status, TaskError},
    IssueId, RepoName, TaskId,
};
use std::{
    io::{self, Write},
    str::FromStr,
    time::Duration,
};
use tokio::time::sleep;

pub async fn run(
    client: &dyn MetisClientInterface,
    wait: bool,
    repo_arg: Option<String>,
    rev_arg: Option<String>,
    image: Option<String>,
    cli_vars: Vec<String>,
    prompt_parts: Vec<String>,
    issue_id: Option<IssueId>,
    context: &CommandContext,
) -> Result<()> {
    let bundle_context = build_context(repo_arg, rev_arg)?;

    let prompt = if prompt_parts.is_empty() {
        bail!("prompt is required")
    } else {
        prompt_parts.join(" ")
    };

    let mut variables = parse_cli_variables(&cli_vars)?;
    variables.insert("PROMPT".to_string(), prompt.clone());

    let image = match image {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("--image must not be empty when provided");
            }
            Some(trimmed.to_string())
        }
        None => None,
    };
    let request =
        CreateJobRequest::new(prompt, image, bundle_context, variables).with_issue_id(issue_id);
    let response = client.create_job(&request).await?;
    let job_id = response.job_id;

    let job = client.get_job(&job_id).await?;
    let mut buffer = Vec::new();
    render_job_records(context.output_format, &[job], &mut buffer)?;
    io::stdout().write_all(&buffer)?;
    io::stdout().flush()?;

    if wait {
        if context.output_format == ResolvedOutputFormat::Pretty {
            eprintln!("Streaming logs for job '{job_id}' via metis-server…");
        }
        let log_output = match context.output_format {
            ResolvedOutputFormat::Jsonl => LogOutputTarget::Stderr,
            ResolvedOutputFormat::Pretty => LogOutputTarget::Stdout,
        };
        stream_job_logs_via_server(client, &job_id, true, log_output).await?;
        wait_for_job_completion_via_server(client, &job_id, context.output_format).await?;
    }

    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LogOutputTarget {
    Stdout,
    Stderr,
}

pub(crate) async fn stream_job_logs_via_server(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    watch: bool,
    output: LogOutputTarget,
) -> Result<()> {
    let query = LogsQuery::new(Some(watch), None);
    let mut writer: Box<dyn Write + Send> = match output {
        LogOutputTarget::Stdout => Box::new(io::stdout()),
        LogOutputTarget::Stderr => Box::new(io::stderr()),
    };

    let mut log_stream = client
        .get_job_logs(job_id, &query)
        .await
        .with_context(|| format!("failed to stream logs for job '{job_id}'"))?;

    while let Some(line) = log_stream.next().await {
        let line = line?;
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
    }

    Ok(())
}

async fn wait_for_job_completion_via_server(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    // Try SSE first, falling back to polling if unavailable.
    let query = EventsQuery {
        types: Some("jobs".to_string()),
        job_ids: Some(job_id.to_string()),
        ..EventsQuery::default()
    };

    match client.subscribe_events(&query, None).await {
        Ok(Some(mut stream)) => {
            // SSE path: listen for job completion/failure events.
            // Also poll periodically as a safety net.
            let mut poll_tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    maybe_event = stream.next() => {
                        match maybe_event {
                            Some(Ok(event)) => {
                                match event.event_type {
                                    SseEventType::JobUpdated => {
                                        // A job update occurred — check its status.
                                        if let Ok(result) = check_job_status(client, job_id, output_format).await {
                                            return result;
                                        }
                                    }
                                    SseEventType::Resync | SseEventType::Snapshot => {
                                        if let Ok(result) = check_job_status(client, job_id, output_format).await {
                                            return result;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Some(Err(_)) | None => {
                                // Stream error or ended — fall back to polling.
                                break;
                            }
                        }
                    }
                    _ = poll_tick.tick() => {
                        if let Ok(result) = check_job_status(client, job_id, output_format).await {
                            return result;
                        }
                    }
                }
            }
        }
        Ok(None) | Err(_) => {
            // SSE not available — use polling.
        }
    }

    // Polling fallback.
    poll_for_job_completion(client, job_id, output_format).await
}

/// Check if a job has reached a terminal status. Returns `Ok(Ok(()))` for complete,
/// `Ok(Err(...))` for failed, and `Err(())` if the job is still running.
async fn check_job_status(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    output_format: ResolvedOutputFormat,
) -> std::result::Result<Result<()>, ()> {
    let response = match client.list_jobs(&SearchJobsQuery::default()).await {
        Ok(r) => r,
        Err(_) => return Err(()),
    };

    if let Some(job) = response
        .jobs
        .iter()
        .find(|job| job.id.as_ref() == job_id.as_ref())
    {
        match job.status_log.current_status() {
            Status::Complete => {
                if output_format == ResolvedOutputFormat::Pretty {
                    eprintln!("Job '{job_id}' completed successfully.");
                }
                return Ok(Ok(()));
            }
            Status::Failed => {
                let reason = job
                    .task
                    .error
                    .as_ref()
                    .map(|e| match e {
                        TaskError::JobEngineError { reason } => reason.clone(),
                        other => format!("{other:?}"),
                    })
                    .or_else(|| {
                        job.status_log.result().and_then(|r| {
                            r.err().map(|e| match e {
                                TaskError::JobEngineError { reason } => reason,
                                other => format!("{other:?}"),
                            })
                        })
                    })
                    .unwrap_or_else(|| "job failed without an error message".to_string());
                return Ok(Err(anyhow::anyhow!("Job '{job_id}' failed: {reason}")));
            }
            _ => {}
        }
    }

    Err(())
}

async fn poll_for_job_completion(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    output_format: ResolvedOutputFormat,
) -> Result<()> {
    loop {
        let response = client.list_jobs(&SearchJobsQuery::default()).await?;
        if let Some(job) = response
            .jobs
            .iter()
            .find(|job| job.id.as_ref() == job_id.as_ref())
        {
            match job.status_log.current_status() {
                Status::Complete => {
                    if output_format == ResolvedOutputFormat::Pretty {
                        eprintln!("Job '{job_id}' completed successfully.");
                    }
                    return Ok(());
                }
                Status::Failed => {
                    let reason = job
                        .task
                        .error
                        .as_ref()
                        .map(|e| match e {
                            TaskError::JobEngineError { reason } => reason.clone(),
                            other => format!("{other:?}"),
                        })
                        .or_else(|| {
                            job.status_log.result().and_then(|r| {
                                r.err().map(|e| match e {
                                    TaskError::JobEngineError { reason } => reason,
                                    other => format!("{other:?}"),
                                })
                            })
                        })
                        .unwrap_or_else(|| "job failed without an error message".to_string());
                    bail!("Job '{job_id}' failed: {reason}");
                }
                _ => {}
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}

fn build_context(repo: Option<String>, rev: Option<String>) -> Result<BundleSpec> {
    let Some(repo) = repo else {
        if rev.is_some() {
            bail!("--rev requires --repo");
        }
        return Ok(BundleSpec::None);
    };

    let trimmed_repo = repo.trim().to_string();
    if trimmed_repo.is_empty() {
        bail!("--repo must not be empty");
    }

    let trimmed_rev = match rev {
        Some(rev) => {
            let trimmed = rev.trim().to_string();
            if trimmed.is_empty() {
                bail!("--rev must not be empty when provided");
            }
            trimmed
        }
        None => "main".to_string(),
    };

    if looks_like_git_url(&trimmed_repo) {
        return Ok(BundleSpec::GitRepository {
            url: trimmed_repo,
            rev: trimmed_rev,
        });
    }

    let repo_name = RepoName::from_str(&trimmed_repo)
        .with_context(|| format!("invalid service repository name '{trimmed_repo}'"))?;
    Ok(BundleSpec::ServiceRepository {
        name: repo_name,
        rev: Some(trimmed_rev),
    })
}

fn looks_like_git_url(repo: &str) -> bool {
    repo.contains("://") || repo.starts_with("git@") || repo.contains('@') && repo.contains(':')
}

/// Parse CLI variable arguments in KEY=VALUE format.
/// Returns a map of variable names to their values.
fn parse_cli_variables(cli_vars: &[String]) -> Result<std::collections::HashMap<String, String>> {
    let mut vars = std::collections::HashMap::new();

    for var_str in cli_vars {
        let trimmed = var_str.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Find the first = sign
        match trimmed.find('=') {
            Some(pos) if pos > 0 && pos < trimmed.len() - 1 => {
                let key = trimmed[..pos].trim().to_string();
                let value = trimmed[pos + 1..].trim().to_string();

                if key.is_empty() {
                    bail!("Invalid variable format '{trimmed}': variable name cannot be empty");
                }

                // Basic validation: key should be a valid identifier
                if !key
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic() || c == '_')
                    .unwrap_or(false)
                {
                    bail!("Invalid variable name '{key}': must start with a letter or underscore");
                }

                if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    bail!(
                        "Invalid variable name '{key}': must contain only alphanumeric characters and underscores"
                    );
                }

                vars.insert(key, value);
            }
            _ => {
                bail!("Invalid variable format '{trimmed}': expected KEY=VALUE");
            }
        }
    }

    Ok(vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::ids;
    use crate::{
        client::MetisClient,
        command::output::{CommandContext, ResolvedOutputFormat},
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use httpmock::prelude::*;
    use httpmock::Mock;
    use metis_common::{
        jobs::{BundleSpec, CreateJobResponse, JobRecord, ListJobsResponse, Task},
        task_status::{Event, Status, TaskStatusLog},
    };
    use reqwest::Client as HttpClient;
    use std::collections::HashMap;

    const TEST_METIS_TOKEN: &str = "test-metis-token";

    fn test_context() -> CommandContext {
        CommandContext::new(ResolvedOutputFormat::Pretty)
    }

    fn task_id(value: &str) -> TaskId {
        ids::task_id(value)
    }

    fn job_record(id: &str, status_log: TaskStatusLog) -> JobRecord {
        JobRecord::new(
            task_id(id),
            Task::new(
                "0".to_string(),
                BundleSpec::None,
                None,
                None,
                None,
                HashMap::new(),
                None,
                None,
                None,
                false,
            ),
            status_log,
        )
    }

    fn mock_get_job(server: &MockServer, job: JobRecord) -> Mock {
        server.mock(|when, then| {
            when.method(GET).path(format!("/v1/jobs/{}", job.id));
            then.status(200).json_body_obj(&job);
        })
    }

    #[tokio::test]
    async fn spawn_uses_injected_client_and_waits_for_completion() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let job_id = task_id("t-job-123");

        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "test prompt".to_string());
        let create_request = CreateJobRequest::new(
            "test prompt".to_string(),
            None,
            BundleSpec::None,
            variables.clone(),
        );
        let create_mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/jobs")
                .json_body_obj(&create_request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let logs_mock = server.mock(|when, then| {
            when.method(GET)
                .path(format!("/v1/jobs/{job_id}/logs"))
                .query_param("watch", "true");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body("data: first log line\n\ndata: second log line\n\n");
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let start_time = Utc::now();
        let completed_jobs = ListJobsResponse::new(vec![job_record(
            job_id.as_ref(),
            TaskStatusLog::from_events(vec![
                Event::Created {
                    at: start_time,
                    status: Status::Created,
                },
                Event::Started { at: start_time },
                Event::Completed {
                    at: start_time + ChronoDuration::seconds(1),
                    last_message: None,
                },
            ]),
        )]);
        let list_mock = server.mock(|when, then| {
            when.method(GET).path("/v1/jobs/");
            then.status(200).json_body_obj(&completed_jobs);
        });

        let context = test_context();
        run(
            &client,
            true,
            None,
            None,
            None,
            vec![],
            vec!["test prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        logs_mock.assert();
        job_mock.assert();
        list_mock.assert();
    }

    #[tokio::test]
    async fn spawn_accepts_service_repository_context() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "test prompt".to_string());
        let request = CreateJobRequest::new(
            "test prompt".to_string(),
            None,
            BundleSpec::ServiceRepository {
                name: RepoName::from_str("dourolabs/service-repo").unwrap(),
                rev: Some("feature".into()),
            },
            variables,
        );
        let job_id = task_id("t-job-service");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            Some("dourolabs/service-repo".into()),
            Some("feature".into()),
            None,
            vec![],
            vec!["test prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_defaults_rev_to_main_for_service_repositories() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "test prompt".to_string());
        let request = CreateJobRequest::new(
            "test prompt".to_string(),
            None,
            BundleSpec::ServiceRepository {
                name: RepoName::from_str("dourolabs/service-repo").unwrap(),
                rev: Some("main".into()),
            },
            variables,
        );
        let job_id = task_id("t-job-service-default-rev");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            Some("dourolabs/service-repo".into()),
            None,
            None,
            vec![],
            vec!["test prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_accepts_git_repository_context_when_repo_looks_like_url() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "test prompt".to_string());
        let request = CreateJobRequest::new(
            "test prompt".to_string(),
            None,
            BundleSpec::GitRepository {
                url: "https://example.com/repo.git".into(),
                rev: "main".into(),
            },
            variables,
        );
        let job_id = task_id("t-job-git");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            Some("https://example.com/repo.git".into()),
            Some("main".into()),
            None,
            vec![],
            vec!["test prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_defaults_rev_to_main_for_git_urls() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "test prompt".to_string());
        let request = CreateJobRequest::new(
            "test prompt".to_string(),
            None,
            BundleSpec::GitRepository {
                url: "https://example.com/repo.git".into(),
                rev: "main".into(),
            },
            variables,
        );
        let job_id = task_id("t-job-git-default-rev");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            Some("https://example.com/repo.git".into()),
            None,
            None,
            vec![],
            vec!["test prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_allows_overriding_image() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let mut variables = HashMap::new();
        variables.insert("PROMPT".to_string(), "custom image".to_string());
        let request = CreateJobRequest::new(
            "custom image".to_string(),
            Some("ghcr.io/example/metis:dev".to_string()),
            BundleSpec::None,
            variables,
        );
        let job_id = task_id("t-job-image");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            None,
            None,
            Some("ghcr.io/example/metis:dev".into()),
            vec![],
            vec!["custom image".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_forwards_cli_variables_into_job_request() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let request = CreateJobRequest::new(
            "variable prompt".to_string(),
            None,
            BundleSpec::None,
            HashMap::from([
                ("PROMPT".to_string(), "variable prompt".to_string()),
                ("FOO".to_string(), "bar".to_string()),
            ]),
        );
        let job_id = task_id("t-job-with-vars");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs").json_body_obj(&request);
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(job_id.clone()));
        });
        let job_mock = mock_get_job(
            &server,
            job_record(
                job_id.as_ref(),
                TaskStatusLog::new(Status::Created, Utc::now()),
            ),
        );

        let context = test_context();
        run(
            &client,
            false,
            None,
            None,
            None,
            vec!["FOO=bar".into(), "PROMPT=from_cli".into()],
            vec!["variable prompt".into()],
            None,
            &context,
        )
        .await
        .unwrap();

        create_mock.assert();
        job_mock.assert();
    }

    #[tokio::test]
    async fn spawn_requires_prompt() {
        let server = MockServer::start();
        let client =
            MetisClient::with_http_client(server.base_url(), TEST_METIS_TOKEN, HttpClient::new())
                .expect("client");
        let create_mock = server.mock(|when, then| {
            when.method(POST).path("/v1/jobs");
            then.status(200)
                .json_body_obj(&CreateJobResponse::new(task_id("unused")));
        });

        let context = test_context();
        let result = run(
            &client,
            false,
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            &context,
        )
        .await;

        assert!(result.is_err());
        create_mock.assert_hits(0);
    }

    #[test]
    fn test_parse_cli_variables() {
        let vars = vec!["FOO=bar".to_string(), "BAZ=qux".to_string()];
        let result = parse_cli_variables(&vars).unwrap();
        assert_eq!(result.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(result.get("BAZ"), Some(&"qux".to_string()));

        // Test with spaces
        let vars = vec!["FOO=bar qux".to_string()];
        let result = parse_cli_variables(&vars).unwrap();
        assert_eq!(result.get("FOO"), Some(&"bar qux".to_string()));

        // Test invalid formats
        assert!(parse_cli_variables(&["invalid".to_string()]).is_err());
        assert!(parse_cli_variables(&["=value".to_string()]).is_err());
        assert!(parse_cli_variables(&["123KEY=value".to_string()]).is_err());
    }
}
