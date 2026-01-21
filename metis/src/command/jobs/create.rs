use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use metis_common::{
    jobs::{BundleSpec, CreateJobRequest, SearchJobsQuery},
    logs::LogsQuery,
    task_status::Status,
    RepoName, TaskId,
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
) -> Result<()> {
    let context = build_context(repo_arg, rev_arg)?;

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
    let request = CreateJobRequest::new(prompt, image, context, variables);
    let response = client.create_job(&request).await?;
    let job_id = response.job_id;

    println!("Requested Metis job {job_id}");

    if wait {
        println!("Streaming logs for job '{job_id}' via metis-server…");
        stream_job_logs_via_server(client, &job_id, true).await?;
        wait_for_job_completion_via_server(client, &job_id).await?;
    }

    Ok(())
}

pub(crate) async fn stream_job_logs_via_server(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
    watch: bool,
) -> Result<()> {
    let query = LogsQuery::new(Some(watch), None);

    let mut log_stream = client
        .get_job_logs(job_id, &query)
        .await
        .with_context(|| format!("failed to stream logs for job '{job_id}'"))?;

    while let Some(line) = log_stream.next().await {
        let line = line?;
        io::stdout().write_all(line.as_bytes())?;
        io::stdout().flush()?;
    }

    Ok(())
}

async fn wait_for_job_completion_via_server(
    client: &dyn MetisClientInterface,
    job_id: &TaskId,
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
                    println!("Job '{job_id}' completed successfully.");
                    return Ok(());
                }
                Status::Failed => {
                    let reason = job
                        .notes
                        .as_deref()
                        .unwrap_or("job failed without an error message");
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
    use crate::client::MockMetisClient;
    use crate::test_utils::ids;
    use chrono::{Duration as ChronoDuration, Utc};
    use metis_common::{
        jobs::{BundleSpec, CreateJobResponse, JobRecord, ListJobsResponse, Task},
        task_status::{Event, Status, TaskStatusLog},
    };
    use std::collections::HashMap;

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
                HashMap::new(),
            ),
            None,
            status_log,
        )
    }

    #[tokio::test]
    async fn spawn_uses_injected_client_and_waits_for_completion() {
        let client = MockMetisClient::default();

        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-123")));
        client.push_log_lines(["first log line\n", "second log line\n"]);
        let start_time = Utc::now();
        client.push_list_jobs_response(ListJobsResponse::new(vec![job_record(
            "t-job-123",
            TaskStatusLog::from_events(vec![
                Event::Created {
                    at: start_time,
                    status: Status::Pending,
                },
                Event::Started { at: start_time },
            ]),
        )]));
        client.push_list_jobs_response(ListJobsResponse::new(vec![job_record(
            "t-job-123",
            TaskStatusLog::from_events(vec![
                Event::Created {
                    at: start_time,
                    status: Status::Pending,
                },
                Event::Started { at: start_time },
                Event::Completed {
                    at: start_time + ChronoDuration::seconds(1),
                    last_message: None,
                },
            ]),
        )]));

        run(
            &client,
            true,
            None,
            None,
            None,
            vec![],
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.prompt, "test prompt");
        assert_eq!(
            request.variables.get("PROMPT"),
            Some(&"test prompt".to_string())
        );
        assert_eq!(request.context, BundleSpec::None);
        assert!(client.create_job_responses.lock().unwrap().is_empty());
        assert!(client.list_jobs_responses.lock().unwrap().is_empty());
        assert!(client.log_responses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn spawn_accepts_service_repository_context() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-service")));

        run(
            &client,
            false,
            Some("dourolabs/service-repo".into()),
            Some("feature".into()),
            None,
            vec![],
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].context,
            BundleSpec::ServiceRepository {
                name: RepoName::from_str("dourolabs/service-repo").unwrap(),
                rev: Some("feature".into())
            }
        );
        assert_eq!(requests[0].prompt, "test prompt");
    }

    #[tokio::test]
    async fn spawn_defaults_rev_to_main_for_service_repositories() {
        let client = MockMetisClient::default();
        client
            .push_create_job_response(CreateJobResponse::new(task_id("t-job-service-default-rev")));

        run(
            &client,
            false,
            Some("dourolabs/service-repo".into()),
            None,
            None,
            vec![],
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].context,
            BundleSpec::ServiceRepository {
                name: RepoName::from_str("dourolabs/service-repo").unwrap(),
                rev: Some("main".into())
            }
        );
    }

    #[tokio::test]
    async fn spawn_accepts_git_repository_context_when_repo_looks_like_url() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-git")));

        run(
            &client,
            false,
            Some("https://example.com/repo.git".into()),
            Some("main".into()),
            None,
            vec![],
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].context,
            BundleSpec::GitRepository {
                url: "https://example.com/repo.git".into(),
                rev: "main".into()
            }
        );
    }

    #[tokio::test]
    async fn spawn_defaults_rev_to_main_for_git_urls() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-git-default-rev")));

        run(
            &client,
            false,
            Some("https://example.com/repo.git".into()),
            None,
            None,
            vec![],
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].context,
            BundleSpec::GitRepository {
                url: "https://example.com/repo.git".into(),
                rev: "main".into()
            }
        );
    }

    #[tokio::test]
    async fn spawn_allows_overriding_image() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-image")));

        run(
            &client,
            false,
            None,
            None,
            Some("ghcr.io/example/metis:dev".into()),
            vec![],
            vec!["custom image".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].image,
            Some("ghcr.io/example/metis:dev".to_string())
        );
        assert_eq!(requests[0].context, BundleSpec::None);
    }

    #[tokio::test]
    async fn spawn_forwards_cli_variables_into_job_request() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse::new(task_id("t-job-with-vars")));

        run(
            &client,
            false,
            None,
            None,
            None,
            vec!["FOO=bar".into(), "PROMPT=from_cli".into()],
            vec!["variable prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        let vars = &requests[0].variables;
        assert_eq!(vars.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(vars.get("PROMPT"), Some(&"variable prompt".to_string()));
    }

    #[tokio::test]
    async fn spawn_requires_prompt() {
        let client = MockMetisClient::default();

        let result = run(&client, false, None, None, None, vec![], vec![]).await;

        assert!(result.is_err());
        assert!(client.recorded_requests().is_empty());
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
