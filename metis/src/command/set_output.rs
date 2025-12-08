use crate::client::MetisClientInterface;
use anyhow::{bail, Context, Result};
use metis_common::job_outputs::{JobOutputPayload, JobOutputType};
use std::{fs, path::PathBuf};

pub async fn run(
    client: &dyn MetisClientInterface,
    job: String,
    last_message_file: PathBuf,
    patch_file: PathBuf,
) -> Result<()> {
    let job_id = job.trim();
    if job_id.is_empty() {
        bail!("Job ID must not be empty.");
    }

    let output_type = resolve_output_type(client, job_id).await?;

    match output_type {
        JobOutputType::Patch => {
            let last_message = fs::read_to_string(&last_message_file).with_context(|| {
                format!(
                    "failed to read --last-message file '{}'",
                    last_message_file.display()
                )
            })?;
            let patch = fs::read_to_string(&patch_file).with_context(|| {
                format!("failed to read --patch file '{}'", patch_file.display())
            })?;

            let payload = JobOutputPayload {
                last_message,
                patch,
            };
            println!("Setting output for job '{job_id}' via metis-server…");
            let response = client.set_job_output(job_id, &payload).await?;
            println!(
                "Output set for job '{}' (type: {:?}). Stored last message length: {}, patch length: {}",
                response.job_id,
                response.output_type,
                response.output.last_message.len(),
                response.output.patch.len()
            );
        }
    }
    Ok(())
}


async fn resolve_output_type(client: &dyn MetisClientInterface, job_id: &str) -> Result<JobOutputType> {
    let job = client
        .get_job(job_id)
        .await
        .with_context(|| format!("failed to fetch job '{job_id}'"))?;
    Ok(job.output_type)
}
