use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use metis_common::{jobs::CreateJobRequest, logs::LogsQuery};
use std::{
    io::{self, Write},
    time::Duration,
};
use tokio::time::sleep;

pub async fn run(
    config: &AppConfig,
    wait: bool,
    from_git_rev_arg: Option<String>,
    prompt_parts: Vec<String>,
) -> Result<()> {
    let prompt = if prompt_parts.is_empty() {
        bail!("prompt is required")
    } else {
        prompt_parts.join(" ")
    };

    let client = MetisClient::from_config(config)?;
    let request = CreateJobRequest {
        prompt,
        from_git_rev: from_git_rev_arg,
    };
    let response = client.create_job(&request).await?;
    let job_id = response.job_id;
    let job_name = response.job_name;
    let namespace = response.namespace;

    println!(
        "Requested Metis job '{}' (id {}) in namespace '{}'.",
        job_name, job_id, namespace
    );

    if wait {
        println!("Streaming logs for job '{}' via metis-server…", job_name);
        stream_job_logs_via_server(&client, &job_id, true).await?;
        wait_for_job_completion_via_server(&client, &job_id, &job_name).await?;
    }

    Ok(())
}

pub(crate) async fn stream_job_logs_via_server(
    client: &MetisClient,
    job_id: &str,
    watch: bool,
) -> Result<()> {
    let mut query = LogsQuery::default();
    query.watch = Some(watch);

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
    client: &MetisClient,
    job_id: &str,
    job_name: &str,
) -> Result<()> {
    loop {
        let response = client.list_jobs().await?;
        if let Some(job) = response.jobs.iter().find(|job| job.id == job_id) {
            match job.status.as_str() {
                "complete" => {
                    println!("Job '{}' completed successfully.", job_name);
                    return Ok(());
                }
                "failed" => {
                    bail!("Job '{}' failed.", job_name);
                }
                _ => {}
            }
        }

        sleep(Duration::from_secs(2)).await;
    }
}
