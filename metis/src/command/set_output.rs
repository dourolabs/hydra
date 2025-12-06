use crate::{client::MetisClient, config::AppConfig};
use anyhow::{bail, Context, Result};
use metis_common::job_outputs::JobOutputPayload;
use std::{fs, path::PathBuf};

pub async fn run(
    config: &AppConfig,
    job: String,
    last_message_file: PathBuf,
    patch_file: PathBuf,
) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }
    let last_message = fs::read_to_string(&last_message_file).with_context(|| {
        format!(
            "failed to read --last-message file '{}'",
            last_message_file.display()
        )
    })?;
    let patch = fs::read_to_string(&patch_file)
        .with_context(|| format!("failed to read --patch file '{}'", patch_file.display()))?;

    let client = MetisClient::from_config(config)?;
    let payload = JobOutputPayload {
        last_message,
        patch,
    };
    println!("Setting output for job '{job_id}' via metis-server…");
    let response = client.set_job_output(job_id, &payload).await?;
    println!(
        "Output set for job '{}'. Stored last message length: {}, patch length: {}",
        response.job_id,
        response.output.last_message.len(),
        response.output.patch.len()
    );
    Ok(())
}
