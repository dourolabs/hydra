use crate::client::MetisClientInterface;
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine as Base64EngineTrait;
use futures::StreamExt;
use metis_common::{
    jobs::{BundleSpec, CreateJobRequest},
    logs::LogsQuery,
    task_status::Status,
};
use rhai::Engine as RhaiEngine;
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tar::Builder;
use tempfile::tempdir;
use tokio::time::sleep;

pub async fn run(
    client: &dyn MetisClientInterface,
    wait: bool,
    from_git_rev_arg: Option<String>,
    repo_url_arg: Option<String>,
    service_repo_arg: Option<String>,
    service_repo_rev_arg: Option<String>,
    context_dir: Option<PathBuf>,
    force_encode_directory: bool,
    force_encode_git_bundle: bool,
    after: Vec<String>,
    cli_vars: Vec<String>,
    program: Option<String>,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let context = build_context(
        from_git_rev_arg,
        repo_url_arg,
        service_repo_arg,
        service_repo_rev_arg,
        context_dir.as_deref(),
        force_encode_directory,
        force_encode_git_bundle,
    )?;

    let prompt = if prompt_parts.is_empty() {
        bail!("prompt is required")
    } else {
        prompt_parts.join(" ")
    };

    let program = if let Some(program_arg) = program {
        Some(load_program(&program_arg)?)
    } else {
        None
    };

    let parent_ids: Vec<String> = after.into_iter().map(|id| id.trim().to_string()).collect();
    if parent_ids.iter().any(|id| id.is_empty()) {
        bail!("--after values must not be empty");
    }

    let mut variables = parse_cli_variables(&cli_vars)?;
    variables.insert("PROMPT".to_string(), prompt.clone());

    let params = vec![prompt.clone()];
    let request = CreateJobRequest {
        prompt,
        program,
        params,
        context,
        parent_ids,
        variables,
    };
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
    job_id: &str,
    watch: bool,
) -> Result<()> {
    let query = LogsQuery {
        watch: Some(watch),
        tail_lines: None,
    };

    let mut log_stream = client
        .get_job_logs(job_id, &query)
        .await
        .with_context(|| format!("failed to stream logs for job '{job_id}'"))?;

    while let Some(line) = log_stream.next().await {
        let line = line?;
        print!("{line}");
        if !line.ends_with('\n') {
            println!();
        }
        io::stdout().flush()?;
    }

    Ok(())
}

async fn wait_for_job_completion_via_server(
    client: &dyn MetisClientInterface,
    job_id: &str,
) -> Result<()> {
    loop {
        let response = client.list_jobs().await?;
        if let Some(job) = response.jobs.iter().find(|job| job.id == job_id) {
            match job.status_log.current_status {
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

fn build_context(
    git_rev: Option<String>,
    git_url: Option<String>,
    service_repo: Option<String>,
    service_repo_rev: Option<String>,
    context_dir: Option<&Path>,
    force_encode_directory: bool,
    force_encode_git_bundle: bool,
) -> Result<BundleSpec> {
    if (force_encode_directory || force_encode_git_bundle) && service_repo.is_some() {
        bail!("--service-repo cannot be combined with context directory encoding options");
    }

    let git_context = match (git_url, git_rev) {
        (Some(url), Some(rev)) => {
            let trimmed_url = url.trim().to_string();
            let trimmed_rev = rev.trim().to_string();
            if trimmed_url.is_empty() || trimmed_rev.is_empty() {
                return Err(anyhow!(
                    "--repo-url and --from must not be empty when provided"
                ));
            }
            Some(BundleSpec::GitRepository {
                url: trimmed_url,
                rev: trimmed_rev,
            })
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(anyhow!(
                "Both --repo-url and --from are required to use a git repository context"
            ))
        }
        (None, None) => None,
    };

    let service_repo_context = match (service_repo, service_repo_rev) {
        (Some(name), rev) => {
            let trimmed_name = name.trim().to_string();
            if trimmed_name.is_empty() {
                bail!("--service-repo must not be empty");
            }

            let trimmed_rev = match rev {
                Some(rev) => {
                    let trimmed = rev.trim().to_string();
                    if trimmed.is_empty() {
                        bail!("--service-repo-rev must not be empty when provided");
                    }
                    Some(trimmed)
                }
                None => None,
            };

            Some(BundleSpec::ServiceRepository {
                name: trimmed_name,
                rev: trimmed_rev,
            })
        }
        (None, Some(_)) => {
            bail!("--service-repo-rev requires --service-repo");
        }
        (None, None) => None,
    };

    if service_repo_context.is_some() && git_context.is_some() {
        bail!("Provide either --service-repo or git context flags, not both");
    }

    if service_repo_context.is_some() && context_dir.is_some() {
        bail!("Provide either --service-repo or --context-dir, not both");
    }

    let mut resolved_context_dir = if service_repo_context.is_some() {
        None
    } else {
        context_dir.map(|path| path.to_path_buf())
    };

    if resolved_context_dir.is_none() && git_context.is_none() && service_repo_context.is_none() {
        let cwd = env::current_dir().context("failed to determine current working directory")?;
        resolved_context_dir = Some(cwd);
    }

    if resolved_context_dir.is_none()
        && service_repo_context.is_none()
        && (force_encode_directory || force_encode_git_bundle)
    {
        bail!("--encode-directory and --encode-git-bundle require --context-dir");
    }

    let dir_context = if let Some(dir) = resolved_context_dir.as_deref() {
        Some(encode_context_directory(
            dir,
            force_encode_directory,
            force_encode_git_bundle,
        )?)
    } else {
        None
    };

    match (dir_context, git_context, service_repo_context) {
        (Some(_), Some(_), _) => Err(anyhow!(
            "Provide either --context-dir or git context flags, not both"
        )),
        (Some(_), _, Some(_)) => Err(anyhow!(
            "Provide either --context-dir or --service-repo, not both"
        )),
        (None, Some(_), Some(_)) => Err(anyhow!(
            "Provide either --service-repo or git context flags, not both"
        )),
        (Some(context), None, None) => Ok(context),
        (None, Some(context), None) => Ok(context),
        (None, None, Some(context)) => Ok(context),
        (None, None, None) => Ok(BundleSpec::None),
    }
}

fn encode_directory(path: &Path) -> Result<BundleSpec> {
    if !path.exists() {
        bail!("Context directory '{}' does not exist", path.display());
    }
    if !path.is_dir() {
        bail!("'{}' is not a directory", path.display());
    }

    let mut archive = Vec::new();
    {
        let mut builder = Builder::new(&mut archive);
        builder
            .append_dir_all(".", path)
            .with_context(|| format!("failed to archive directory '{}'", path.display()))?;
        builder
            .finish()
            .context("failed to finalize context directory archive")?;
    }

    Ok(BundleSpec::TarGz {
        archive_base64: Base64Engine.encode(archive),
    })
}

fn encode_context_directory(
    path: &Path,
    force_directory: bool,
    force_git_bundle: bool,
) -> Result<BundleSpec> {
    if force_directory && force_git_bundle {
        bail!("--encode-directory and --encode-git-bundle cannot be used together");
    }

    if force_directory {
        return encode_directory(path);
    }

    if force_git_bundle || is_git_directory(path)? {
        return Ok(BundleSpec::GitBundle {
            bundle_base64: encode_git_bundle(path)?,
        });
    }

    encode_directory(path)
}

fn load_program(program_arg: &str) -> Result<String> {
    let trimmed = program_arg.trim();
    if trimmed.is_empty() {
        bail!("--program value must not be empty");
    }

    let program_source = if Path::new(trimmed).exists() {
        fs::read_to_string(trimmed)
            .with_context(|| format!("failed to read program file '{}'", trimmed))?
    } else {
        program_arg.to_string()
    };

    validate_program_syntax(&program_source)?;
    Ok(program_source)
}

fn validate_program_syntax(program: &str) -> Result<()> {
    let mut engine = RhaiEngine::new();
    // TODO: can we just max these out here? at minimum these constants need to be shared
    // with the server.
    engine.set_max_expr_depths(256, 256);
    engine.set_max_call_levels(128);
    engine.set_max_operations(50_000);
    engine
        .compile(program)
        .map(|_| ())
        .map_err(|err| anyhow!("invalid Rhai program: {err}"))
}

fn is_git_directory(path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output();

    match output {
        Ok(output) if output.status.success() => Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("true")),
        Ok(_) => Ok(false),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| "failed to check if directory is a git repository"),
    }
}

fn encode_git_bundle(path: &Path) -> Result<String> {
    let tmp_dir = tempdir().context("failed to create temporary directory for git bundle")?;
    let bundle_path = tmp_dir.path().join("context.bundle");

    let status = Command::new("git")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("-C")
        .arg(path)
        .arg("bundle")
        .arg("create")
        .arg("--quiet")
        .arg(&bundle_path)
        .arg("HEAD")
        .status()
        .with_context(|| format!("failed to create git bundle for '{}'", path.display()))?;

    if !status.success() {
        bail!(
            "git bundle create failed for '{}'; ensure there is at least one commit",
            path.display()
        );
    }

    let bundle_bytes = fs::read(&bundle_path)
        .with_context(|| format!("failed to read git bundle '{}'", bundle_path.display()))?;

    Ok(Base64Engine.encode(bundle_bytes))
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
                    bail!(
                        "Invalid variable format '{}': variable name cannot be empty",
                        trimmed
                    );
                }

                // Basic validation: key should be a valid identifier
                if !key
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic() || c == '_')
                    .unwrap_or(false)
                {
                    bail!(
                        "Invalid variable name '{}': must start with a letter or underscore",
                        key
                    );
                }

                if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    bail!(
                        "Invalid variable name '{}': must contain only alphanumeric characters and underscores",
                        key
                    );
                }

                vars.insert(key, value);
            }
            _ => {
                bail!("Invalid variable format '{}': expected KEY=VALUE", trimmed);
            }
        }
    }

    Ok(vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockMetisClient;
    use chrono::{Duration as ChronoDuration, Utc};
    use metis_common::{
        jobs::{BundleSpec, CreateJobResponse, JobSummary, ListJobsResponse},
        task_status::{Status, TaskStatusLog},
    };
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn spawn_uses_injected_client_and_waits_for_completion() {
        let tmp_dir = tempdir().unwrap();
        let client = MockMetisClient::default();

        client.push_create_job_response(CreateJobResponse {
            job_id: "job-123".into(),
        });
        client.push_log_lines(["first log line\n", "second log line\n"]);
        let start_time = Utc::now();
        client.push_list_jobs_response(ListJobsResponse {
            jobs: vec![JobSummary {
                id: "job-123".into(),
                notes: None,
                program: None,
                params: vec![],
                status_log: TaskStatusLog {
                    creation_time: start_time,
                    start_time: Some(start_time),
                    end_time: None,
                    current_status: Status::Running,
                },
            }],
        });
        client.push_list_jobs_response(ListJobsResponse {
            jobs: vec![JobSummary {
                id: "job-123".into(),
                notes: None,
                program: None,
                params: vec![],
                status_log: TaskStatusLog {
                    creation_time: start_time,
                    start_time: Some(start_time),
                    end_time: Some(start_time + ChronoDuration::seconds(1)),
                    current_status: Status::Complete,
                },
            }],
        });

        run(
            &client,
            true,
            None,
            None,
            None,
            None,
            Some(tmp_dir.path().to_path_buf()),
            true,
            false,
            vec![],
            vec![],
            None,
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.prompt, "test prompt");
        assert!(request.program.is_none());
        assert_eq!(request.params, vec!["test prompt".to_string()]);
        assert!(request.parent_ids.is_empty());
        assert_eq!(
            request.variables.get("PROMPT"),
            Some(&"test prompt".to_string())
        );
        assert!(matches!(
            request.context,
            BundleSpec::TarGz { ref archive_base64 } if !archive_base64.is_empty()
        ));
        assert!(client.create_job_responses.lock().unwrap().is_empty());
        assert!(client.list_jobs_responses.lock().unwrap().is_empty());
        assert!(client.log_responses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn spawn_accepts_service_repository_context() {
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse {
            job_id: "job-service".into(),
        });

        run(
            &client,
            false,
            None,
            None,
            Some("service-repo".into()),
            Some("feature".into()),
            None,
            false,
            false,
            vec![],
            vec![],
            None,
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].context,
            BundleSpec::ServiceRepository {
                name: "service-repo".into(),
                rev: Some("feature".into())
            }
        );
        assert_eq!(requests[0].params, vec!["test prompt".to_string()]);
    }

    #[tokio::test]
    async fn spawn_forwards_cli_variables_into_job_request() {
        let tmp_dir = tempdir().unwrap();
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse {
            job_id: "job-with-vars".into(),
        });

        run(
            &client,
            false,
            None,
            None,
            None,
            None,
            Some(tmp_dir.path().to_path_buf()),
            true,
            false,
            vec![],
            vec!["FOO=bar".into(), "PROMPT=from_cli".into()],
            None,
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
    async fn spawn_accepts_inline_program_and_validates() {
        let tmp_dir = tempdir().unwrap();
        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse {
            job_id: "job-inline-program".into(),
        });

        run(
            &client,
            false,
            None,
            None,
            None,
            None,
            Some(tmp_dir.path().to_path_buf()),
            true,
            false,
            vec![],
            vec![],
            Some("let x = 1 + 2;".into()),
            vec!["test prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program.as_deref(), Some("let x = 1 + 2;"));
        assert_eq!(requests[0].params, vec!["test prompt".to_string()]);
    }

    #[tokio::test]
    async fn spawn_reads_program_from_file() {
        let tmp_dir = tempdir().unwrap();
        let program_path = tmp_dir.path().join("script.rhai");
        fs::write(&program_path, "let answer = 42;").unwrap();

        let client = MockMetisClient::default();
        client.push_create_job_response(CreateJobResponse {
            job_id: "job-file-program".into(),
        });

        run(
            &client,
            false,
            None,
            None,
            None,
            None,
            Some(tmp_dir.path().to_path_buf()),
            true,
            false,
            vec![],
            vec![],
            Some(program_path.to_string_lossy().into_owned()),
            vec!["file prompt".into()],
        )
        .await
        .unwrap();

        let requests = client.recorded_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program.as_deref(), Some("let answer = 42;"));
        assert_eq!(requests[0].params, vec!["file prompt".to_string()]);
    }

    #[tokio::test]
    async fn spawn_rejects_invalid_program() {
        let tmp_dir = tempdir().unwrap();
        let client = MockMetisClient::default();

        let result = run(
            &client,
            false,
            None,
            None,
            None,
            None,
            Some(tmp_dir.path().to_path_buf()),
            true,
            false,
            vec![],
            vec![],
            Some("let =".into()),
            vec!["bad prompt".into()],
        )
        .await;

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
        assert!(parse_cli_variables(&vec!["invalid".to_string()]).is_err());
        assert!(parse_cli_variables(&vec!["=value".to_string()]).is_err());
        assert!(parse_cli_variables(&vec!["123KEY=value".to_string()]).is_err());
    }
}
