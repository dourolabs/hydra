use crate::{client::MetisClient, config::AppConfig};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD as Base64Engine;
use base64::Engine;
use futures::StreamExt;
use metis_common::{
    jobs::{CreateJobRequest, CreateJobRequestContext},
    logs::LogsQuery,
};
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
    config: &AppConfig,
    wait: bool,
    from_git_rev_arg: Option<String>,
    repo_url_arg: Option<String>,
    context_dir: Option<PathBuf>,
    force_encode_directory: bool,
    force_encode_git_bundle: bool,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = if prompt_parts.is_empty() {
        bail!("prompt is required")
    } else {
        prompt_parts.join(" ")
    };

    let client = MetisClient::from_config(config)?;
    let context = build_context(
        from_git_rev_arg,
        repo_url_arg,
        context_dir.as_deref(),
        force_encode_directory,
        force_encode_git_bundle,
    )?;
    let request = CreateJobRequest { prompt, context };
    let response = client.create_job(&request).await?;
    let job_id = response.job_id;

    println!("Requested Metis job {job_id}");

    if wait {
        println!("Streaming logs for job '{job_id}' via metis-server…");
        stream_job_logs_via_server(&client, &job_id, true).await?;
        wait_for_job_completion_via_server(&client, &job_id).await?;
    }

    Ok(())
}

pub(crate) async fn stream_job_logs_via_server(
    client: &MetisClient,
    job_id: &str,
    watch: bool,
) -> Result<()> {
    let query = LogsQuery { watch: Some(watch) };

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

async fn wait_for_job_completion_via_server(client: &MetisClient, job_id: &str) -> Result<()> {
    loop {
        let response = client.list_jobs().await?;
        if let Some(job) = response.jobs.iter().find(|job| job.id == job_id) {
            match job.status.as_str() {
                "complete" => {
                    println!("Job '{job_id}' completed successfully.");
                    return Ok(());
                }
                "failed" => {
                    bail!("Job '{job_id}' failed.");
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
    context_dir: Option<&Path>,
    force_encode_directory: bool,
    force_encode_git_bundle: bool,
) -> Result<CreateJobRequestContext> {
    let git_context = match (git_url, git_rev) {
        (Some(url), Some(rev)) => {
            let trimmed_url = url.trim().to_string();
            let trimmed_rev = rev.trim().to_string();
            if trimmed_url.is_empty() || trimmed_rev.is_empty() {
                return Err(anyhow!(
                    "--repo-url and --from must not be empty when provided"
                ));
            }
            Some(CreateJobRequestContext::GitRepository {
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

    let mut resolved_context_dir = context_dir.map(|path| path.to_path_buf());

    if resolved_context_dir.is_none() && git_context.is_none() {
        let cwd = env::current_dir().context("failed to determine current working directory")?;
        resolved_context_dir = Some(cwd);
    }

    if resolved_context_dir.is_none() && (force_encode_directory || force_encode_git_bundle) {
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

    match (dir_context, git_context) {
        (Some(_), Some(_)) => Err(anyhow!(
            "Provide either --context-dir or git context flags, not both"
        )),
        (Some(context), None) => Ok(context),
        (None, Some(context)) => Ok(context),
        (None, None) => Ok(CreateJobRequestContext::None),
    }
}

fn encode_directory(path: &Path) -> Result<CreateJobRequestContext> {
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

    Ok(CreateJobRequestContext::UploadDirectory {
        archive_base64: Base64Engine.encode(archive),
    })
}

fn encode_context_directory(
    path: &Path,
    force_directory: bool,
    force_git_bundle: bool,
) -> Result<CreateJobRequestContext> {
    if force_directory && force_git_bundle {
        bail!("--encode-directory and --encode-git-bundle cannot be used together");
    }

    if force_directory {
        return encode_directory(path);
    }

    if force_git_bundle || is_git_directory(path)? {
        return Ok(CreateJobRequestContext::GitBundle {
            bundle_base64: encode_git_bundle(path)?,
        });
    }

    encode_directory(path)
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
